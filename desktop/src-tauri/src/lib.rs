use std::io::Write;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, Ordering};
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

#[cfg(feature = "agent-research")]
mod codex_runtime;
mod dictation_cleanup;
mod dictation_commands;
mod dictation_context;
mod dictation_diff;
mod dictation_numbers;
mod dictation_polish;
mod local_server;
mod meeting_detection;
#[cfg(target_os = "macos")]
mod native_meeting_pill;
#[cfg(target_os = "macos")]
mod native_pill;
mod polish_models;
mod polish_runtime;
mod polish_validator;
pub mod speech_audio;
mod speech_models;

#[cfg(feature = "agent-research")]
macro_rules! generate_app_handlers {
    () => {
        tauri::generate_handler![
            set_stealth,
            set_tray_recording,
            set_dictation,
            set_agent_shortcut,
            dictation_stop,
            set_tray_visible,
            perm_status
        ]
    };
}

#[cfg(not(feature = "agent-research"))]
macro_rules! generate_app_handlers {
    () => {
        tauri::generate_handler![
            set_stealth,
            set_tray_recording,
            set_dictation,
            dictation_stop,
            set_tray_visible,
            perm_status
        ]
    };
}

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

fn meeting_app_label(meeting_app: &str) -> String {
    match meeting_app.trim().to_ascii_lowercase().as_str() {
        "zoom" => "Zoom".into(),
        "teams" => "Microsoft Teams".into(),
        "meet" => "Google Meet".into(),
        "webex" => "Webex".into(),
        "facetime" => "FaceTime".into(),
        "slack" => "Slack Huddle".into(),
        "discord" => "Discord".into(),
        "your call" | "" | "call" => "your call".into(),
        _ => meeting_app.trim().chars().take(40).collect(),
    }
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
    let label = meeting_app_label(meeting_app);
    let (title, body) = if label == "your call" {
        (
            "Meeting detected".to_string(),
            "My Assistant is checking microphone and meeting audio. Use the control above your meeting to open or stop.".to_string(),
        )
    } else {
        (
            format!("Meeting detected · {label}"),
            format!("My Assistant is checking microphone and {label} audio. Use the control above your meeting to open or stop."),
        )
    };
    if let Err(error) = notifications.builder().title(title).body(body).show() {
        log::warn!("meeting notification failed: {error}");
    }
}

pub(crate) fn show_main_window() {
    let Some(app) = APP.get() else { return };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn present_native_catch_up(result: Result<String, String>) {
    let Some(app) = APP.get() else { return };
    let _ = app.run_on_main_thread(move || native_meeting_pill::set_assist_result(result));
}

fn present_meeting_state(app: &AppHandle, state: local_server::MeetingUiState) {
    let recording = state.phase == "recording";
    let status = match state.phase.as_str() {
        "starting" => " Starting meeting…",
        "needs_attention" => " ! Meeting needs attention",
        "pausing" => " … Pausing meeting",
        "paused" => " Ⅱ Meeting paused",
        "finalizing" => " … Finalizing meeting",
        _ => "",
    };
    *MEETING_TRAY_STATUS
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = status.into();
    set_tray_recording(app.clone(), recording);

    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        native_meeting_pill::set(&state);
    });
}

#[cfg(all(target_os = "macos", debug_assertions))]
fn meeting_pill_preview_state(phase: String) -> local_server::MeetingUiState {
    let detail = match phase.as_str() {
        "recording" => "Microphone + meeting audio confirmed",
        "needs_attention" => "Meeting audio unavailable · open My Assistant to fix it",
        "pausing" => "Releasing microphone and meeting audio",
        "paused" => "Microphone and meeting audio released",
        "finalizing" => "Stopping capture, saving transcript, and preparing notes",
        _ => "Checking microphone + meeting audio",
    };
    local_server::MeetingUiState {
        phase,
        title: "Customer launch review".into(),
        app: "zoom".into(),
        started_at: Some(0),
        microphone: "ready".into(),
        meeting_audio: "ready".into(),
        transcription: "ready".into(),
        detail: detail.into(),
        transcript: vec![
            local_server::MeetingUiLine {
                speaker: "Aditi".into(),
                text: "Let’s keep the launch date and confirm the final owner.".into(),
                is_self: false,
            },
            local_server::MeetingUiLine {
                speaker: "You".into(),
                text: "I’ll send the revised rollout plan after this call.".into(),
                is_self: true,
            },
        ],
    }
}

