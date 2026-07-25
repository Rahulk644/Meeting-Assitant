//! meeting-tap (Windows) — continuous meeting transcription from system audio + mic, optional video.
//!
//! REAL / verifiable spine (portable): arg parsing (`helpers_core::args`), the 2 s config-pull loop
//! (`GET /api/state`), authoritative `coreaudio` state POSTs and caption POSTs (`POST /ingest`), the
//! `Segmenter → wav → whisper → caption` pipeline, `--selftest`, `--list-apps`, the parent watchdog,
//! and the **no-SIGTERM** shutdown: it finalizes on `SetConsoleCtrlHandler` (CTRL_C/BREAK/CLOSE) AND
//! on stdin-EOF (PROTOCOL §Shutdown).
//!
//! TODO(on-device): the actual WASAPI process-loopback + mic sample capture and windows-capture
//! video (see `platform::audio` / `platform::video`). Detection via the WASAPI Audio Session API is
//! written against the `windows` crate; capture streaming is the piece needing a device to verify.
//! Until then capture threads run but feed no samples (contract-valid: no captions, clean shutdown).

use std::sync::{Arc, Mutex};

use helpers_core::args::Args;
use helpers_core::protocol::{AudioState, AudioStateEvent, RecordingStarted};
use helpers_core::segmenter::{DEFAULT_MIC_RMS, DEFAULT_SPEECH_RMS};
use helpers_core::watchdog;

use platform::apps;
use platform::audio::{self, Capture, CallTarget};
use platform::pipeline::{DynConfig, Shared};
use platform::resample;
use platform::shutdown;
use platform::{net, text};

fn main() {
    let args = Args::from_env();

    if args.has("selftest") {
        std::process::exit(selftest());
    }

    if args.has("list-apps") {
        for (exe, title) in audio::list_apps() {
            println!("{exe}\t{title}");
        }
        return;
    }

    // Positional 1 = guiPort (only if not a flag); positional 2 = whisperURL.
    let gui_port = args.positional(0).unwrap_or_else(|| "8787".into());
    let whisper_url = args
        .positional(1)
        .unwrap_or_else(|| "http://127.0.0.1:8091/inference".into());

    let app_sel = args.opt("app", "auto");
    let mic_on = args.opt("mic", "on") != "off";
    let gate = args.opt("gate", "on") != "off";
    let cli_video_mode = args.opt("video", "off");
    let rec_dir = args.opt("rec-dir", "data/meetings/rec");
    let temp_dir = args.opt("temp-dir", &std::env::temp_dir().to_string_lossy());
    let sys_rms = args.opt("rms", "").parse::<f32>().unwrap_or(DEFAULT_SPEECH_RMS);
    let mic_rms = args.opt("mic-rms", "").parse::<f32>().unwrap_or(DEFAULT_MIC_RMS);
    let min_conf = args.opt("min-conf", "0.5").parse::<f32>().unwrap_or(0.5);
    let expressions = args.has("expressions");
    let default_user = std::env::var("USERNAME").unwrap_or_else(|_| "Me".into());
    let me = args.opt("me", if default_user.is_empty() { "Me" } else { &default_user });

    let gui_token = args.opt_env("ASSISTANT_GUI_TOKEN", "gui-token", "");
    let stt_token = args.opt_env("ASSISTANT_STT_TOKEN", "stt-token", "");

    let _ = std::fs::create_dir_all(&temp_dir);

    // Graceful stop: console-ctrl + stdin-EOF (no SIGTERM on Windows) + dead parent.
    shutdown::install_console_handler();
    shutdown::watch_stdin_eof();
    watchdog::watch_parent(|| {
        platform::log(">> parent app exited — releasing capture");
        shutdown::request_stop();
    });

    let dyn_cfg = Arc::new(Mutex::new(DynConfig {
        min_conf,
        expressions,
        ..DynConfig::default()
    }));
    let shared = Arc::new(Shared {
        whisper_url: whisper_url.clone(),
        stt_token,
        gui_port: gui_port.clone(),
        gui_token: gui_token.clone(),
        temp_dir: std::path::PathBuf::from(&temp_dir),
        dyn_cfg: dyn_cfg.clone(),
    });

    platform::log(&format!(
        ">> meeting-tap ready — gate:{} gui:{gui_port} whisper:{whisper_url} app:{app_sel} mic:{mic_on} me:{me}",
        if gate { "on" } else { "off" }
    ));

    let mut cap = Capture::default();
    let mut was_in_call = false;
    let mut was_app = String::new();
    let mut was_title = String::new();

    loop {
        if shutdown::stopping() {
            break;
        }

        // Pull server config and update the dynamic transcription knobs.
        let mut module_meetings = false;
        let mut sharing = false;
        // `manual` is a string tri-state on the wire ("on"/"auto"/"off"/"paused"). Track the two
        // decisions it drives instead of a single bool.
        let mut manual_on = false;
        let mut manual_off = false;
        let mut auto_detect = true;
        let mut browser_meet = true;
        let mut video_mode = cli_video_mode.clone();
        if let Some(cfg) = net::pull_config(&gui_port, &gui_token) {
            module_meetings = cfg.modules.as_ref().map(|m| m.meetings).unwrap_or(false);
            sharing = cfg.sharing;
            manual_on = cfg.manual_force_on();
            manual_off = cfg.manual_force_off();
            auto_detect = cfg.auto_detect;
            browser_meet = cfg.browser_meet;
            if let Some(vm) = cfg.video_mode.clone() {
                video_mode = vm;
            }
            if let Ok(mut d) = dyn_cfg.lock() {
                d.language = cfg.language.clone().unwrap_or_else(|| "auto".into());
                d.translate = cfg.translate;
                d.vocab = cfg.vocab.clone().unwrap_or_default();
            }
        }

        // Meetings module off ⇒ never capture, report not-in-meeting.
        if !module_meetings {
            report_detected(&gui_port, &gui_token, false, "", "", &mut was_in_call, &mut was_app, &mut was_title);
            cap.stop();
            responsive_sleep();
            continue;
        }

        // Detect the current call target from Active audio sessions.
        let detected = audio::detect_call(&app_sel, browser_meet);
        let target: Option<CallTarget> = detected.or_else(|| {
            if gate {
                None
            } else {
                // --gate off: manual always-capture of full system audio.
                Some(CallTarget { key: "call".into(), title: String::new(), pid: 0, exe: String::new() })
            }
        });
        let is_detected = target.is_some();
        let key = target.as_ref().map(|t| t.key.clone()).unwrap_or_default();
        let title = target.as_ref().map(|t| t.title.clone()).unwrap_or_default();

        report_detected(
            &gui_port, &gui_token, is_detected, &key, &title,
            &mut was_in_call, &mut was_app, &mut was_title,
        );

        // Manual "on" forces capture; "off"/"paused" forces stop; "auto" defers to detection.
        let should_capture = manual_on || (!manual_off && auto_detect && is_detected);
        if should_capture {
            if let Some(t) = &target {
                cap.start(
                    &video_mode, sharing, &rec_dir, t, mic_on, &me, sys_rms, mic_rms, shared.clone(),
                );
                if let Some((file, started_at)) = cap.take_recording() {
                    net::post_ingest(
                        &gui_port,
                        &gui_token,
                        &RecordingStarted { kind: "recording".into(), file, started_at },
                    );
                }
            }
        } else {
            cap.stop();
        }

        responsive_sleep();
    }

    // Finalize: stop capture (writes any mp4 moov atom), brief grace, exit.
    platform::log(">> shutting down — finalizing");
    cap.stop();
    std::process::exit(0);
}

