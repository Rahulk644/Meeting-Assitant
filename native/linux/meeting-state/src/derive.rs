//! meeting-state pure logic — REAL, unit-tested. Control derivation from menu labels, participant
//! name extraction/filtering, active-speaker parsing, participant-source classification, useful
//! window-title selection, and per-app in-meeting title heuristics. All OS-agnostic and verifiable
//! without a Linux desktop; the AT-SPI reads that FEED these (menu titles, tile descriptions) are
//! the TODO(on-device) part.
//!
//! Contract: `native/PROTOCOL.md` §5 + the Swift `deriveControls` / name helpers. Matching is
//! substring, case-insensitive, English-only — exactly as macOS (selectors need on-device `--dump`
//! tuning, especially Zoom-on-Linux which is Qt and requires `QT_ACCESSIBILITY=1`).

/// Result of deriving mute/video/share state from menu-item labels.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Controls {
    pub self_muted: Option<bool>,
    pub video_on: Option<bool>,
    pub sharing: Option<bool>,
    pub in_meeting: bool,
}

/// True if some single title contains ALL `needles` (lowercased substring).
fn has(titles_lower: &[String], needles: &[&str]) -> bool {
    titles_lower
        .iter()
        .any(|t| needles.iter().all(|n| t.contains(n)))
}

/// Derive controls from menu-item titles. Order matters: "unmute" is checked before "mute" because
/// "unmute" contains "mute". Identical logic for every app (no per-app label table).
pub fn derive_controls(titles: &[String]) -> Controls {
    let low: Vec<String> = titles.iter().map(|t| t.to_ascii_lowercase()).collect();
    let mut c = Controls::default();

    if has(&low, &["unmute", "audio"]) {
        c.self_muted = Some(true);
    } else if has(&low, &["mute", "audio"]) {
        c.self_muted = Some(false);
    }

    if has(&low, &["stop", "video"]) {
        c.video_on = Some(true);
    } else if has(&low, &["start", "video"]) {
        c.video_on = Some(false);
    }

    if has(&low, &["stop", "share"]) {
        c.sharing = Some(true);
    } else if has(&low, &["start", "share"]) {
        c.sharing = Some(false);
    }

    c.in_meeting = c.self_muted.is_some() || c.sharing.is_some();
    c
}

/// UI words that disqualify a candidate participant name (whole-word, lowercased).
const UI_WORDS: [&str; 24] = [
    "zoom", "audio", "video", "mute", "unmute", "muted", "share", "sharing", "settings", "setting",
    "microphone", "mic", "speaker", "camera", "waiting", "participants", "participant", "reactions",
    "reaction", "profile", "record", "recording", "screen", "gallery",
];

/// Substring before the first `(`, trimmed (Swift `canonName`). `"Rahul Khatri (Me)"` → `"Rahul Khatri"`.
pub fn canon_name(s: &str) -> String {
    let base = match s.find('(') {
        Some(i) => &s[..i],
        None => s,
    };
    base.trim().to_string()
}

