use std::io::Write;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tauri::plugin::PermissionState;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_notification::NotificationExt;

mod codex_runtime;
mod local_server;
#[cfg(target_os = "macos")]
mod native_pill;
pub mod speech_audio;
mod speech_models;

// Absolute path to the repo, baked at build time. In DEV this is where everything lives. In a PACKAGED build
// the app's own files come from the bundle's Resources dir instead (see init_paths). REPO is consulted only
// as a one-time migration source for the pre-0.1 model location; it is never a packaged runtime dependency.
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const SERVER: &str = "http://127.0.0.1:8790";
static SERVER_TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

// Resolved once at startup (init_paths). RES_BASE = where the app's OWN files live (dev: the repo; packaged:
// the .app's read-only Resources dir). DATA_DIR / MODELS_DIR = WRITABLE locations for saved meetings and the
// downloaded model (dev: the repo; packaged: ~/Library/Application Support/My Assistant, since the bundle is
// read-only). The in-process server reads them directly; Swift helpers get RES_BASE-relative paths.
static RES_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static DATA_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static MODELS_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
fn res_base() -> String {
    RES_BASE.get().cloned().unwrap_or_else(|| REPO.to_string())
}
fn data_dir() -> String {
    DATA_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| format!("{}/data", REPO))
}
// The source web assets live in the repo's app/ in dev, but the minified release bundle stages them under webui/ —
// a plain "app/" resource dir would collide with the crate's own `app` binary during Tauri's dev staging.
fn web_dir() -> String {
    if cfg!(debug_assertions) {
        format!("{}/desktop/web-dist", res_base())
    } else {
        format!("{}/webui", res_base())
    }
}
fn init_paths(app: &tauri::App) {
    let (res, data, models) = if cfg!(debug_assertions) {
        (
            REPO.to_string(),
            format!("{}/data", REPO),
            format!("{}/stt/whisper", REPO),
        )
    } else {
        let res = app
            .path()
            .resource_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| REPO.to_string());
        let home = std::env::var("HOME").unwrap_or_default();
        (
            res,
            format!("{}/Library/Application Support/My Assistant/data", home),
            format!("{}/Library/Application Support/My Assistant/models", home),
        )
    };
    let _ = std::fs::create_dir_all(&data);
    let _ = std::fs::create_dir_all(&models);
    let _ = RES_BASE.set(res);
    let _ = DATA_DIR.set(data);
    let _ = MODELS_DIR.set(models);
}

// The Node-based beta could reuse a model from the build checkout through ASSISTANT_REPO. The Rust-native
// release stores models in Application Support, so an in-place app upgrade needs to adopt that old copy once
// instead of asking the same user for another 845 MB download. Other recipients simply have no candidate and
// still see the normal first-run download prompt.
fn legacy_model_dirs() -> Vec<String> {
    if cfg!(debug_assertions) {
        return Vec::new();
    }
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        format!("{REPO}/stt/whisper"),
        format!("{home}/Library/Application Support/My Assistant/data/stt/whisper"),
        format!("{home}/Library/Application Support/My Assistant/stt/whisper"),
    ]
}

fn notify_meeting_started(app: &AppHandle, meeting_app: &str) {
    let notifications = app.notification();
    let permission = notifications
        .permission_state()
        .unwrap_or(PermissionState::Denied);
    let permission = if matches!(
        permission,
        PermissionState::Prompt | PermissionState::PromptWithRationale
    ) {
        notifications
            .request_permission()
            .unwrap_or(PermissionState::Denied)
    } else {
        permission
    };
    if permission != PermissionState::Granted {
        return;
    }
    let body = if meeting_app.is_empty() || meeting_app == "your call" {
        "A live call was detected. My Assistant is ready to take notes.".to_string()
    } else {
        format!("A live {meeting_app} call was detected. My Assistant is ready to take notes.")
    };
    if let Err(error) = notifications
        .builder()
        .title("Meeting detected")
        .body(body)
        .show()
    {
        log::warn!("meeting notification failed: {error}");
    }
}

// Sidecars the app spawns — all killed when it quits (so the mic is never left held).
static CHILDREN: Mutex<Vec<Child>> = Mutex::new(Vec::new());
fn track(child: std::io::Result<Child>) {
    if let Ok(c) = child {
        if let Ok(mut v) = CHILDREN.lock() {
            v.push(c);
        }
    }
}
fn kill_children() {
    if let Ok(mut v) = CHILDREN.lock() {
        let kids: Vec<Child> = v.drain(..).collect();
        // SIGTERM first, not SIGKILL: meeting-tap traps SIGTERM to finalize its in-progress .mp4 (write the
        // moov atom so a recording made right up to quit stays playable). SIGKILL is uncatchable and would
        // skip that, truncating the file. Give the handlers a brief grace, then hard-kill anything still alive.
        for c in &kids {
            let _ = Command::new("/bin/kill").arg(c.id().to_string()).status();
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        for mut c in kids {
            let _ = c.kill();
        }
    }
}

fn port_open(port: u16) -> bool {
    format!("127.0.0.1:{}", port)
        .parse()
        .ok()
        .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).ok())
        .is_some()
}

// Forward helper NDJSON without spawning /usr/bin/curl for every AX state change. This tiny localhost
// request stays dependency-free and removes another short-lived process from the meeting hot path.
fn post_local_ingest(line: &str) {
    let Some(addr) = "127.0.0.1:8790".parse().ok() else {
        return;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else {
        return;
    };
    let request = format!(
        "POST /ingest HTTP/1.1\r\nHost: 127.0.0.1:8790\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        SERVER_TOKEN.get().map(String::as_str).unwrap_or(""), line.len(), line
    );
    let _ = stream.write_all(request.as_bytes());
}

// Spawn the Accessibility detector (speaker names + in-call state) DIRECTLY so macOS attributes its
// Accessibility check to the app itself. Its NDJSON is forwarded to the authenticated server /ingest route.
fn start_meeting_state() {
    let ms = format!("{}/meeting-state/meeting-state", res_base());
    if !Path::new(&ms).exists() {
        return;
    }
    let parent_pid = std::process::id().to_string();
    if let Ok(mut child) = Command::new(&ms)
        .env("ASSISTANT_PARENT_PID", &parent_pid)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(out) = child.stdout.take() {
            std::thread::spawn(move || {
                for line in
                    std::io::BufRead::lines(std::io::BufReader::new(out)).map_while(Result::ok)
                {
                    if line.trim().is_empty() {
                        continue;
                    }
                    post_local_ingest(&line);
                }
            });
        }
        if let Ok(mut v) = CHILDREN.lock() {
            v.push(child);
        }
    }
}

