use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Manager,
};

// ponytail: dev-only — make sure the GUI/relay server (SSE + meeting API) is up so the window isn't
// blank. The full capture stack (whisper-server + the Swift helpers, which need the Screen-Recording
// grant) still comes from ./run.sh for now; wiring those in as app-managed sidecars is a later slice.
fn ensure_gui_server() {
    let addr = "127.0.0.1:8790".parse().unwrap();
    if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok() {
        return; // already running (typically from ./run.sh)
    }
    let server = concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/server.js");
    let _ = Command::new("node").arg(server).spawn();
}

// Hide/show the window from screen-share & screenshots — NSWindow.sharingType = .none on macOS,
// WDA_EXCLUDEFROMCAPTURE on Windows. Still fully visible to the user. Called from the UI
// (Settings → Privacy → Invisible to screen-share).
#[tauri::command]
fn set_stealth(window: tauri::WebviewWindow, on: bool) -> Result<(), String> {
    window.set_content_protected(on).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            ensure_gui_server();

            // Verification/testing hook: launch with ASSISTANT_STEALTH=1 to start hidden from capture.
            if std::env::var("ASSISTANT_STEALTH").is_ok() {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_content_protected(true);
                }
            }

            // Menu-bar tray: show / hide / quit. (Stealth lives in Settings → Privacy.)
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
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![set_stealth])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
