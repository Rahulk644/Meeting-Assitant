use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Manager,
};

// Absolute path to the repo, baked at build time. Fine for a build you run on your own machine; a fully
// portable installer would instead bundle app/ + the Swift binaries + the whisper model as app resources.
const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

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

// Hide/show the window from screen-share & screenshots (NSWindow.sharingType = .none on macOS,
// WDA_EXCLUDEFROMCAPTURE on Windows) — still visible to the user. From Settings → Privacy.
#[tauri::command]
fn set_stealth(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    window.set_content_protected(on).map_err(|e| e.to_string())
}

// Animate the menu-bar tray with an audio-wave title while a call records (the UI calls this on start/stop).
static TRAY_REC: AtomicBool = AtomicBool::new(false);
#[tauri::command]
fn set_tray_recording(app: tauri::AppHandle, on: bool) {
    if on {
        if TRAY_REC.swap(true, Ordering::SeqCst) { return; } // already animating
        std::thread::spawn(move || {
            let frames = ["▁▂▃▅▃▂", "▂▃▅▇▅▃", "▃▅▇▅▃▂", "▂▄▆▄▂▁"];
            let mut i = 0usize;
            while TRAY_REC.load(Ordering::SeqCst) {
                if let Some(t) = app.tray_by_id("tray") { let _ = t.set_title(Some(format!(" {}", frames[i % frames.len()]))); }
                i += 1;
                std::thread::sleep(Duration::from_millis(300));
            }
            if let Some(t) = app.tray_by_id("tray") { let _ = t.set_title(None::<&str>); }
        });
    } else {
        TRAY_REC.store(false, Ordering::SeqCst);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())?;
            }

            start_stack();

            if std::env::var("ASSISTANT_STEALTH").is_ok() {
                if let Some(w) = app.get_webview_window("main") { let _ = w.set_content_protected(true); }
            }

            let show = MenuItem::with_id(app, "show", "Show Assistant", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Assistant", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &sep, &quit])?;
            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("Assistant")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => { if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.set_focus(); } }
                    "hide" => { if let Some(w) = app.get_webview_window("main") { let _ = w.hide(); } }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![set_stealth, set_tray_recording])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::Exit = event { kill_children(); } // release the mic on quit
        });
}
