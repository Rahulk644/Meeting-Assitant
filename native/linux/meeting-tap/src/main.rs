//! meeting-tap (Linux) — system+mic audio capture → whisper → caption POST, plus optional video.
//! PROTOCOL.md §1.
//!
//! REAL / verifiable (the spine): arg parsing (positionals + env-over-flag tokens), the 2 s config
//! pull, the `manual`/`autoDetect`/`videoMode`/`sharing` capture-gate logic, the segmenter → WAV →
//! whisper → caption/state pipeline (`platform::control` + `helpers_core`), meeting-state reporting
//! with `source:"coreaudio"`, detection helpers + `--selftest` (`detect.rs`), `--list-apps`, SIGTERM
//! finalize, and parent-death exit.
//!
//! TODO(on-device): the Pulse capture (needs a live server + default-monitor resolution — `audio.rs`),
//! per-app PipeWire scoping (feature `pipewire-scope`), call detection for `--app auto|<id>`, and the
//! GStreamer/portal video path (`video.rs`). Each degrades gracefully and logs — e.g. `--app all`
//! reports/records the global monitor; `--app auto` currently reports "not in a call" until detection
//! is wired.

mod audio;
mod detect;
mod video;

use helpers_core::args::Args;
use helpers_core::protocol::{
    AudioState, AudioStateEvent, Caption, CaptionTag, PulledConfig, RecordingStarted,
};
use helpers_core::segmenter::{Segmenter, DEFAULT_MAX_SEG, DEFAULT_MIC_RMS, DEFAULT_SPEECH_RMS};
use helpers_core::wav::encode_wav16;
use helpers_core::watchdog::{is_alive, parent_pid_from_env};
use helpers_core::epoch_ms;
use platform::control::ControlPlane;
use platform::display::{self, DisplayServer};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use video::{Recorder, VideoMode};

/// Immutable, per-invocation config (from argv + env).
struct StartupCfg {
    gui_port: String,
    whisper_url: String,
    gui_token: String,
    stt_token: String,
    app_sel: String,
    mic_on: bool,
    speech_rms: f32,
    mic_rms: f32,
    min_conf: f64,
    expressions: bool,
    #[allow(dead_code)]
    gate: bool,
    video_mode0: VideoMode,
    rec_dir: PathBuf,
    temp_dir: PathBuf,
    max_seg: f64,
    me: String,
}

/// Mutable config pulled from `/api/state` every 2 s.
#[derive(Clone)]
struct Runtime {
    module_meetings: bool,
    sharing: bool,
    video_mode: VideoMode,
    translate: bool,
    language: String,
    // `manual` is a string tri-state on the wire ("on"/"auto"/"off"/"paused"); track the two
    // decisions it drives ("on" forces capture, "off"/"paused" forces stop, else auto).
    manual_on: bool,
    manual_off: bool,
    auto_detect: bool,
    #[allow(dead_code)]
    browser_meet: bool,
    vocab: String,
}

fn main() {
    let args = Args::from_env();

    if args.has("selftest") {
        let (ok, msg) = detect::selftest();
        if ok {
            eprintln!("SELFTEST OK");
            std::process::exit(0);
        } else {
            eprintln!("SELFTEST FAIL: {msg}");
            std::process::exit(1);
        }
    }
    if args.has("list-apps") {
        audio::list_apps();
        std::process::exit(0);
    }

    let cfg = parse_cfg(&args);
    run(cfg);
}

fn parse_cfg(args: &Args) -> StartupCfg {
    let gui_port = args.positional(0).unwrap_or_else(|| "8787".to_string());
    let whisper_url = args
        .positional(1)
        .unwrap_or_else(|| "http://127.0.0.1:8091/inference".to_string());

    let me_default = platform::full_user_name();
    StartupCfg {
        gui_port,
        whisper_url,
        gui_token: args.opt_env("ASSISTANT_GUI_TOKEN", "gui-token", ""),
        stt_token: args.opt_env("ASSISTANT_STT_TOKEN", "stt-token", ""),
        app_sel: args.opt("app", "auto"),
        mic_on: args.opt("mic", "on") != "off",
        speech_rms: args.opt("rms", "0.006").parse().unwrap_or(DEFAULT_SPEECH_RMS),
        mic_rms: args.opt("mic-rms", "0.02").parse().unwrap_or(DEFAULT_MIC_RMS),
        min_conf: args.opt("min-conf", "0.5").parse().unwrap_or(0.5),
        expressions: args.has("expressions"),
        gate: args.opt("gate", "on") != "off",
        video_mode0: VideoMode::parse(&args.opt("video", "off")),
        rec_dir: PathBuf::from(args.opt("rec-dir", "data/meetings/rec")),
        temp_dir: args
            .get("temp-dir")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir),
        max_seg: args.opt("max-seg", "25").parse().unwrap_or(DEFAULT_MAX_SEG),
        me: args.opt("me", &me_default),
    }
}

