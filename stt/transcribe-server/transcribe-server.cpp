// transcribe-server: a minimal HTTP wrapper around transcribe.cpp that mimics whisper.cpp's
// `whisper-server` — POST /inference (multipart: file=@wav [+ language, translate]) -> {"text": "..."}.
// The model is loaded ONCE at startup and held hot (unlike transcribe-cli, which reloads per call), so it
// drops in behind our capturer's existing HTTP call with zero client changes. Idle-unload is handled by the
// parent (server.js) killing us, exactly as it does whisper-server.
#include "transcribe.h"
#include "transcribe/whisper.h"
#include "httplib.h"
#include <cstdio>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <mutex>
#include <string>
#include <vector>
#include <algorithm>
#include <cerrno>
#include <chrono>
#include <csignal>
#include <thread>
#include <unistd.h>

// Our capturer writes a fixed 16 kHz mono s16 WAV (writeWav16). Scan for the "data" chunk and expand to
// float PCM — robust to header size without pulling in a general WAV parser.
static bool parse_wav16(const std::string &b, std::vector<float> &out) {
    if (b.size() < 12 || b.compare(0, 4, "RIFF") != 0 || b.compare(8, 4, "WAVE") != 0) return false;
    size_t i = 12;
    while (i + 8 <= b.size()) {
        uint32_t sz = (uint8_t)b[i+4] | ((uint8_t)b[i+5] << 8) | ((uint8_t)b[i+6] << 16) | ((uint32_t)(uint8_t)b[i+7] << 24);
        if (b.compare(i, 4, "data") == 0) {
            size_t start = i + 8, n = std::min((size_t)sz, b.size() - start), ns = n / 2;
            out.resize(ns);
            for (size_t k = 0; k < ns; k++) {
                int16_t s = (int16_t)((uint8_t)b[start + 2*k] | ((uint16_t)(uint8_t)b[start + 2*k + 1] << 8));
                out[k] = s / 32768.0f;
            }
            return true;
        }
        i += 8 + sz + (sz & 1);
    }
    return false;
}

static std::string json_escape(const std::string &s) {
    std::string o;
    for (size_t i = 0; i < s.size();) {
        const unsigned char c = static_cast<unsigned char>(s[i]);
        if (c >= 0x80) {
            size_t width = 0;
            if (c >= 0xC2 && c <= 0xDF) width = 2;
            else if (c >= 0xE0 && c <= 0xEF) width = 3;
            else if (c >= 0xF0 && c <= 0xF4) width = 4;
            bool valid = width != 0 && i + width <= s.size();
            for (size_t j = 1; valid && j < width; ++j) {
                const unsigned char continuation = static_cast<unsigned char>(s[i + j]);
                valid = continuation >= 0x80 && continuation <= 0xBF;
            }
            if (valid && width == 3) {
                const unsigned char second = static_cast<unsigned char>(s[i + 1]);
                valid = !((c == 0xE0 && second < 0xA0) || (c == 0xED && second > 0x9F));
            } else if (valid && width == 4) {
                const unsigned char second = static_cast<unsigned char>(s[i + 1]);
                valid = !((c == 0xF0 && second < 0x90) || (c == 0xF4 && second > 0x8F));
            }
            if (valid) {
                o.append(s, i, width);
                i += width;
            } else {
                o += "\xEF\xBF\xBD";
                ++i;
            }
            continue;
        }
        if (c == '"' || c == '\\') { o += '\\'; o += c; }
        else if (c == '\n' || c == '\r' || c == '\t') o += ' ';
        else if (c < 0x20) { /* drop */ }
        else o += c;
        ++i;
    }
    return o;
}

static bool selftest_json_escape() {
    const std::string valid = "है \"ready\"";
    if (json_escape(valid) != "है \\\"ready\\\"") return false;
    std::string invalid = "partial";
    invalid.push_back(static_cast<char>(0xE0));
    return json_escape(invalid) == "partial\xEF\xBF\xBD";
}

static struct transcribe_model   *g_model = nullptr;
static struct transcribe_session *g_ctx   = nullptr;
static std::mutex g_mtx;   // transcribe_session is single-result; serialize runs

// The GUI normally stops this child explicitly. This watchdog also handles crashes, forced upgrades, and
// test-run replacement: once the owning GUI PID disappears, the model server releases its port and memory.
static void start_parent_watchdog() {
    const char *raw = getenv("ASSISTANT_PARENT_PID");
    if (!raw || !*raw) return;
    char *end = nullptr;
    long parsed = strtol(raw, &end, 10);
    if (!end || *end != '\0' || parsed <= 1) return;
    const pid_t parent = static_cast<pid_t>(parsed);
    std::thread([parent] {
        for (;;) {
            std::this_thread::sleep_for(std::chrono::milliseconds(500));
            if (kill(parent, 0) == -1 && errno == ESRCH) std::_Exit(0);
        }
    }).detach();
}

