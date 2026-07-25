//! agent-action "brain" — REAL, unit-tested. All OS-agnostic logic (UTF-16 splice, the
//! MutationDisposition state machine, verify-settle-rollback, undo-payload construction,
//! fingerprints, the action-gate predicate, and `--selftest`) lives here on the [`EditTarget`]
//! trait. The concrete atspi target lives in `atspi_target.rs` (TODO on-device).
//!
//! Contract: `native/PROTOCOL.md` §4. Offsets are UTF-16 code units to match the macOS AX range
//! semantics the fingerprints were built against (the emoji self-test pins this); the atspi adapter
//! converts to/from AT-SPI character offsets at its boundary.

use helpers_core::fingerprint::{content_fingerprint, state_fingerprint};
use helpers_core::protocol::{ActionResponse, ExpectedTarget, UndoPayload};

pub const MAX_TEXT_LEN: usize = 64_000;
/// The one disposable editor agent-action may drive on Linux (PROTOCOL.md §4). macOS uses
/// `com.apple.TextEdit`; Windows `notepad.exe`; Linux GNOME Text Editor.
pub const ALLOWED_BUNDLE: &str = "org.gnome.TextEditor";
/// The dashboard app's own identity, excluded from the "window behind the dashboard" gate.
pub const DASHBOARD_BUNDLE: &str = "com.myassistant.desktop";

// ---------------------------------------------------------------------------
// UTF-16 splice (Swift `replacing`)
// ---------------------------------------------------------------------------

/// Splice `text` into `value` over the UTF-16 `range` = (location, length). `None` if the range is
/// out of bounds or the result would exceed [`MAX_TEXT_LEN`] UTF-16 units.
pub fn replacing(value: &str, range: (i64, i64), text: &str) -> Option<String> {
    let (loc, len) = range;
    if loc < 0 || len < 0 {
        return None;
    }
    let units: Vec<u16> = value.encode_utf16().collect();
    let n = units.len() as i64;
    if loc > n || loc + len > n {
        return None;
    }
    let (loc, len) = (loc as usize, len as usize);
    let mut out: Vec<u16> = Vec::with_capacity(units.len() + text.len());
    out.extend_from_slice(&units[..loc]);
    out.extend(text.encode_utf16());
    out.extend_from_slice(&units[loc + len..]);
    if out.len() > MAX_TEXT_LEN {
        return None;
    }
    Some(String::from_utf16_lossy(&out))
}

/// UTF-16 length of a string (the unit selections/ranges are measured in).
pub fn utf16_len(s: &str) -> i64 {
    s.encode_utf16().count() as i64
}

// ---------------------------------------------------------------------------
// MutationDisposition state machine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationDisposition {
    Verified,
    Unchanged,
    Unexpected(String),
    Unreadable,
}