// Bring up the native capture helpers. The UI/API server now runs in-process in Rust, so the packaged app
// no longer embeds or keeps a 108 MB Node runtime alive. The transcription model server remains on-demand.
fn start_stack() {
    if cfg!(debug_assertions) {
        return;
    } // dev: capture comes from ./run.sh

    // transcribe-server is NOT started here — the in-process server spawns it on demand when speech arrives
    // and unloads it after the configured idle period, so the model stays out of RAM between sessions.
    let tap = format!("{}/stt/meeting-tap", res_base());
    if Path::new(&tap).exists() {
        let recdir = format!("{}/meetings/rec", data_dir());
        let temp = format!("{}/tmp", data_dir());
        let token = SERVER_TOKEN.get().cloned().unwrap_or_default();
        let parent_pid = std::process::id().to_string();
        track(
            Command::new(tap)
                .args([
                    "8790",
                    "http://127.0.0.1:8091/inference",
                    "--gate",
                    "on",
                    "--rec-dir",
                    &recdir,
                    "--temp-dir",
                    &temp,
                ])
                .env("ASSISTANT_GUI_TOKEN", &token)
                .env("ASSISTANT_STT_TOKEN", &token)
                .env("ASSISTANT_PARENT_PID", &parent_pid)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn(),
        );
    }
    start_meeting_state(); // names + participants — direct child so Accessibility works
}

// ---- Tauri commands the UI calls (window.__TAURI__.core.invoke) ------------------------------------
#[tauri::command]
fn set_stealth(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    window.set_content_protected(on).map_err(|e| e.to_string())
}

// Animate the menu-bar tray icon with an audio-wave title while a call records (the UI calls this on start/stop).
static TRAY_REC: AtomicBool = AtomicBool::new(false);
#[tauri::command]
fn set_tray_recording(app: tauri::AppHandle, on: bool) {
    // Tray-title mutations are AppKit calls → they MUST run on the main thread (off-main aborts with
    // SIGABRT, the same rule dict_feedback follows). A #[tauri::command] runs on a worker thread and the
    // animation loop is its own thread, so every set_title is marshalled onto main via run_on_main_thread.
    fn set_title(app: &tauri::AppHandle, s: String) {
        let a = app.clone();
        let _ = app.run_on_main_thread(move || {
            if let Some(t) = a.tray_by_id("tray") {
                let _ = t.set_title(Some(s));
            }
        });
    }
    if on {
        if TRAY_REC.swap(true, Ordering::SeqCst) {
            return;
        } // already animating
        std::thread::spawn(move || {
            let frames = ["▁▂▃▅▃▂", "▂▃▅▇▅▃", "▃▅▇▅▃▂", "▂▄▆▄▂▁"];
            let mut i = 0usize;
            while TRAY_REC.load(Ordering::SeqCst) {
                set_title(&app, format!(" {}", frames[i % frames.len()]));
                i += 1;
                std::thread::sleep(Duration::from_millis(300));
            }
            set_title(&app, String::new()); // clear — None doesn't clear on macOS
        });
    } else {
        TRAY_REC.store(false, Ordering::SeqCst);
        set_title(&app, String::new());
    }
}

// ---- in-app dictation (Handy's model) --------------------------------------------------------------
// Runs in this app process so the paste inherits the app's own Accessibility grant: the hotkey is a Carbon
// global shortcut (no permission needed), ⌘V is synthesized via CGEvent, and the mic is recorded in-process
// with cpal (asking for the Microphone grant on first use). UI events go straight to the in-process server.
use std::time::Instant;

// A live recording. A cpal input Stream is `!Send`, so it can't sit in DICT (a static Mutex shared across
// threads) — instead its owning thread parks on `stop` and drops the Stream on that same thread. The
// capture callback appends mono f32 into `samples`; `rate`/`chans` report the device's real format.
struct Recorder {
    generation: u64,
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: Arc<AtomicU32>,
    handle: Option<JoinHandle<()>>,
}
struct DictState {
    enabled: bool,
    rec: Option<Recorder>,
    started: Option<Instant>,
    generation: u64,
}
static DICT: Mutex<DictState> = Mutex::new(DictState {
    enabled: false,
    rec: None,
    started: None,
    generation: 0,
});

// Open the default input device and stream mono f32 into `samples` until `stop` is set. Built on a
// dedicated thread because a cpal Stream is `!Send`; the first-run mic-permission prompt fires here.
// Handles the three sample formats macOS inputs actually produce (F32 on the built-in mic, I16/U16 on
// some USB interfaces). ~120s safety cap so a stuck key can never hold the mic open forever.
fn spawn_capture(
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: Arc<AtomicU32>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let Some(dev) = host.default_input_device() else {
            return;
        };
        let Ok(cfg) = dev.default_input_config() else {
            return;
        };
        rate.store(cfg.sample_rate().0, Ordering::Relaxed);
        let ch = (cfg.channels() as usize).max(1); // guard: chunks(0) panics, and a panic out of cpal's C callback aborts
        let fmt = cfg.sample_format();
        let scfg: cpal::StreamConfig = cfg.into();
        let err = |e| log::error!("dictation cpal error: {e}");
        let s = samples.clone();
        // Keep the first channel of every frame → mono, matching whisper's expected input.
        let stream = match fmt {
            cpal::SampleFormat::F32 => dev.build_input_stream(
                &scfg,
                move |d: &[f32], _: &cpal::InputCallbackInfo| {
                    let mut b = s.lock().unwrap();
                    for f in d.chunks(ch) {
                        b.push(f[0]);
                    }
                },
                err,
                None,
            ),
            cpal::SampleFormat::I16 => dev.build_input_stream(
                &scfg,
                move |d: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut b = s.lock().unwrap();
                    for f in d.chunks(ch) {
                        b.push(f[0] as f32 / 32768.0);
                    }
                },
                err,
                None,
            ),
            cpal::SampleFormat::U16 => dev.build_input_stream(
                &scfg,
                move |d: &[u16], _: &cpal::InputCallbackInfo| {
                    let mut b = s.lock().unwrap();
                    for f in d.chunks(ch) {
                        b.push((f[0] as f32 - 32768.0) / 32768.0);
                    }
                },
                err,
                None,
            ),
            _ => return,
        };
        let Ok(stream) = stream else {
            return;
        };
        if stream.play().is_err() {
            return;
        }
        let mut n: u32 = 0;
        while !stop.load(Ordering::Relaxed) && n < 12_000 {
            std::thread::sleep(Duration::from_millis(10));
            n += 1;
        }
        drop(stream); // stop capture on the SAME thread that built it (Stream is !Send)
    })
}

// Write mono 16 kHz f32 samples as a 16-bit PCM WAV for the STT server.
fn write_wav16k(path: &Path, samples16k: &[f32]) -> bool {
    use std::os::unix::fs::OpenOptionsExt;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let Ok(file) = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
    else {
        return false;
    };
    let Ok(mut w) = hound::WavWriter::new(file, spec) else {
        return false;
    };
    for v in samples16k {
        let _ = w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16);
    }
    w.finalize().is_ok()
}

