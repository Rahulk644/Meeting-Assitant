use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
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
fn is_running(name: &str) -> bool {
    Command::new("/usr/bin/pgrep").arg("-f").arg(name).output().map(|o| o.status.success()).unwrap_or(false)
}

// Bring up the on-device stack. The Node GUI/relay server is always needed for the UI (it also spawns
// meeting-state for names + in-call detection). whisper-server + the capturer are the transcription
// pipeline: a PACKAGED build hosts them itself (no dev commands), while `npm run dev` leaves them to
// ./run.sh under Terminal's Screen-Recording grant.
fn start_stack() {
    if !port_open(8790) {
        if let Some(node) = find_bin("node") {
            track(Command::new(node)
                .arg(format!("{}/app/server.js", REPO)).arg("--ingest")
                .env("PORT", "8790")
                .stdout(Stdio::null()).stderr(Stdio::null()).spawn());
        }
    }
    if cfg!(debug_assertions) { return; } // dev: capture comes from ./run.sh

    if !port_open(8091) {
        let model = format!("{}/stt/whisper/ggml-large-v3-turbo-q8_0.bin", REPO);
        if let (Some(ws), true) = (find_bin("whisper-server"), Path::new(&model).exists()) {
            track(Command::new(ws)
                .args(["-m", &model, "--host", "127.0.0.1", "--port", "8091", "-t", "4"])
                .stdout(Stdio::null()).stderr(Stdio::null()).spawn());
        }
    }
    let tap = format!("{}/stt/meeting-tap", REPO);
    if Path::new(&tap).exists() && !is_running("meeting-tap") {
        track(Command::new(tap)
            .args(["8790", "http://127.0.0.1:8091/inference", "--gate", "on"])
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn());
    }
}

// Hide/show the window from screen-share & screenshots (NSWindow.sharingType = .none on macOS,
// WDA_EXCLUDEFROMCAPTURE on Windows) — still visible to the user. From Settings → Privacy.
#[tauri::command]
fn set_stealth(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    window.set_content_protected(on).map_err(|e| e.to_string())
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
            TrayIconBuilder::new()
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
        .invoke_handler(tauri::generate_handler![set_stealth])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::Exit = event { kill_children(); } // release the mic on quit
        });
}
