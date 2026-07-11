use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
};

// Absolute path to the repo, baked at build time. In DEV this is where everything lives. In a PACKAGED build
// the app's own files come from the bundle's Resources dir instead (see init_paths); REPO is still passed to
// the server as ASSISTANT_REPO so a build machine that already has the model never re-downloads it.
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const SERVER: &str = "http://127.0.0.1:8790";

// Resolved once at startup (init_paths). RES_BASE = where the app's OWN files live (dev: the repo; packaged:
// the .app's read-only Resources dir). DATA_DIR / MODELS_DIR = WRITABLE locations for saved meetings and the
// downloaded model (dev: the repo; packaged: ~/Library/Application Support/My Assistant, since the bundle is
// read-only). Node reads these via env; the Swift helpers get RES_BASE-relative paths.
static RES_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static DATA_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static MODELS_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
fn res_base() -> String { RES_BASE.get().cloned().unwrap_or_else(|| REPO.to_string()) }
fn data_dir() -> String { DATA_DIR.get().cloned().unwrap_or_else(|| format!("{}/data", REPO)) }
// The web assets (server.js + UI) live in the repo's app/ in dev, but the bundle stages them under webui/ —
// a plain "app/" resource dir would collide with the crate's own `app` binary during Tauri's dev staging.
fn web_dir() -> String { if cfg!(debug_assertions) { format!("{}/app", res_base()) } else { format!("{}/webui", res_base()) } }
fn init_paths(app: &tauri::App) {
    let (res, data, models) = if cfg!(debug_assertions) {
        (REPO.to_string(), format!("{}/data", REPO), format!("{}/stt/whisper", REPO))
    } else {
        let res = app.path().resource_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| REPO.to_string());
        let home = std::env::var("HOME").unwrap_or_default();
        (res,
         format!("{}/Library/Application Support/My Assistant/data", home),
         format!("{}/Library/Application Support/My Assistant/models", home))
    };
    let _ = std::fs::create_dir_all(&data);
    let _ = std::fs::create_dir_all(&models);
    let _ = RES_BASE.set(res);
    let _ = DATA_DIR.set(data);
    let _ = MODELS_DIR.set(models);
}

// Sidecars the app spawns — all killed when it quits (so the mic is never left held).
static CHILDREN: Mutex<Vec<Child>> = Mutex::new(Vec::new());
fn track(child: std::io::Result<Child>) {
    if let Ok(c) = child {
        if let Ok(mut v) = CHILDREN.lock() { v.push(c); }
    }
}
fn kill_children() {
    if let Ok(mut v) = CHILDREN.lock() {
        for mut c in v.drain(..) { let _ = c.kill(); }
    }
}

// A GUI app doesn't inherit your shell PATH, so resolve tool locations explicitly.
fn find_bin(name: &str) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let dirs = ["/opt/homebrew/bin".into(), "/usr/local/bin".into(), "/usr/bin".into(), format!("{}/.local/bin", home)];
    dirs.iter().map(|d: &String| format!("{}/{}", d, name)).find(|p| Path::new(p).exists())
}
fn port_open(port: u16) -> bool {
    format!("127.0.0.1:{}", port).parse().ok()
        .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_millis(300)).ok())
        .is_some()
}

// Spawn the Accessibility detector (speaker names + in-call state) DIRECTLY — not via node — so macOS
// attributes its Accessibility check to the app itself. A grandchild spawned through node is NOT covered by
// the app's Accessibility grant (mic/screen-recording DO reach a direct child, but AX does not reach a
// grandchild), which is why names never showed. Its NDJSON is forwarded to the server's /ingest.
fn start_meeting_state() {
    let ms = format!("{}/meeting-state/meeting-state", res_base());
    if !Path::new(&ms).exists() { return; }
    if let Ok(mut child) = Command::new(&ms).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
        if let Some(out) = child.stdout.take() {
            std::thread::spawn(move || {
                for line in std::io::BufRead::lines(std::io::BufReader::new(out)).map_while(Result::ok) {
                    if line.trim().is_empty() { continue; }
                    let _ = Command::new("curl")
                        .args(["-s", "-X", "POST", "http://127.0.0.1:8790/ingest",
                               "-H", "Content-Type: application/json", "-d", line.as_str()])
                        .stdout(Stdio::null()).stderr(Stdio::null()).status();
                }
            });
        }
        if let Ok(mut v) = CHILDREN.lock() { v.push(child); }
    }
}

// Bring up the on-device stack. The Node GUI/relay server is always needed for the UI (it also spawns
// meeting-state for names + in-call detection). whisper-server + the capturer are the transcription
// pipeline: a PACKAGED build hosts them itself (no dev commands), while `npm run dev` leaves them to
// ./run.sh under Terminal's Screen-Recording grant.
fn start_stack() {
    if !port_open(8790) {
        if let Some(node) = find_bin("node") {
            track(Command::new(node)
                .arg(format!("{}/server.js", web_dir()))
                .env("PORT", "8790")
                // Packaged: the app spawns meeting-state itself (a DIRECT child, so it gets the AX grant) —
                // tell node to skip its own copy. In dev (debug), node owns meeting-state under Terminal's grant.
                .env("ASSISTANT_RUST_MEETING_STATE", if cfg!(debug_assertions) { "0" } else { "1" })
                // Packaged: point node at the WRITABLE data + models dirs (the bundle is read-only) and pass
                // the baked repo path so a machine that already has the model skips the first-run download.
                .env("ASSISTANT_DATA_DIR", data_dir())
                .env("ASSISTANT_MODELS_DIR", MODELS_DIR.get().cloned().unwrap_or_default())
                .env("ASSISTANT_REPO", REPO)
                .stdout(Stdio::null()).stderr(Stdio::null()).spawn());
        }
    }
    if cfg!(debug_assertions) { return; } // dev: capture comes from ./run.sh

    // whisper-server is NOT started here — node spawns it on demand when a call begins and unloads it when
    // idle (it's ~1.5 GB), so it's off RAM between calls.
    let tap = format!("{}/stt/meeting-tap", res_base());
    if Path::new(&tap).exists() {
        let recdir = format!("{}/meetings/rec", data_dir());
        let log = std::fs::File::create("/tmp/assistant-tap.log").map(Stdio::from).unwrap_or_else(|_| Stdio::null());
        track(Command::new(tap)
            .args(["8790", "http://127.0.0.1:8091/inference", "--gate", "on", "--rec-dir", &recdir])
            .stdout(Stdio::null()).stderr(log).spawn());
    }
    start_meeting_state(); // names + participants — direct child so Accessibility works
}