fn ingest(value: serde_json::Value) {
    // This server lives in the same process. Direct delivery is fast, preserves event order, and avoids a
    // short-lived curl process for every start/stop/result. Native helpers still use authenticated HTTP.
    if !local_server::ingest_local(value) {
        log::warn!("dictation event dropped before local server startup");
    }
}
fn emit_dstate(phase: &str, generation: u64) {
    ingest(
        serde_json::json!({ "type": "dictation-state", "phase": phase, "generation": generation }),
    );
}

// AppHandle stashed at setup so the dictation fns can give RELIABLE feedback: a start/stop SOUND (the
// primary "is it on?" cue, like Handy) + the menu-bar title "● Dictating" + the pill window (shown/hidden
// from Rust here, so it no longer depends on the overlay webview calling back — which was silently failing).
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

// Dictation preferences pushed from Settings via set_dictation. The in-process dictation owns recording,
// the paste, the sound cues and the pill, so these rows only take effect if THIS process honors them —
// they used to flow to the removed `meeting-tap --dictate` and silently no-op'd. clipboard: 0 = restore
// the user's previous clipboard after paste ("Never save"), 1 = leave the dictated text on it (Save/Replace).
// overlay_pos: 0 bottom-center · 1 bottom-right · 2 top-right · 3 top-center · 4 left-center · 5 right-center.
struct DictPrefs {
    sound: bool,
    volume: f32,
    clipboard: u8,
    overlay: bool,
    overlay_pos: u8,
}
static DICT_PREFS: Mutex<DictPrefs> = Mutex::new(DictPrefs {
    sound: true,
    volume: 0.7,
    clipboard: 0,
    overlay: true,
    overlay_pos: 0,
});
fn dict_prefs() -> (bool, f32, u8, bool, u8) {
    let p = DICT_PREFS.lock().unwrap_or_else(|e| e.into_inner());
    (p.sound, p.volume, p.clipboard, p.overlay, p.overlay_pos)
}

fn play(sound: &str) {
    // "Dictation sound" off → no cue at all. Otherwise play at the Settings "Feedback volume" (afplay -v 0..1).
    let (on, vol, _, _, _) = dict_prefs();
    if !on {
        return;
    }
    // Reap afplay (.status() waits) on a detached thread so it doesn't accumulate as a zombie on each
    // start/stop cue — a bare spawn() is never reaped and would pile up over a long session.
    let path = format!("/System/Library/Sounds/{}.aiff", sound);
    let v = format!("{:.2}", vol.clamp(0.0, 1.0));
    std::thread::spawn(move || {
        let _ = Command::new("afplay").arg("-v").arg(v).arg(path).status();
    });
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum DictFeedback {
    Listening,
    Transcribing,
    Idle,
}

fn dict_feedback(phase: DictFeedback, generation: u64) {
    if let Some(app) = APP.get() {
        let (_, _, _, overlay, overlay_pos) = dict_prefs();
        let elapsed_seconds = DICT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .started
            .map(|started| started.elapsed().as_secs())
            .unwrap_or(0);
        let app = app.clone();
        let handle = app.clone();
        let _ = handle.run_on_main_thread(move || {
            if DICT.lock().unwrap_or_else(|e| e.into_inner()).generation != generation {
                return;
            }
            if let Some(t) = app.tray_by_id("tray") {
                let title = match phase {
                    DictFeedback::Listening => " ● Dictating",
                    DictFeedback::Transcribing => " ● Finishing",
                    DictFeedback::Idle => "",
                };
                let _ = t.set_title(Some(title));
            }
            #[cfg(target_os = "macos")]
            native_pill::set(
                if overlay { phase } else { DictFeedback::Idle },
                overlay_pos,
                elapsed_seconds,
                &DICT_TRIGGER_LABEL
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()),
            );
        });
    }
}

fn dict_set_clip(s: &str) {
    if let Ok(mut c) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(mut si) = c.stdin.take() {
            use std::io::Write;
            let _ = si.write_all(s.as_bytes());
        }
        let _ = c.wait();
    }
}
fn dict_get_clip() -> String {
    Command::new("pbpaste")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}
// Paste = set clipboard + synth ⌘V + restore the old clipboard. enigo posts the keystroke from THIS
// process, so the app's Accessibility grant authorizes it (the correct identity — the whole fix).
// ponytail: 180ms restore delay — enough for the target to read the paste; a slow first paste could miss.
fn dict_paste(text: &str) {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    // "Clipboard behaviour": Never save (0) → snapshot the old clipboard and restore it after the paste;
    // Save/Replace (1) → leave the dictated text on the clipboard (skip the restore).
    let (_, _, clip, _, _) = dict_prefs();
    let restore = clip == 0;
    let old = if restore {
        dict_get_clip()
    } else {
        String::new()
    };
    dict_set_clip(text);
    // Synthesize ⌘V with the Command flag set EXPLICITLY on the V (keycode 9) event — the proven,
    // deterministic paste (same as the old working meeting-tap code), not enigo's modifier tracking which
    // silently produced nothing. Needs Accessibility (the app's grant). Thread-safe (no HIToolbox layout call).
    if let Ok(src) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) {
        if let Ok(down) = CGEvent::new_keyboard_event(src.clone(), 9, true) {
            down.set_flags(CGEventFlags::CGEventFlagCommand);
            down.post(CGEventTapLocation::HID);
        }
        if let Ok(up) = CGEvent::new_keyboard_event(src, 9, false) {
            up.set_flags(CGEventFlags::CGEventFlagCommand);
            up.post(CGEventTapLocation::HID);
        }
    }
    std::thread::sleep(Duration::from_millis(250));
    if restore {
        dict_set_clip(&old);
    }
}

