//! Meeting detection helpers — REAL, unit-tested. `platform_from_evidence` (URL/window-title →
//! platform key) and `useful_meeting_title` (strip browser chrome, reject generic/control titles)
//! are pure and drive both call detection and the `--selftest`. On Linux the "evidence" text is a
//! window title / URL harvested from X11 (or AT-SPI on Wayland); the matching rules are identical to
//! the macOS Swift `platformFromEvidence` / `usefulMeetingTitle`.

/// Map lowercased window/URL evidence to a platform key, or `None`. Order matters (first match wins).
/// `_app_id` is the Linux app identity; only used for the generic `call`/`voice chat` fallback.
pub fn platform_from_evidence(text: &str, app_id: &str) -> Option<String> {
    let t = text.to_ascii_lowercase();
    let c = |needle: &str| t.contains(needle);

    if c("meet.google.com/") || c("google meet") {
        return Some("meet".into());
    }
    if c("zoom.us/wc/") || c("zoom.us/j/") || c("zoom meeting") {
        return Some("zoom".into());
    }
    if (c("teams.microsoft.com") && (c("meeting") || c("call"))) || c("microsoft teams meeting") {
        return Some("teams".into());
    }
    if c("webex.com/meet") || c("webex meeting") {
        return Some("webex".into());
    }
    if c("slack huddle") || c("in a huddle") || c("huddle window") {
        return Some("slack".into());
    }
    if c("discord") && (c("voice connected") || c("in call") || c("call with")) {
        return Some("discord".into());
    }
    if c("meet.jit.si/") || c("jitsi meet") {
        return Some("jitsi".into());
    }
    if c("whereby.com/") || c("whereby meeting") {
        return Some("whereby".into());
    }
    if c("chime.aws/") || c("amazon chime") {
        return Some("chime".into());
    }
    if c("app.gather.town/") || c("gather meeting") {
        return Some("gather".into());
    }
    if c("riverside.fm/studio") || c("riverside studio") {
        return Some("riverside".into());
    }
    if c("around.co/") || c("around meeting") {
        return Some("around".into());
    }
    if c("butter.us/") || c("butter session") {
        return Some("butter".into());
    }
    if c("call") || c("voice chat") {
        return evidence_key(app_id);
    }
    None
}

/// Map an evidence-app identity to its key (Slack/Discord/WhatsApp/Telegram/Signal), macOS bundle
/// ids and Linux ids both. `None` if unknown.
fn evidence_key(app_id: &str) -> Option<String> {
    let id = app_id.to_ascii_lowercase();
    let key = if id.contains("slack") {
        "slack"
    } else if id.contains("discord") {
        "discord"
    } else if id.contains("whatsapp") {
        "whatsapp"
    } else if id.contains("telegram") || id.contains("keepcoder") {
        "telegram"
    } else if id.contains("signal") {
        "signal"
    } else {
        return None;
    };
    Some(key.to_string())
}

const GENERIC: [&str; 7] = [
    "meeting", "zoom meeting", "microsoft teams", "webex", "facetime", "slack", "discord",
];

/// Pick a human-meaningful meeting title from candidate window titles (Swift `usefulMeetingTitle`).
/// Strips browser/Teams suffixes, rejects generic + control-panel titles, accepts the first title of
/// length 3..=120. Returns `""` if none qualify.
pub fn useful_meeting_title(titles: &[String], _key: &str) -> String {
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
        if (3..=120).contains(&count) && !GENERIC.contains(&low.as_str()) && !bad {
            return title;
        }
    }
    String::new()
}

/// Text cleaning for transcripts (Swift `cleanTranscript`): drop `[...]`, `*...*`, `(...)` spans,
/// collapse whitespace, trim; empty if no letter/digit remains.
pub fn clean_transcript(raw: &str) -> String {
    let mut s = strip_pair(raw, '[', ']');
    s = strip_pair(&s, '*', '*');
    s = strip_pair(&s, '(', ')');
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if !collapsed.chars().any(|c| c.is_alphanumeric()) {
        return String::new();
    }
    collapsed
}

/// `keepExpressions` variant (Swift): remove literal `[BLANK_AUDIO]`, convert `*..*`/`(..)` to `[..]`,
/// collapse whitespace, trim.
pub fn keep_expressions(raw: &str) -> String {
    let s = raw.replace("[BLANK_AUDIO]", " ");
    let s = rebracket(&s, '*', '*');
    let s = rebracket(&s, '(', ')');
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove a single-level `open..close` span (non-nested, like the Swift `\[[^\]]*\]` regex),
/// replacing the delimiters with spaces. Correctly toggles when `open == close` (e.g. `*..*`).
fn strip_pair(s: &str, open: char, close: char) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for ch in s.chars() {
        if !inside {
            if ch == open {
                inside = true;
                out.push(' ');
            } else {
                out.push(ch);
            }
        } else if ch == close {
            inside = false;
            out.push(' ');
        }
        // else: inside a span → drop the char
    }
    out
}

/// Convert a single-level `open..close` span to `[..]` (Swift `keepExpressions`). Toggles when
/// `open == close`, so `*laughs*` → `[laughs]` (not `[laughs[`).
fn rebracket(s: &str, open: char, close: char) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for ch in s.chars() {
        if !inside && ch == open {
            out.push('[');
            inside = true;
        } else if inside && ch == close {
            out.push(']');
            inside = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// The six pure self-test checks (Swift `--selftest`). Returns `(passed, failing_message)`.
pub fn selftest() -> (bool, &'static str) {
    if platform_from_evidence("https://meet.google.com/abc-defg-hij", "com.google.Chrome").as_deref()
        != Some("meet")
    {
        return (false, "Google Meet URL");
    }
    if platform_from_evidence("YouTube — Google Chrome", "com.google.Chrome").is_some() {
        return (false, "ordinary browser audio rejected");
    }
    if platform_from_evidence("Slack huddle window", "com.tinyspeck.slackmacgap").as_deref()
        != Some("slack")
    {
        return (false, "Slack huddle");
    }
    if platform_from_evidence("Discord voice connected", "com.hnc.Discord").as_deref()
        != Some("discord")
    {
        return (false, "Discord voice");
    }
    if !useful_meeting_title(&["Zoom Meeting".to_string()], "zoom").is_empty() {
        return (false, "generic title rejected");
    }
    if useful_meeting_title(&["Customer launch review - Google Chrome".to_string()], "meet")
        != "Customer launch review"
    {
        return (false, "browser suffix removed");
    }
    (true, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selftest_passes() {
        assert_eq!(selftest(), (true, ""));
    }

    #[test]
    fn evidence_matching() {
        assert_eq!(platform_from_evidence("zoom.us/j/123", "").as_deref(), Some("zoom"));
        assert_eq!(platform_from_evidence("Jitsi Meet", "").as_deref(), Some("jitsi"));
        assert_eq!(platform_from_evidence("random window", "").as_deref(), None);
        // generic 'call' fallback resolves the platform via the app identity.
        assert_eq!(
            platform_from_evidence("call with team", "org.telegram.desktop").as_deref(),
            Some("telegram")
        );
        assert_eq!(platform_from_evidence("call ongoing", "unknown.app").as_deref(), None);
    }

    #[test]
    fn transcript_cleaning() {
        assert_eq!(clean_transcript("hello [noise] world"), "hello world");
        assert_eq!(clean_transcript("[BLANK_AUDIO]"), "");
        assert_eq!(keep_expressions("wow *laughs* nice"), "wow [laughs] nice");
    }
}