// ---- overlay: the floating Dictation Pill ----------------------------------------------------------
// One borderless, transparent, always-on-top window loading app/overlay.html?win=pill from the local
// server. It's click-through (set_ignore_cursor_events) so it floats over whatever you're dictating into
// and can NEVER steal focus — the dictated text always pastes into your document. (The menu-bar dropdown
// is a native macOS menu, built below, not a webview.)
const PILL_W: f64 = 208.0;
const PILL_H: f64 = 46.0;

fn make_overlay(app: &AppHandle, label: &str, win: &str, w: f64, h: f64, clickthrough: bool) {
    if app.get_webview_window(label).is_some() { return; }
    let url = format!("{}/overlay.html?win={}", SERVER, win);
    let Ok(u) = url.parse() else { return };
    let built = WebviewWindowBuilder::new(app, label, WebviewUrl::External(u))
        .inner_size(w, h)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .shadow(false)
        .resizable(false)
        .visible(false)
        .title("")
        .build();
    if let Ok(window) = built {
        let _ = window.set_visible_on_all_workspaces(true);
        if clickthrough { let _ = window.set_ignore_cursor_events(true); }
    }
}

// Place the pill on the screen it currently lives on, at the Settings "Overlay position".
// overlay_pos: 0 bottom-center · 1 bottom-right · 2 top-right · 3 top-center · 4 left-center · 5 right-center.
fn position_pill(app: &AppHandle, w: f64, h: f64) {
    let Some(win) = app.get_webview_window("pill") else { return };
    let _ = win.set_size(LogicalSize::new(w, h));
    // Use the monitor the pill is ON (fall back to primary), and compute in that monitor's GLOBAL logical
    // frame: origin (mx,my) + size (sw,sh). The old code used primary_monitor()+bare size, which assumes the
    // pill sits on the primary at (0,0) — with an external display attached the right/left-edge positions
    // then landed off the visible screen (the symmetric centre positions happened to stay visible, which is
    // exactly why "centre works, right/left don't"). Clamping keeps the pill fully on-screen regardless.
    let mon = win.current_monitor().ok().flatten()
        .or_else(|| win.primary_monitor().ok().flatten());
    let Some(mon) = mon else { return };
    let sf = mon.scale_factor();
    let mx = mon.position().x as f64 / sf;   // monitor top-left in the global logical space
    let my = mon.position().y as f64 / sf;
    let sw = mon.size().width as f64 / sf;
    let sh = mon.size().height as f64 / sf;
    let (_, _, _, _, pos) = dict_prefs();
    let m = 12.0;                             // screen-edge margin
    let top = 30.0;                           // top rows clear the menu bar / notch
    let (mut x, mut y) = match pos {
        1 => (sw - w - m, sh - h - m),        // bottom-right
        2 => (sw - w - m, top),               // top-right
        3 => ((sw - w) / 2.0, top),           // top-center
        4 => (m, (sh - h) / 2.0),             // left-center
        5 => (sw - w - m, (sh - h) / 2.0),    // right-center
        _ => ((sw - w) / 2.0, sh - h - m),    // bottom-center (default) — hug the bottom rim, like Handy
    };
    x = x.clamp(m, (sw - w - m).max(m));      // never let the pill fall off this monitor
    y = y.clamp(top, (sh - h - m).max(top));
    let _ = win.set_position(LogicalPosition::new(mx + x, my + y));
    eprintln!("dictation: pill pos={} mon=({:.0},{:.0} {:.0}x{:.0}) -> ({:.0},{:.0})", pos, mx, my, sw, sh, mx + x, my + y);
}