// Sidecars the app spawns — all killed when it quits (so the mic is never left held).
static CHILDREN: Mutex<Vec<Child>> = Mutex::new(Vec::new());
static CAPTURE_HELPER_SUPERVISOR_STOP: AtomicBool = AtomicBool::new(false);
fn kill_children() {
    CAPTURE_HELPER_SUPERVISOR_STOP.store(true, Ordering::SeqCst);
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

fn capture_helper_exited(pid: u32) -> bool {
    let Ok(mut children) = CHILDREN.lock() else {
        return false;
    };
    let Some(index) = children.iter().position(|child| child.id() == pid) else {
        return true;
    };
    match children[index].try_wait() {
        Ok(Some(status)) => {
            log::warn!("meeting capture helper exited pid={pid} status={status}");
            // `try_wait(Some)` already reaped the process. A second wait returns the cached status and
            // makes the ownership/reaping contract explicit before the Child handle is dropped.
            let mut child = children.remove(index);
            let _ = child.wait();
            true
        }
        Ok(None) => false,
        Err(error) => {
            log::warn!("meeting capture helper status failed pid={pid}: {error}");
            let mut child = children.remove(index);
            let _ = child.kill();
            let _ = child.wait();
            true
        }
    }
}

fn spawn_meeting_capture_helper() -> Option<u32> {
    let tap = format!("{}/stt/meeting-tap", res_base());
    if !Path::new(&tap).exists() {
        return None;
    }
    let recdir = format!("{}/meetings/rec", data_dir());
    let temp = format!("{}/tmp", data_dir());
    let token = SERVER_TOKEN.get().cloned().unwrap_or_default();
    let parent_pid = std::process::id().to_string();
    let child = Command::new(tap)
        .args([
            "8790",
            // Keep provider selection and credentials in Rust. The capture helper sends private WAV bytes
            // only to the authenticated in-process API and never learns a private-worker bearer token.
            "http://127.0.0.1:8790/api/transcribe",
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
        .spawn()
        .ok()?;
    let pid = child.id();
    CHILDREN
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(child);
    Some(pid)
}

fn supervise_meeting_capture_helper() {
    std::thread::spawn(|| {
        let mut consecutive_exits = 0u32;
        while !CAPTURE_HELPER_SUPERVISOR_STOP.load(Ordering::SeqCst) {
            let Some(pid) = spawn_meeting_capture_helper() else {
                consecutive_exits = consecutive_exits.saturating_add(1);
                std::thread::sleep(capture_helper_restart_delay(consecutive_exits));
                continue;
            };
            let started = Instant::now();
            while !CAPTURE_HELPER_SUPERVISOR_STOP.load(Ordering::SeqCst)
                && !capture_helper_exited(pid)
            {
                std::thread::sleep(Duration::from_millis(500));
            }
            if CAPTURE_HELPER_SUPERVISOR_STOP.load(Ordering::SeqCst) {
                return;
            }
            consecutive_exits = if started.elapsed() >= Duration::from_secs(30) {
                1
            } else {
                consecutive_exits.saturating_add(1)
            };
            let delay = capture_helper_restart_delay(consecutive_exits);
            log::warn!(
                "restarting meeting capture helper attempt={} delayMs={}",
                consecutive_exits,
                delay.as_millis()
            );
            std::thread::sleep(delay);
        }
    });
}

fn capture_helper_restart_delay(consecutive_exits: u32) -> Duration {
    Duration::from_millis(match consecutive_exits {
        0 | 1 => 500,
        2 => 1_000,
        3 => 2_000,
        _ => 5_000,
    })
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
    CAPTURE_HELPER_SUPERVISOR_STOP.store(false, Ordering::SeqCst);
    supervise_meeting_capture_helper();
    start_meeting_state(); // names + participants — direct child so Accessibility works
}

// ---- Tauri commands the UI calls (window.__TAURI__.core.invoke) ------------------------------------
#[tauri::command]
fn set_stealth(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if let Some(app) = APP.get() {
        let _ = app.run_on_main_thread(move || {
            native_pill::set_content_protected(on);
            native_meeting_pill::set_content_protected(on);
        });
    }
    window.set_content_protected(on).map_err(|e| e.to_string())
}

// Animate the menu-bar tray icon only after both meeting channels are ready. Non-recording lifecycle text is
// retained here so the waveform thread and transient dictation feedback restore the authoritative status.
static TRAY_REC: AtomicBool = AtomicBool::new(false);
static MEETING_TRAY_STATUS: Mutex<String> = Mutex::new(String::new());
fn meeting_tray_status() -> String {
    MEETING_TRAY_STATUS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}
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
            set_title(&app, meeting_tray_status()); // None does not clear title text on macOS
        });
    } else {
        TRAY_REC.store(false, Ordering::SeqCst);
        set_title(&app, meeting_tray_status());
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
    context: Arc<Mutex<dictation_context::DictationContext>>,
    target: dictation_context::DictationTarget,
    handle: Option<JoinHandle<()>>,
    live_handle: Option<JoinHandle<()>>,
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

// The AppKit pill reads this lock-free meter on its main-thread animation tick. Store the smoothed f32 as
// bits because AtomicF32 is not available in std. The capture callback is the only writer while recording.
static DICT_AUDIO_LEVEL: AtomicU32 = AtomicU32::new(0);

fn normalized_audio_level(rms: f32) -> f32 {
    if !rms.is_finite() || rms <= 0.0 {
        return 0.0;
    }
    // Treat roughly -48 dBFS as the room-noise floor and -20 dBFS as a full visual response. Laptop mics at
    // normal speaking distance rarely approach digital full scale; reserving the top for -12 dB made nearly
    // all real speech look like the same tiny movement.
    let db = 20.0 * rms.max(0.000_001).log10();
    let level = ((db + 48.0) / 28.0).clamp(0.0, 1.0);
    if level < 0.06 {
        0.0
    } else {
        level
    }
}

fn append_dict_samples<I>(buffer: &mut Vec<f32>, mono_samples: I)
where
    I: Iterator<Item = f32>,
{
    let mut sum_squares = 0.0f32;
    let mut peak = 0.0f32;
    let mut count = 0usize;
    for sample in mono_samples {
        let sample = sample.clamp(-1.0, 1.0);
        buffer.push(sample);
        sum_squares += sample * sample;
        peak = peak.max(sample.abs());
        count += 1;
    }
    if count == 0 {
        return;
    }
    let rms = (sum_squares / count as f32).sqrt();
    // RMS supplies stability; a restrained peak contribution makes consonants and short syllables visible
    // instead of disappearing between 80 ms UI ticks.
    let measured = normalized_audio_level(rms.max(peak * 0.35));
    let previous = f32::from_bits(DICT_AUDIO_LEVEL.load(Ordering::Relaxed)).clamp(0.0, 1.0);
    let response = if measured > previous { 0.72 } else { 0.20 };
    let smoothed = previous + (measured - previous) * response;
    DICT_AUDIO_LEVEL.store(smoothed.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

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
                    let mut b = s.lock().unwrap_or_else(|error| error.into_inner());
                    append_dict_samples(&mut b, d.chunks(ch).map(|frame| frame[0]));
                },
                err,
                None,
            ),
            cpal::SampleFormat::I16 => dev.build_input_stream(
                &scfg,
                move |d: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut b = s.lock().unwrap_or_else(|error| error.into_inner());
                    append_dict_samples(
                        &mut b,
                        d.chunks(ch).map(|frame| frame[0] as f32 / 32768.0),
                    );
                },
                err,
                None,
            ),
            cpal::SampleFormat::U16 => dev.build_input_stream(
                &scfg,
                move |d: &[u16], _: &cpal::InputCallbackInfo| {
                    let mut b = s.lock().unwrap_or_else(|error| error.into_inner());
                    append_dict_samples(
                        &mut b,
                        d.chunks(ch)
                            .map(|frame| (frame[0] as f32 - 32768.0) / 32768.0),
                    );
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

fn clean_live_hypothesis(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control() || character.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(600)
        .collect()
}

fn join_live_segments(prefix: &str, tail: &str) -> String {
    if prefix.is_empty() {
        return tail.to_string();
    }
    if tail.is_empty() {
        return prefix.to_string();
    }
    let tail_attaches = tail
        .chars()
        .next()
        .is_some_and(|character| ".,!?;:%…)]}".contains(character));
    let prefix_attaches = prefix
        .chars()
        .next_back()
        .is_some_and(|character| "([{#@/₹$€£".contains(character));
    format!(
        "{prefix}{}{tail}",
        if tail_attaches || prefix_attaches {
            ""
        } else {
            " "
        }
    )
}

fn live_display_text(
    snapshot: &local_server::LiveStreamSnapshot,
    smart: &DictSmartPrefs,
    context: dictation_context::DictationContext,
) -> String {
    let full = clean_live_hypothesis(&snapshot.full);
    let committed = clean_live_hypothesis(&snapshot.committed);
    let tentative = clean_live_hypothesis(&snapshot.tentative);
    let segmented = join_live_segments(&committed, &tentative);
    if !smart.enabled
        || !smart.live_cleanup
        || context.kind != dictation_cleanup::DictationTargetKind::NaturalLanguage
        || committed.is_empty()
    {
        return if full.is_empty() { segmented } else { full };
    }

    let cleaned = dictation_cleanup::cleanup(dictation_cleanup::CleanupRequest {
        text: committed,
        language: smart.language.clone(),
        target: context.kind,
        starts_sentence: context.starts_sentence,
        vocabulary: smart.vocabulary.clone(),
    });
    join_live_segments(&cleaned.text, &tentative)
}

fn spawn_live_preview(
    generation: u64,
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: Arc<AtomicU32>,
    target: dictation_context::DictationTarget,
) -> Option<JoinHandle<()>> {
    if !local_server::active_speech_supports_live_streaming() {
        return None;
    }
    Some(std::thread::spawn(move || {
        let live_started = Instant::now();
        let mut displayed_updates = 0u32;
        let smart = dict_smart_prefs();
        if let Err(error) = local_server::live_stream_begin(generation) {
            log::warn!("dictation live preview skipped generation={generation}: {error}");
            return;
        }
        if stop.load(Ordering::Relaxed) {
            local_server::live_stream_reset(generation);
            return;
        }
        let mut resampler = speech_audio::StreamingResampler16k::default();
        let mut last_displayed = String::new();
        loop {
            let stopping = stop.load(Ordering::Relaxed);
            let source_rate = rate.load(Ordering::Relaxed);
            let pcm = {
                let samples = samples.lock().unwrap_or_else(|error| error.into_inner());
                resampler.take_available(&samples, source_rate)
            };
            if !pcm.is_empty() {
                match local_server::live_stream_feed(generation, &pcm) {
                    Ok(Some(snapshot)) => {
                        let context_handle = target.context();
                        let context = *context_handle
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        let hypothesis = live_display_text(&snapshot, &smart, context);
                        if !hypothesis.is_empty() && hypothesis != last_displayed {
                            if !target.preview(hypothesis.clone()) {
                                log::warn!(
                                    "dictation live preview target stopped generation={generation} updates={displayed_updates}"
                                );
                                local_server::live_stream_reset(generation);
                                return;
                            }
                            displayed_updates += 1;
                            if displayed_updates == 1 {
                                log::info!(
                                    "dictation live preview visible generation={generation} firstUpdateMs={}",
                                    live_started.elapsed().as_millis()
                                );
                            }
                            last_displayed = hypothesis.clone();
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log::warn!(
                            "dictation live preview stopped generation={generation}: {error}"
                        );
                        break;
                    }
                }
            }
            if stopping {
                break;
            }
            std::thread::sleep(Duration::from_millis(240));
        }
        match local_server::live_stream_finalize(generation) {
            Ok(Some(snapshot)) => {
                let context_handle = target.context();
                let context = *context_handle
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let hypothesis = live_display_text(&snapshot, &smart, context);
                if !hypothesis.is_empty()
                    && hypothesis != last_displayed
                    && target.preview(hypothesis)
                {
                    displayed_updates += 1;
                }
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("dictation live finalization failed generation={generation}: {error}");
                local_server::live_stream_reset(generation);
            }
        }
        log::info!(
            "dictation live preview complete generation={generation} updates={displayed_updates} elapsedMs={}",
            live_started.elapsed().as_millis()
        );
    }))
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

#[derive(Clone)]
struct DictSmartPrefs {
    enabled: bool,
    live_cleanup: bool,
    model_id: String,
    language: String,
    vocabulary: Vec<dictation_cleanup::VocabularyTerm>,
}
static DICT_SMART_PREFS: Mutex<Option<DictSmartPrefs>> = Mutex::new(None);

fn dict_smart_prefs() -> DictSmartPrefs {
    DICT_SMART_PREFS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
        .unwrap_or_else(|| DictSmartPrefs {
            enabled: false,
            live_cleanup: false,
            model_id: polish_models::DEFAULT_POLISH_MODEL_ID.to_string(),
            language: "auto".into(),
            vocabulary: Vec::new(),
        })
}
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
    PostProcessing,
    ShowingChanges,
    Inserted,
    Fallback,
    Copied,
    NoSpeech,
    Error,
    Idle,
}

fn dict_feedback(phase: DictFeedback, generation: u64) {
    if let Some(app) = APP.get() {
        let (_, _, _, overlay, overlay_pos) = dict_prefs();
        let elapsed_millis = DICT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .started
            .map(|started| started.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let app = app.clone();
        let handle = app.clone();
        let _ = handle.run_on_main_thread(move || {
            if DICT.lock().unwrap_or_else(|e| e.into_inner()).generation != generation {
                return;
            }
            if let Some(t) = app.tray_by_id("tray") {
                let title = match phase {
                    DictFeedback::Listening => " ● Dictating".into(),
                    DictFeedback::Transcribing => " ● Transcribing".into(),
                    DictFeedback::PostProcessing => " ● Post-processing".into(),
                    DictFeedback::ShowingChanges => " ● Changes".into(),
                    DictFeedback::Inserted | DictFeedback::Fallback => " ✓ Inserted".into(),
                    DictFeedback::Copied => " Dictation copied".into(),
                    DictFeedback::NoSpeech => " No speech".into(),
                    DictFeedback::Error => " Dictation error".into(),
                    DictFeedback::Idle => meeting_tray_status(),
                };
                let _ = t.set_title(Some(title));
            }
            #[cfg(target_os = "macos")]
            native_pill::set(
                if overlay { phase } else { DictFeedback::Idle },
                overlay_pos,
                elapsed_millis,
                f32::from_bits(DICT_AUDIO_LEVEL.load(Ordering::Relaxed)).clamp(0.0, 1.0),
            );
        });
    }
}

fn dict_change_feedback(
    generation: u64,
    original: String,
    replacement: String,
    additional_changes: usize,
) {
    let Some(app) = APP.get() else {
        return;
    };
    let (_, _, _, overlay, overlay_pos) = dict_prefs();
    let app = app.clone();
    let handle = app.clone();
    let _ = handle.run_on_main_thread(move || {
        if DICT
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .generation
            != generation
        {
            return;
        }
        if let Some(tray) = app.tray_by_id("tray") {
            let _ = tray.set_title(Some(" ● Changes"));
        }
        #[cfg(target_os = "macos")]
        if overlay {
            native_pill::set_change(overlay_pos, &original, &replacement, additional_changes);
        } else {
            native_pill::set(DictFeedback::Idle, overlay_pos, 0, 0.0);
        }
    });
}

fn dict_processing_feedback(phase: DictFeedback, generation: u64, _text: String) {
    // Provisional and reconciled transcript text belongs in the focused editor. The pill remains a compact
    // recording/status control; only the short cut/replace proof is rendered there after processing.
    dict_feedback(phase, generation);
}

// Drive the native waveform from the capture callback's real microphone envelope. Tick only the pill here;
// the menu-bar title does not need to be redrawn twelve times per second.
#[cfg(target_os = "macos")]
fn dict_pill_tick(generation: u64) {
    let Some(app) = APP.get() else {
        return;
    };
    let (_, _, _, overlay, overlay_pos) = dict_prefs();
    let (active, elapsed_millis) = {
        let dict = DICT.lock().unwrap_or_else(|error| error.into_inner());
        (
            dict.generation == generation && dict.rec.is_some(),
            dict.started
                .map(|started| started.elapsed().as_millis() as u64)
                .unwrap_or(0),
        )
    };
    if !active {
        return;
    }
    let audio_level = f32::from_bits(DICT_AUDIO_LEVEL.load(Ordering::Relaxed)).clamp(0.0, 1.0);
    let app = app.clone();
    let handle = app.clone();
    let _ = handle.run_on_main_thread(move || {
        let still_listening = {
            let dict = DICT.lock().unwrap_or_else(|error| error.into_inner());
            dict.generation == generation && dict.rec.is_some()
        };
        if still_listening {
            let phase = if overlay {
                DictFeedback::Listening
            } else {
                DictFeedback::Idle
            };
            native_pill::set(phase, overlay_pos, elapsed_millis, audio_level);
        }
    });
}

fn dict_set_clip(s: &str) -> Result<(), String> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|error| format!("clipboard write could not start: {error}"))?;
    child
        .stdin
        .take()
        .ok_or("clipboard writer was unavailable")?
        .write_all(s.as_bytes())
        .map_err(|error| format!("clipboard write failed: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("clipboard writer did not finish: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("clipboard writer failed".into())
    }
}
fn dict_get_clip() -> Result<String, String> {
    Command::new("pbpaste")
        .output()
        .map_err(|error| format!("clipboard read failed: {error}"))
        .and_then(|output| {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                Err("clipboard reader failed".into())
            }
        })
}
// Paste = set clipboard + synth ⌘V + restore the old clipboard. enigo posts the keystroke from THIS
// process, so the app's Accessibility grant authorizes it (the correct identity — the whole fix).
// ponytail: 180ms restore delay — enough for the target to read the paste; a slow first paste could miss.
fn dict_paste(text: &str) -> Result<(), String> {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    if !macos_accessibility_client::accessibility::application_is_trusted() {
        return Err("Accessibility permission is not available".into());
    }
    // Build both events before touching the clipboard. A construction failure therefore cannot replace the
    // user's clipboard without delivering a paste.
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| "keyboard event source is unavailable")?;
    let down = CGEvent::new_keyboard_event(source.clone(), 9, true)
        .map_err(|_| "paste key-down event could not be created")?;
    let up = CGEvent::new_keyboard_event(source, 9, false)
        .map_err(|_| "paste key-up event could not be created")?;
    down.set_flags(CGEventFlags::CGEventFlagCommand);
    up.set_flags(CGEventFlags::CGEventFlagCommand);

    // "Clipboard behaviour": Never save (0) → snapshot the old clipboard and restore it after the paste;
    // Save/Replace (1) → leave the dictated text on the clipboard (skip the restore).
    let (_, _, clip, _, _) = dict_prefs();
    let restore = clip == 0;
    let old = if restore {
        Some(dict_get_clip()?)
    } else {
        None
    };
    dict_set_clip(text)?;
    // Synthesize ⌘V with the Command flag set EXPLICITLY on the V (keycode 9) event — the proven,
    // deterministic paste (same as the old working meeting-tap code), not enigo's modifier tracking which
    // silently produced nothing. Needs Accessibility (the app's grant). Thread-safe (no HIToolbox layout call).
    down.post(CGEventTapLocation::HID);
    up.post(CGEventTapLocation::HID);
    std::thread::sleep(Duration::from_millis(250));
    if let Some(old) = old {
        // The insertion already happened. A restore failure should be logged but must not falsely report that
        // the target missed the dictation.
        if let Err(error) = dict_set_clip(&old) {
            log::warn!("dictation clipboard restore failed: {error}");
        }
    }
    Ok(())
}

// Two modes off one trigger: tap-to-talk toggles on each press; push-to-talk starts on press and
// stops on release. Ordinary key combinations use Carbon and do not need Input Monitoring. Bare-modifier
// triggers such as the default Right Command use the separate event tap and do need Input Monitoring.
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
    DICT_AUDIO_LEVEL.store(0, Ordering::Relaxed);
    let target = dictation_context::classify_async(res_base());
    let context = target.context();
    let handle = spawn_capture(stop.clone(), samples.clone(), rate.clone());
    let live_handle = spawn_live_preview(
        generation,
        stop.clone(),
        samples.clone(),
        rate.clone(),
        target.clone(),
    );
    g.rec = Some(Recorder {
        generation,
        stop,
        samples,
        rate,
        context,
        target,
        handle: Some(handle),
        live_handle,
    });
    g.started = Some(Instant::now());
    drop(g);
    // Start the heavy speech engine while capture is in progress. This removes model verification/startup
    // from the visible finishing path in the common case and is safe to repeat after an idle unload.
    local_server::warm_speech_model();
    let smart = dict_smart_prefs();
    if smart.enabled {
        polish_runtime::warm(smart.model_id);
    }
    log::info!("acceptance:right-command phase=listening generation={generation}");
    play("Tink");
    dict_feedback(DictFeedback::Listening, generation);
    emit_dstate("listening", generation);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(80));
        let active = {
            let dict = DICT.lock().unwrap_or_else(|e| e.into_inner());
            dict.generation == generation && dict.rec.is_some()
        };
        if !active {
            break;
        }
        #[cfg(target_os = "macos")]
        dict_pill_tick(generation);
    });
    // Esc-to-cancel is watched inside the event tap (keycode 53) and gated on recording — no separate grab.
}
fn dictation_stop_rec() {
    let mut g = DICT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(rec) = g.rec.take() else {
        return;
    };
    rec.stop.store(true, Ordering::Relaxed);
    DICT_AUDIO_LEVEL.store(0, Ordering::Relaxed);
    let ms = g
        .started
        .map(|t| t.elapsed().as_millis() as u64)
        .unwrap_or(0);
    g.started = None;
    drop(g);
    let finishing_phase = if rec.target.has_preview() {
        (DictFeedback::PostProcessing, "post-processing")
    } else {
        (DictFeedback::Transcribing, "transcribing")
    };
    log::info!(
        "acceptance:right-command phase={} generation={} capturedMs={ms}",
        finishing_phase.1,
        rec.generation,
    );
    play("Pop"); // immediate stop cue (encode + transcription run on the thread below)
                 // Stop must be visible immediately. VAD/model startup happens on the worker and can take noticeable time;
                 // leaving the old "Listening" state up during that work makes a successful second tap look ignored.
    dict_processing_feedback(finishing_phase.0, rec.generation, String::new());
    emit_dstate(finishing_phase.1, rec.generation);
    let generation = rec.generation;
    let job = Arc::new(DictationJob::new());
    let watchdog = Arc::clone(&job);
    let timeout_target = rec.target.clone();
    std::thread::spawn(move || {
        std::thread::sleep(DICTATION_PROCESSING_TIMEOUT);
        if watchdog.timeout() {
            // A timeout must not strand a provisional span in the editor. The helper removes only the
            // exact text owned by this dictation session, so this remains safe even if focus changed.
            let _ = timeout_target.cancel();
            log::warn!(
                "dictation processing deadline reached generation={generation} deadlineMs={}",
                DICTATION_PROCESSING_TIMEOUT.as_millis()
            );
            emit_dstate("error", generation);
            dict_feedback(DictFeedback::Error, generation);
            std::thread::sleep(Duration::from_millis(1200));
            dict_ui_idle(generation);
            watchdog.complete();
        }
    });
    std::thread::spawn(move || dictation_finish(rec, ms, job));
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
const DICTATION_PROCESSING_TIMEOUT: Duration = Duration::from_secs(40);
const DICT_JOB_RUNNING: u8 = 0;
const DICT_JOB_FINISHING: u8 = 1;
const DICT_JOB_TIMED_OUT: u8 = 2;
const DICT_JOB_COMPLETE: u8 = 3;

/// A per-utterance terminal-state arbiter. The worker must claim finishing before it can paste; the watchdog
/// can claim timeout only while work is still running. That prevents a late transcription from appearing in
/// another app after the UI has already reported a timeout.
struct DictationJob {
    state: AtomicU8,
}
impl DictationJob {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(DICT_JOB_RUNNING),
        }
    }

    fn running(&self) -> bool {
        self.state.load(Ordering::Acquire) == DICT_JOB_RUNNING
    }

    fn claim_finish(&self) -> bool {
        self.state
            .compare_exchange(
                DICT_JOB_RUNNING,
                DICT_JOB_FINISHING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn timeout(&self) -> bool {
        self.state
            .compare_exchange(
                DICT_JOB_RUNNING,
                DICT_JOB_TIMED_OUT,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    fn complete(&self) {
        self.state.store(DICT_JOB_COMPLETE, Ordering::Release);
    }
}
// Reset the pill + tray to idle — but ONLY if no newer dictation is already recording. dictation_finish runs
// on a detached thread and is bounded by a watchdog; by the time it ends the user may have started
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
fn dictation_finish(mut rec: Recorder, ms: u64, job: Arc<DictationJob>) {
    let generation = rec.generation;
    let finish_started = Instant::now();
    rec.stop.store(true, Ordering::Relaxed);
    if let Some(h) = rec.handle.take() {
        let _ = h.join();
    } // stream fully stopped, buffer settled
    if let Some(h) = rec.live_handle.take() {
        let _ = h.join();
    } // the selected model's incremental session finalized/reset before authoritative inference
    let rate = rec.rate.load(Ordering::Relaxed);
    let raw = std::mem::take(
        &mut *rec
            .samples
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
    );
    log::info!(
        "dictation capture settled generation={generation} samples={} settleMs={}",
        raw.len(),
        finish_started.elapsed().as_millis()
    );
    // The selected model owns its audio policy. Whisper compacts silence; Parakeet, Nemotron, and the
    // Hinglish specialists retain acoustic context after a speech-presence gate.
    let speech = speech_audio::preprocess_speech(
        &raw,
        rate,
        local_server::active_speech_preprocessing_policy(),
    );
    if !job.running() {
        return;
    }
    // Silero VAD: keep only speech → no hallucinated loop on silence
    // Acquire AFTER the VAD (per-clip, safe to run in parallel): serialize only the shared tail — the /tmp
    // WAV write, the upload, and the clipboard save/paste/restore — so overlapping finishes can't clobber
    // each other. Recover the guard on poison instead of panicking the finish thread.
    let _finish = DICT_FINISH.lock().unwrap_or_else(|e| e.into_inner());
    if !job.running() {
        return;
    }
    let seq = DICT_TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let wav = Path::new(&data_dir())
        .join("tmp")
        .join(format!("dictation-{}-{seq}.wav", std::process::id()));
    // No speech detected → skip the engine entirely (no paste, no hallucinated text).
    if speech.is_empty() {
        if !job.claim_finish() {
            return;
        }
        let _ = std::fs::remove_file(&wav);
        let _ = rec.target.cancel();
        emit_dstate("no-speech", generation);
        dict_feedback(DictFeedback::NoSpeech, generation);
        drop(_finish);
        std::thread::sleep(Duration::from_millis(900));
        dict_ui_idle(generation);
        job.complete();
        return;
    }
    if !write_wav16k(&wav, &speech) {
        if !job.claim_finish() {
            return;
        }
        let _ = std::fs::remove_file(&wav);
        let _ = rec.target.cancel();
        log::warn!("dictation audio encoding failed generation={generation}");
        emit_dstate("error", generation);
        dict_feedback(DictFeedback::Error, generation);
        drop(_finish);
        std::thread::sleep(Duration::from_millis(1200));
        dict_ui_idle(generation);
        job.complete();
        return;
    }
    let auth = format!(
        "Authorization: Bearer {}",
        SERVER_TOKEN.get().map(String::as_str).unwrap_or("")
    );
    let file_form = format!("file=@{}", wav.to_string_lossy());
    let transcription_started = Instant::now();
    let out = Command::new("/usr/bin/curl")
        .args([
            "-sS",
            "--connect-timeout",
            "2",
            "--max-time",
            "25",
            "-H",
            &auth,
            &format!("{}/api/transcribe", SERVER),
            "-F",
            &file_form,
        ])
        .output();
    let _ = std::fs::remove_file(&wav);
    if !job.running() {
        return;
    }
    let response = out
        .as_ref()
        .ok()
        .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok());
    let raw_text = response
        .as_ref()
        .and_then(|value| {
            value
                .get("text")
                .and_then(|t| t.as_str())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default();
    let transport_ok = out
        .as_ref()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !transport_ok || raw_text.is_empty() {
        let reason = response
            .as_ref()
            .and_then(|value| value.get("error"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| {
                out.as_ref()
                    .ok()
                    .map(|value| String::from_utf8_lossy(&value.stderr).trim().to_string())
            })
            .filter(|value| !value.is_empty())
            .or_else(|| out.as_ref().err().map(ToString::to_string))
            .unwrap_or_else(|| "Transcription returned no text".into());
        if !job.claim_finish() {
            return;
        }
        let _ = rec.target.cancel();
        log::warn!(
            "dictation transcription failed generation={generation} elapsedMs={} reason={reason}",
            transcription_started.elapsed().as_millis()
        );
        emit_dstate("error", generation);
        dict_feedback(DictFeedback::Error, generation);
        drop(_finish);
        std::thread::sleep(Duration::from_millis(1200));
        dict_ui_idle(generation);
        job.complete();
        return;
    }
    log::info!(
        "dictation transcription complete generation={generation} elapsedMs={}",
        transcription_started.elapsed().as_millis()
    );

    let smart = dict_smart_prefs();
    let context = *rec
        .context
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // Once live text is already visible, final recognition is perceived as reconciliation and cleanup. Keep
    // that whole wait under one truthful post-processing state; final-only models retain Transcribing until
    // recognition finishes, then enter this state only when Smart Polish is actually enabled.
    if rec.target.has_preview() || smart.enabled {
        emit_dstate("post-processing", generation);
        dict_processing_feedback(DictFeedback::PostProcessing, generation, raw_text.clone());
    }
    let processing_started = Instant::now();
    let mut final_text = raw_text.clone();
    let mut mechanical = None;
    let mut model_accepted = false;
    let mut fallback = false;
    let mut decision = "smart_polish_disabled".to_string();

    if !job.running() {
        return;
    }
    if smart.enabled {
        let cleaned = dictation_cleanup::cleanup(dictation_cleanup::CleanupRequest {
            text: raw_text.clone(),
            language: smart.language.clone(),
            target: context.kind,
            starts_sentence: context.starts_sentence,
            vocabulary: smart.vocabulary.clone(),
        });
        final_text = cleaned.text.clone();
        // Unknown targets get conservative deterministic cleanup only. Contextual model editing is enabled
        // solely when the bounded Accessibility classifier positively identifies natural-language text.
        let model_allowed = context.kind == dictation_cleanup::DictationTargetKind::NaturalLanguage;
        if model_allowed {
            let outcome = dictation_polish::run(
                &cleaned.text,
                &smart.model_id,
                &cleaned.protected,
                &smart.vocabulary,
            );
            final_text = outcome.text;
            model_accepted = outcome.accepted;
            fallback = outcome.fallback;
            decision = outcome.decision;
            if !job.running() {
                return;
            }
        } else {
            decision = match context.kind {
                dictation_cleanup::DictationTargetKind::Secure => "secure_target_model_skipped",
                dictation_cleanup::DictationTargetKind::CodeOrTerminal => {
                    "code_target_model_skipped"
                }
                _ => "model_skipped",
            }
            .into();
        }
        mechanical = Some(cleaned);
    }

    let secure = context.kind == dictation_cleanup::DictationTargetKind::Secure;
    // Secure-field text must never enter the overlay, even as a short changed span.
    let visual_changes = if secure {
        Vec::new()
    } else {
        dictation_diff::diff_spans(&raw_text, &final_text)
    };
    let change_preview = dictation_diff::most_meaningful(&visual_changes).map(|change| {
        (
            change.original.clone(),
            change.replacement.clone(),
            visual_changes.len().saturating_sub(1),
        )
    });
    let showed_changes = change_preview.is_some();
    if let Some((original, replacement, additional_changes)) = change_preview {
        emit_dstate("showing-changes", generation);
        dict_change_feedback(generation, original, replacement, additional_changes);
    }

    // Claim the terminal path before touching the target. If the watchdog won, this worker exits without a
    // late paste; if this worker wins, the remaining clipboard/UI work is short and deterministic.
    if !job.claim_finish() {
        return;
    }
    // A streaming-compatible editor already contains a tracked provisional span, so reconcile that span
    // directly with the final polished text. Final-only models retain the proven one-paste path. If a field
    // changed underneath the Accessibility session, copy the final text instead of risking a duplicate paste.
    let (pasted, copied) = if rec.target.has_preview() {
        if rec.target.commit(final_text.clone()) {
            (true, false)
        } else {
            log::warn!("dictation target reconciliation failed generation={generation}");
            (false, dict_set_clip(&final_text).is_ok())
        }
    } else if rec.target.requires_copy_fallback() {
        log::warn!(
            "dictation target changed after live text began generation={generation}; copied final text"
        );
        (false, dict_set_clip(&final_text).is_ok())
    } else {
        match dict_paste(&final_text) {
            Ok(()) => (true, false),
            Err(error) => {
                log::warn!("dictation paste failed generation={generation}: {error}");
                (false, dict_set_clip(&final_text).is_ok())
            }
        }
    };
    if !secure {
        let cleanup_changes = mechanical
            .as_ref()
            .map(|result| serde_json::to_value(&result.changes).unwrap_or_default())
            .unwrap_or_else(|| serde_json::json!([]));
        ingest(serde_json::json!({
            "type": "dictation",
            "text": final_text,
            "original": raw_text,
            "ms": ms,
            "processingMs": processing_started.elapsed().as_millis() as u64,
            "smartPolish": smart.enabled,
            "modelId": if smart.enabled { Some(smart.model_id) } else { None },
            "modelAccepted": model_accepted,
            "fallback": fallback,
            "paste": if pasted { "inserted" } else if copied { "copied" } else { "failed" },
            "decision": decision,
            "targetKind": context.kind,
            "changes": visual_changes,
            "cleanupTrace": cleanup_changes
        }));
    } else {
        ingest(serde_json::json!({
            "type": "dictation-private",
            "ms": ms,
            "smartPolish": smart.enabled,
            "targetKind": context.kind,
            "paste": if pasted { "inserted" } else if copied { "copied" } else { "failed" }
        }));
    }
    // The serialized upload/paste tail is complete. Do not make a following dictation wait for this
    // generation's success animation to finish.
    drop(_finish);
    if showed_changes && pasted {
        // Paste already happened; this only lets the native cut/replace proof finish independently.
        std::thread::sleep(Duration::from_millis(620));
    }
    let (phase, feedback, delay) = if pasted {
        (
            if fallback { "fallback" } else { "inserted" },
            if fallback {
                DictFeedback::Fallback
            } else {
                DictFeedback::Inserted
            },
            700,
        )
    } else if copied {
        ("copied", DictFeedback::Copied, 1800)
    } else {
        ("error", DictFeedback::Error, 1200)
    };
    emit_dstate(phase, generation);
    dict_feedback(feedback, generation);
    std::thread::sleep(Duration::from_millis(delay));
    dict_ui_idle(generation);
    job.complete();
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
    DICT_AUDIO_LEVEL.store(0, Ordering::Relaxed);
    log::info!(
        "acceptance:right-command phase=cancelled generation={}",
        rec.generation
    );
    dict_ui_idle(rec.generation);
    std::thread::spawn(move || {
        rec.stop.store(true, Ordering::Relaxed);
        let _ = rec.target.cancel();
        if let Some(h) = rec.handle.take() {
            let _ = h.join();
        }
        if let Some(h) = rec.live_handle.take() {
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
#[cfg(feature = "agent-research")]
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

#[cfg(feature = "agent-research")]
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

#[cfg(feature = "agent-research")]
const OPEN_AGENTS_SCRIPT: &str = "window.__myAssistantOpenAgents?.()";

#[cfg(feature = "agent-research")]
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

#[cfg(feature = "agent-research")]
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
static DICT_LISTEN_WATCHER_UP: AtomicBool = AtomicBool::new(false);
static DICT_LISTEN_REQUESTED: AtomicBool = AtomicBool::new(false);

// A listen-only event tap can still be created without Input Monitoring, but macOS then limits it to this
// app's own foreground events. Preflight the actual global-listening grant so that a local-only tap is never
// mistaken for a working Right Command shortcut.
#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[cfg(target_os = "macos")]
fn dict_listen_access() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}

#[cfg(not(target_os = "macos"))]
fn dict_listen_access() -> bool {
    true
}

#[cfg(target_os = "macos")]
fn request_dict_listen_access() -> bool {
    unsafe { CGRequestListenEventAccess() }
}

#[cfg(not(target_os = "macos"))]
fn request_dict_listen_access() -> bool {
    true
}

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

fn watch_for_dict_listen_access() {
    if DICT_LISTEN_WATCHER_UP.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        if DICT_MOD_MASK.load(Ordering::Relaxed) == 0 {
            DICT_LISTEN_WATCHER_UP.store(false, Ordering::SeqCst);
            return;
        }
        if dict_listen_access() {
            DICT_LISTEN_WATCHER_UP.store(false, Ordering::SeqCst);
            log::info!("dictation: global shortcut listening permission is now allowed");
            start_modifier_tap();
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}

fn ensure_modifier_tap(request_access: bool) {
    let allowed = dict_listen_access();
    log::info!("dictation: global shortcut listening allowed={allowed}");
    if allowed {
        start_modifier_tap();
        return;
    }

    if request_access && !DICT_LISTEN_REQUESTED.swap(true, Ordering::SeqCst) {
        let granted = request_dict_listen_access();
        log::info!("dictation: global shortcut listening request granted={granted}");
        if granted || dict_listen_access() {
            start_modifier_tap();
            return;
        }
    }
    // The user may grant Input Monitoring while the app remains open. Poll only while a bare-modifier
    // shortcut is selected, then start the global tap immediately—no relaunch or settings re-save required.
    watch_for_dict_listen_access();
}

// A dedicated thread owns the tap and its CFRunLoop — the tap is !Send. The watched modifier can be swapped
// live via DICT_MOD_MASK, so rebinding needs no teardown. Permission is checked before claiming DICT_TAP_UP;
// this prevents the foreground-only half-working state that a non-authorized listen-only tap creates.
fn start_modifier_tap() {
    if !dict_listen_access() {
        watch_for_dict_listen_access();
        return;
    }
    if DICT_TAP_UP
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
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
        let mut permission_revoked = false;
        match tap {
            Ok(tap) => match tap.mach_port.create_runloop_source(0) {
                Ok(src) => {
                    let rl = CFRunLoop::get_current();
                    unsafe {
                        rl.add_source(&src, kCFRunLoopCommonModes);
                    }
                    tap.enable();
                    log::info!("dictation: global modifier event-tap running");
                    // Run the loop in short slices and re-enable after each: macOS disables an event tap under
                    // load (kCGEventTapDisabledByTimeout), and without re-enabling, the modifier trigger would
                    // silently die until relaunch. This mirrors Handy's handy-keys run loop; enable() is idempotent.
                    let mut permission_tick = 0u8;
                    loop {
                        CFRunLoop::run_in_mode(
                            unsafe { kCFRunLoopDefaultMode },
                            Duration::from_millis(250),
                            false,
                        );
                        tap.enable();
                        permission_tick = permission_tick.wrapping_add(1);
                        if permission_tick >= 8 {
                            permission_tick = 0;
                            if !dict_listen_access() {
                                permission_revoked = true;
                                log::warn!(
                                    "dictation: global shortcut listening permission was removed"
                                );
                                break;
                            }
                        }
                    }
                }
                Err(_) => {
                    log::error!("dictation: modifier tap runloop-source failed");
                }
            },
            Err(_) => {
                log::error!(
                    "dictation: global modifier event-tap failed to create despite permission preflight"
                );
            }
        }
        DICT_MOD_DOWN.store(false, Ordering::Relaxed);
        DICT_TAP_UP.store(false, Ordering::SeqCst);
        if permission_revoked && DICT_MOD_MASK.load(Ordering::Relaxed) != 0 {
            watch_for_dict_listen_access();
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
    #[serde(default)]
    smart_polish: bool,
    #[serde(default)]
    live_cleanup: bool,
    #[serde(default = "default_polish_model_id")]
    polish_model: String,
    #[serde(default = "default_dictation_language")]
    language: String,
    #[serde(default)]
    vocabulary: Vec<dictation_cleanup::VocabularyTerm>,
}
fn def_true() -> bool {
    true
}
fn def_vol() -> u8 {
    70
}
fn default_polish_model_id() -> String {
    polish_models::DEFAULT_POLISH_MODEL_ID.to_string()
}
fn default_dictation_language() -> String {
    "auto".into()
}

#[tauri::command]
fn set_dictation(app: AppHandle, cfg: DictCfg) {
    *DICT_SMART_PREFS
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(DictSmartPrefs {
        enabled: cfg.smart_polish,
        live_cleanup: cfg.live_cleanup,
        model_id: polish_models::polish_model(&cfg.polish_model)
            .map(|model| model.id.to_string())
            .unwrap_or_else(default_polish_model_id),
        language: if cfg.language.trim().is_empty() {
            default_dictation_language()
        } else {
            cfg.language.clone()
        },
        vocabulary: cfg.vocabulary.clone(),
    });
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
        // Right Command requires Input Monitoring to receive events while another app is in front. Request it
        // once when this enabled shortcut is applied, and recover in-place if the user grants it later.
        ensure_modifier_tap(true);
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
// modal shows "Allowed" instead of a blind "Open System Settings". Input Monitoring is checked separately
// from Accessibility: the former makes a bare Right Command global, while the latter authorizes paste and
// structured app context. Screen Recording is needed only for optional video capture. Microphone and System
// Audio Recording are deliberately not guessed here: the UI labels them "Requested on first use" and provides
// recovery links. These statuses are checked non-prompting; the permission request/use is the explicit path.
#[derive(serde::Serialize)]
struct PermStatus {
    input_monitoring: bool,
    accessibility: bool,
    screen_recording: bool,
}
#[tauri::command]
fn perm_status() -> PermStatus {
    #[cfg(target_os = "macos")]
    {
        PermStatus {
            input_monitoring: dict_listen_access(),
            accessibility: macos_accessibility_client::accessibility::application_is_trusted(),
            screen_recording: core_graphics::access::ScreenCaptureAccess.preflight(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        PermStatus {
            input_monitoring: true,
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
            let presentation_handle = handle.clone();
            let server = match local_server::start(
                web_dir(),
                data_dir(),
                MODELS_DIR.get().cloned().unwrap_or_default(),
                res_base(),
                legacy_model_dirs(),
                Arc::new(move |meeting_app| notify_meeting_started(&notify_handle, &meeting_app)),
                Arc::new(move |state| present_meeting_state(&presentation_handle, state)),
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
            polish_runtime::configure(
                res_base(),
                MODELS_DIR.get().cloned().unwrap_or_default(),
                SERVER_TOKEN.get().cloned().unwrap_or_default(),
            );
            start_stack();
            let _ = APP.set(handle.clone()); // dictation fns use this for sound/tray/pill feedback
            #[cfg(all(target_os = "macos", debug_assertions))]
            if let Ok(preview) = std::env::var("ASSISTANT_PILL_PREVIEW") {
                let phase = match preview.as_str() {
                    "transcribing" => DictFeedback::Transcribing,
                    "post-processing" => DictFeedback::PostProcessing,
                    _ => DictFeedback::Listening,
                };
                native_pill::set(phase, 0, 7_000, 0.72);
            }
            #[cfg(all(target_os = "macos", debug_assertions))]
            if let Ok(phase) = std::env::var("ASSISTANT_MEETING_PILL_PREVIEW") {
                native_meeting_pill::set(&meeting_pill_preview_state(phase));
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
                        #[cfg(all(target_os = "macos", debug_assertions))]
                        if let Ok(phase) = std::env::var("ASSISTANT_MEETING_PILL_PREVIEW") {
                            // Re-order after the Tauri main window becomes visible; an AppKit panel created
                            // before the application finishes its first activation can otherwise remain ordered
                            // behind the initial WebView during visual regression previews.
                            native_meeting_pill::set(&meeting_pill_preview_state(phase.clone()));
                            let preview_handle = h2.clone();
                            let snapshot_path =
                                std::env::var("ASSISTANT_MEETING_PILL_SNAPSHOT").ok();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_secs(2));
                                let _ = preview_handle.run_on_main_thread(move || {
                                    // A developer preview must remain capturable even when the persisted
                                    // production preference hides overlays from screen sharing.
                                    native_meeting_pill::set_content_protected(false);
                                    native_meeting_pill::set(&meeting_pill_preview_state(phase));
                                    if let Some(path) = snapshot_path.as_deref() {
                                        if std::env::var("ASSISTANT_MEETING_PILL_PREVIEW_EXPANDED")
                                            .as_deref()
                                            == Ok("1")
                                        {
                                            native_meeting_pill::expand_for_preview();
                                        }
                                        native_meeting_pill::write_preview_snapshot(path);
                                    }
                                });
                            });
                        }
                        // The click-through AppKit pill is created lazily by dict_feedback() while visible.
                    });
                });
            }

            // Right Command is the product default. Settings re-pushes the persisted choice after the UI loads.
            DICT_MOD_MASK.store(modifier_mask("command_right"), Ordering::Relaxed);
            // Reuse an existing grant immediately. The settings sync requests a missing Input Monitoring grant
            // only after the UI is ready, and the permission watcher starts the tap without a relaunch.
            ensure_modifier_tap(false);

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
        .invoke_handler(generate_app_handlers!())
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
                    polish_runtime::shutdown();
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
    fn microphone_meter_has_a_quiet_floor_and_monotonic_speech_range() {
        assert_eq!(normalized_audio_level(0.0), 0.0);
        assert_eq!(normalized_audio_level(0.001), 0.0);
        let conversational = normalized_audio_level(0.03);
        let louder = normalized_audio_level(0.12);
        assert!(conversational > 0.0);
        assert!(louder > conversational);
        assert!(normalized_audio_level(1.0) <= 1.0);
    }

    #[test]
    fn live_cleanup_changes_only_the_committed_prefix() {
        let snapshot = local_server::LiveStreamSnapshot {
            full: "um send twenty five percent now".into(),
            committed: "um send twenty five percent".into(),
            tentative: "now".into(),
            is_final: false,
        };
        let smart = DictSmartPrefs {
            enabled: true,
            live_cleanup: true,
            model_id: polish_models::DEFAULT_POLISH_MODEL_ID.into(),
            language: "en".into(),
            vocabulary: Vec::new(),
        };
        let natural = dictation_context::DictationContext {
            kind: dictation_cleanup::DictationTargetKind::NaturalLanguage,
            starts_sentence: true,
        };
        assert_eq!(
            live_display_text(&snapshot, &smart, natural),
            "Send 25% now"
        );

        let mut after_finish = smart.clone();
        after_finish.live_cleanup = false;
        assert_eq!(
            live_display_text(&snapshot, &after_finish, natural),
            snapshot.full
        );

        let code = dictation_context::DictationContext {
            kind: dictation_cleanup::DictationTargetKind::CodeOrTerminal,
            starts_sentence: false,
        };
        assert_eq!(live_display_text(&snapshot, &smart, code), snapshot.full);
    }

    #[test]
    fn live_segment_joining_preserves_attached_punctuation() {
        assert_eq!(join_live_segments("Hello", ", world"), "Hello, world");
        assert_eq!(join_live_segments("Hello", "world"), "Hello world");
        assert_eq!(join_live_segments("", "tentative"), "tentative");
    }

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
    fn meeting_notifications_use_human_platform_names() {
        assert_eq!(meeting_app_label("zoom"), "Zoom");
        assert_eq!(meeting_app_label("meet"), "Google Meet");
        assert_eq!(meeting_app_label(""), "your call");
    }

    #[test]
    fn capture_helper_restart_backoff_is_bounded() {
        assert_eq!(capture_helper_restart_delay(1), Duration::from_millis(500));
        assert_eq!(capture_helper_restart_delay(2), Duration::from_secs(1));
        assert_eq!(capture_helper_restart_delay(3), Duration::from_secs(2));
        assert_eq!(capture_helper_restart_delay(20), Duration::from_secs(5));
    }

    #[cfg(feature = "agent-research")]
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

    #[cfg(feature = "agent-research")]
    #[test]
    fn agent_shortcut_routes_through_the_durable_webview_entrypoint() {
        assert_eq!(OPEN_AGENTS_SCRIPT, "window.__myAssistantOpenAgents?.()");
    }

    #[test]
    fn dictation_worker_and_watchdog_have_one_terminal_owner() {
        let worker_wins = DictationJob::new();
        assert!(worker_wins.claim_finish());
        assert!(!worker_wins.timeout());
        worker_wins.complete();
        assert!(!worker_wins.running());

        let watchdog_wins = DictationJob::new();
        assert!(watchdog_wins.timeout());
        assert!(!watchdog_wins.claim_finish());
        watchdog_wins.complete();
        assert!(!watchdog_wins.running());
    }
}
