//! meeting-state (Windows) — track meeting-app state and stream NDJSON `StateLine`s.
//!
//! REAL / verifiable spine (portable): arg dispatch (`--once`/`--dump`/`--selftest`/`--interval`/
//! `--depth`), the unbuffered line emission (flush after each line), the control-label derivation +
//! participant-name filtering (`platform::meetingstate`, which drives `--selftest`), and the parent
//! watchdog (fail closed when the parent dies).
//!
//! In-call signal: the **primary** signal is an Active WASAPI audio session for the app's PID
//! (`platform::audio::pid_has_active_session`), matching the Swift design where Core Audio is
//! authoritative; UIA window title + toolbar labels enrich it.
//!
//! Wake signal: on macOS this helper slept until a `DistributedNotificationCenter` ping from
//! meeting-tap. On Windows there is no such channel; per PROTOCOL the wake is **core-mediated**
//! (meeting-tap POSTs `coreaudio` state → core → this helper's stdin/lifecycle). This build polls on
//! `--interval` and fails closed on parent death; the core starts/stops it around real calls.
//!
//! TODO(on-device): the UIA toolbar/participant harvesting (`platform::uia::read_meeting`) — the
//! Zoom/Teams participant selectors are the most device-specific piece (the Swift `--dump` workflow).

use std::io::Write;

use helpers_core::args::Args;
use helpers_core::protocol::{MeetingState, StateLine};
use helpers_core::watchdog;

use platform::meetingstate;
use platform::{apps, audio, procs, uia, window};

fn main() {
    let args = Args::from_env();
    let interval = args.opt("interval", "1").parse::<f64>().unwrap_or(1.0).max(0.5);
    let _depth = args.opt("depth", "14").parse::<u32>().unwrap_or(14);

    if args.has("selftest") {
        std::process::exit(selftest());
    }

    if args.has("dump") {
        dump();
        return;
    }

    if args.has("once") {
        let state = snapshot().unwrap_or_default();
        emit(&StateLine {
            kind: "state".into(),
            source: "accessibility".into(),
            ts: Some(helpers_core::epoch_ms()),
            ax_trusted: None,
            msg: None,
            state: Some(state),
        });
        return;
    }

    // watch (default): poll on interval, emit on change, fail closed on parent death.
    let parent = watchdog::parent_pid_from_env();
    let mut last: Option<MeetingState> = None;
    loop {
        if let Some(pid) = parent {
            if !watchdog::is_alive(pid) {
                std::process::exit(0);
            }
        }
        let state = snapshot().unwrap_or_default();
        if last.as_ref() != Some(&state) {
            emit(&StateLine {
                kind: "state".into(),
                source: "accessibility".into(),
                ts: Some(helpers_core::epoch_ms()),
                ax_trusted: None,
                msg: None,
                state: Some(state.clone()),
            });
            last = Some(state);
        }
        std::thread::sleep(std::time::Duration::from_secs_f64(interval));
    }
}

/// First running meeting app, preferring dedicated conferencing apps over chat apps over browsers.
fn running_meeting_app() -> Option<(String, u32)> {
    let procs = procs::all_processes();
    if let Some((pid, exe)) = procs.iter().find(|(_, e)| apps::dedicated_key(e).is_some()) {
        return Some((apps::dedicated_key(exe).unwrap().to_string(), *pid));
    }
    if let Some((pid, exe)) = procs.iter().find(|(_, e)| apps::evidence_key(e).is_some()) {
        return Some((apps::evidence_key(exe).unwrap().to_string(), *pid));
    }
    if let Some((pid, _)) = procs.iter().find(|(_, e)| apps::is_browser(e)) {
        return Some(("browser".to_string(), *pid));
    }
    None
}

/// Build the current meeting state, or `None` if no meeting app is running.
fn snapshot() -> Option<MeetingState> {
    let (key, pid) = running_meeting_app()?;
    let reads = uia::read_meeting(pid);

    let mut s = MeetingState {
        app: key,
        title: reads.window_title,
        ..Default::default()
    };
    // Toolbar/menu action labels ⇒ mute/share/video (and imply inMeeting).
    meetingstate::derive_controls(&reads.button_labels, &mut s);
    // Primary in-call signal: the app is actively playing/capturing audio.
    if audio::pid_has_active_session(pid) {
        s.in_meeting = true;
    }
    s.participants = reads.participants;
    s.active_speaker = reads.active_speaker;
    Some(s)
}

/// Emit one NDJSON line, flushed (stdout must be unbuffered per line).
fn emit(line: &StateLine) {
    if let Ok(s) = serde_json::to_string(line) {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = writeln!(lock, "{s}");
        let _ = lock.flush();
    }
}

/// Dump the detected meeting app + harvested labels/titles to stderr (selector authoring aid).
fn dump() {
    match running_meeting_app() {
        Some((key, pid)) => {
            eprintln!("== {key} pid={pid} ==");
            eprintln!("-- window titles --");
            for t in window::titles_for_pid(pid) {
                eprintln!("  {t}");
            }
            let reads = uia::read_meeting(pid);
            eprintln!("-- button/menu labels (build mute/share/video selectors from here) --");
            for l in reads.button_labels {
                eprintln!("  {l}");
            }
        }
        None => eprintln!("No supported meeting app running."),
    }
}

fn selftest() -> i32 {
    let mut ok = true;
    let mut check = |cond: bool, msg: &str| {
        if !cond {
            eprintln!("SELFTEST FAIL: {msg}");
            ok = false;
        }
    };

    // Codable round-trip.
    let mut s = MeetingState::default();
    s.app = "zoom".into();
    s.in_meeting = true;
    s.self_muted = Some(true);
    let json = serde_json::to_string(&s).unwrap();
    check(json.contains("\"selfMuted\":true"), "state serializes camelCase");

    // Control derivation — the load-bearing logic.
    let mut m = MeetingState::default();
    meetingstate::derive_controls(&["Unmute".into(), "Start video".into(), "Stop share".into()], &mut m);
    check(
        m.self_muted == Some(true) && m.video_on == Some(false) && m.sharing == Some(true) && m.in_meeting,
        "muted/sharing derive",
    );
    let mut n = MeetingState::default();
    meetingstate::derive_controls(&["File".into(), "Edit".into()], &mut n);
    check(n.self_muted.is_none() && !n.in_meeting, "no-meeting derive stays nil");

    // Name filtering.
    check(!meetingstate::valid_name("Set up professional audio in Audio settings"), "reject UI chrome");
    check(meetingstate::valid_name("Rahul Khatri"), "accept a real name");
    check(meetingstate::valid_name("Sharen"), "keep ui-word substring name");
    check(meetingstate::canon_name("Rahul Khatri (Me)") == "Rahul Khatri", "canon strips (Me)");

    if ok {
        eprintln!("SELFTEST OK");
        0
    } else {
        1
    }
}