// Two modes off one trigger: tap-to-talk toggles on each press; push-to-talk starts on press and
// stops on release. The Carbon hotkey reports press AND release (kEventHotKeyReleased), so PTT needs no
// key tap → no Input Monitoring. Mode = DICT_PTT (set from Settings via set_dictation).
static DICT_PTT: AtomicBool = AtomicBool::new(false); // false = tap-to-talk, true = push-to-talk
fn dictation_start() {
    let mut g = DICT.lock().unwrap_or_else(|e| e.into_inner());
    if !g.enabled || g.rec.is_some() {
        return;
    }
    g.generation = g.generation.wrapping_add(1).max(1);
    let generation = g.generation;
    // Start capturing IMMEDIATELY, in-process — no child to launch, so the mic opens as fast as macOS
    // can hand us the device (Handy measures ~230ms for the same step; we skip the ~300ms process spawn).
    let stop = Arc::new(AtomicBool::new(false));
    let samples = Arc::new(Mutex::new(Vec::<f32>::with_capacity(16_000 * 6)));
    let rate = Arc::new(AtomicU32::new(16_000));
    let handle = spawn_capture(stop.clone(), samples.clone(), rate.clone());
    g.rec = Some(Recorder {
        generation,
        stop,
        samples,
        rate,
        handle: Some(handle),
    });
    g.started = Some(Instant::now());
    drop(g);
    log::info!("acceptance:right-command phase=listening generation={generation}");
    play("Tink");
    dict_feedback(DictFeedback::Listening, generation);
    emit_dstate("listening", generation);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let active = {
            let dict = DICT.lock().unwrap_or_else(|e| e.into_inner());
            dict.generation == generation && dict.rec.is_some()
        };
        if !active {
            break;
        }
        dict_feedback(DictFeedback::Listening, generation);
    });
    // Esc-to-cancel is watched inside the event tap (keycode 53) and gated on recording — no separate grab.
}
fn dictation_stop_rec() {
    let mut g = DICT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(rec) = g.rec.take() else {
        return;
    };
    let ms = g
        .started
        .map(|t| t.elapsed().as_millis() as u64)
        .unwrap_or(0);
    g.started = None;
    drop(g);
    log::info!(
        "acceptance:right-command phase=transcribing generation={} capturedMs={ms}",
        rec.generation
    );
    play("Pop"); // immediate stop cue (encode + transcription run on the thread below)
                 // Stop must be visible immediately. VAD/model startup happens on the worker and can take noticeable time;
                 // leaving the old "Listening" state up during that work makes a successful second tap look ignored.
    emit_dstate("transcribing", rec.generation);
    dict_feedback(DictFeedback::Transcribing, rec.generation);
    std::thread::spawn(move || dictation_finish(rec, ms));
}
fn dictation_toggle() {
    let recording = DICT.lock().unwrap_or_else(|e| e.into_inner()).rec.is_some();
    if recording {
        dictation_stop_rec()
    } else {
        dictation_start()
    }
}
// Serializes the finish tail so two quick back-to-back dictations can't race the shared /tmp WAV or clipboard.
static DICT_FINISH: Mutex<()> = Mutex::new(());
static DICT_TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
// Reset the pill + tray to idle — but ONLY if no newer dictation is already recording. dictation_finish runs
// on a detached thread and can take up to 60s (transcription); by the time it ends the user may have started
// a new dictation that now owns the pill (dictation_stop_rec take()s the Recorder, so a new start is allowed
// the instant Stop is pressed). Without this guard the old, slow finish stomps the new "listening" pill to idle.
fn dict_ui_idle(generation: u64) {
    {
        let g = DICT.lock().unwrap_or_else(|e| e.into_inner());
        if g.generation != generation || g.rec.is_some() {
            return;
        }
        // Keep the generation check and delivery atomic relative to a newer start.
        emit_dstate("idle", generation);
    }
    log::info!("acceptance:right-command phase=idle generation={generation}");
    dict_feedback(DictFeedback::Idle, generation);
}
fn dictation_finish(mut rec: Recorder, ms: u64) {
    let generation = rec.generation;
    rec.stop.store(true, Ordering::Relaxed);
    if let Some(h) = rec.handle.take() {
        let _ = h.join();
    } // stream fully stopped, buffer settled
    let rate = rec.rate.load(Ordering::Relaxed);
    let raw = std::mem::take(&mut *rec.samples.lock().unwrap());
    // The selected model owns its audio policy. Whisper compacts silence; Parakeet, Nemotron, and the
    // Hinglish specialists retain acoustic context after a speech-presence gate.
    let speech = speech_audio::preprocess_speech(
        &raw,
        rate,
        local_server::active_speech_preprocessing_policy(),
    );
    // Silero VAD: keep only speech → no hallucinated loop on silence
    // Acquire AFTER the VAD (per-clip, safe to run in parallel): serialize only the shared tail — the /tmp
    // WAV write, the upload, and the clipboard save/paste/restore — so overlapping finishes can't clobber
    // each other. Recover the guard on poison instead of panicking the finish thread.
    let _finish = DICT_FINISH.lock().unwrap_or_else(|e| e.into_inner());
    let seq = DICT_TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let wav = Path::new(&data_dir())
        .join("tmp")
        .join(format!("dictation-{}-{seq}.wav", std::process::id()));
    // No speech detected → skip the engine entirely (no paste, no hallucinated text).
    if speech.is_empty() || !write_wav16k(&wav, &speech) {
        dict_ui_idle(generation);
        return;
    }
    let auth = format!(
        "Authorization: Bearer {}",
        SERVER_TOKEN.get().map(String::as_str).unwrap_or("")
    );
    let file_form = format!("file=@{}", wav.to_string_lossy());
    let out = Command::new("/usr/bin/curl")
        .args([
            "-s",
            "--max-time",
            "60",
            "-H",
            &auth,
            &format!("{}/api/transcribe", SERVER),
            "-F",
            &file_form,
        ])
        .output();
    let _ = std::fs::remove_file(&wav);
    let text = out
        .ok()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .and_then(|v| {
            v.get("text")
                .and_then(|t| t.as_str())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default();
    if !text.is_empty() {
        dict_paste(&text);
        ingest(serde_json::json!({ "type": "dictation", "text": text, "ms": ms }));
    }
    dict_ui_idle(generation);
}

// Esc while recording → throw the clip away: stop capture, no resample / VAD / transcribe / paste. The
// stop+join runs off-thread because this is invoked from the Esc hotkey handler (the main thread).
fn dictation_cancel() {
    let rec = {
        let mut g = DICT.lock().unwrap_or_else(|e| e.into_inner());
        g.started = None;
        g.rec.take()
    };
    let Some(mut rec) = rec else {
        return;
    };
    log::info!(
        "acceptance:right-command phase=cancelled generation={}",
        rec.generation
    );
    dict_ui_idle(rec.generation);
    std::thread::spawn(move || {
        rec.stop.store(true, Ordering::Relaxed);
        if let Some(h) = rec.handle.take() {
            let _ = h.join();
        }
    });
}

// Cancel key (Esc by default) is watched INSIDE the event tap as a KeyDown — the way Handy does it — NOT via a
// separate global-shortcut registration. Registering/unregistering a global shortcut from within a global
// shortcut handler deadlocked the app: the plugin fires its handler inside its OWN main-thread CGEventTap
// callback, and tauri's run_on_main_thread runs SYNCHRONOUSLY when already on the main thread, so the
// "deferred" unregister re-entered the plugin's mutex and wedged the whole UI. One tap, no re-entrancy.
// DICT_CANCEL_KEY is the macOS keycode to treat as cancel (53 = Esc); dictation_cancel() self-gates on recording.
static DICT_CANCEL_KEY: AtomicI64 = AtomicI64::new(53);
fn cancel_keycode(key: &str) -> i64 {
    match key.trim().to_ascii_lowercase().as_str() {
        "space" | "spacebar" => 49,
        "enter" | "return" => 36,
        "tab" => 48,
        _ => 53, // Esc — the default and the fallback for anything we don't map
    }
}

// ---- rebindable start/stop hotkey -----------------------------------------------------------------
// The dictation shortcut is rebindable from Settings (default Right Command). We keep the currently-registered
// Shortcut so a change can unregister the old one before adding the new. A dictation hotkey MUST carry a
// modifier — a bare global key would be swallowed system-wide.
#[derive(serde::Deserialize, Default)]
struct HotkeySpec {
    #[serde(default)]
    mods: Vec<String>,
    #[serde(default)]
    key: String,
    // A bare-modifier trigger (Handy-style), e.g. "command_right", "option", "control_left". Non-empty → the
    // event tap below drives dictation and no Carbon hotkey is registered (Carbon can't bind a lone modifier).
    #[serde(default)]
    modifier: String,
}
static DICT_HOTKEY: Mutex<Option<tauri_plugin_global_shortcut::Shortcut>> = Mutex::new(None);
static AGENT_HOTKEY: Mutex<Option<tauri_plugin_global_shortcut::Shortcut>> = Mutex::new(None);
static DICT_TRIGGER_LABEL: Mutex<String> = Mutex::new(String::new());

fn dict_trigger_label(hotkey: Option<&HotkeySpec>) -> String {
    let Some(hotkey) = hotkey else {
        return "Right ⌘".into();
    };
    let modifier = match hotkey.modifier.as_str() {
        "command_left" => "Left ⌘",
        "command_right" => "Right ⌘",
        "control_left" => "Left ⌃",
        "control_right" => "Right ⌃",
        "option_left" => "Left ⌥",
        "option_right" => "Right ⌥",
        "shift_left" => "Left ⇧",
        "shift_right" => "Right ⇧",
        _ => "",
    };
    if !modifier.is_empty() {
        return modifier.into();
    }
    let mut label = String::new();
    for item in &hotkey.mods {
        label.push_str(match item.as_str() {
            "cmd" | "command" | "meta" => "⌘",
            "ctrl" | "control" => "⌃",
            "alt" | "opt" | "option" => "⌥",
            "shift" => "⇧",
            _ => "",
        });
    }
    label.push_str(&hotkey.key);
    if label.is_empty() {
        "Shortcut".into()
    } else {
        label
    }
}

fn key_to_code(key: &str) -> Option<tauri_plugin_global_shortcut::Code> {
    use std::str::FromStr;
    let k = key.trim();
    let name = if k.chars().count() == 1 {
        let c = k.chars().next().unwrap().to_ascii_uppercase();
        if c.is_ascii_alphabetic() {
            format!("Key{}", c)
        } else if c.is_ascii_digit() {
            format!("Digit{}", c)
        } else {
            return None;
        }
    } else {
        match k.to_ascii_lowercase().as_str() {
            "space" | "spacebar" => "Space".to_string(),
            "enter" | "return" => "Enter".to_string(),
            "tab" => "Tab".to_string(),
            "esc" | "escape" => "Escape".to_string(),
            "period" | "." => "Period".to_string(),
            "comma" | "," => "Comma".to_string(),
            _ => return None,
        }
    };
    tauri_plugin_global_shortcut::Code::from_str(&name).ok()
}

// tap toggles (or, in push-to-talk, press starts / release stops). The handler no-ops until dictation is on.
fn on_dict_hotkey(state: tauri_plugin_global_shortcut::ShortcutState) {
    use tauri_plugin_global_shortcut::ShortcutState;
    let ptt = DICT_PTT.load(Ordering::Relaxed);
    match state {
        ShortcutState::Pressed => {
            if ptt {
                dictation_start()
            } else {
                dictation_toggle()
            }
        }
        ShortcutState::Released => {
            if ptt {
                dictation_stop_rec()
            }
        }
    }
}

// Swap the registered dictation shortcut. IDEMPOTENT: re-requesting the SAME shortcut (which the UI does on
// every settings-sync + on boot) is a no-op, so we never briefly unregister the working hotkey. A genuine
// change registers the NEW shortcut first and only then drops the OLD, so a failed registration can never
// leave the app with no hotkey at all. Returns false if the spec can't be registered.
fn register_dict_hotkey(app: &AppHandle, spec: &HotkeySpec) -> bool {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Modifiers, Shortcut};
    let Some(code) = key_to_code(&spec.key) else {
        return false;
    };
    let mut mods = Modifiers::empty();
    for m in &spec.mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" | "option" | "opt" => mods |= Modifiers::ALT,
            "cmd" | "meta" | "super" | "win" => mods |= Modifiers::SUPER,
            _ => {}
        }
    }
    if mods.is_empty() {
        return false;
    } // refuse a modifier-less global hotkey — it'd swallow that key everywhere
    let sc = Shortcut::new(Some(mods), code);
    if DICT_HOTKEY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        == Some(&sc)
    {
        return true;
    } // unchanged → already live
    let gs = app.global_shortcut();
    match gs.on_shortcut(sc, |_app, _scut, event| on_dict_hotkey(event.state())) {
        Ok(_) => {
            let mut cur = DICT_HOTKEY.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(prev) = cur.take() {
                if prev != sc {
                    let _ = gs.unregister(prev);
                }
            }
            *cur = Some(sc);
            log::info!("dictation: hotkey registered ({:?} + {:?})", mods, code);
            true
        }
        Err(e) => {
            log::error!(
                "dictation: hotkey registration failed ({:?} + {:?}): {e}",
                mods,
                code
            );
            false
        }
    }
}