// Make the pill float over EVERY app — including the one you're typing in. A plain always-on-top window
// belongs to My Assistant and macOS stops floating it once another app is active; raising the NSWindow level
// and letting it join all Spaces (incl. full-screen) is what actually keeps it on top everywhere.
#[cfg(target_os = "macos")]
fn pill_float_over_all_apps(app: &AppHandle) {
    use objc2::{msg_send, runtime::{AnyClass, AnyObject}};
    let Some(win) = app.get_webview_window("pill") else { return };
    let Ok(ptr) = win.ns_window() else { return };
    let ns = ptr as *mut AnyObject;
    if ns.is_null() { return; }
    unsafe {
        // A plain NSWindow CANNOT render inside ANOTHER app's full-screen space, at any window level — the
        // pill vanished the moment you were in a full-screen app (Claude Code, etc.). The reliable fix, and
        // exactly what Handy does via tauri-nspanel, is to make the pill a NON-ACTIVATING NSPanel: swap its
        // Objective-C class to NSPanel and add the nonactivating style bit. Then a full-screen-capable
        // collection behavior + a high level float it over full-screen apps too, and — being nonactivating —
        // it never pulls focus off what you're dictating into. Idempotent → safe to re-apply on each show.
        if let Some(panel_cls) = AnyClass::get("NSPanel") {
            let _ = AnyObject::set_class(&*ns, panel_cls);
        }
        let mask: usize = msg_send![ns, styleMask];
        let _: () = msg_send![ns, setStyleMask: mask | 128usize];   // + NSWindowStyleMaskNonactivatingPanel (1<<7)
        // NSScreenSaverWindowLevel (1000) sits above full-screen content; canJoinAllSpaces(1) puts the panel
        // on every space (incl. other apps' full-screen spaces); fullScreenAuxiliary(256) lets it render there.
        let _: () = msg_send![ns, setLevel: 1000isize];
        let _: () = msg_send![ns, setCollectionBehavior: 257usize];
        // orderFrontRegardless brings it forward WITHOUT activating the app — also fixes the plain "pill
        // never appears" case where macOS doesn't order a borderless non-focusable window onto the screen.
        let _: () = msg_send![ns, orderFrontRegardless];
    }
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
        let _ = app.run_on_main_thread(move || { if let Some(t) = a.tray_by_id("tray") { let _ = t.set_title(Some(s)); } });
    }
    if on {
        if TRAY_REC.swap(true, Ordering::SeqCst) { return; } // already animating
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
// Runs entirely IN this app process (com.myassistant.desktop): the hotkey is a Carbon global shortcut
// (needs NO macOS permission), the ⌘V paste is synthesized via CGEvent → the app's OWN Accessibility
// grant, and — like Handy — the mic is recorded IN-PROCESS with cpal. The OLD design spawned
// `meeting-tap --dictate` per keypress, which cost ~300ms of process-launch lag before the mic even
// opened (why it "felt slower than Handy"). Recording in-process means the APP itself asks for the
// Microphone grant the first time you dictate (Info.plist carries NSMicrophoneUsageDescription), then it
// sticks under the app's own identity. Progress/results still POST to the server's /ingest, so the pill
// overlay and Dictation view need no changes.
use std::time::Instant;

// A live recording. A cpal input Stream is `!Send`, so it can't sit in DICT (a static Mutex shared across
// threads) — instead its owning thread parks on `stop` and drops the Stream on that same thread. The
// capture callback appends mono f32 into `samples`; `rate`/`chans` report the device's real format.
struct Recorder {
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: Arc<AtomicU32>,
    handle: Option<JoinHandle<()>>,
}
struct DictState { enabled: bool, rec: Option<Recorder>, started: Option<Instant> }
static DICT: Mutex<DictState> = Mutex::new(DictState { enabled: false, rec: None, started: None });

// Open the default input device and stream mono f32 into `samples` until `stop` is set. Built on a
// dedicated thread because a cpal Stream is `!Send`; the first-run mic-permission prompt fires here.
// Handles the three sample formats macOS inputs actually produce (F32 on the built-in mic, I16/U16 on
// some USB interfaces). ~120s safety cap so a stuck key can never hold the mic open forever.
fn spawn_capture(stop: Arc<AtomicBool>, samples: Arc<Mutex<Vec<f32>>>, rate: Arc<AtomicU32>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let Some(dev) = host.default_input_device() else { return; };
        let Ok(cfg) = dev.default_input_config() else { return; };
        rate.store(cfg.sample_rate().0, Ordering::Relaxed);
        let ch = (cfg.channels() as usize).max(1); // guard: chunks(0) panics, and a panic out of cpal's C callback aborts
        let fmt = cfg.sample_format();
        let scfg: cpal::StreamConfig = cfg.into();
        let err = |e| eprintln!("dictation cpal error: {e}");
        let s = samples.clone();
        // Keep the first channel of every frame → mono, matching whisper's expected input.
        let stream = match fmt {
            cpal::SampleFormat::F32 => dev.build_input_stream(&scfg,
                move |d: &[f32], _: &cpal::InputCallbackInfo| { let mut b = s.lock().unwrap(); for f in d.chunks(ch) { b.push(f[0]); } }, err, None),
            cpal::SampleFormat::I16 => dev.build_input_stream(&scfg,
                move |d: &[i16], _: &cpal::InputCallbackInfo| { let mut b = s.lock().unwrap(); for f in d.chunks(ch) { b.push(f[0] as f32 / 32768.0); } }, err, None),
            cpal::SampleFormat::U16 => dev.build_input_stream(&scfg,
                move |d: &[u16], _: &cpal::InputCallbackInfo| { let mut b = s.lock().unwrap(); for f in d.chunks(ch) { b.push((f[0] as f32 - 32768.0) / 32768.0); } }, err, None),
            _ => return,
        };
        let Ok(stream) = stream else { return; };
        if stream.play().is_err() { return; }
        let mut n: u32 = 0;
        while !stop.load(Ordering::Relaxed) && n < 12_000 { std::thread::sleep(Duration::from_millis(10)); n += 1; }
        drop(stream); // stop capture on the SAME thread that built it (Stream is !Send)
    })
}

// Downsample captured mono f32 to 16 kHz (whisper's + Silero's rate) with a BOX AVERAGE over each source
// window (a cheap low-pass) instead of plain decimation — decimating 48k→16k by taking every 3rd sample
// aliased speech's high-frequency energy back down as noise. (The mic is 48 kHz on this Mac; any rate ok.)
fn resample_16k(samples: &[f32], rate: u32) -> Vec<f32> {
    if rate <= 16_000 || rate == 0 { return samples.to_vec(); }
    let ratio = rate as f64 / 16_000.0;
    let out_len = (samples.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let start = (i as f64 * ratio) as usize;
        let end = (((i + 1) as f64 * ratio) as usize).clamp(start + 1, samples.len());
        let win = &samples[start..end];
        out.push(win.iter().sum::<f32>() / win.len() as f32);
    }
    out
}

