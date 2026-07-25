//! Meeting-app tables + evidence classification. **Fully portable and real** (pure string logic);
//! this is the Windows analogue of the bundle-ID tables in `meeting-tap.swift`, keyed on Windows
//! process **exe names** instead of macOS bundle IDs (per PROTOCOL: "Bundle-ID app table →
//! exe-name/AUMID (Windows)"). It also ports `platformFromEvidence` / `usefulMeetingTitle`
//! verbatim, which give `meeting-tap --selftest` real coverage.

/// `(key, exe-names)`. Dedicated conferencing apps: an Active audio session alone is a strong call
/// signal (no window-evidence gate needed).
pub const DEDICATED: &[(&str, &[&str])] = &[
    ("zoom", &["zoom.exe", "cpthost.exe"]),
    ("teams", &["ms-teams.exe", "teams.exe"]),
    ("webex", &["webex.exe", "ciscocollabhost.exe", "atmgr.exe"]),
    ("goto", &["gotomeeting.exe", "goto.exe", "g2mstart.exe"]),
    ("ringcentral", &["ringcentral.exe", "glip.exe"]),
    ("skype", &["skype.exe"]),
    ("bluejeans", &["bluejeans.exe"]),
];

/// Multipurpose chat apps: need BOTH an Active session AND meeting-shaped window evidence.
pub const EVIDENCE: &[(&str, &[&str])] = &[
    ("slack", &["slack.exe"]),
    ("discord", &["discord.exe"]),
    ("whatsapp", &["whatsapp.exe"]),
    ("telegram", &["telegram.exe"]),
    ("signal", &["signal.exe"]),
];

/// Browsers: need window/tab evidence of a real meeting URL.
pub const BROWSERS: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "arc.exe",
    "chromium.exe",
];

fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Look up a dedicated-app key by exe name (case-insensitive).
pub fn dedicated_key(exe: &str) -> Option<&'static str> {
    DEDICATED
        .iter()
        .find(|(_, exes)| exes.iter().any(|e| eq_ci(e, exe)))
        .map(|(k, _)| *k)
}

/// Look up an evidence-app key by exe name (case-insensitive).
pub fn evidence_key(exe: &str) -> Option<&'static str> {
    EVIDENCE
        .iter()
        .find(|(_, exes)| exes.iter().any(|e| eq_ci(e, exe)))
        .map(|(k, _)| *k)
}

pub fn is_browser(exe: &str) -> bool {
    BROWSERS.iter().any(|e| eq_ci(e, exe))
}

/// Any exe we consider a possible call source.
pub fn is_known_call_exe(exe: &str) -> bool {
    dedicated_key(exe).is_some() || evidence_key(exe).is_some() || is_browser(exe)
}

/// Classify a meeting platform from window/tab text evidence for a chat app or browser. A verbatim
/// port of the Swift `platformFromEvidence` — background browser audio (e.g. a YouTube tab) yields
/// `None`, so it can never start a recording.
pub fn platform_from_evidence(text: &str, exe: &str) -> Option<String> {
    let t = text.to_lowercase();
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
        return evidence_key(exe).map(|k| k.to_string());
    }
    None
}

/// Pick a human-meaningful meeting title from window titles, stripping browser/app chrome. A verbatim
/// port of the Swift `usefulMeetingTitle`.
pub fn useful_meeting_title(titles: &[String], _key: &str) -> String {
    const GENERIC: &[&str] = &[
        "meeting",
        "zoom meeting",
        "microsoft teams",
        "webex",
        "facetime",
        "slack",
        "discord",
    ];
    const SUFFIXES: &[&str] = &[
        " - Google Chrome",
        " — Mozilla Firefox",
        " - Microsoft Edge",
        " — Microsoft\u{200b} Edge",
        " - Brave",
        " | Microsoft Teams",
    ];
    for raw in titles {
        let mut title = raw.trim().to_string();
        for suffix in SUFFIXES {
            if let Some(stripped) = title.strip_suffix(suffix) {
                title = stripped.to_string();
            }
        }
        let lower = title.to_lowercase();
        let len = title.chars().count();
        if len >= 3
            && len <= 120
            && !GENERIC.contains(&lower.as_str())
            && !lower.contains("floating video window")
            && !lower.contains("meeting controls")
            && !lower.contains("audio settings")
            && !lower.contains("http")
        {
            return title;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_tables_case_insensitive() {
        assert_eq!(dedicated_key("Zoom.exe"), Some("zoom"));
        assert_eq!(dedicated_key("ms-teams.exe"), Some("teams"));
        assert_eq!(evidence_key("Slack.exe"), Some("slack"));
        assert!(is_browser("CHROME.EXE"));
        assert!(dedicated_key("notepad.exe").is_none());
    }

    #[test]
    fn evidence_classification_matches_swift() {
        assert_eq!(
            platform_from_evidence("https://meet.google.com/abc-defg-hij", "chrome.exe").as_deref(),
            Some("meet")
        );
        assert_eq!(platform_from_evidence("YouTube — Google Chrome", "chrome.exe"), None);
        assert_eq!(
            platform_from_evidence("Slack huddle window", "slack.exe").as_deref(),
            Some("slack")
        );
        assert_eq!(
            platform_from_evidence("Discord voice connected", "discord.exe").as_deref(),
            Some("discord")
        );
    }

    #[test]
    fn title_selection_matches_swift() {
        assert!(useful_meeting_title(&["Zoom Meeting".into()], "zoom").is_empty());
        assert_eq!(
            useful_meeting_title(&["Customer launch review - Google Chrome".into()], "meet"),
            "Customer launch review"
        );
    }
}
