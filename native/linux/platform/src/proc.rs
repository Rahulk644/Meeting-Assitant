//! Meeting-app process enumeration — REAL `sysinfo`. Feature `procenum`.
//!
//! The Linux analog of the macOS `MEETING_APPS` bundle-ID table (meeting-state.swift). macOS matches
//! `NSRunningApplication.bundleIdentifier`; Linux has no bundle IDs, so we match the process
//! **binary name** (`application.process.binary`, per PROTOCOL.md's "map per platform" note). The
//! emitted `app` string is the same canonical key vocabulary the core already knows
//! (`zoom`, `teams`, `slack`, `discord`, `telegram`, `webex`, `browser`, …).
//!
//! REAL and verifiable: the table + matching logic (unit-tested against synthetic names). The live
//! `sysinfo` scan is a thin std call. Selector tuning of exact Linux binary names is the only
//! on-device unknown, marked below.

use sysinfo::{ProcessesToUpdate, System};

/// One canonical meeting-app key and the Linux process-binary names that map to it. Names are
/// matched case-insensitively against the process basename (exact or, for the `*_contains` set,
/// substring). Table order is the tie-break, mirroring the Swift table's first-wins.
pub struct MeetingApp {
    pub key: &'static str,
    /// Exact basename matches (case-insensitive).
    pub exact: &'static [&'static str],
    /// Substring matches, for launchers/wrappers with version suffixes.
    pub contains: &'static [&'static str],
}

// TODO(on-device): these Linux binary names are the well-known ones as of 2024–2025 but MUST be
// confirmed with `meeting-state --dump` on a real box — packaging (deb/rpm/Flatpak/Snap) and
// electron wrappers rename binaries (e.g. Zoom is `zoom` but Snap wraps it; Teams community build is
// `teams-for-linux`). The keys are frozen; only the match strings may need widening on-device.
pub const MEETING_APPS: &[MeetingApp] = &[
    MeetingApp { key: "zoom", exact: &["zoom", "ZoomLauncher"], contains: &["zoom"] },
    MeetingApp {
        key: "teams",
        exact: &["teams", "teams-for-linux", "msteams", "microsoft-teams"],
        contains: &["teams-for-linux", "msteams"],
    },
    MeetingApp {
        key: "webex",
        exact: &["CiscoCollabHost", "webex", "webexmeetingsapp"],
        contains: &["webex", "ciscocollab"],
    },
    MeetingApp { key: "slack", exact: &["slack"], contains: &["slack"] },
    MeetingApp { key: "discord", exact: &["Discord", "discord"], contains: &["discord"] },
    MeetingApp {
        key: "goto",
        exact: &["GoToMeeting", "GoTo", "goto"],
        contains: &["gotomeeting"],
    },
    MeetingApp {
        key: "telegram",
        exact: &["telegram-desktop", "Telegram", "telegram"],
        contains: &["telegram"],
    },
    MeetingApp {
        key: "signal",
        exact: &["signal-desktop", "signal"],
        contains: &["signal-desktop"],
    },
    MeetingApp { key: "whatsapp", exact: &["whatsapp", "whatsapp-for-linux"], contains: &["whatsapp"] },
    MeetingApp {
        key: "browser",
        exact: &[
            "firefox", "firefox-bin", "firefox-esr", "chrome", "google-chrome",
            "google-chrome-stable", "chromium", "chromium-browser", "brave", "brave-browser",
            "microsoft-edge", "microsoft-edge-stable", "msedge", "opera", "vivaldi", "vivaldi-bin",
        ],
        contains: &["chrome", "chromium", "firefox", "brave", "edge", "opera", "vivaldi"],
    },
];

/// True if the canonical key represents a web browser (Google Meet / Jitsi / etc. live in a tab).
pub fn key_is_browser(key: &str) -> bool {
    key == "browser"
}

/// Match a single process basename to a canonical key, table order first-wins.
pub fn key_for_binary(binary: &str) -> Option<&'static str> {
    let lower = binary.to_ascii_lowercase();
    for app in MEETING_APPS {
        if app.exact.iter().any(|e| e.eq_ignore_ascii_case(binary))
            || app.contains.iter().any(|c| lower.contains(&c.to_ascii_lowercase()))
        {
            return Some(app.key);
        }
    }
    None
}

/// A running meeting app: its canonical key + pid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningApp {
    pub key: &'static str,
    pub pid: u32,
}

/// Scan the process table for a running meeting app. When `preferred_key` is `Some`, only a process
/// mapping to that key is returned (the meeting-tap → meeting-state hand-off passes the app the
/// audio detector locked onto). Returns the first table-order match.
pub fn running_meeting_app(preferred_key: Option<&str>) -> Option<RunningApp> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    // Iterate the table in order (deterministic) and, for each key, look for any matching process.
    for app in MEETING_APPS {
        if let Some(want) = preferred_key {
            if want != app.key {
                continue;
            }
        }
        for (pid, proc_) in sys.processes() {
            let name = proc_.name().to_string_lossy();
            if matches_app(app, &name) {
                return Some(RunningApp { key: app.key, pid: pid.as_u32() });
            }
        }
    }
    None
}

fn matches_app(app: &MeetingApp, binary: &str) -> bool {
    let lower = binary.to_ascii_lowercase();
    app.exact.iter().any(|e| e.eq_ignore_ascii_case(binary))
        || app.contains.iter().any(|c| lower.contains(&c.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_binaries() {
        assert_eq!(key_for_binary("zoom"), Some("zoom"));
        assert_eq!(key_for_binary("teams-for-linux"), Some("teams"));
        assert_eq!(key_for_binary("CiscoCollabHost"), Some("webex"));
        assert_eq!(key_for_binary("google-chrome-stable"), Some("browser"));
        assert_eq!(key_for_binary("firefox"), Some("browser"));
        assert_eq!(key_for_binary("Discord"), Some("discord"));
    }

    #[test]
    fn unknown_binary_is_none() {
        assert_eq!(key_for_binary("gnome-calculator"), None);
    }

    #[test]
    fn browser_key_flagged() {
        assert!(key_is_browser("browser"));
        assert!(!key_is_browser("zoom"));
    }
}
