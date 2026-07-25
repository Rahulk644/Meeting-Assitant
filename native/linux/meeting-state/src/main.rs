//! meeting-state (Linux) — track meeting-app state. PROTOCOL.md §5.
//!
//! REAL / verifiable: arg/mode dispatch, the whole derive brain (`derive.rs`, unit-tested), meeting-
//! app process enumeration (`platform::proc`), X11 window titles (`platform::win`), unbuffered NDJSON
//! emission + change dedup, the ≥2 s watch-interval clamp, parent-death fail-closed, and `--selftest`.
//!
//! TODO(on-device): AT-SPI menu/participant reads (`atspi_tree.rs`) — needs a live tree (+
//! QT_ACCESSIBILITY for Zoom). WAYLAND LIMITATION: X11 window enumeration is impossible under
//! Wayland (`platform::win` returns nothing), so titles fall back to AT-SPI window names — a hard
//! capability loss documented at the call site. Wake is done by polling on the clamped interval; the
//! intended optimization is a zbus session-bus "meeting-activity" signal (TODO on-device), replacing
//! the macOS DistributedNotificationCenter.

mod atspi_tree;
mod derive;

use helpers_core::epoch_ms;
use helpers_core::protocol::{MeetingState, StateLine};
use helpers_core::watchdog::{is_alive, parent_pid_from_env};
use platform::display::{self, DisplayServer};
use platform::win::X11;
use std::io::Write;
use std::time::Duration;

enum Mode {
    Watch,
    Once,
    Dump,
    Selftest,
}

struct Config {
    mode: Mode,
    interval: f64,
    #[allow(dead_code)]
    depth: usize,
}

fn parse_args() -> Config {
    let mut cfg = Config { mode: Mode::Watch, interval: 1.0, depth: 14 };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--once" => cfg.mode = Mode::Once,
            "--dump" => cfg.mode = Mode::Dump,
            "--selftest" => cfg.mode = Mode::Selftest,
            "--interval" => {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<f64>().ok()) {
                    cfg.interval = v;
                }
                i += 1;
            }
            "--depth" => {
                if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<usize>().ok()) {
                    cfg.depth = v;
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    cfg
}