fn shortcut_from_spec(spec: &HotkeySpec) -> Option<tauri_plugin_global_shortcut::Shortcut> {
    use tauri_plugin_global_shortcut::{Modifiers, Shortcut};
    let code = key_to_code(&spec.key)?;
    let mut mods = Modifiers::empty();
    for modifier in &spec.mods {
        match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" | "option" | "opt" => mods |= Modifiers::ALT,
            "cmd" | "meta" | "super" | "win" => mods |= Modifiers::SUPER,
            _ => {}
        }
    }
    (!mods.is_empty() && spec.modifier.is_empty()).then(|| Shortcut::new(Some(mods), code))
}

const OPEN_AGENTS_SCRIPT: &str = "window.__myAssistantOpenAgents?.()";

fn open_agents_from_shortcut(window: tauri::WebviewWindow) {
    let main_window = window.clone();
    let _ = window.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        if let Ok(pointer) = main_window.ns_window() {
            use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

            // A hidden app window normally stays attached to its previous Space. For a global
            // Agent shortcut that is unsafe: the context inspector would see the app behind that
            // old window instead of the app where the shortcut was pressed. Move this window to
            // the currently active Space before it becomes key, and never combine the mutually
            // exclusive CanJoinAllSpaces and MoveToActiveSpace behaviors.
            let native_window: &NSWindow = unsafe { &*pointer.cast() };
            let mut behavior = native_window.collectionBehavior();
            behavior.remove(NSWindowCollectionBehavior::CanJoinAllSpaces);
            behavior.insert(NSWindowCollectionBehavior::MoveToActiveSpace);
            native_window.setCollectionBehavior(behavior);
        }

        // Route once while hidden and again after focus. The direct window function remains
        // reliable when WKWebView has throttled a hidden page; a one-shot CustomEvent could be
        // missed while restoring the window.
        let _ = main_window.eval(OPEN_AGENTS_SCRIPT);
        let _ = main_window.show();
        let _ = main_window.unminimize();
        let _ = main_window.set_focus();
        let _ = main_window.eval(OPEN_AGENTS_SCRIPT);
    });
}