// Silero VAD — the SAME neural voice-activity detector Handy uses. Keep ONLY the speech regions of the
// 16 kHz clip, dropping silence / room-tone that Whisper otherwise turns into a fluent hallucinated loop
// (the "completely garbled" recordings). Returns empty when there's no speech at all, so an accidental
// hotkey press with nothing said inserts nothing. The model + onnxruntime are statically linked (via the
// `ort` crate, exactly as Handy ships it); if init ever fails we fall back to the full clip rather than
// silently drop audio.
//
// CACHED like Handy: Handy's SileroVad holds the ONNX session for the app's lifetime and calls reset()
// between utterances (clearing only the LSTM hidden/cell state — NO model reload). We do the same: build
// the detector on first use, keep it here, and reset() before each clip. That moves the ~one-time model
// load off every dictation's "Transcribing" phase. The detector owns an `ort` Session (Send), so it lives
// safely in a static Mutex even though each dictation finishes on its own short-lived thread.
static VAD: Mutex<Option<voice_activity_detector::VoiceActivityDetector>> = Mutex::new(None);
fn vad_extract_speech(samples16k: &[f32]) -> Vec<f32> {
    use voice_activity_detector::{IteratorExt, VoiceActivityDetector};
    let mut guard = VAD.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        match VoiceActivityDetector::builder().sample_rate(16_000i64).chunk_size(512usize).build() {
            Ok(v) => *guard = Some(v),
            Err(_) => return samples16k.to_vec(), // model unavailable → keep the whole clip rather than drop audio
        }
    }
    let vad = guard.as_mut().unwrap();
    vad.reset(); // clear the LSTM state so this utterance doesn't inherit the previous one's recurrent context
    // threshold 0.5 = Silero's default speech probability; pad 3 chunks (~96 ms) so onsets/tails aren't clipped.
    let mut out = Vec::with_capacity(samples16k.len());
    for chunk in samples16k.iter().copied().label(&mut *vad, 0.5f32, 3usize) {
        if chunk.is_speech() { out.extend(chunk.iter().copied()); }
    }
    out
}

// Write mono 16 kHz f32 samples as a 16-bit PCM WAV for the STT server.
fn write_wav16k(path: &str, samples16k: &[f32]) -> bool {
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let Ok(mut w) = hound::WavWriter::create(path, spec) else { return false; };
    for v in samples16k { let _ = w.write_sample((v.clamp(-1.0, 1.0) * 32767.0) as i16); }
    w.finalize().is_ok()
}

fn ingest(json: &str) {
    // Fire-and-forget on a detached thread. This spawns a BLOCKING `curl`, and emit_dstate() calls ingest from
    // on_dict_hotkey and the Esc-cancel handler — both of which run on the MAIN (UI) thread. Blocking there
    // froze the whole app for up to --max-time seconds and left the pill stuck on screen (Esc "hangs"). Never
    // block the UI thread on a subprocess; the SSE broadcast is fire-and-forget and pill visibility is owned by
    // dict_feedback (a direct window show/hide), so ordering here doesn't matter.
    let json = json.to_string();
    std::thread::spawn(move || {
        let _ = Command::new("curl")
            .args(["-s", "--max-time", "5", "-X", "POST", &format!("{}/ingest", SERVER),
                   "-H", "Content-Type: application/json", "-d", &json])
            .stdout(Stdio::null()).stderr(Stdio::null()).status();
    });
}
fn emit_dstate(phase: &str) { ingest(&format!("{{\"type\":\"dictation-state\",\"phase\":\"{}\"}}", phase)); }

// AppHandle stashed at setup so the dictation fns can give RELIABLE feedback: a start/stop SOUND (the
// primary "is it on?" cue, like Handy) + the menu-bar title "● Dictating" + the pill window (shown/hidden
// from Rust here, so it no longer depends on the overlay webview calling back — which was silently failing).
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

// Dictation preferences pushed from Settings via set_dictation. The in-process dictation owns recording,
// the paste, the sound cues and the pill, so these rows only take effect if THIS process honors them —
// they used to flow to the removed `meeting-tap --dictate` and silently no-op'd. clipboard: 0 = restore
// the user's previous clipboard after paste ("Never save"), 1 = leave the dictated text on it (Save/Replace).
// overlay_pos: 0 bottom-center · 1 bottom-right · 2 top-right · 3 top-center · 4 left-center · 5 right-center.
struct DictPrefs { sound: bool, volume: f32, clipboard: u8, overlay: bool, overlay_pos: u8 }
static DICT_PREFS: Mutex<DictPrefs> = Mutex::new(DictPrefs { sound: true, volume: 0.7, clipboard: 0, overlay: true, overlay_pos: 0 });
fn dict_prefs() -> (bool, f32, u8, bool, u8) {
    let p = DICT_PREFS.lock().unwrap_or_else(|e| e.into_inner());
    (p.sound, p.volume, p.clipboard, p.overlay, p.overlay_pos)
}

fn play(sound: &str) {
    // "Dictation sound" off → no cue at all. Otherwise play at the Settings "Feedback volume" (afplay -v 0..1).
    let (on, vol, _, _, _) = dict_prefs();
    if !on { return; }
    // Reap afplay (.status() waits) on a detached thread so it doesn't accumulate as a zombie on each
    // start/stop cue — a bare spawn() is never reaped and would pile up over a long session.
    let path = format!("/System/Library/Sounds/{}.aiff", sound);
    let v = format!("{:.2}", vol.clamp(0.0, 1.0));
    std::thread::spawn(move || { let _ = Command::new("afplay").arg("-v").arg(v).arg(path).status(); });
}
fn dict_feedback(on: bool) {
    if let Some(app) = APP.get() {
        // MUST run on the main thread — tray/window (Cocoa) calls from the transcription thread abort the
        // app (SIGABRT). run_on_main_thread marshals it safely from whatever thread we're on.
        let (_, _, _, overlay, _) = dict_prefs(); // "Dictation overlay" off → tray title still animates, pill stays hidden
        let app = app.clone();
        let handle = app.clone();
        let _ = handle.run_on_main_thread(move || {
            if let Some(t) = app.tray_by_id("tray") { let _ = t.set_title(Some(if on { " ● Dictating" } else { "" })); }
            if let Some(w) = app.get_webview_window("pill") {
                if on && overlay {
                    position_pill(&app, PILL_W, PILL_H);
                    let _ = w.show();
                    #[cfg(target_os = "macos")]
                    pill_float_over_all_apps(&app); // float over the app you're typing in — applied at show-time (can't blank the pill)
                } else { let _ = w.hide(); }
            }
        });
    }
}

