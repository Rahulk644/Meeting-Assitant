//! AT-SPI role → macOS `AX*` role-token mapping — REAL, pure, unit-tested.
//!
//! `native/PROTOCOL.md` §3 is explicit: the emitted `role` strings must stay the `AX*` tokens the
//! core already understands, on every platform. macOS emits raw AX role strings; on Linux the
//! accessibility tree speaks AT-SPI role names (`"push button"`, `"entry"`, `"password text"`, …),
//! so we translate them here into the same `AX*` vocabulary agent-context / agent-action /
//! dictation-target were reverse-engineered against.
//!
//! Input is the canonical AT-SPI role name (lower-case, space-separated — what `atspi::Role`'s
//! display / `.name()` yields). Unknown roles fall back to `"AXUnknown"`, exactly like the Swift
//! `axString(kAXRoleAttribute) ?? "AXUnknown"`.

/// The four AX role tokens the helpers treat as editable text controls (verbatim from the Swift
/// `editableRoles` in dictation-target / agent-context / agent-action).
pub const AX_EDITABLE_ROLES: [&str; 4] = ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"];

/// True if `ax_role` is one of the editable AX tokens. Editability still also requires the AT-SPI
/// `State::Editable`/settable-value check at the call site — this only gates on the role token.
pub fn ax_role_is_editable(ax_role: &str) -> bool {
    AX_EDITABLE_ROLES.contains(&ax_role)
}

/// Map an AT-SPI role name to the AX role token the core expects. `password text` maps to
/// `AXTextField`; secure-ness is signalled separately (a caller sets the `AXSecureTextField`
/// *subrole* — see [`atspi_secure_subrole`]) so the `isSecure` structural check still fires.
pub fn atspi_role_to_ax(atspi_role: &str) -> &'static str {
    match atspi_role.trim().to_ascii_lowercase().as_str() {
        // --- editable text ---
        "entry" | "text" | "password text" => "AXTextField",
        // Multi-line document/rich-text bodies behave like AXTextArea.
        "document text" | "document frame" | "document web" | "paragraph" => "AXTextArea",
        "terminal" => "AXTextArea",
        "combo box" => "AXComboBox",
        "autocomplete" => "AXComboBox",
        // --- buttons / actionable ---
        "push button" | "button" => "AXButton",
        "toggle button" => "AXButton",
        "menu button" => "AXMenuButton",
        "link" => "AXLink",
        // --- selection controls ---
        "check box" | "check menu item" => "AXCheckBox",
        "radio button" | "radio menu item" => "AXRadioButton",
        "slider" => "AXSlider",
        "spin button" => "AXIncrementor",
        "scroll bar" => "AXScrollBar",
        "progress bar" => "AXProgressIndicator",
        // --- static / text display ---
        "label" | "static" | "caption" | "text node" => "AXStaticText",
        "heading" => "AXHeading",
        "image" | "icon" => "AXImage",
        // --- structure / containers ---
        "frame" | "window" | "dialog" | "alert" => "AXWindow",
        "panel" | "filler" | "section" | "grouping" => "AXGroup",
        "scroll pane" => "AXScrollArea",
        "list" => "AXList",
        "list item" => "AXStaticText",
        "table" | "tree table" | "tree" => "AXTable",
        "table row" | "tree item" => "AXRow",
        "table cell" | "column header" | "row header" => "AXCell",
        "menu bar" => "AXMenuBar",
        "menu" | "popup menu" => "AXMenu",
        "menu item" => "AXMenuItem",
        "tool bar" => "AXToolbar",
        "page tab list" => "AXTabGroup",
        "page tab" => "AXRadioButton",
        "separator" => "AXSplitter",
        "status bar" => "AXStaticText",
        // Fallback — the Swift `?? "AXUnknown"`.
        _ => "AXUnknown",
    }
}

/// The AX subrole a secure/password AT-SPI control should advertise so the shared `isSecure`
/// structural check ("`securetextfield`" substring) fires identically to macOS. Returns
/// `Some("AXSecureTextField")` for the `password text` role, else `None`.
pub fn atspi_secure_subrole(atspi_role: &str) -> Option<&'static str> {
    if atspi_role.trim().eq_ignore_ascii_case("password text") {
        Some("AXSecureTextField")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editable_roles_map_correctly() {
        assert_eq!(atspi_role_to_ax("entry"), "AXTextField");
        assert_eq!(atspi_role_to_ax("Entry"), "AXTextField");
        assert_eq!(atspi_role_to_ax("password text"), "AXTextField");
        assert_eq!(atspi_role_to_ax("terminal"), "AXTextArea");
        assert_eq!(atspi_role_to_ax("combo box"), "AXComboBox");
        assert!(ax_role_is_editable("AXTextField"));
        assert!(ax_role_is_editable("AXComboBox"));
        assert!(!ax_role_is_editable("AXButton"));
    }

    #[test]
    fn unknown_falls_back() {
        assert_eq!(atspi_role_to_ax("some-future-role"), "AXUnknown");
    }

    #[test]
    fn secure_subrole_only_for_password() {
        assert_eq!(atspi_secure_subrole("password text"), Some("AXSecureTextField"));
        assert_eq!(atspi_secure_subrole("entry"), None);
    }

    #[test]
    fn window_like_roles() {
        assert_eq!(atspi_role_to_ax("frame"), "AXWindow");
        assert_eq!(atspi_role_to_ax("dialog"), "AXWindow");
        assert_eq!(atspi_role_to_ax("push button"), "AXButton");
    }
}