fn main() {
    let cfg = parse_args();

    if let Mode::Selftest = cfg.mode {
        if derive::selftest() {
            eprintln!("SELFTEST OK");
            std::process::exit(0);
        } else {
            eprintln!("SELFTEST FAIL");
            std::process::exit(1);
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    match cfg.mode {
        Mode::Once => rt.block_on(run_once()),
        Mode::Dump => rt.block_on(run_dump()),
        Mode::Watch => rt.block_on(run_watch(cfg.interval)),
        Mode::Selftest => unreachable!(),
    }
}

/// One snapshot line, exit 0. Unlike macOS we do NOT hard-fail when AT-SPI is off: X11 window titles
/// are accessibility-independent and remain useful, so we emit whatever we can.
async fn run_once() {
    let state = snapshot(None).await;
    emit_state(&state);
}

/// Dump diagnostics to STDERR for selector authoring, then exit.
async fn run_dump() {
    let running = platform::proc::running_meeting_app(None);
    let Some(app) = running else {
        eprintln!("No supported meeting app running.");
        std::process::exit(1);
    };
    eprintln!("== {} pid={} ==", app.key, app.pid);
    eprintln!("-- window titles (X11; empty on Wayland) --");
    if let Some(x) = x11_conn() {
        let mut titles = x.window_titles_for_pid(app.pid);
        if titles.is_empty() {
            titles = x.all_window_titles();
        }
        for t in titles {
            eprintln!("  {t}");
        }
    } else {
        eprintln!("  (no X11 connection — Wayland or headless; window enumeration unavailable)");
    }
    eprintln!("-- menu titles (AT-SPI; TODO on-device) --");
    for t in atspi_tree::menu_titles(app.pid).await {
        eprintln!("  {t}");
    }
    eprintln!("-- build participant/active-speaker selectors from the live tree with QT_ACCESSIBILITY=1 --");
    std::process::exit(0);
}

/// The watch loop: poll on the clamped interval, emit on change, fail closed on parent death.
async fn run_watch(cli_interval: f64) {
    // Effective interval floor is 2.0 s (Swift `max(interval, 2.0)`).
    let interval = Duration::from_secs_f64(cli_interval.max(2.0));
    let parent = parent_pid_from_env();
    let mut last_state: Option<MeetingState> = None;
    let mut permission_status_sent = false;

    loop {
        // Fail closed the moment the parent dies.
        if let Some(pid) = parent {
            if !is_alive(pid) {
                std::process::exit(0);
            }
        }

        // Informational status line when accessibility is off (emitted once until it returns).
        if atspi_tree::ax_trusted().await {
            permission_status_sent = false;
        } else if !permission_status_sent {
            emit_status(false, "Accessibility off — speaker names disabled for this meeting");
            permission_status_sent = true;
        }

        let state = snapshot(last_state.as_ref().map(|s| s.app.as_str()).filter(|s| !s.is_empty())).await;
        if last_state.as_ref() != Some(&state) {
            emit_state(&state);
            last_state = Some(state);
        }

        // TODO(on-device): replace this poll with a zbus session-bus subscription to the
        // "meeting-activity" wake signal (published by meeting-tap), only reconciling when woken —
        // the Linux replacement for macOS DistributedNotificationCenter. Polling is the safe baseline.
        tokio::time::sleep(interval).await;
    }
}

/// Build a meeting-state snapshot. `preferred_key` biases process detection to the app the audio
/// detector locked onto (meeting-tap → meeting-state hand-off).
async fn snapshot(preferred_key: Option<&str>) -> MeetingState {
    let Some(app) = platform::proc::running_meeting_app(preferred_key) else {
        return MeetingState::default(); // app "", inMeeting false
    };
    let server = display::detect();

    // --- window titles ---
    let titles = window_titles(server, app.pid).await;

    // --- controls (AT-SPI menu labels; TODO on-device → empty) ---
    let menu = atspi_tree::menu_titles(app.pid).await;
    let controls = derive::derive_controls(&menu);

    // --- participants / active speaker (Zoom only; TODO on-device → empty) ---
    let zoom = if app.key == "zoom" {
        atspi_tree::zoom_names(app.pid).await
    } else {
        atspi_tree::ZoomNames::default()
    };

    let window_meeting = derive::window_in_meeting(&titles, app.key);
    let in_meeting = controls.in_meeting || window_meeting || !zoom.participants.is_empty();
    let title = derive::useful_window_title(&titles);

    MeetingState {
        app: app.key.to_string(),
        title: if title.is_empty() { None } else { Some(title) },
        in_meeting,
        self_muted: controls.self_muted,
        video_on: controls.video_on,
        sharing: controls.sharing,
        participants: zoom.participants,
        active_speaker: zoom.active_speaker,
    }
}

/// Window titles: X11 real; Wayland falls back to AT-SPI window names (documented hard loss).
async fn window_titles(server: DisplayServer, pid: u32) -> Vec<String> {
    if server.can_enumerate_windows() {
        if let Some(x) = x11_conn() {
            let by_pid = x.window_titles_for_pid(pid);
            if !by_pid.is_empty() {
                return by_pid;
            }
            return x.all_window_titles();
        }
    }
    // WAYLAND LIMITATION: no cross-client window enumeration — AT-SPI window names are the only
    // source, and only if a11y is on. This is strictly weaker than X11.
    atspi_tree::window_names(pid).await
}

fn x11_conn() -> Option<X11> {
    X11::connect()
}

// ---- NDJSON emission (unbuffered: flush after every line) ----

fn emit_state(state: &MeetingState) {
    let line = StateLine {
        kind: "state".to_string(),
        source: "accessibility".to_string(),
        ts: Some(epoch_ms()),
        ax_trusted: None,
        msg: None,
        state: Some(state.clone()),
    };
    write_json_line(&line);
}

fn emit_status(ax_trusted: bool, msg: &str) {
    let line = StateLine {
        kind: "status".to_string(),
        source: "accessibility".to_string(),
        ts: None,
        ax_trusted: Some(ax_trusted),
        msg: Some(msg.to_string()),
        state: None,
    };
    write_json_line(&line);
}

fn write_json_line(line: &StateLine) {
    if let Ok(s) = serde_json::to_string(line) {
        let stdout = std::io::stdout();
        let mut h = stdout.lock();
        let _ = h.write_all(s.as_bytes());
        let _ = h.write_all(b"\n");
        let _ = h.flush();
    }
}