fn dict_set_clip(s: &str) {
    if let Ok(mut c) = Command::new("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(mut si) = c.stdin.take() { use std::io::Write; let _ = si.write_all(s.as_bytes()); }
        let _ = c.wait();
    }
}
fn dict_get_clip() -> String {
    Command::new("pbpaste").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
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
    let old = if restore { dict_get_clip() } else { String::new() };
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
    if restore { dict_set_clip(&old); }
}

// Two modes off ONE hotkey (⌃⇧D): tap-to-talk toggles on each press; push-to-talk starts on press and
// stops on release. The Carbon hotkey reports press AND release (kEventHotKeyReleased), so PTT needs no
// key tap → no Input Monitoring. Mode = DICT_PTT (set from Settings via set_dictation).
static DICT_PTT: AtomicBool = AtomicBool::new(false);   // false = tap-to-talk, true = push-to-talk
fn dictation_start() {
    let mut g = DICT.lock().unwrap();
    if !g.enabled || g.rec.is_some() { return; }
    // Start capturing IMMEDIATELY, in-process — no child to launch, so the mic opens as fast as macOS
    // can hand us the device (Handy measures ~230ms for the same step; we skip the ~300ms process spawn).
    let stop = Arc::new(AtomicBool::new(false));
    let samples = Arc::new(Mutex::new(Vec::<f32>::with_capacity(16_000 * 6)));
    let rate = Arc::new(AtomicU32::new(16_000));
    let handle = spawn_capture(stop.clone(), samples.clone(), rate.clone());
    g.rec = Some(Recorder { stop, samples, rate, handle: Some(handle) });
    g.started = Some(Instant::now());
    drop(g);
    play("Tink"); dict_feedback(true); emit_dstate("listening");
    // Esc-to-cancel is watched inside the event tap (keycode 53) and gated on recording — no separate grab.
}
fn dictation_stop_rec() {
    let mut g = DICT.lock().unwrap();
    let Some(rec) = g.rec.take() else { return; };
    let ms = g.started.map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
    g.started = None;
    drop(g);
    play("Pop");   // immediate stop cue (encode + transcription run on the thread below)
    std::thread::spawn(move || dictation_finish(rec, ms));
}
fn dictation_toggle() {
    let recording = DICT.lock().unwrap().rec.is_some();
    if recording { dictation_stop_rec() } else { dictation_start() }
}
// Serializes the finish tail so two quick back-to-back dictations can't race the shared /tmp WAV or clipboard.
static DICT_FINISH: Mutex<()> = Mutex::new(());
fn dictation_finish(mut rec: Recorder, ms: u64) {
    rec.stop.store(true, Ordering::Relaxed);
    if let Some(h) = rec.handle.take() { let _ = h.join(); } // stream fully stopped, buffer settled
    let rate = rec.rate.load(Ordering::Relaxed);
    let raw = std::mem::take(&mut *rec.samples.lock().unwrap());
    let mono16k = resample_16k(&raw, rate);
    let speech = vad_extract_speech(&mono16k); // Silero VAD: keep only speech → no hallucinated loop on silence
    emit_dstate("transcribing");
    // Acquire AFTER the VAD (per-clip, safe to run in parallel): serialize only the shared tail — the /tmp
    // WAV write, the upload, and the clipboard save/paste/restore — so overlapping finishes can't clobber
    // each other. Recover the guard on poison instead of panicking the finish thread.
    let _finish = DICT_FINISH.lock().unwrap_or_else(|e| e.into_inner());
    let wav = "/tmp/myassistant-dictation.wav".to_string();
    // No speech detected → skip the engine entirely (no paste, no hallucinated text).
    if speech.is_empty() || !write_wav16k(&wav, &speech) { emit_dstate("idle"); dict_feedback(false); return; }
    let out = Command::new("curl")
        .args(["-s", "--max-time", "60", &format!("{}/api/transcribe", SERVER), "-F", &format!("file=@{}", wav)])
        .output();
    let text = out.ok()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(|s| s.trim().to_string()))
        .unwrap_or_default();
    if !text.is_empty() {
        dict_paste(&text);
        ingest(&serde_json::json!({ "type": "dictation", "text": text, "ms": ms }).to_string());
    }
    emit_dstate("idle");
    dict_feedback(false);
}

// Esc while recording → throw the clip away: stop capture, no resample / VAD / transcribe / paste. The
// stop+join runs off-thread because this is invoked from the Esc hotkey handler (the main thread).
fn dictation_cancel() {
    let rec = { let mut g = DICT.lock().unwrap(); g.started = None; g.rec.take() };
    let Some(mut rec) = rec else { return; };
    emit_dstate("idle");
    dict_feedback(false);
    std::thread::spawn(move || { rec.stop.store(true, Ordering::Relaxed); if let Some(h) = rec.handle.take() { let _ = h.join(); } });
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
        "enter" | "return"   => 36,
        "tab"                => 48,
        _ => 53, // Esc — the default and the fallback for anything we don't map
    }
}

// ---- rebindable start/stop hotkey -----------------------------------------------------------------
// The dictation shortcut is rebindable from Settings (default ⌃⇧D). We keep the currently-registered
// Shortcut so a change can unregister the old one before adding the new. A dictation hotkey MUST carry a
// modifier — a bare global key would be swallowed system-wide.
#[derive(serde::Deserialize, Default)]
struct HotkeySpec {
    #[serde(default)] mods: Vec<String>,
    #[serde(default)] key: String,
    // A bare-modifier trigger (Handy-style), e.g. "command_right", "option", "control_left". Non-empty → the
    // event tap below drives dictation and no Carbon hotkey is registered (Carbon can't bind a lone modifier).
    #[serde(default)] modifier: String,
}
static DICT_HOTKEY: Mutex<Option<tauri_plugin_global_shortcut::Shortcut>> = Mutex::new(None);

