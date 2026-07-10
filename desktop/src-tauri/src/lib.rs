use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
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
const PILL_W: f64 = 360.0;
const PILL_H: f64 = 72.0;
const TRAY_W: f64 = 316.0;
const TRAY_H: f64 = 470.0;

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
        let _ = win.set_position(LogicalPosition::new((sw - w) / 2.0, sh - h - 84.0));
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
            if let Some(t) = app.tray_by_id("tray") { let _ = t.set_title(Some("")); } // clear — None doesn't clear on macOS
        });
    } else {
        TRAY_REC.store(false, Ordering::SeqCst);
        if let Some(t) = app.tray_by_id("tray") { let _ = t.set_title(Some("")); }
    }
}

// Show/hide the tray popover just under the menu-bar icon (from its click position).
fn toggle_tray(app: &AppHandle, icon_x_physical: f64) {
    let Some(w) = app.get_webview_window("tray") else { return };
    if w.is_visible().unwrap_or(false) { let _ = w.hide(); return; }
    let sf = w.scale_factor().unwrap_or(1.0);
    let icon_x = icon_x_physical / sf;
    let x = (icon_x - TRAY_W + 14.0).max(8.0);
    let _ = w.set_size(LogicalSize::new(TRAY_W, TRAY_H));
    let _ = w.set_position(LogicalPosition::new(x, 30.0)); // just below the ~24px menu bar
    let _ = w.show();
    let _ = w.set_focus();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())?;
            }

            start_stack();
            let handle = app.handle().clone();
            make_overlay(&handle, "pill", "pill", PILL_W, PILL_H, true);  // click-through: never steals focus
            make_overlay(&handle, "tray", "tray", TRAY_W, TRAY_H, false); // interactive popover
            position_pill(&handle, PILL_W, PILL_H);

            if std::env::var("ASSISTANT_STEALTH").is_ok() {
                if let Some(w) = app.get_webview_window("main") { let _ = w.set_content_protected(true); }
            }

            // Right-click keeps a minimal native menu; left-click opens the custom popover.
            let show = MenuItem::with_id(app, "show", "Show My Assistant", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit My Assistant", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &sep, &quit])?;
            TrayIconBuilder::with_id("tray")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("My Assistant")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => { if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.set_focus(); } }
                    "hide" => { if let Some(w) = app.get_webview_window("main") { let _ = w.hide(); } }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, position, .. } = event {
                        toggle_tray(tray.app_handle(), position.x);
                    }
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
        .invoke_handler(tauri::generate_handler![set_stealth, set_tray_recording, overlay_action])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::Exit = event { kill_children(); } // release the mic on quit
        });
}