/// Whether `s` is a plausible human name (Swift `validName`).
pub fn valid_name(s: &str) -> bool {
    let t = s.trim();
    let count = t.chars().count();
    if !(2..=40).contains(&count) {
        return false;
    }
    if !t.chars().any(|c| c.is_alphabetic()) {
        return false;
    }
    if t.split_whitespace().count() > 5 {
        return false;
    }
    // Tokenize by non-letters, lowercase; reject if any whole token is a UI word.
    for token in t.split(|c: char| !c.is_alphabetic()) {
        if token.is_empty() {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        if UI_WORDS.contains(&lower.as_str()) {
            return false;
        }
    }
    true
}

/// Extract a name from a tile description: needs a comma AND an audio/video/muted hint (Swift
/// `nameFromTileDesc`). `"Rahul Khatri, Computer audio muted"` → `Some("Rahul Khatri")`.
pub fn name_from_tile_desc(d: &str) -> Option<String> {
    let low = d.to_ascii_lowercase();
    if !d.contains(',') || !(low.contains("audio") || low.contains("video") || low.contains("muted")) {
        return None;
    }
    let before = d.split(',').next().unwrap_or("");
    let name = canon_name(before);
    if valid_name(&name) {
        Some(name)
    } else {
        None
    }
}

/// Extract a name from a "View X's profile" description (Swift `nameFromProfileDesc`).
pub fn name_from_profile_desc(d: &str) -> Option<String> {
    let rest = d.strip_prefix("View ")?;
    let mut trimmed = rest;
    for suffix in ["'s profile", "\u{2019}s profile", " profile"] {
        if let Some(stripped) = rest.strip_suffix(suffix) {
            trimmed = stripped;
            break;
        }
    }
    let name = canon_name(trimmed);
    if valid_name(&name) {
        Some(name)
    } else {
        None
    }
}

/// Extract the active speaker from a "Talking: X" value (Swift `nameFromTalking`). Returns the
/// capitalized name; this signal is authoritative and overrides the tile heuristic.
pub fn name_from_talking(v: &str) -> Option<String> {
    let low = v.to_ascii_lowercase();
    let idx = low.strip_prefix("talking:").map(|_| "talking:".len())?;
    let after = v[idx..].trim();
    let name = canon_name(after);
    if valid_name(&name) {
        Some(capitalized(&name))
    } else {
        None
    }
}

/// Capitalize the first letter of each whitespace-separated word (Swift `.capitalized`).
fn capitalized(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>()
                        + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Classify where a participant name came from (Swift `participantSource`).
pub fn participant_source(context: &str, floating: bool, profile: bool) -> &'static str {
    if context.to_ascii_lowercase().contains("participant") {
        "participant_list"
    } else if floating {
        "active_speaker_tile"
    } else if profile {
        "profile_button"
    } else {
        "video_tile"
    }
}

const GENERIC_TITLES: [&str; 7] = [
    "meeting", "zoom meeting", "microsoft teams", "webex", "facetime", "slack", "discord",
];

/// Pick a useful window title (Swift `usefulWindowTitle`): strips browser suffixes, rejects generic
/// / control-panel titles, accepts the first title of length 3..=120. `""` if none qualify.
pub fn useful_window_title(titles: &[String]) -> String {
    const SUFFIXES: [&str; 5] = [
        " - Google Chrome",
        " \u{2014} Mozilla Firefox",
        " - Microsoft Edge",
        " \u{2014} Safari",
        " | Microsoft Teams",
    ];
    for raw in titles {
        let mut title = raw.trim().to_string();
        for suffix in SUFFIXES {
            if let Some(stripped) = title.strip_suffix(suffix) {
                title = stripped.to_string();
                break;
            }
        }
        let count = title.chars().count();
        let low = title.to_ascii_lowercase();
        let bad = low.contains("floating video window")
            || low.contains("meeting controls")
            || low.contains("audio settings")
            || low.contains("http");
        if (3..=120).contains(&count) && !GENERIC_TITLES.contains(&low.as_str()) && !bad {
            return title;
        }
    }
    String::new()
}

/// Per-app window-title in-meeting heuristic (Swift `windowInMeeting`).
pub fn window_in_meeting(titles: &[String], app_key: &str) -> bool {
    for t in titles {
        let low = t.to_ascii_lowercase();
        match app_key {
            "zoom" if low.contains("zoom meeting") => return true,
            "teams" if low.contains("meeting") || low.contains("| microsoft teams") => return true,
            _ => {}
        }
    }
    false
}

/// The pure self-test assertions (mirrors the Swift `selftest`, minus the AX/codable smoke).
pub fn selftest() -> bool {
    let mut ok = true;

    // Control derivation.
    let c1 = derive_controls(&s(&["Unmute Audio", "Start Video", "Stop Share"]));
    ok &= c1.self_muted == Some(true) && c1.video_on == Some(false) && c1.sharing == Some(true) && c1.in_meeting;
    let c2 = derive_controls(&s(&["Mute Audio", "Stop Video", "Start Share"]));
    ok &= c2.self_muted == Some(false) && c2.video_on == Some(true) && c2.sharing == Some(false);
    let c3 = derive_controls(&s(&["File", "Edit", "Help"]));
    ok &= c3.self_muted.is_none() && !c3.in_meeting;

    // Name filtering.
    ok &= !valid_name("Set up professional audio in Audio settings");
    ok &= !valid_name("Waiting for host to start the meeting");
    ok &= valid_name("Rahul Khatri");
    ok &= valid_name("Sharen"); // whole-word: "sharen" != "share"
    ok &= canon_name("Rahul Khatri (Me)") == "Rahul Khatri";
    ok &= canon_name("Rahul Khatri (Host, me)") == "Rahul Khatri";
    ok &= name_from_tile_desc("Rahul Khatri (Me), Computer audio muted").as_deref() == Some("Rahul Khatri");
    ok &= name_from_tile_desc("Set up professional audio in Audio settings, Muted").is_none();
    ok &= participant_source("Participants", false, true) == "participant_list";
    ok &= participant_source("zoom floating video window", true, false) == "active_speaker_tile";

    ok
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selftest_passes() {
        assert!(selftest());
    }

    #[test]
    fn derive_unmute_means_muted() {
        let c = derive_controls(&s(&["Unmute Audio"]));
        assert_eq!(c.self_muted, Some(true));
        assert!(c.in_meeting);
    }

    #[test]
    fn talking_value_parsed_and_capitalized() {
        assert_eq!(name_from_talking("Talking: rahul khatri").as_deref(), Some("Rahul Khatri"));
        assert!(name_from_talking("Talking: mute").is_none()); // UI word rejected
    }

    #[test]
    fn profile_desc_curly_apostrophe() {
        assert_eq!(name_from_profile_desc("View Rahul\u{2019}s profile").as_deref(), Some("Rahul"));
        assert_eq!(name_from_profile_desc("View Rahul Khatri's profile").as_deref(), Some("Rahul Khatri"));
    }

    #[test]
    fn useful_title_strips_browser_suffix() {
        let titles = s(&["Customer launch review - Google Chrome"]);
        assert_eq!(useful_window_title(&titles), "Customer launch review");
        // "zoom meeting" is in the generic-reject list, so it yields "".
        assert_eq!(useful_window_title(&s(&["Zoom Meeting"])), "");
    }

    #[test]
    fn generic_titles_rejected() {
        assert_eq!(useful_window_title(&s(&["Meeting"])), "");
        assert_eq!(useful_window_title(&s(&["Microsoft Teams"])), "");
    }

    #[test]
    fn window_in_meeting_zoom() {
        assert!(window_in_meeting(&s(&["Zoom Meeting"]), "zoom"));
        assert!(!window_in_meeting(&s(&["Zoom - Licensed Account"]), "zoom"));
    }
}