fn key_to_code(key: &str) -> Option<tauri_plugin_global_shortcut::Code> {
    use std::str::FromStr;
    let k = key.trim();
    let name = if k.chars().count() == 1 {
        let c = k.chars().next().unwrap().to_ascii_uppercase();
        if c.is_ascii_alphabetic() { format!("Key{}", c) }
        else if c.is_ascii_digit() { format!("Digit{}", c) }
        else { return None; }
    } else {
        match k.to_ascii_lowercase().as_str() {
            "space" | "spacebar" => "Space".to_string(),
            "enter" | "return"   => "Enter".to_string(),
            "tab"                => "Tab".to_string(),
            "esc" | "escape"     => "Escape".to_string(),
            "period" | "."       => "Period".to_string(),
            "comma"  | ","       => "Comma".to_string(),
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
        ShortcutState::Pressed  => if ptt { dictation_start() } else { dictation_toggle() },
        ShortcutState::Released => if ptt { dictation_stop_rec() },
    }
}

// Swap the registered dictation shortcut. IDEMPOTENT: re-requesting the SAME shortcut (which the UI does on
// every settings-sync + on boot) is a no-op, so we never briefly unregister the working hotkey. A genuine
// change registers the NEW shortcut first and only then drops the OLD, so a failed registration can never
// leave the app with no hotkey at all. Returns false if the spec can't be registered.
fn register_dict_hotkey(app: &AppHandle, spec: &HotkeySpec) -> bool {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, Modifiers};
    let Some(code) = key_to_code(&spec.key) else { return false; };
    let mut mods = Modifiers::empty();
    for m in &spec.mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control"          => mods |= Modifiers::CONTROL,
            "shift"                     => mods |= Modifiers::SHIFT,
            "alt" | "option" | "opt"    => mods |= Modifiers::ALT,
            "cmd" | "meta" | "super" | "win" => mods |= Modifiers::SUPER,
            _ => {}
        }
    }
    if mods.is_empty() { return false; } // refuse a modifier-less global hotkey — it'd swallow that key everywhere
    let sc = Shortcut::new(Some(mods), code);
    if DICT_HOTKEY.lock().unwrap_or_else(|e| e.into_inner()).as_ref() == Some(&sc) { return true; } // unchanged → already live
    let gs = app.global_shortcut();
    match gs.on_shortcut(sc.clone(), |_app, _scut, event| on_dict_hotkey(event.state())) {
        Ok(_) => {
            let mut cur = DICT_HOTKEY.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(prev) = cur.take() { if prev != sc { let _ = gs.unregister(prev); } }
            *cur = Some(sc);
            eprintln!("dictation: hotkey registered ({:?} + {:?})", mods, code);
            true
        }
        Err(e) => { eprintln!("dictation: hotkey registration FAILED ({:?} + {:?}): {e}", mods, code); false }
    }
}

// ---- bare-modifier trigger (Handy-style: right ⌘, Option, etc. as a hold/tap-to-talk key) --------------
// Carbon RegisterEventHotKey (the plugin above) CANNOT bind a lone modifier. So when the trigger IS a bare
// modifier we run a listen-only CGEventTap on .flagsChanged and watch that modifier's device-flag bit go
// down/up, feeding the SAME on_dict_hotkey(Pressed/Released). Push-to-talk on → hold-to-talk, off →
// tap-to-talk — identical to Handy, and it reuses everything we already built.
static DICT_MOD_MASK: AtomicU64 = AtomicU64::new(0);    // device-flag bit to watch; 0 = no modifier trigger (use Carbon)
static DICT_MOD_DOWN: AtomicBool = AtomicBool::new(false); // last observed state of that modifier (edge detection)
static DICT_TAP_UP: AtomicBool = AtomicBool::new(false);   // is the event-tap thread live? (guards against a double tap)

// Modifier name (from the UI) → macOS device-dependent flag bit(s). The *_right / *_left device masks are what
// let us tell the two ⌘ keys apart — the generic Command flag can't. Fn has no device bit, so it rides the
// generic SecondaryFn mask. These constants are the classic NX_DEVICE*KEYMASK values.
fn modifier_mask(name: &str) -> u64 {
    match name.trim().to_ascii_lowercase().as_str() {
        "control_left"  | "ctrl_left"  => 0x0000_0001,
        "shift_left"                    => 0x0000_0002,
        "shift_right"                   => 0x0000_0004,
        "command_left"  | "cmd_left"   => 0x0000_0008,
        "command_right" | "cmd_right"  => 0x0000_0010,
        "option_left"   | "alt_left"   => 0x0000_0020,
        "option_right"  | "alt_right"  => 0x0000_0040,
        "control_right" | "ctrl_right" => 0x0000_2000,
        // either-side (device bits OR'd together):
        "command" | "cmd"              => 0x0000_0018,
        "option"  | "alt" | "opt"      => 0x0000_0060,
        "control" | "ctrl"             => 0x0000_2001,
        "shift"                        => 0x0000_0006,
        "fn"                           => 0x0080_0000, // SecondaryFn (Fn has no L/R device bit)
        _ => 0,
    }
}

