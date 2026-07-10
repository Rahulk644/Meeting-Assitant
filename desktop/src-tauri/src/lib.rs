use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
};

// Absolute path to the repo, baked at build time. Fine for a build you run on your own machine; a fully
// portable installer would instead bundle app/ + the Swift binaries + the whisper model as app resources.
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const SERVER: &str = "http://127.0.0.1:8790";

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
    let ms = format!("{}/meeting-state/meeting-state", REPO);
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
                .arg(format!("{}/app/server.js", REPO))
                .env("PORT", "8790")
                // Packaged: the app spawns meeting-state itself (a DIRECT child, so it gets the AX grant) —
                // tell node to skip its own copy. In dev (debug), node owns meeting-state under Terminal's grant.
                .env("ASSISTANT_RUST_MEETING_STATE", if cfg!(debug_assertions) { "0" } else { "1" })
                .stdout(Stdio::null()).stderr(Stdio::null()).spawn());
        }
    }
    if cfg!(debug_assertions) { return; } // dev: capture comes from ./run.sh

    // whisper-server is NOT started here — node spawns it on demand when a call begins and unloads it when
    // idle (it's ~1.5 GB), so it's off RAM between calls.
    let tap = format!("{}/stt/meeting-tap", REPO);
    if Path::new(&tap).exists() {
        let recdir = format!("{}/data/meetings/rec", REPO);
        let log = std::fs::File::create("/tmp/assistant-tap.log").map(Stdio::from).unwrap_or_else(|_| Stdio::null());
        track(Command::new(tap)
            .args(["8790", "http://127.0.0.1:8091/inference", "--gate", "on", "--rec-dir", &recdir])
            .stdout(Stdio::null()).stderr(log).spawn());
    }
    start_meeting_state(); // names + participants — direct child so Accessibility works
}

// ---- overlays: the floating Dictation Pill + the Menu Bar Tray popover -----------------------------
// Two borderless, transparent, always-on-top windows loading app/overlay.html from the local server. The
// PILL is click-through (set_ignore_cursor_events) so it floats over whatever you're dictating into and can
// NEVER steal focus — the dictated text always pastes into your document. (Making the pill's buttons
// mouse-clickable needs a non-activating NSPanel; that's a follow-up.) The TRAY is interactive.
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

