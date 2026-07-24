//! Meeting-state derivation — control-label logic + participant-name filtering. **Fully portable and
//! real** (pure string logic). Ports the load-bearing bits of `meeting-state.swift`:
//! `deriveControls`, `validName`, `canonName`, `nameFromTileDesc`. These drive `meeting-state
//! --selftest`. On Windows the raw labels come from UI Automation toolbar buttons instead of macOS
//! menu-bar items, but the ACTION-vs-STATE derivation is identical.

use helpers_core::protocol::MeetingState;

/// A meeting app shows a toggle labelled with the ACTION, not the current state:
///   muted   → "Unmute" ;  unmuted → "Mute"
///   sharing → "Stop share" / "Stop sharing" ;  not → "Share" / "Start share"
///   video   → "Stop video" / "Turn camera off" ;  off → "Start video" / "Turn camera on"
///
/// Presence of any in-call control implies `inMeeting = true`. Matches the Swift `deriveControls`.
pub fn derive_controls(labels: &[String], s: &mut MeetingState) {
    let low: Vec<String> = labels.iter().map(|l| l.to_lowercase()).collect();
    // needle-set match: some label contains ALL of the needles.
    let has = |needles: &[&str]| -> bool {
        low.iter().any(|t| needles.iter().all(|n| t.contains(n)))
    };

    // Check "unmute" before "mute" ("unmute" contains "mute").
    if has(&["unmute"]) {
        s.self_muted = Some(true);
    } else if has(&["mute"]) {
        s.self_muted = Some(false);
    }

    if has(&["stop", "video"]) || has(&["turn", "camera", "off"]) {
        s.video_on = Some(true);
    } else if has(&["start", "video"]) || has(&["turn", "camera", "on"]) {
        s.video_on = Some(false);
    }

    if has(&["stop", "share"]) {
        s.sharing = Some(true);
    } else if has(&["start", "share"]) || (has(&["share"]) && !has(&["stop"])) {
        s.sharing = Some(false);
    }

    if s.self_muted.is_some() || s.sharing.is_some() {
        s.in_meeting = true;
    }
}

/// Accept a plausible human name; reject Zoom/Teams UI chrome by whole-word match so real names that
/// merely embed a UI word as a substring ("Sharen", "Mike") survive. Ports the Swift `validName`.
pub fn valid_name(s: &str) -> bool {
    let n = s.trim();
    let len = n.chars().count();
    if len < 2 || len > 40 {
        return false;
    }
    if !n.chars().any(|c| c.is_alphabetic()) {
        return false;
    }
    if n.split_whitespace().count() > 5 {
        return false;
    }
    const UI_WORDS: &[&str] = &[
        "zoom", "audio", "video", "mute", "unmute", "muted", "share", "sharing", "settings",
        "setting", "microphone", "mic", "speaker", "camera", "waiting", "participants",
        "participant", "reactions", "reaction", "profile", "record", "recording", "screen",
        "gallery",
    ];
    let tokens: std::collections::HashSet<String> = n
        .to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    !UI_WORDS.iter().any(|w| tokens.contains(*w))
}

/// Drop a trailing "(...)" annotation so the same person dedupes:
/// "Rahul Khatri (Me)" / "Rahul Khatri (Host, me)" → "Rahul Khatri". Ports the Swift `canonName`.
pub fn canon_name(s: &str) -> String {
    let base = match s.find('(') {
        Some(i) => &s[..i],
        None => s,
    };
    base.trim().to_string()
}

/// "Rahul Khatri, Computer audio muted, Video off" → "Rahul Khatri". Ports `nameFromTileDesc`.
pub fn name_from_tile_desc(d: &str) -> Option<String> {
    let low = d.to_lowercase();
    if !d.contains(',') || !(low.contains("audio") || low.contains("video") || low.contains("muted")) {
        return None;
    }
    let first = d.split(',').next().unwrap_or("").trim();
    let name = canon_name(first);
    if valid_name(&name) {
        Some(name)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_muted_and_sharing() {
        let mut m = MeetingState::default();
        derive_controls(
            &["Unmute".into(), "Start video".into(), "Stop share".into()],
            &mut m,
        );
        assert_eq!(m.self_muted, Some(true));
        assert_eq!(m.video_on, Some(false));
        assert_eq!(m.sharing, Some(true));
        assert!(m.in_meeting);
    }

    #[test]
    fn controls_unmuted() {
        let mut u = MeetingState::default();
        derive_controls(
            &["Mute".into(), "Stop video".into(), "Start share".into()],
            &mut u,
        );
        assert_eq!(u.self_muted, Some(false));
        assert_eq!(u.video_on, Some(true));
        assert_eq!(u.sharing, Some(false));
    }

    #[test]
    fn no_controls_stays_nil() {
        let mut n = MeetingState::default();
        derive_controls(&["File".into(), "Edit".into(), "Help".into()], &mut n);
        assert_eq!(n.self_muted, None);
        assert!(!n.in_meeting);
    }

    #[test]
    fn name_filtering() {
        assert!(!valid_name("Set up professional audio in Audio settings"));
        assert!(!valid_name("Waiting for host to start the meeting"));
        assert!(valid_name("Rahul Khatri"));
        assert!(valid_name("Sharen"));
        assert_eq!(canon_name("Rahul Khatri (Me)"), "Rahul Khatri");
        assert_eq!(canon_name("Rahul Khatri (Host, me)"), "Rahul Khatri");
        assert_eq!(
            name_from_tile_desc("Rahul Khatri (Me), Computer audio muted").as_deref(),
            Some("Rahul Khatri")
        );
        assert!(name_from_tile_desc("Set up professional audio in Audio settings, Muted").is_none());
    }
}