// Spawned once (and retried when a modifier trigger is first set, in case Accessibility was just granted). A
// dedicated thread owns the tap and its CFRunLoop — the tap is !Send and the loop parks the thread forever.
// The WATCHED modifier is swapped live via DICT_MOD_MASK, so rebinding needs no teardown. Needs Accessibility
// (the same grant the paste uses); if it isn't granted the tap can't be created → we log and give up until relaunch.
fn start_modifier_tap() {
    if DICT_TAP_UP.swap(true, Ordering::SeqCst) { return; } // already running — don't create a second tap
    std::thread::spawn(|| {
        use core_graphics::event::{CGEventTap, CGEventTapLocation, CGEventTapPlacement, CGEventTapOptions, CGEventType};
        use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes, kCFRunLoopDefaultMode};
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
                            // Marshal to the MAIN thread so start/stop runs on the same thread as the Carbon
                            // hotkey path (keeps CoreAudio/AppKit on main) and keeps this tap callback fast.
                            if let Some(app) = APP.get() {
                                let _ = app.run_on_main_thread(move || {
                                    use tauri_plugin_global_shortcut::ShortcutState;
                                    on_dict_hotkey(if down { ShortcutState::Pressed } else { ShortcutState::Released });
                                });
                            }
                        }
                    }
                } else if matches!(etype, CGEventType::KeyDown) {
                    // Cancel key (Esc, keycode 53) → discard the recording, Handy-style. field 9 =
                    // kCGKeyboardEventKeycode. dictation_cancel() no-ops when not recording, so this stays a
                    // cheap atomic check on each keystroke; marshaled to main (async from this tap thread).
                    if event.get_integer_value_field(9) == DICT_CANCEL_KEY.load(Ordering::Relaxed) {
                        if let Some(app) = APP.get() { let _ = app.run_on_main_thread(dictation_cancel); }
                    }
                }
                None // listen-only: never modify or swallow the event
            },
        );
        match tap {
            Ok(tap) => match tap.mach_port.create_runloop_source(0) {
                Ok(src) => {
                    let rl = CFRunLoop::get_current();
                    unsafe { rl.add_source(&src, kCFRunLoopCommonModes); }
                    tap.enable();
                    eprintln!("dictation: modifier event-tap running");
                    // Run the loop in short slices and re-enable after each: macOS disables an event tap under
                    // load (kCGEventTapDisabledByTimeout), and without re-enabling, the modifier trigger would
                    // silently die until relaunch. This mirrors Handy's handy-keys run loop; enable() is idempotent.
                    loop {
                        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, Duration::from_millis(250), false);
                        tap.enable();
                    }
                }
                Err(_) => { eprintln!("dictation: modifier tap runloop-source failed"); DICT_TAP_UP.store(false, Ordering::SeqCst); }
            },
            Err(_) => { eprintln!("dictation: modifier event-tap FAILED to create (grant Accessibility, then relaunch)"); DICT_TAP_UP.store(false, Ordering::SeqCst); }
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
    #[serde(default = "def_true")] sound: bool,
    #[serde(default = "def_vol")] volume: u8,
    #[serde(default)] clipboard: String,      // "restore" | "save" | "replace"
    #[serde(default = "def_true")] overlay: bool,
    #[serde(default)] overlay_pos: String,    // bottom-center | bottom-right | top-right | top-center | left-center | right-center
    #[serde(default)] hotkey: Option<HotkeySpec>,
    #[serde(default)] cancel: Option<HotkeySpec>, // the "Cancel" key armed while recording (default Esc)
    #[serde(default)] prompt: bool,
}
fn def_true() -> bool { true }
fn def_vol() -> u8 { 70 }

#[tauri::command]
fn set_dictation(app: AppHandle, cfg: DictCfg) {
    DICT.lock().unwrap().enabled = cfg.enabled;
    DICT_PTT.store(cfg.ptt, Ordering::Relaxed);
    {
        let mut p = DICT_PREFS.lock().unwrap_or_else(|e| e.into_inner());
        p.sound = cfg.sound;
        p.volume = (cfg.volume.min(100) as f32) / 100.0;
        p.clipboard = if cfg.clipboard == "restore" || cfg.clipboard.is_empty() { 0 } else { 1 };
        p.overlay = cfg.overlay;
        p.overlay_pos = match cfg.overlay_pos.as_str() { "bottom-right" => 1, "top-right" => 2, "top-center" => 3, "left-center" => 4, "right-center" => 5, _ => 0 };
    }
    // Cancel key (armed only while recording). Unmappable / missing → None → Esc.
    DICT_CANCEL_KEY.store(cfg.cancel.as_ref().map(|c| cancel_keycode(&c.key)).unwrap_or(53), Ordering::Relaxed);
    // Trigger: a bare modifier (event tap) OR a Carbon combo — never both at once.
    let mm = cfg.hotkey.as_ref().map(|hk| modifier_mask(&hk.modifier)).unwrap_or(0);
    if cfg.enabled && mm != 0 {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        DICT_MOD_MASK.store(mm, Ordering::Relaxed);
        DICT_MOD_DOWN.store(false, Ordering::Relaxed); // resync edge state to the new modifier
        // retire any Carbon hotkey so the old ⌃⇧D can't fire alongside the modifier
        if let Some(prev) = DICT_HOTKEY.lock().unwrap_or_else(|e| e.into_inner()).take() { let _ = app.global_shortcut().unregister(prev); }
        start_modifier_tap(); // idempotent; also retries the tap now that Accessibility may have just been granted
        eprintln!("dictation: modifier trigger set ({} = {:#x})", cfg.hotkey.as_ref().map(|h| h.modifier.as_str()).unwrap_or(""), mm);
    } else {
        DICT_MOD_MASK.store(0, Ordering::Relaxed); // fall back to a Carbon combo
        if let Some(hk) = cfg.hotkey.as_ref() { register_dict_hotkey(&app, hk); }
    }
    if cfg.enabled && cfg.prompt {
        // Prompts ONLY when not already trusted; silent once granted (the stable cert makes it stick). This
        // is what re-asks once after a signature change / TCC reset, then never again — the paste needs it.
        let _ = macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
    }
}
// Tray "Stop" → finish the current dictation (only if one is recording).
#[tauri::command]
fn dictation_stop() { dictation_stop_rec(); }