/// The sole constructor (Swift `mutationDisposition`).
pub fn disposition(original: &str, expected: &str, actual: Option<&str>) -> MutationDisposition {
    match actual {
        None => MutationDisposition::Unreadable,
        Some(a) if a == expected => MutationDisposition::Verified,
        Some(a) if a == original => MutationDisposition::Unchanged,
        Some(a) => MutationDisposition::Unexpected(a.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Action-gate predicate (Swift `isActionGateWindow`)
// ---------------------------------------------------------------------------

/// Whether an on-screen window qualifies as "the ordinary app window behind the dashboard" — the
/// fail-closed gate confirming the captured editor is still front-behind before we mutate. Pure so
/// it is self-testable; the atspi adapter supplies the live values (TODO on-device on Wayland,
/// where window geometry/stacking is not queryable at all — see adapter).
#[allow(clippy::too_many_arguments)]
pub fn is_action_gate_window(
    owner_pid: i64,
    layer: i64,
    alpha: f64,
    width: f64,
    height: f64,
    bundle_id: &str,
    is_regular_app: bool,
) -> bool {
    owner_pid > 0
        && owner_pid != std::process::id() as i64
        && layer == 0
        && alpha > 0.0
        && width >= 80.0
        && height >= 60.0
        && bundle_id != DASHBOARD_BUNDLE
        && is_regular_app
}

// ---------------------------------------------------------------------------
// The mutable, verifiable target
// ---------------------------------------------------------------------------

/// A resolved editable AT-SPI control. `value`/`range`/`role`/`label` are the snapshot captured at
/// resolve time; the `set_*`/`read_value` methods hit the live control.
pub trait EditTarget {
    fn role(&self) -> &str;
    fn label(&self) -> Option<&str>;
    fn value(&self) -> &str;
    fn range(&self) -> Option<(i64, i64)>;

    /// Set only the selected text (replace selection / insert at caret). Returns whether the write
    /// call itself succeeded (not verification).
    fn set_selected_text(&mut self, text: &str) -> bool;
    /// Overwrite the entire control value. Returns whether the write call succeeded.
    fn set_whole_value(&mut self, text: &str) -> bool;
    /// Re-read the value after a ~30 ms settle (Swift `valueEquals`' reread).
    fn read_value_settled(&self) -> Option<String>;
    /// Set the selection range; returns success.
    fn set_selection(&mut self, range: (i64, i64)) -> bool;
    /// Whether the current selection equals `range`.
    fn selection_equals(&self, range: (i64, i64)) -> bool;
}

/// Apply `draft_text` / `replace_selection`. `expected` is echoed into the undo payload on success.
pub fn apply_mutation(
    action_is_replace_selection: bool,
    text: &str,
    target: &mut dyn EditTarget,
    expected: ExpectedTarget,
) -> ActionResponse {
    // Choose the range to splice over.
    let range = match target.range() {
        Some(r) => r,
        None if target.value().is_empty() => (0, 0),
        None => {
            return ActionResponse::failure(
                "unsupported_target",
                "The focused control did not expose a cursor or selection.",
            )
        }
    };
    if action_is_replace_selection && range.1 == 0 {
        return ActionResponse::failure(
            "stale_target",
            "There is no longer selected text to replace.",
        );
    }
    let original = target.value().to_string();
    let next = match replacing(&original, range, text) {
        Some(n) => n,
        None => {
            return ActionResponse::failure(
                "invalid_request",
                "The proposed edit exceeded the safe text boundary.",
            )
        }
    };
    let role = target.role().to_string();
    let label = target.label().map(|s| s.to_string());
    let original_selection = target.range();

    // Primary path: set the selected text, settle, verify.
    let set_ok = target.set_selected_text(text);
    let actual = target.read_value_settled();
    if set_ok && actual.as_deref() == Some(next.as_str()) {
        return applied(expected, original, original_selection, &role, label.as_deref(), &next);
    }
    match disposition(&original, &next, actual.as_deref()) {
        MutationDisposition::Verified => {
            applied(expected, original, original_selection, &role, label.as_deref(), &next)
        }
        MutationDisposition::Unreadable => ActionResponse::failure(
            "verification_failed",
            "The target value became unreadable after Apply. Review the field immediately.",
        ),
        MutationDisposition::Unexpected(actual_value) => rollback_unexpected(
            target,
            &original,
            &actual_value,
            original_selection,
            "apply_rolled_back",
            "The target transformed the draft, so the original text and selection were restored.",
        ),
        MutationDisposition::Unchanged => {
            // Fallback: overwrite the whole value.
            if target.set_whole_value(&next)
                && target.read_value_settled().as_deref() == Some(next.as_str())
            {
                return applied(expected, original, original_selection, &role, label.as_deref(), &next);
            }
            match disposition(&original, &next, target.read_value_settled().as_deref()) {
                MutationDisposition::Verified => {
                    applied(expected, original, original_selection, &role, label.as_deref(), &next)
                }
                MutationDisposition::Unchanged => ActionResponse::failure(
                    "apply_failed",
                    "The target app rejected the text change without changing the field.",
                ),
                MutationDisposition::Unreadable => ActionResponse::failure(
                    "verification_failed",
                    "The target value became unreadable after Apply. Review the field immediately.",
                ),
                MutationDisposition::Unexpected(actual_value) => rollback_unexpected(
                    target,
                    &original,
                    &actual_value,
                    original_selection,
                    "apply_rolled_back",
                    "The target transformed the draft, so the original text and selection were restored.",
                ),
            }
        }
    }
}

fn applied(
    expected: ExpectedTarget,
    original_value: String,
    original_selection: Option<(i64, i64)>,
    role: &str,
    label: Option<&str>,
    next: &str,
) -> ActionResponse {
    let undo = UndoPayload {
        expected,
        original_value,
        original_selection_location: original_selection.map(|(l, _)| l),
        original_selection_length: original_selection.map(|(_, len)| len),
        applied_content_fingerprint: content_fingerprint(role, label, next),
    };
    ActionResponse {
        ok: true,
        status: "applied".to_string(),
        verified: true,
        undo: Some(undo),
        error: None,
    }
}

/// Restore the safe prior state after an unexpected divergence.
fn rollback_unexpected(
    target: &mut dyn EditTarget,
    original_value: &str,
    _actual_value: &str,
    restore_selection: Option<(i64, i64)>,
    rolled_status: &str,
    rolled_msg: &str,
) -> ActionResponse {
    if replace_whole_text(target, original_value, restore_selection) {
        ActionResponse::failure(rolled_status, rolled_msg)
    } else {
        ActionResponse::failure(
            "verification_failed",
            "The target changed unexpectedly and exact rollback could not be verified. Review the field immediately.",
        )
    }
}

/// Overwrite the whole value (with a select-all + selected-text fallback), optionally restoring a
/// selection afterward. Returns whether the end state is verified.
fn replace_whole_text(
    target: &mut dyn EditTarget,
    value: &str,
    restore_selection: Option<(i64, i64)>,
) -> bool {
    let whole_ok = target.set_whole_value(value)
        && target.read_value_settled().as_deref() == Some(value);
    if !whole_ok {
        let previous_selection = target.range();
        let Some(current) = target.read_value_settled() else {
            return false;
        };
        let full_range = (0, utf16_len(&current));
        let ok = target.set_selection(full_range)
            && target.set_selected_text(value)
            && target.read_value_settled().as_deref() == Some(value);
        if !ok {
            if let Some(prev) = previous_selection {
                let _ = target.set_selection(prev);
            }
            return false;
        }
    }
    if let Some(restore) = restore_selection {
        if !(target.set_selection(restore) && target.selection_equals(restore)) {
            return false;
        }
    }
    true
}

/// Undo: restore `original_value` (+ selection), gated by the applied-content fingerprint.
pub fn perform_undo(
    undo: &UndoPayload,
    target: &mut dyn EditTarget,
) -> ActionResponse {
    // Applied-content guard: field must still hold exactly what we applied.
    let live_fp = content_fingerprint(target.role(), target.label(), target.value());
    if live_fp != undo.applied_content_fingerprint {
        return ActionResponse::failure(
            "stale_target",
            "The field changed after Apply, so automatic undo was cancelled.",
        );
    }
    let original_selection = match (undo.original_selection_location, undo.original_selection_length)
    {
        (Some(l), Some(len)) => Some((l, len)),
        _ => None,
    };
    let applied_selection = target.range();
    let original_value = undo.original_value.clone();
    // The value captured at undo-resolve time (== the applied text, since the fingerprint matched).
    let applied_value = target.value().to_string();

    if replace_whole_text(target, &original_value, original_selection) {
        return ActionResponse {
            ok: true,
            status: "undone".to_string(),
            verified: true,
            undo: None,
            error: None,
        };
    }

    let actual = target.read_value_settled();
    match disposition(&applied_value, &original_value, actual.as_deref()) {
        MutationDisposition::Verified => ActionResponse::failure(
            "verification_failed",
            "Undo restored the original text but could not verify the original selection. Review immediately.",
        ),
        MutationDisposition::Unchanged => ActionResponse::failure(
            "undo_failed",
            "The target app rejected Undo without changing the field.",
        ),
        MutationDisposition::Unreadable => ActionResponse::failure(
            "verification_failed",
            "The target value became unreadable during Undo. Review the field immediately.",
        ),
        MutationDisposition::Unexpected(actual_value) => rollback_unexpected(
            target,
            &applied_value,
            &actual_value,
            applied_selection,
            "undo_rolled_back",
            "Undo could not be verified, so the applied text was restored.",
        ),
    }
}

/// The 12 pure self-test assertions (Swift `selfTest`).
pub fn selftest() -> bool {
    let mut ok = true;
    ok &= replacing("A🙂B", (1, 2), "ok").as_deref() == Some("AokB");
    ok &= replacing("hello", (5, 0), "!").as_deref() == Some("hello!");
    ok &= replacing("hello", (6, 0), "!").is_none();
    ok &= content_fingerprint("AXTextArea", Some("Body"), "x")
        == content_fingerprint("AXTextArea", Some("Body"), "x");
    ok &= state_fingerprint("AXTextArea", None, "x", Some((0, 0)))
        != state_fingerprint("AXTextArea", None, "x", Some((1, 0)));
    ok &= disposition("before", "after", Some("after")) == MutationDisposition::Verified;
    ok &= disposition("before", "after", Some("before")) == MutationDisposition::Unchanged;
    ok &= disposition("before", "after", None) == MutationDisposition::Unreadable;
    ok &= disposition("before", "after", Some("changed"))
        == MutationDisposition::Unexpected("changed".to_string());
    ok &= is_action_gate_window(42, 0, 1.0, 800.0, 600.0, ALLOWED_BUNDLE, true);
    ok &= !is_action_gate_window(42, 0, 1.0, 800.0, 600.0, DASHBOARD_BUNDLE, true);
    ok &= !is_action_gate_window(42, 0, 1.0, 800.0, 600.0, "com.example.helper", false);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock control whose value/selection mutate in memory.
    struct Mock {
        value: String,
        selection: Option<(i64, i64)>,
        // If set, set_selected_text splices `text` over the current selection and updates value.
        honor_selected_text: bool,
    }

    impl EditTarget for Mock {
        fn role(&self) -> &str {
            "AXTextArea"
        }
        fn label(&self) -> Option<&str> {
            Some("Body")
        }
        fn value(&self) -> &str {
            &self.value
        }
        fn range(&self) -> Option<(i64, i64)> {
            self.selection
        }
        fn set_selected_text(&mut self, text: &str) -> bool {
            if !self.honor_selected_text {
                return false;
            }
            let range = self.selection.unwrap_or((utf16_len(&self.value), 0));
            if let Some(v) = replacing(&self.value, range, text) {
                self.value = v;
                self.selection = Some((range.0 + utf16_len(text), 0));
                true
            } else {
                false
            }
        }
        fn set_whole_value(&mut self, text: &str) -> bool {
            self.value = text.to_string();
            true
        }
        fn read_value_settled(&self) -> Option<String> {
            Some(self.value.clone())
        }
        fn set_selection(&mut self, range: (i64, i64)) -> bool {
            self.selection = Some(range);
            true
        }
        fn selection_equals(&self, range: (i64, i64)) -> bool {
            self.selection == Some(range)
        }
    }

    fn expected() -> ExpectedTarget {
        ExpectedTarget {
            bundle_id: ALLOWED_BUNDLE.into(),
            process_identifier: 42,
            window_title: None,
            focused_role: "AXTextArea".into(),
            focused_label: Some("Body".into()),
            focused_fingerprint: "fp".into(),
        }
    }

    #[test]
    fn selftest_passes() {
        assert!(selftest());
    }

    #[test]
    fn apply_draft_at_caret_succeeds_and_builds_undo() {
        let mut m = Mock { value: "Hi ".into(), selection: Some((3, 0)), honor_selected_text: true };
        let resp = apply_mutation(false, "there", &mut m, expected());
        assert!(resp.ok && resp.verified);
        assert_eq!(resp.status, "applied");
        assert_eq!(m.value, "Hi there");
        let undo = resp.undo.expect("undo built");
        assert_eq!(undo.original_value, "Hi ");
        assert_eq!(
            undo.applied_content_fingerprint,
            content_fingerprint("AXTextArea", Some("Body"), "Hi there")
        );
    }

    #[test]
    fn replace_selection_with_no_selection_is_stale() {
        let mut m = Mock { value: "Hi".into(), selection: Some((2, 0)), honor_selected_text: true };
        let resp = apply_mutation(true, "x", &mut m, expected());
        assert_eq!(resp.status, "stale_target");
        assert!(!resp.ok);
    }

    #[test]
    fn undo_restores_original_when_fingerprint_matches() {
        // Field currently holds "Hi there"; undo record wants it back to "Hi ".
        let mut m = Mock { value: "Hi there".into(), selection: Some((8, 0)), honor_selected_text: true };
        let undo = UndoPayload {
            expected: expected(),
            original_value: "Hi ".into(),
            original_selection_location: Some(3),
            original_selection_length: Some(0),
            applied_content_fingerprint: content_fingerprint("AXTextArea", Some("Body"), "Hi there"),
        };
        let resp = perform_undo(&undo, &mut m);
        assert_eq!(resp.status, "undone");
        assert_eq!(m.value, "Hi ");
    }

    #[test]
    fn undo_refused_when_field_changed() {
        let mut m = Mock { value: "edited".into(), selection: Some((6, 0)), honor_selected_text: true };
        let undo = UndoPayload {
            expected: expected(),
            original_value: "Hi ".into(),
            original_selection_location: None,
            original_selection_length: None,
            applied_content_fingerprint: content_fingerprint("AXTextArea", Some("Body"), "Hi there"),
        };
        let resp = perform_undo(&undo, &mut m);
        assert_eq!(resp.status, "stale_target");
    }
}
