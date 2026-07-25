//! Focused-field classification — REAL, unit-tested. Pure given the captured focus attributes, so
//! the kind decision (secure / codeOrTerminal / naturalLanguage / unknown), the `startsSentence`
//! rule, and `liveEditable` are all verifiable without a Linux desktop. The atspi attribute capture
//! is the TODO(on-device) part (in `atspi_focus.rs`).
//!
//! Contract: `native/PROTOCOL.md` §2 + the Swift `classifyTarget`. `liveEditable` is set
//! conservatively false on Wayland (no stable caret guarantee), which also disables `--session`.

use helpers_core::protocol::{DictationClassification, FieldKind};
use platform::display::DisplayServer;
use platform::text::is_secure;

/// Attributes captured from the focused element (the OS-specific adapter fills this in).
#[derive(Debug, Clone, Default)]
pub struct FocusInput {
    /// App identity: Linux desktop/app id or WM_CLASS.
    pub app_id: String,
    pub app_name: String,
    /// Already-mapped `AX*` role token.
    pub role_ax: String,
    pub subrole: Option<String>,
    /// Composed label (title/description/placeholder).
    pub label: Option<String>,
    /// (location, length) caret/selection in AT-SPI character offsets.
    pub selection: Option<(i64, i64)>,
    /// Up to 8 characters immediately before the caret (for `startsSentence`).
    pub preceding: Option<String>,
    /// Whether selected-text + selection-range are settable (needed for live editing).
    pub settable: bool,
    /// Whether a text-for-caret-range query succeeded (stable-caret evidence).
    pub caret_query_ok: bool,
}

/// The classification context the adapter produces. `trusted` is false when AT-SPI is unavailable.
pub struct ClassifyContext {
    pub trusted: bool,
    pub focus: Option<FocusInput>,
}

/// Linux code/terminal app identities (desktop ids / WM_CLASS) and app-name substrings.
fn is_code_app(app_id: &str, app_name: &str) -> bool {
    const IDS: [&str; 22] = [
        "org.gnome.Terminal",
        "org.gnome.Console",
        "org.kde.konsole",
        "konsole",
        "gnome-terminal",
        "xterm",
        "alacritty",
        "org.alacritty.Alacritty",
        "kitty",
        "io.elementary.terminal",
        "com.raggesilver.BlackBox",
        "code",
        "code-oss",
        "code-insiders",
        "com.visualstudio.code",
        "codium",
        "cursor",
        "dev.zed.Zed",
        "zed",
        "sublime_text",
        "com.sublimetext.4",
        "jetbrains",
    ];
    for id in IDS {
        if app_id == id || app_id.starts_with(&format!("{id}.")) || app_id.starts_with("jetbrains-") {
            return true;
        }
    }
    let name = app_name.to_ascii_lowercase();
    const NAME_HINTS: [&str; 8] = [
        "terminal",
        "konsole",
        "alacritty",
        "kitty",
        "visual studio code",
        "cursor",
        "zed",
        "sublime text",
    ];
    NAME_HINTS.iter().any(|h| name.contains(h))
}

/// `startsSentence` (Swift rule): true at field start, after a newline, after all-whitespace, or
/// when the last non-space/tab character before the caret is sentence-ending punctuation.
pub fn starts_sentence(selection: Option<(i64, i64)>, preceding: Option<&str>) -> bool {
    let Some((location, _)) = selection else {
        return false;
    };
    if location == 0 {
        return true;
    }
    let Some(before) = preceding else {
        return false;
    };
    if before.ends_with('\n') {
        return true;
    }
    // Swift trims `.whitespaces` = spaces + tabs (NOT newlines), then takes the last char.
    let trimmed = before.trim_matches(|c| c == ' ' || c == '\t');
    match trimmed.chars().last() {
        None => true,
        Some(last) => ".!?।".contains(last),
    }
}