// Settings "Show tray icon" → show/hide the menu-bar icon. Tray mutations are AppKit → main thread.
#[tauri::command]
fn set_tray_visible(app: AppHandle, on: bool) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || { if let Some(t) = a.tray_by_id("tray") { let _ = t.set_visible(on); } });
}

// Settings → Permissions: report whether the grants the enabled features need are ACTUALLY in place, so the
// modal shows "Allowed" instead of a blind "Open System Settings". Accessibility is what the hotkey event-tap
// and the ⌘V paste run under (this very process, so its own trust is the right thing to check); Screen
// Recording is what the meeting capture needs (same app bundle → this process's preflight reflects the grant).
// Both are checked NON-prompting — opening a System Settings pane is the explicit grant path.
#[derive(serde::Serialize)]
struct PermStatus { accessibility: bool, screen_recording: bool }
#[tauri::command]
fn perm_status() -> PermStatus {
    #[cfg(target_os = "macos")]
    { PermStatus {
        accessibility: macos_accessibility_client::accessibility::application_is_trusted(),
        screen_recording: core_graphics::access::ScreenCaptureAccess::default().preflight(),
    } }
    #[cfg(not(target_os = "macos"))]
    { PermStatus { accessibility: true, screen_recording: true } }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())?;
            }

            init_paths(app);   // resolve RES_BASE / DATA_DIR / MODELS_DIR before anything spawns
            start_stack();
            let handle = app.handle().clone();
            let _ = APP.set(handle.clone());   // dictation fns use this for sound/tray/pill feedback
            // Create the pill only AFTER the local server is listening — otherwise its webview loads the
            // localhost URL before node is up, the load fails, and the window stays blank forever (that's
            // why the pill "never showed"). The tray is a native menu now, so no tray overlay window.
            {
                let h = handle.clone();
                std::thread::spawn(move || {
                    for _ in 0..80 { if port_open(8790) { break; } std::thread::sleep(Duration::from_millis(250)); }
                    std::thread::sleep(Duration::from_millis(400)); // let the HTTP route come fully up
                    let h2 = h.clone();
                    let _ = h.run_on_main_thread(move || {
                        // Create the main window FRESH here, after node answers — NOT from tauri.conf at launch.
                        // A config window races node startup: its first load fails (connection refused), and a
                        // later navigate() loads the DOM but WKWebView never runs the page's inline <script> on
                        // that recovered frame → the whole UI was a dead static shell (nav clicks, dictation,
                        // settings, config-sync all silently broken; only the pill, built fresh below, worked).
                        // Building main the same proven way — WebviewWindowBuilder after node-up — makes its
                        // inline script actually execute. tauri.conf's windows list is now empty for this reason.
                        if h2.get_webview_window("main").is_none() {
                            if let Ok(u) = format!("{}/", SERVER).parse() {
                                let _ = WebviewWindowBuilder::new(&h2, "main", WebviewUrl::External(u))
                                    .title("My Assistant")
                                    .inner_size(1120.0, 720.0)
                                    .min_inner_size(900.0, 600.0)
                                    .center()
                                    .resizable(true)
                                    .build();
                            }
                        }
                        if let Some(w) = h2.get_webview_window("main") { let _ = w.show(); let _ = w.unminimize(); let _ = w.set_focus(); }
                        make_overlay(&h2, "pill", "pill", PILL_W, PILL_H, true); // click-through: never steals focus
                        position_pill(&h2, PILL_W, PILL_H);
                    });
                });
            }

            // Global dictation hotkey — default ⌃⇧D (Carbon RegisterEventHotKey, needs NO macOS permission).
            // Registered via register_dict_hotkey so Settings can rebind it live; the handler no-ops until
            // dictation is on. The UI re-pushes the stored shortcut on load (set_dictation).
            register_dict_hotkey(&handle, &HotkeySpec { mods: vec!["ctrl".into(), "shift".into()], key: "D".into(), ..Default::default() });
            start_modifier_tap(); // bring up the bare-modifier event tap (right ⌘ / Option triggers); no-op if Accessibility isn't granted yet

            if std::env::var("ASSISTANT_STEALTH").is_ok() {
                if let Some(w) = app.get_webview_window("main") { let _ = w.set_content_protected(true); }
            }

            // Left-click drops a reliable NATIVE menu. The custom transparent popover dismissed itself on
            // blur, so clicking the icon just closed it with nothing shown — native menus always work.
            let show = MenuItem::with_id(app, "show", "Show My Assistant", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit My Assistant", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &settings, &sep, &quit])?;
            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .tooltip("My Assistant")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => { if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.unminimize(); let _ = w.set_focus(); } }
                    "settings" => { if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.set_focus(); let _ = w.eval("try{setView('settings')}catch(e){}"); } }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

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
        .invoke_handler(tauri::generate_handler![set_stealth, set_tray_recording, set_dictation, dictation_stop, set_tray_visible, perm_status])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                tauri::RunEvent::Exit => {
                    // Stop the in-process recorder on quit — signal its thread, which drops the cpal Stream and
                    // releases the mic. (It lives in DICT, not CHILDREN, so it wouldn't be caught below.)
                    if let Ok(mut g) = DICT.lock() { if let Some(rec) = g.rec.take() { rec.stop.store(true, Ordering::Relaxed); } }
                    kill_children(); // release the mic on quit (meeting capture + other sidecars)
                }
                // Clicking the Dock icon when no window is open re-shows the main window (macOS convention),
                // so the app is never "running but unreachable" after the red-X hide.
                tauri::RunEvent::Reopen { .. } => {
                    if let Some(w) = app_handle.get_webview_window("main") {
                        let _ = w.show(); let _ = w.unminimize(); let _ = w.set_focus();
                    }
                }
                _ => {}
            }
        });
}