#[tauri::command]
fn set_agent_shortcut(app: AppHandle, spec: Option<HotkeySpec>) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    log::info!("agent: global shortcut configuration requested");
    let global = app.global_shortcut();
    let Some(spec) = spec else {
        if let Some(previous) = AGENT_HOTKEY
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = global.unregister(previous);
        }
        log::info!("agent: global shortcut cleared");
        return Ok(());
    };
    let shortcut = shortcut_from_spec(&spec).ok_or_else(|| {
        "Agent shortcut needs a supported key plus at least one modifier".to_string()
    })?;
    if AGENT_HOTKEY
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        == Some(&shortcut)
    {
        log::info!("agent: requested global shortcut is already registered");
        return Ok(());
    }
    if let Err(error) = global.on_shortcut(shortcut, |app, _shortcut, event| {
        if event.state != ShortcutState::Pressed {
            return;
        }
        log::info!("agent: global shortcut pressed");
        if let Some(window) = app.get_webview_window("main") {
            open_agents_from_shortcut(window);
        }
    }) {
        log::error!("agent: global shortcut registration failed: {error}");
        return Err(format!("Agent shortcut could not be registered: {error}"));
    }
    let mut current = AGENT_HOTKEY
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(previous) = current.replace(shortcut) {
        if previous != shortcut {
            let _ = global.unregister(previous);
        }
    }
    log::info!("agent: global shortcut registered");
    Ok(())
}

// ---- bare-modifier trigger (Handy-style: right ⌘, Option, etc. as a hold/tap-to-talk key) --------------
// Carbon RegisterEventHotKey (the plugin above) CANNOT bind a lone modifier. So when the trigger IS a bare
// modifier we run a listen-only CGEventTap on .flagsChanged and watch that modifier's device-flag bit go
// down/up, feeding the SAME on_dict_hotkey(Pressed/Released). Push-to-talk on → hold-to-talk, off →
// tap-to-talk — identical to Handy, and it reuses everything we already built.
static DICT_MOD_MASK: AtomicU64 = AtomicU64::new(0); // device-flag bit to watch; 0 = no modifier trigger (use Carbon)
static DICT_MOD_DOWN: AtomicBool = AtomicBool::new(false); // last observed state of that modifier (edge detection)
static DICT_TAP_UP: AtomicBool = AtomicBool::new(false); // is the event-tap thread live? (guards against a double tap)

// Modifier name (from the UI) → macOS device-dependent flag bit(s). The *_right / *_left device masks are what
// let us tell the two ⌘ keys apart — the generic Command flag can't. Fn has no device bit, so it rides the
// generic SecondaryFn mask. These constants are the classic NX_DEVICE*KEYMASK values.
fn modifier_mask(name: &str) -> u64 {
    match name.trim().to_ascii_lowercase().as_str() {
        "control_left" | "ctrl_left" => 0x0000_0001,
        "shift_left" => 0x0000_0002,
        "shift_right" => 0x0000_0004,
        "command_left" | "cmd_left" => 0x0000_0008,
        "command_right" | "cmd_right" => 0x0000_0010,
        "option_left" | "alt_left" => 0x0000_0020,
        "option_right" | "alt_right" => 0x0000_0040,
        "control_right" | "ctrl_right" => 0x0000_2000,
        // either-side (device bits OR'd together):
        "command" | "cmd" => 0x0000_0018,
        "option" | "alt" | "opt" => 0x0000_0060,
        "control" | "ctrl" => 0x0000_2001,
        "shift" => 0x0000_0006,
        "fn" => 0x0080_0000, // SecondaryFn (Fn has no L/R device bit)
        _ => 0,
    }
}

// Spawned once (and retried when a modifier trigger is first set, in case Accessibility was just granted). A
// dedicated thread owns the tap and its CFRunLoop — the tap is !Send and the loop parks the thread forever.
// The WATCHED modifier is swapped live via DICT_MOD_MASK, so rebinding needs no teardown. Needs Accessibility
// (the same grant the paste uses); if it isn't granted the tap can't be created → we log and give up until relaunch.
fn start_modifier_tap() {
    if DICT_TAP_UP.swap(true, Ordering::SeqCst) {
        return;
    } // already running — don't create a second tap
    std::thread::spawn(|| {
        use core_foundation::runloop::{kCFRunLoopCommonModes, kCFRunLoopDefaultMode, CFRunLoop};
        use core_graphics::event::{
            CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        };
        let tap = CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::FlagsChanged, CGEventType::KeyDown],
            |_proxy, etype, event| {
                // Only modifier-change events carry a trigger. Ignore the TapDisabled* control events macOS
                // delivers here — reading their flags would spoof a spurious press/release.
                if matches!(etype, CGEventType::FlagsChanged) {
                    let mask = DICT_MOD_MASK.load(Ordering::Relaxed);
                    if mask != 0 {
                        // act only when OUR modifier's device bit transitions
                        let down = (event.get_flags().bits() & mask) != 0;
                        if down != DICT_MOD_DOWN.swap(down, Ordering::Relaxed) {
                            // Privacy-safe acceptance breadcrumb: no audio or transcript content. Physical
                            // RC matrix runs can prove edge order/timing from the packaged app's disk log.
                            log::info!(
                                "acceptance:right-command modifier-edge down={down} mask={mask:#x}"
                            );
                            // Marshal to the MAIN thread so start/stop runs on the same thread as the Carbon
                            // hotkey path (keeps CoreAudio/AppKit on main) and keeps this tap callback fast.
                            if let Some(app) = APP.get() {
                                let _ = app.run_on_main_thread(move || {
                                    use tauri_plugin_global_shortcut::ShortcutState;
                                    on_dict_hotkey(if down {
                                        ShortcutState::Pressed
                                    } else {
                                        ShortcutState::Released
                                    });
                                });
                            }
                        }
                    }
                } else if matches!(etype, CGEventType::KeyDown) {
                    // Cancel key (Esc, keycode 53) → discard the recording, Handy-style. field 9 =
                    // kCGKeyboardEventKeycode. dictation_cancel() no-ops when not recording, so this stays a
                    // cheap atomic check on each keystroke; marshaled to main (async from this tap thread).
                    if event.get_integer_value_field(9) == DICT_CANCEL_KEY.load(Ordering::Relaxed) {
                        if let Some(app) = APP.get() {
                            let _ = app.run_on_main_thread(dictation_cancel);
                        }
                    }
                }
                None // listen-only: never modify or swallow the event
            },
        );
        match tap {
            Ok(tap) => match tap.mach_port.create_runloop_source(0) {
                Ok(src) => {
                    let rl = CFRunLoop::get_current();
                    unsafe {
                        rl.add_source(&src, kCFRunLoopCommonModes);
                    }
                    tap.enable();
                    log::info!("dictation: modifier event-tap running");
                    // Run the loop in short slices and re-enable after each: macOS disables an event tap under
                    // load (kCGEventTapDisabledByTimeout), and without re-enabling, the modifier trigger would
                    // silently die until relaunch. This mirrors Handy's handy-keys run loop; enable() is idempotent.
                    loop {
                        CFRunLoop::run_in_mode(
                            unsafe { kCFRunLoopDefaultMode },
                            Duration::from_millis(250),
                            false,
                        );
                        tap.enable();
                    }
                }
                Err(_) => {
                    log::error!("dictation: modifier tap runloop-source failed");
                    DICT_TAP_UP.store(false, Ordering::SeqCst);
                }
            },
            Err(_) => {
                log::error!("dictation: modifier event-tap failed to create (grant Accessibility, then relaunch)");
                DICT_TAP_UP.store(false, Ordering::SeqCst);
            }
        }
    });
}