int main(int argc, char **argv) {
    if (argc == 2 && std::strcmp(argv[1], "--selftest") == 0) {
        if (!selftest_json_escape()) {
            fprintf(stderr, "transcribe-server self-test failed\n");
            return 1;
        }
        fprintf(stderr, "transcribe-server self-test passed\n");
        return 0;
    }
    start_parent_watchdog();
    const char *token_env = getenv("ASSISTANT_STT_TOKEN");
    std::string model_path, host = "127.0.0.1", token = token_env ? token_env : "";
    int port = 8091;
    for (int i = 1; i < argc; i++) {
        std::string a = argv[i];
        if      (a == "-m"     && i + 1 < argc) model_path = argv[++i];
        else if (a == "--host" && i + 1 < argc) host = argv[++i];
        else if (a == "--port" && i + 1 < argc) port = atoi(argv[++i]);
        else if (a == "--token" && i + 1 < argc) token = argv[++i];
        // -t / other whisper-server flags are accepted and ignored for CLI compatibility
        else if (a == "-t"     && i + 1 < argc) ++i;
    }
    if (model_path.empty() || token.empty()) {
        fprintf(stderr, "usage: ASSISTANT_STT_TOKEN=SECRET transcribe-server -m model.gguf [--host H] [--port P]\n");
        return 2;
    }

    transcribe_init_backends_default();
    struct transcribe_model_load_params mp; transcribe_model_load_params_init(&mp);
    if (transcribe_model_load_file(model_path.c_str(), &mp, &g_model) != TRANSCRIBE_OK) {
        fprintf(stderr, "transcribe-server: model load failed: %s\n", model_path.c_str()); return 1;
    }
    struct transcribe_session_params sp; transcribe_session_params_init(&sp);
    if (transcribe_session_init(g_model, &sp, &g_ctx) != TRANSCRIBE_OK) {
        fprintf(stderr, "transcribe-server: session init failed\n"); return 1;
    }

    httplib::Server srv;
    srv.set_payload_max_length(64ULL * 1024ULL * 1024ULL);
    auto authorized = [&token](const httplib::Request &req, httplib::Response &res) {
        if (req.get_header_value("Authorization") != "Bearer " + token) {
            res.status = 401;
            res.set_content("unauthorized", "text/plain");
            return false;
        }
        return true;
    };
    srv.Get("/", [authorized](const httplib::Request &req, httplib::Response &res) mutable {
        if (!authorized(req, res)) return;
        res.set_content("ok", "text/plain");
    });
    srv.Post("/inference", [authorized](const httplib::Request &req, httplib::Response &res) mutable {
        if (!authorized(req, res)) return;
        auto file = req.get_file_value("file");
        std::vector<float> pcm;
        if (file.content.empty() || !parse_wav16(file.content, pcm) || pcm.empty()) {
            res.set_content("{\"text\":\"\"}", "application/json"); return;
        }
        std::string lang = req.has_file("language") ? req.get_file_value("language").content : "auto";
        bool translate   = req.has_file("translate") && req.get_file_value("translate").content == "true";
        std::string prompt = req.has_file("prompt") ? req.get_file_value("prompt").content : "";  // custom vocabulary / bias

        std::lock_guard<std::mutex> lk(g_mtx);
        struct transcribe_run_params rp; transcribe_run_params_init(&rp);
        if (translate) rp.task = TRANSCRIBE_TASK_TRANSLATE;
        if (!lang.empty() && lang != "auto") rp.language = lang.c_str();  // else auto-detect

        // Anti-hallucination (replaces Silero VAD): the fluent garbage on noise/silence comes from Whisper's
        // TEMPERATURE FALLBACK escalating (0 → 0.2 → 0.4 …) when a greedy decode looks "bad". Pin temperature=0
        // with fallback OFF (temperature_inc=0) so non-speech decodes to nothing instead of a confident lie,
        // and make the built-in no-speech gate a bit stricter. Clean speech is unaffected.
        struct transcribe_whisper_run_ext wx; transcribe_whisper_run_ext_init(&wx);
        wx.temperature = 0.0f;
        wx.temperature_inc = 0.0f;        // no fallback → no fluent hallucination on noise
        wx.no_speech_thold = 0.5f;        // (default 0.6) slightly stricter silence rejection
        if (!prompt.empty()) wx.initial_prompt = prompt.c_str();  // bias decoding toward the user's vocabulary (lib copies the bytes)
        if (transcribe_model_accepts_ext_kind(g_model, TRANSCRIBE_EXT_SLOT_RUN, TRANSCRIBE_EXT_KIND_WHISPER_RUN))
            rp.family = &wx.ext;

        std::string text;
        const transcribe_status run_status = transcribe_run(g_ctx, pcm.data(), (int)pcm.size(), &rp);
        const bool truncated = run_status == TRANSCRIBE_ERR_OUTPUT_TRUNCATED;
        if (run_status == TRANSCRIBE_OK || truncated) {
            const char *t = transcribe_full_text(g_ctx);
            if (t) text = t;
        } else {
            fprintf(stderr, "transcribe-server: inference failed: %s\n", transcribe_status_string(run_status));
        }
        // trim leading space Whisper often prefixes
        size_t b = text.find_first_not_of(' ');
        if (b == std::string::npos) text.clear(); else text = text.substr(b);
        res.set_content("{\"text\":\"" + json_escape(text) + "\",\"truncated\":" +
                        (truncated ? "true" : "false") + "}", "application/json");
    });

    fprintf(stderr, "transcribe-server ready on %s:%d (model held hot)\n", host.c_str(), port);
    srv.listen(host.c_str(), port);
    return 0;
}