fn run(cfg: StartupCfg) {
    let server = display::detect();
    let audio_server = platform::audio::detect();
    eprintln!(
        ">> meeting-tap ready — display:{server} audio:{audio_server} gui:{} whisper:{} app:{} mic:{} me:{:?}",
        cfg.gui_port, cfg.whisper_url, cfg.app_sel, cfg.mic_on, cfg.me
    );
    if cfg.app_sel != "all" && !audio_server.supports_native_app_scoping() {
        eprintln!(
            ">> per-app scoping requested (--app {}) but {} has no native node-link path; \
             detection/scoping will degrade to the global monitor once wired",
            cfg.app_sel, audio_server
        );
    }
    let _ = std::fs::create_dir_all(&cfg.temp_dir);

    let cp = Arc::new(ControlPlane::new(&cfg.gui_port, cfg.gui_token.clone()));
    let recorder = Arc::new(Mutex::new(Recorder::new(cfg.rec_dir.clone())));
    let runtime = Arc::new(Mutex::new(Runtime {
        module_meetings: false,
        sharing: false,
        video_mode: cfg.video_mode0,
        translate: false,
        language: "auto".to_string(),
        manual_on: false,
        manual_off: false,
        auto_detect: true,
        browser_meet: true,
        vocab: String::new(),
    }));
    let running = Arc::new(AtomicBool::new(true));
    let capturing = Arc::new(AtomicBool::new(false));

    // --- audio capture threads (TODO on-device: need a live Pulse server) ---
    let (tx, rx) = mpsc::channel::<audio::Captured>();
    let sys_seg = Segmenter::new("Speaker", false, "sys", cfg.speech_rms).with_max_seg(cfg.max_seg);
    audio::spawn_capture(audio::default_monitor_device(), sys_seg, tx.clone(), running.clone());
    if cfg.mic_on {
        let mic_seg = Segmenter::new(cfg.me.clone(), true, "mic", cfg.mic_rms).with_max_seg(cfg.max_seg);
        audio::spawn_capture(None, mic_seg, tx.clone(), running.clone());
    }
    drop(tx);

    // --- utterance processor thread ---
    spawn_processor(rx, cp.clone(), runtime.clone(), capturing.clone(), &cfg);

    // --- signal handling: SIGTERM/SIGINT → finalize mp4, exit after grace ---
    install_signal_handler(recorder.clone(), running.clone());

    // --- config + gate loop (main thread) ---
    let parent = parent_pid_from_env();
    let mut gate = GateState::default();
    loop {
        if let Some(pid) = parent {
            if !is_alive(pid) {
                eprintln!(">> parent app exited — releasing capture");
                shutdown(&recorder, &running);
            }
        }
        if let Some(pc) = cp.pull_config() {
            update_runtime(&runtime, pc, cfg.video_mode0);
        }
        apply_capture_gate(&cfg, server, &runtime, &recorder, &capturing, &cp, &mut gate);
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn update_runtime(runtime: &Mutex<Runtime>, pc: PulledConfig, fallback_video: VideoMode) {
    // Compute the manual tri-state before `pc` is partially moved by the field reads below.
    let manual_on = pc.manual_force_on();
    let manual_off = pc.manual_force_off();
    let mut r = runtime.lock().unwrap();
    r.module_meetings = pc.modules.map(|m| m.meetings).unwrap_or(false);
    r.sharing = pc.sharing;
    r.video_mode = pc.video_mode.as_deref().map(VideoMode::parse).unwrap_or(fallback_video);
    r.translate = pc.translate;
    r.language = pc.language.unwrap_or_else(|| "auto".to_string());
    r.manual_on = manual_on;
    r.manual_off = manual_off;
    r.auto_detect = pc.auto_detect;
    r.browser_meet = pc.browser_meet;
    r.vocab = pc.vocab.unwrap_or_default();
}

#[derive(Default)]
struct GateState {
    was_in_call: bool,
    was_key: String,
    was_title: String,
}

/// The core gating decision (Swift `applyCaptureGate` + `gate`). Reports state on change and
/// starts/stops capture + recording.
fn apply_capture_gate(
    cfg: &StartupCfg,
    server: DisplayServer,
    runtime: &Mutex<Runtime>,
    recorder: &Mutex<Recorder>,
    capturing: &AtomicBool,
    cp: &ControlPlane,
    gate: &mut GateState,
) {
    let rt = runtime.lock().unwrap().clone();
    recorder.lock().unwrap().update_share(rt.sharing);

    // Meetings module off → force "not in a call".
    if !rt.module_meetings {
        report_detected(false, "", "", cp, gate);
        capturing.store(false, Ordering::Relaxed);
        recorder.lock().unwrap().stop();
        return;
    }

    let target = capture_target(&cfg.app_sel, &rt);
    let detected = target.is_some();
    let (key, title) = target.unwrap_or_default();
    report_detected(detected, &key, &title, cp, gate);

    // manual "on" forces capture; "off"/"paused" forces stop; else auto requires autoDetect + a call.
    let capture = rt.manual_on || (!rt.manual_off && rt.auto_detect && detected);
    if capture {
        capturing.store(true, Ordering::Relaxed);
        if let Some((file, started_at)) =
            recorder.lock().unwrap().start(rt.video_mode, rt.sharing, server)
        {
            cp.post_recording(&RecordingStarted {
                kind: "recording".to_string(),
                file,
                started_at,
            });
        }
    } else {
        capturing.store(false, Ordering::Relaxed);
        recorder.lock().unwrap().stop();
    }
}

/// Which app to scope to. `all` → global monitor ("call"). `auto`/`<id>` → real detection is
/// TODO(on-device); until wired we report no call (safe: no false positives).
fn capture_target(app_sel: &str, _rt: &Runtime) -> Option<(String, String)> {
    if app_sel == "all" {
        return Some(("call".to_string(), String::new()));
    }
    // TODO(on-device): enumerate active audio streams (Pulse sink-inputs / PipeWire nodes) whose
    // binary ∈ the meeting-app set; for browsers/chat apps additionally gather window/URL evidence
    // (platform::win on X11) and require detect::platform_from_evidence to yield a key, honoring
    // Runtime.browser_meet. Title via detect::useful_meeting_title. Returns Some((key, title)).
    None
}

fn report_detected(on: bool, key: &str, title: &str, cp: &ControlPlane, gate: &mut GateState) {
    let changed =
        on != gate.was_in_call || (on && (key != gate.was_key || title != gate.was_title));
    if !changed {
        return;
    }
    gate.was_in_call = on;
    gate.was_key = if on { key.to_string() } else { String::new() };
    gate.was_title = if on { title.to_string() } else { String::new() };

    let state = if on {
        AudioState {
            in_meeting: true,
            app: Some(key.to_string()),
            meeting_title: if title.is_empty() { None } else { Some(title.to_string()) },
            title: if title.is_empty() { None } else { Some(title.to_string()) },
        }
    } else {
        AudioState { in_meeting: false, ..Default::default() }
    };
    // Keep source:"coreaudio" on every OS — the core keys on it to treat this as authoritative.
    cp.post_state(&AudioStateEvent {
        kind: "state".to_string(),
        source: "coreaudio".to_string(),
        state,
    });
}

/// Process completed utterances: WAV → whisper → confidence filter → clean → caption POST.
fn spawn_processor(
    rx: mpsc::Receiver<audio::Captured>,
    cp: Arc<ControlPlane>,
    runtime: Arc<Mutex<Runtime>>,
    capturing: Arc<AtomicBool>,
    cfg: &StartupCfg,
) {
    let whisper_url = cfg.whisper_url.clone();
    let stt_token = cfg.stt_token.clone();
    let min_conf = cfg.min_conf;
    let expressions = cfg.expressions;

    std::thread::spawn(move || {
        for captured in rx {
            // Drop utterances that arrive while we are not in a captured call.
            if !capturing.load(Ordering::Relaxed) {
                continue;
            }
            let rt = runtime.lock().unwrap().clone();
            let wav = encode_wav16(&captured.utt.samples, 16000, 1);
            let filename = format!(
                "meeting-{}-{}-{}.wav",
                captured.tag,
                captured.utt.index,
                uuid::Uuid::new_v4()
            );
            let Some(t) = cp.transcribe(
                &whisper_url,
                &stt_token,
                wav,
                &filename,
                &rt.language,
                rt.translate,
                &rt.vocab,
            ) else {
                continue;
            };
            if t.confidence < min_conf {
                eprintln!(
                    ">> {} seg#{} {:.1}s dropped: confidence {:.2} below {:.2}",
                    captured.tag,
                    captured.utt.index,
                    captured.utt.seconds(),
                    t.confidence,
                    min_conf
                );
                continue;
            }
            let clean = if expressions {
                detect::keep_expressions(&t.text)
            } else {
                detect::clean_transcript(&t.text)
            };
            if clean.chars().count() > 1 {
                cp.post_caption(&Caption {
                    kind: CaptionTag::Caption,
                    speaker: captured.speaker,
                    text: clean,
                    is_self: captured.is_self,
                    ts: epoch_ms(),
                });
            }
        }
    });
}

/// SIGTERM/SIGINT → finalize any in-progress mp4, then exit after a short grace (PROTOCOL.md §1).
fn install_signal_handler(recorder: Arc<Mutex<Recorder>>, running: Arc<AtomicBool>) {
    use signal_hook::consts::{SIGINT, SIGTERM};
    use signal_hook::iterator::Signals;

    let mut signals = match Signals::new([SIGTERM, SIGINT]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(">> could not install signal handler: {e}");
            return;
        }
    };
    std::thread::spawn(move || {
        if signals.forever().next().is_some() {
            eprintln!(">> shutting down — finalizing");
            shutdown(&recorder, &running);
        }
    });
}

/// Stop capture, finalize the mp4, exit 0 after a brief grace for finalization.
fn shutdown(recorder: &Mutex<Recorder>, running: &AtomicBool) -> ! {
    running.store(false, Ordering::Relaxed);
    recorder.lock().unwrap().stop();
    std::thread::sleep(Duration::from_millis(300));
    std::process::exit(0);
}