// UI → push the full dictation config into this process. Everything the in-process dictation owns (sound,
// volume, clipboard, overlay, the rebindable hotkey) is applied here so the Settings rows actually take
// effect. On an EXPLICIT enable (prompt=true) we ask for Accessibility under THIS app's identity so the
// grant covers the paste; boot passes prompt=false so a relaunch never nags.
#[derive(serde::Deserialize)]
struct DictCfg {
    enabled: bool,
    ptt: bool,
    #[serde(default = "def_true")]
    sound: bool,
    #[serde(default = "def_vol")]
    volume: u8,
    #[serde(default)]
    clipboard: String, // "restore" | "save" | "replace"
    #[serde(default = "def_true")]
    overlay: bool,
    #[serde(default)]
    overlay_pos: String, // bottom-center | bottom-right | top-right | top-center | left-center | right-center
    #[serde(default)]
    hotkey: Option<HotkeySpec>,
    #[serde(default)]
    cancel: Option<HotkeySpec>, // the "Cancel" key armed while recording (default Esc)
    #[serde(default)]
    prompt: bool,
}
fn def_true() -> bool {
    true
}
fn def_vol() -> u8 {
    70
}

#[tauri::command]
fn set_dictation(app: AppHandle, cfg: DictCfg) {
    let active_generation = {
        let mut dict = DICT.lock().unwrap_or_else(|e| e.into_inner());
        dict.enabled = cfg.enabled;
        dict.rec.as_ref().map(|rec| rec.generation)
    };
    DICT_PTT.store(cfg.ptt, Ordering::Relaxed);
    {
        let mut p = DICT_PREFS.lock().unwrap_or_else(|e| e.into_inner());
        p.sound = cfg.sound;
        p.volume = (cfg.volume.min(100) as f32) / 100.0;
        p.clipboard = if cfg.clipboard == "restore" || cfg.clipboard.is_empty() {
            0
        } else {
            1
        };
        p.overlay = cfg.overlay;
        p.overlay_pos = match cfg.overlay_pos.as_str() {
            "bottom-right" => 1,
            "top-right" => 2,
            "top-center" => 3,
            "left-center" => 4,
            "right-center" => 5,
            _ => 0,
        };
    }
    *DICT_TRIGGER_LABEL
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = dict_trigger_label(cfg.hotkey.as_ref());
    if !cfg.enabled && active_generation.is_some() {
        dictation_cancel();
    } else if let Some(generation) = active_generation {
        dict_feedback(DictFeedback::Listening, generation);
    }
    // Cancel key (armed only while recording). Unmappable / missing → None → Esc.
    DICT_CANCEL_KEY.store(
        cfg.cancel
            .as_ref()
            .map(|c| cancel_keycode(&c.key))
            .unwrap_or(53),
        Ordering::Relaxed,
    );
    // Trigger: a bare modifier (event tap) OR a Carbon combo — never both at once.
    let mm = cfg
        .hotkey
        .as_ref()
        .map(|hk| modifier_mask(&hk.modifier))
        .unwrap_or(0);
    if cfg.enabled && mm != 0 {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let previous = DICT_MOD_MASK.swap(mm, Ordering::Relaxed);
        // Re-pushing unrelated dictation settings must not erase a key-down edge while the user is tapping.
        if previous != mm {
            DICT_MOD_DOWN.store(false, Ordering::Relaxed);
        }
        // Retire any Carbon hotkey so an old key combination cannot fire alongside the modifier.
        if let Some(prev) = DICT_HOTKEY.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = app.global_shortcut().unregister(prev);
        }
        start_modifier_tap(); // idempotent; also retries the tap now that Accessibility may have just been granted
        log::info!(
            "dictation: modifier trigger set ({} = {:#x})",
            cfg.hotkey
                .as_ref()
                .map(|h| h.modifier.as_str())
                .unwrap_or(""),
            mm
        );
    } else {
        DICT_MOD_MASK.store(0, Ordering::Relaxed); // fall back to a Carbon combo
        if let Some(hk) = cfg.hotkey.as_ref() {
            register_dict_hotkey(&app, hk);
        }
    }
    if cfg.enabled && cfg.prompt {
        // Prompts ONLY when not already trusted; silent once granted (the stable cert makes it stick). This
        // is what re-asks once after a signature change / TCC reset, then never again — the paste needs it.
        let _ = macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
    }
}
// Tray "Stop" → finish the current dictation (only if one is recording).
#[tauri::command]
fn dictation_stop() {
    dictation_stop_rec();
}

// Settings "Show tray icon" → show/hide the menu-bar icon. Tray mutations are AppKit → main thread.
#[tauri::command]
fn set_tray_visible(app: AppHandle, on: bool) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(t) = a.tray_by_id("tray") {
            let _ = t.set_visible(on);
        }
    });
}