/// Sleep ~2 s but wake early on a stop request.
fn responsive_sleep() {
    for _ in 0..10 {
        if shutdown::stopping() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// POST an authoritative `coreaudio` state event, but only on a real change (mirrors Swift
/// `reportDetected`). `source:"coreaudio"` is kept on every OS — the core keys on that string.
#[allow(clippy::too_many_arguments)]
fn report_detected(
    port: &str,
    token: &str,
    in_call: bool,
    app: &str,
    title: &str,
    was_in_call: &mut bool,
    was_app: &mut String,
    was_title: &mut String,
) {
    let changed = in_call != *was_in_call || (in_call && (app != was_app || title != was_title));
    if !changed {
        return;
    }
    *was_in_call = in_call;
    *was_app = if in_call { app.to_string() } else { String::new() };
    *was_title = if in_call { title.to_string() } else { String::new() };

    let state = if in_call {
        AudioState {
            in_meeting: true,
            app: Some(app.to_string()),
            meeting_title: if title.is_empty() { None } else { Some(title.to_string()) },
            title: if title.is_empty() { None } else { Some(title.to_string()) },
        }
    } else {
        AudioState { in_meeting: false, ..Default::default() }
    };
    net::post_ingest(
        port,
        token,
        &AudioStateEvent { kind: "state".into(), source: "coreaudio".into(), state },
    );
}

/// Pure-logic smoke test — exercises the portable spine against `helpers-core`.
fn selftest() -> i32 {
    let mut ok = true;
    let mut check = |cond: bool, msg: &str| {
        if !cond {
            eprintln!("SELFTEST FAIL: {msg}");
            ok = false;
        }
    };
    check(
        apps::platform_from_evidence("https://meet.google.com/abc-defg-hij", "chrome.exe").as_deref()
            == Some("meet"),
        "Google Meet URL",
    );
    check(
        apps::platform_from_evidence("YouTube — Google Chrome", "chrome.exe").is_none(),
        "ordinary browser audio rejected",
    );
    check(apps::dedicated_key("Zoom.exe") == Some("zoom"), "zoom exe table");
    check(
        apps::useful_meeting_title(&["Zoom Meeting".into()], "zoom").is_empty(),
        "generic title rejected",
    );
    check(resample::to_16k_mono(&vec![0.0f32; 48_000], 48_000).len() == 16_000, "48k→16k resample");
    check(text::clean("[BLANK_AUDIO] hi (music)") == "hi", "transcript cleaning");
    if ok {
        eprintln!("SELFTEST OK");
        0
    } else {
        1
    }
}
