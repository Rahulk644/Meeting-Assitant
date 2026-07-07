use std::net::TcpStream;
use std::process::Command;
use std::time::Duration;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
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

            // Menu-bar tray: show / hide / stealth / quit.
            let show = MenuItem::with_id(app, "show", "Show Assistant", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
            // Keystone for the future Copilot: hide the window from screen-share / screenshots
            // (NSWindow.sharingType = .none on macOS, WDA_EXCLUDEFROMCAPTURE on Windows) while it stays
            // fully visible to you. Off by default here; the Copilot overlay will default it on.
            let stealth = CheckMenuItem::with_id(
                app,
                "stealth",
                "Stealth · hide from screen share",
                true,
                false,
                None::<&str>,
            )?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Assistant", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &stealth, &sep, &quit])?;

            let stealth_h = stealth.clone();
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("Assistant")
                .on_menu_event(move |app, event| match event.id.as_ref() {
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
                    "stealth" => {
                        // CheckMenuItem toggles its own tick on click; mirror it onto the window.
                        if let (Some(w), Ok(on)) =
                            (app.get_webview_window("main"), stealth_h.is_checked())
                        {
                            let _ = w.set_content_protected(on);
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