// Settings → Permissions: report whether the grants the enabled features need are ACTUALLY in place, so the
// modal shows "Allowed" instead of a blind "Open System Settings". Accessibility is what the hotkey event-tap
// and the ⌘V paste run under (this very process, so its own trust is the right thing to check); Screen
// Recording is needed only for optional video capture. Microphone and System Audio Recording are deliberately
// not guessed here: the UI labels them "Requested on first use" and provides recovery links. The two reliable
// statuses below are checked NON-prompting — opening a System Settings pane is the explicit grant path.
#[derive(serde::Serialize)]
struct PermStatus {
    accessibility: bool,
    screen_recording: bool,
}
#[tauri::command]
fn perm_status() -> PermStatus {
    #[cfg(target_os = "macos")]
    {
        PermStatus {
            accessibility: macos_accessibility_client::accessibility::application_is_trusted(),
            screen_recording: core_graphics::access::ScreenCaptureAccess.preflight(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermStatus {
            accessibility: true,
            screen_recording: true,
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Always initialise logging: in a packaged build this is the only way logs reach disk (the
            // app's stderr goes nowhere on a double-click launch). Writes to the OS log dir.
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    .level(log::LevelFilter::Info)
                    .build(),
            )?;

            init_paths(app); // resolve RES_BASE / DATA_DIR / MODELS_DIR before anything spawns
            let handle = app.handle().clone();
            let notify_handle = handle.clone();
            let server = match local_server::start(
                web_dir(),
                data_dir(),
                MODELS_DIR.get().cloned().unwrap_or_default(),
                res_base(),
                legacy_model_dirs(),
                Arc::new(move |meeting_app| notify_meeting_started(&notify_handle, &meeting_app)),
            ) {
                Ok(server) => server,
                Err(error) => {
                    // No WebView or helper has started yet. Exit immediately and cleanly so a conflicting
                    // listener cannot leave an unreachable tray process or receive a bootstrap request.
                    log::error!("startup refused: {error}");
                    std::process::exit(1);
                }
            };
            let _ = SERVER_TOKEN.set(server.token);
            start_stack();
            let _ = APP.set(handle.clone()); // dictation fns use this for sound/tray/pill feedback
            #[cfg(all(target_os = "macos", debug_assertions))]
            if let Ok(preview) = std::env::var("ASSISTANT_PILL_PREVIEW") {
                let phase = if preview == "transcribing" {
                    DictFeedback::Transcribing
                } else {
                    DictFeedback::Listening
                };
                native_pill::set(phase, 0, 7, "Right ⌘");
            }
            // Create the main WebView only after the local server is listening. The dictation pill and tray
            // are native AppKit surfaces and do not allocate additional WebViews.
            {
                let h = handle.clone();
                std::thread::spawn(move || {
                    for _ in 0..80 {
                        if port_open(8790) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    std::thread::sleep(Duration::from_millis(400)); // let the HTTP route come fully up
                    let h2 = h.clone();
                    let _ = h.run_on_main_thread(move || {
                        // Create the main window FRESH here, after the server answers — NOT from tauri.conf at launch.
                        // A config window can race server startup: its first load fails (connection refused), and a
                        // later navigate() loads the DOM but WKWebView never runs the page's inline <script> on
                        // that recovered frame → the whole UI was a dead static shell (nav clicks, dictation,
                        // settings, config-sync all silently broken; only the pill, built fresh below, worked).
                        // Building main with WebviewWindowBuilder after server startup makes its
                        // inline script actually execute. tauri.conf's windows list is now empty for this reason.
                        if h2.get_webview_window("main").is_none() {
                            let token = SERVER_TOKEN.get().map(String::as_str).unwrap_or("");
                            if let Ok(u) = format!("{}/?bootstrap={}", SERVER, token).parse() {
                                let _ =
                                    WebviewWindowBuilder::new(&h2, "main", WebviewUrl::External(u))
                                        .title("My Assistant")
                                        .inner_size(1120.0, 720.0)
                                        .min_inner_size(900.0, 600.0)
                                        .center()
                                        .resizable(true)
                                        .build();
                            }
                        }
                        if let Some(w) = h2.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                        // The click-through AppKit pill is created lazily by dict_feedback() while visible.
                    });
                });
            }

            // Right Command is the product default. Settings re-pushes the persisted choice after the UI loads.
            DICT_MOD_MASK.store(modifier_mask("command_right"), Ordering::Relaxed);
            start_modifier_tap(); // bring up the bare-modifier event tap (right ⌘ / Option triggers); no-op if Accessibility isn't granted yet

            if std::env::var("ASSISTANT_STEALTH").is_ok() {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_content_protected(true);
                }
            }

            // Left-click drops a reliable NATIVE menu. The custom transparent popover dismissed itself on
            // blur, so clicking the icon just closed it with nothing shown — native menus always work.
            let show = MenuItem::with_id(app, "show", "Show My Assistant", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit My Assistant", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &settings, &sep, &quit])?;
            // The bundle always ships a tray icon, but default_window_icon() is an Option — don't .unwrap()
            // it on the setup path (a None would panic the whole setup closure → the app never launches).
            // Fall back to a text-only tray instead of crashing.
            let mut tray = TrayIconBuilder::with_id("tray")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .tooltip("My Assistant")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    "settings" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                            let _ = w.eval("try{setView('settings')}catch(e){}");
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // macOS convention: the red X HIDES the main window (the app keeps running in the tray).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_stealth,
            set_tray_recording,
            set_dictation,
            set_agent_shortcut,
            dictation_stop,
            set_tray_visible,
            perm_status
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                tauri::RunEvent::Exit => {
                    // Stop the in-process recorder on quit — signal its thread, which drops the cpal Stream and
                    // releases the mic. (It lives in DICT, not CHILDREN, so it wouldn't be caught below.)
                    if let Ok(mut g) = DICT.lock() {
                        if let Some(rec) = g.rec.take() {
                            rec.stop.store(true, Ordering::Relaxed);
                        }
                    }
                    local_server::shutdown();
                    kill_children(); // release the mic on quit (meeting capture + other sidecars)
                }
                // Clicking the Dock icon when no window is open re-shows the main window (macOS convention),
                // so the app is never "running but unreachable" after the red-X hide.
                tauri::RunEvent::Reopen { .. } => {
                    if let Some(w) = app_handle.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.unminimize();
                        let _ = w.set_focus();
                    }
                }
                _ => {}
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictation_pill_uses_the_configured_trigger_label() {
        assert_eq!(
            dict_trigger_label(Some(&HotkeySpec {
                modifier: "command_right".into(),
                ..Default::default()
            })),
            "Right ⌘"
        );
        assert_eq!(
            dict_trigger_label(Some(&HotkeySpec {
                mods: vec!["ctrl".into(), "shift".into()],
                key: "D".into(),
                modifier: String::new(),
            })),
            "⌃⇧D"
        );
    }

    #[test]
    fn agent_shortcut_requires_a_combination_not_a_bare_modifier() {
        assert!(shortcut_from_spec(&HotkeySpec {
            mods: vec!["ctrl".into(), "shift".into()],
            key: "A".into(),
            modifier: String::new(),
        })
        .is_some());
        assert!(shortcut_from_spec(&HotkeySpec {
            modifier: "command_right".into(),
            ..Default::default()
        })
        .is_none());
        assert!(shortcut_from_spec(&HotkeySpec {
            key: "A".into(),
            ..Default::default()
        })
        .is_none());
    }

    #[test]
    fn agent_shortcut_routes_through_the_durable_webview_entrypoint() {
        assert_eq!(OPEN_AGENTS_SCRIPT, "window.__myAssistantOpenAgents?.()");
    }
}
