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
    for (char c : s) {
        if (c == '"' || c == '\\') { o += '\\'; o += c; }
        else if (c == '\n' || c == '\r' || c == '\t') o += ' ';
        else if ((unsigned char)c < 0x20) { /* drop */ }
        else o += c;
    }
    return o;
}

static struct transcribe_model   *g_model = nullptr;
static struct transcribe_session *g_ctx   = nullptr;
static std::mutex g_mtx;   // transcribe_session is single-result; serialize runs

int main(int argc, char **argv) {
    std::string model_path, host = "127.0.0.1";
    int port = 8091;
    for (int i = 1; i < argc; i++) {
        std::string a = argv[i];
        if      (a == "-m"     && i + 1 < argc) model_path = argv[++i];
        else if (a == "--host" && i + 1 < argc) host = argv[++i];
        else if (a == "--port" && i + 1 < argc) port = atoi(argv[++i]);
        // -t / other whisper-server flags are accepted and ignored for CLI compatibility
        else if (a == "-t"     && i + 1 < argc) ++i;
    }
    if (model_path.empty()) { fprintf(stderr, "usage: transcribe-server -m model.gguf [--host H] [--port P]\n"); return 2; }

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
    srv.Get("/", [](const httplib::Request &, httplib::Response &res) { res.set_content("ok", "text/plain"); });
    srv.Post("/inference", [](const httplib::Request &req, httplib::Response &res) {
        auto file = req.get_file_value("file");
        std::vector<float> pcm;
        if (file.content.empty() || !parse_wav16(file.content, pcm) || pcm.empty()) {
            res.set_content("{\"text\":\"\"}", "application/json"); return;
        }
        std::string lang = req.has_file("language") ? req.get_file_value("language").content : "auto";
        bool translate   = req.has_file("translate") && req.get_file_value("translate").content == "true";

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
        if (transcribe_model_accepts_ext_kind(g_model, TRANSCRIBE_EXT_SLOT_RUN, TRANSCRIBE_EXT_KIND_WHISPER_RUN))
            rp.family = &wx.ext;

        std::string text;
        if (transcribe_run(g_ctx, pcm.data(), (int)pcm.size(), &rp) == TRANSCRIBE_OK) {
            const char *t = transcribe_full_text(g_ctx);
            if (t) text = t;
        }
        // trim leading space Whisper often prefixes
        size_t b = text.find_first_not_of(' ');
        if (b == std::string::npos) text.clear(); else text = text.substr(b);
        res.set_content("{\"text\":\"" + json_escape(text) + "\"}", "application/json");
    });

    fprintf(stderr, "transcribe-server ready on %s:%d (model held hot)\n", host.c_str(), port);
    srv.listen(host.c_str(), port);
    return 0;
}