/// Classify the focused field into the four-field `Result`.
pub fn classify(ctx: &ClassifyContext, server: DisplayServer) -> DictationClassification {
    // Fail-closed if AT-SPI is unavailable (Swift trust gate → secure).
    if !ctx.trusted {
        return secure_result();
    }
    let Some(f) = ctx.focus.as_ref() else {
        return unknown_result();
    };

    if is_secure(&f.role_ax, f.subrole.as_deref(), f.label.as_deref()) {
        return secure_result();
    }
    if is_code_app(&f.app_id, &f.app_name) {
        return DictationClassification {
            kind: FieldKind::CodeOrTerminal,
            secure: false,
            starts_sentence: false,
            live_editable: false,
        };
    }
    if !platform::roles::ax_role_is_editable(&f.role_ax) {
        return unknown_result();
    }

    // naturalLanguage. liveEditable requires a guaranteed-stable caret: X11 only (Wayland → false),
    // a collapsed caret (selection length 0), settable text, and a working caret-range query.
    let collapsed_caret = matches!(f.selection, Some((_, 0)));
    let live_editable = server.can_inject_global_input()
        && collapsed_caret
        && f.settable
        && f.caret_query_ok;

    DictationClassification {
        kind: FieldKind::NaturalLanguage,
        secure: false,
        starts_sentence: starts_sentence(f.selection, f.preceding.as_deref()),
        live_editable,
    }
}

fn secure_result() -> DictationClassification {
    DictationClassification {
        kind: FieldKind::Secure,
        secure: true,
        starts_sentence: false,
        live_editable: false,
    }
}

fn unknown_result() -> DictationClassification {
    DictationClassification {
        kind: FieldKind::Unknown,
        secure: false,
        starts_sentence: false,
        live_editable: false,
    }
}

/// The three pure self-test assertions (Swift `--selftest`).
pub fn selftest() -> bool {
    is_code_app("org.gnome.Terminal", "Terminal")
        && is_secure("AXSecureTextField", None, None)
        && !is_secure("AXTextArea", None, Some("Message"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selftest_passes() {
        assert!(selftest());
    }

    #[test]
    fn code_apps_detected() {
        assert!(is_code_app("code", "Visual Studio Code"));
        assert!(is_code_app("org.kde.konsole", "Konsole"));
        assert!(is_code_app("jetbrains-idea", "IntelliJ IDEA"));
        assert!(is_code_app("dev.zed.Zed", "Zed"));
        assert!(!is_code_app("org.gnome.TextEditor", "Text Editor"));
    }

    #[test]
    fn starts_sentence_rules() {
        assert!(starts_sentence(Some((0, 0)), None)); // field start
        assert!(!starts_sentence(None, None)); // no caret
        assert!(starts_sentence(Some((5, 0)), Some("Hello.\n"))); // after newline
        assert!(starts_sentence(Some((5, 0)), Some("End. "))); // after '.' + space
        assert!(!starts_sentence(Some((5, 0)), Some("word "))); // mid-sentence
        assert!(starts_sentence(Some((5, 0)), Some("   "))); // all whitespace
        assert!(starts_sentence(Some((5, 0)), Some("वाक्य।"))); // Devanagari danda
    }

    #[test]
    fn classify_secure_untrusted() {
        let ctx = ClassifyContext { trusted: false, focus: None };
        let r = classify(&ctx, DisplayServer::X11);
        assert_eq!(r.kind, FieldKind::Secure);
        assert!(r.secure);
    }

    #[test]
    fn classify_natural_language_live_on_x11_not_wayland() {
        let focus = FocusInput {
            role_ax: "AXTextArea".into(),
            selection: Some((0, 0)),
            settable: true,
            caret_query_ok: true,
            ..Default::default()
        };
        let ctx = ClassifyContext { trusted: true, focus: Some(focus.clone()) };
        let x = classify(&ctx, DisplayServer::X11);
        assert_eq!(x.kind, FieldKind::NaturalLanguage);
        assert!(x.live_editable);
        assert!(x.starts_sentence); // caret at 0

        let ctx2 = ClassifyContext { trusted: true, focus: Some(focus) };
        let w = classify(&ctx2, DisplayServer::Wayland);
        assert!(!w.live_editable, "Wayland must not promise live editing");
    }

    #[test]
    fn classify_unknown_for_noneditable_role() {
        let focus = FocusInput { role_ax: "AXButton".into(), ..Default::default() };
        let ctx = ClassifyContext { trusted: true, focus: Some(focus) };
        assert_eq!(classify(&ctx, DisplayServer::X11).kind, FieldKind::Unknown);
    }
}