// Place the pill centred, near the bottom of the primary screen.
fn position_pill(app: &AppHandle, w: f64, h: f64) {
    let Some(win) = app.get_webview_window("pill") else { return };
    let _ = win.set_size(LogicalSize::new(w, h));
    if let Ok(Some(mon)) = win.primary_monitor() {
        let sf = mon.scale_factor();
        let sw = mon.size().width as f64 / sf;
        let sh = mon.size().height as f64 / sf;
        let _ = win.set_position(LogicalPosition::new((sw - w) / 2.0, sh - h - 10.0)); // hug the bottom rim, like Handy
    }
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

// ---- Tauri commands the overlay JS calls (window.__TAURI__.core.invoke) ----------------------------
#[tauri::command]
fn set_stealth(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    window.set_content_protected(on).map_err(|e| e.to_string())
}

// One entry point for every overlay button. `on`/`min` ride along for the pill.
#[tauri::command]
fn overlay_action(app: AppHandle, action: String, on: Option<bool>, min: Option<bool>) {
    let open_main = |a: &AppHandle| {
        if let Some(w) = a.get_webview_window("main") { let _ = w.show(); let _ = w.unminimize(); let _ = w.set_focus(); }
    };
    match action.as_str() {
        "pill_visible" => {
            if let Some(w) = app.get_webview_window("pill") {
                if on.unwrap_or(false) { position_pill(&app, PILL_W, PILL_H); let _ = w.show(); }
                else { let _ = w.hide(); }
            }
        }
        "pill_size" => { // minimized chip vs full pill — resize + re-centre so clicks map to the visible pill
            if min.unwrap_or(false) { position_pill(&app, 132.0, 44.0); }
            else { position_pill(&app, PILL_W, PILL_H); }
        }
        "tray_hide" => { if let Some(w) = app.get_webview_window("tray") { let _ = w.hide(); } }
        "open_main" => open_main(&app),
        "settings"  => { open_main(&app); if let Some(w) = app.get_webview_window("main") { let _ = w.eval("try{setView('settings')}catch(e){}"); } }
        "open_writing" => { open_main(&app); if let Some(w) = app.get_webview_window("main") { let _ = w.eval("try{setView('writing')}catch(e){}"); } }
        "stop_dictation" => dictation_stop(),
        "quit" => app.exit(0),
        _ => {}
    }
    if action != "pill_visible" && action != "pill_size" {
        if let Some(w) = app.get_webview_window("tray") { let _ = w.hide(); }
    }
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
fn vad_extract_speech(samples16k: &[f32]) -> Vec<f32> {
    use voice_activity_detector::{IteratorExt, VoiceActivityDetector};
    let mut vad = match VoiceActivityDetector::builder().sample_rate(16_000i64).chunk_size(512usize).build() {
        Ok(v) => v,
        Err(_) => return samples16k.to_vec(),
    };
    // threshold 0.5 = Silero's default speech probability; pad 3 chunks (~96 ms) so onsets/tails aren't clipped.
    let mut out = Vec::with_capacity(samples16k.len());
    for chunk in samples16k.iter().copied().label(&mut vad, 0.5f32, 3usize) {
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
    let _ = Command::new("curl")
        .args(["-s", "--max-time", "5", "-X", "POST", &format!("{}/ingest", SERVER), // bounded: never wedge a finish thread if node stalls
               "-H", "Content-Type: application/json", "-d", json])
        .stdout(Stdio::null()).stderr(Stdio::null()).status();
}
fn emit_dstate(phase: &str) { ingest(&format!("{{\"type\":\"dictation-state\",\"phase\":\"{}\"}}", phase)); }

// AppHandle stashed at setup so the dictation fns can give RELIABLE feedback: a start/stop SOUND (the
// primary "is it on?" cue, like Handy) + the menu-bar title "● Dictating" + the pill window (shown/hidden
// from Rust here, so it no longer depends on the overlay webview calling back — which was silently failing).
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();
fn play(sound: &str) {
    // Reap afplay (.status() waits) on a detached thread so it doesn't accumulate as a zombie on each
    // start/stop cue — a bare spawn() is never reaped and would pile up over a long session.
    let path = format!("/System/Library/Sounds/{}.aiff", sound);
    std::thread::spawn(move || { let _ = Command::new("afplay").arg(path).status(); });
}
fn dict_feedback(on: bool) {
    if let Some(app) = APP.get() {
        // MUST run on the main thread — tray/window (Cocoa) calls from the transcription thread abort the
        // app (SIGABRT). run_on_main_thread marshals it safely from whatever thread we're on.
        let app = app.clone();
        let handle = app.clone();
        let _ = handle.run_on_main_thread(move || {
            if let Some(t) = app.tray_by_id("tray") { let _ = t.set_title(Some(if on { " ● Dictating" } else { "" })); }
            if let Some(w) = app.get_webview_window("pill") {
                if on {
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
    let old = dict_get_clip();
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
    dict_set_clip(&old);
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

// UI → turn dictation on/off. On an EXPLICIT enable (prompt=true) we ask for Accessibility under THIS
// app's identity so the grant covers the paste; boot passes prompt=false so a relaunch never nags.
#[tauri::command]
fn set_dictation(enabled: bool, ptt: bool, prompt: bool) {
    DICT.lock().unwrap().enabled = enabled;
    DICT_PTT.store(ptt, Ordering::Relaxed);
    let _ = prompt;
    if enabled {
        // Prompts ONLY when not already trusted; silent once granted (the stable cert makes it stick). This
        // is what re-asks once after a signature change / TCC reset, then never again — the paste needs it.
        let _ = macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
    }
}
// Tray "Stop" → finish the current dictation (only if one is recording).
#[tauri::command]
fn dictation_stop() { dictation_stop_rec(); }

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())?;
            }

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
                        make_overlay(&h2, "pill", "pill", PILL_W, PILL_H, true); // click-through: never steals focus
                        position_pill(&h2, PILL_W, PILL_H);
                    });
                });
            }

            // Global dictation hotkey (⌃⇧D) — Carbon RegisterEventHotKey, needs NO macOS permission. Tap
            // to start, tap to stop. Registered unconditionally; the handler no-ops until dictation is on.
            {
                use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, Modifiers, Code, ShortcutState};
                let sc = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyD);
                let _ = handle.global_shortcut().on_shortcut(sc, |_app, _scut, event| {
                    let ptt = DICT_PTT.load(Ordering::Relaxed);
                    match event.state() {
                        ShortcutState::Pressed => if ptt { dictation_start() } else { dictation_toggle() },
                        ShortcutState::Released => if ptt { dictation_stop_rec() },
                    }
                });
            }

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
            match event {
                // macOS convention: the red X HIDES the main window (the app keeps running in the tray).
                tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                // The tray popover dismisses when it loses focus, like a real menu.
                tauri::WindowEvent::Focused(false) if window.label() == "tray" => { let _ = window.hide(); }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![set_stealth, set_tray_recording, overlay_action, set_dictation, dictation_stop])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // Stop the in-process recorder on quit — signal its thread, which drops the cpal Stream and
                // releases the mic. (It lives in DICT, not CHILDREN, so it wouldn't be caught below.)
                if let Ok(mut g) = DICT.lock() { if let Some(rec) = g.rec.take() { rec.stop.store(true, Ordering::Relaxed); } }
                kill_children(); // release the mic on quit (meeting capture + other sidecars)
            }
        });
}
