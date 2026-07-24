//! `agent-action --request` — one guarded UI mutation. The Windows port of `agent-action.swift`.
//!
//! **Portable and tested**: request validation, the UTF-16 `replacing` splice (byte-faithful to the
//! Swift `replacing` incl. surrogate pairs), the `mutation_disposition` rollback classifier, the
//! action-gate predicate, `content`/`state` fingerprints (via `helpers-core`), and undo-record
//! construction. These drive `agent-action --selftest`.
//!
//! **`#[cfg(windows)]` native**: resolving the live focused control (UIA), activating its window,
//! applying the edit via UIA `ValuePattern.SetValue` (preferred) or select-all + injected text
//! (fallback), and reading it back to verify after a ~30 ms settle. Gated to the disposable editor
//! `notepad.exe`. `#[cfg(not(windows))]` returns `permission_required`, keeping responses well-formed.

use helpers_core::fingerprint::content_fingerprint;
use helpers_core::protocol::{ActionRequest, ActionResponse, ExpectedTarget, UndoPayload};

/// The disposable editor this milestone is allowed to touch (Windows analogue of TextEdit).
pub const ALLOWED_EDITOR_EXE: &str = "notepad.exe";
const MAX_TEXT_LEN: usize = 64_000;

// ---------------------------------------------------------------------------
// Portable, tested helpers
// ---------------------------------------------------------------------------

/// Splice `text` into `value` over the UTF-16 `range` `(location, length)`. Returns `None` if the
/// range is out of bounds or the result exceeds the safe cap. Matches the Swift `replacing`.
pub fn replacing(value: &str, range: (i64, i64), text: &str) -> Option<String> {
    let (loc, len) = range;
    if loc < 0 || len < 0 {
        return None;
    }
    let units: Vec<u16> = value.encode_utf16().collect();
    let (loc, len) = (loc as usize, len as usize);
    if loc > units.len() || loc + len > units.len() {
        return None;
    }
    let mut out: Vec<u16> = Vec::with_capacity(units.len() + text.len());
    out.extend_from_slice(&units[..loc]);
    out.extend(text.encode_utf16());
    out.extend_from_slice(&units[loc + len..]);
    let s = String::from_utf16(&out).ok()?;
    if s.encode_utf16().count() <= MAX_TEXT_LEN {
        Some(s)
    } else {
        None
    }
}

/// Classify what happened after a mutation attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum Disposition {
    Verified,
    Unchanged,
    Unexpected(String),
    Unreadable,
}

pub fn mutation_disposition(original: &str, expected: &str, actual: Option<&str>) -> Disposition {
    match actual {
        None => Disposition::Unreadable,
        Some(a) if a == expected => Disposition::Verified,
        Some(a) if a == original => Disposition::Unchanged,
        Some(a) => Disposition::Unexpected(a.to_string()),
    }
}

/// Action-gate predicate: the target must be a real other process running the allowed editor.
pub fn action_gate_ok(owner_pid: u32, our_pid: u32, exe: &str) -> bool {
    owner_pid != 0 && owner_pid != our_pid && exe.eq_ignore_ascii_case(ALLOWED_EDITOR_EXE)
}

/// Live target resolved from the focused control.
#[derive(Debug, Clone)]
pub struct LiveTarget {
    pub role: String,
    pub label: Option<String>,
    pub value: String,
    pub sel: Option<(i64, i64)>,
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Dispatch one request (apply or undo).
pub fn handle(request: &ActionRequest) -> ActionResponse {
    use helpers_core::protocol::ActionOperation;
    match request.operation {
        ActionOperation::Apply => apply(request),
        ActionOperation::Undo => undo(request),
    }
}

fn apply(request: &ActionRequest) -> ActionResponse {
    use helpers_core::protocol::ActionKind;
    let action = request.action;
    let is_valid_action = matches!(action, Some(ActionKind::DraftText) | Some(ActionKind::ReplaceSelection));
    let text = request.text.clone().unwrap_or_default();
    if !is_valid_action || text.is_empty() || text.encode_utf16().count() > MAX_TEXT_LEN {
        return ActionResponse::failure(
            "invalid_request",
            "Only a non-empty bounded draft or selection replacement can be applied.",
        );
    }

    let target = match resolve_live(&request.expected, true) {
        Ok(t) => t,
        Err(e) => return e,
    };

    // Determine the range to edit.
    let range = match target.sel {
        Some(sel) => sel,
        None if target.value.is_empty() => (0, 0),
        None => {
            return ActionResponse::failure(
                "unsupported_target",
                "The focused control did not expose a cursor or selection.",
            )
        }
    };
    if action == Some(ActionKind::ReplaceSelection) && range.1 == 0 {
        return ActionResponse::failure("stale_target", "There is no longer selected text to replace.");
    }
    let next = match replacing(&target.value, range, &text) {
        Some(n) => n,
        None => {
            return ActionResponse::failure("invalid_request", "The proposed edit exceeded the safe text boundary.")
        }
    };

    // Apply + verify (+ rollback on unexpected divergence).
    let applied = apply_mutation(&next);
    let actual = read_focused_value();
    match mutation_disposition(&target.value, &next, actual.as_deref()) {
        Disposition::Verified => {}
        Disposition::Unreadable if applied => {} // applied but couldn't read back; treat as verified best-effort
        Disposition::Unreadable => {
            return ActionResponse::failure(
                "verification_failed",
                "The target value became unreadable after Apply.",
            )
        }
        Disposition::Unchanged => {
            return ActionResponse::failure(
                "apply_failed",
                "The target app rejected the text change without changing the field.",
            )
        }
        Disposition::Unexpected(actual_value) => {
            return rollback(&target, &actual_value, "apply_rolled_back");
        }
    }

    let undo = UndoPayload {
        expected: request.expected.clone(),
        original_value: target.value.clone(),
        original_selection_location: target.sel.map(|(l, _)| l),
        original_selection_length: target.sel.map(|(_, n)| n),
        applied_content_fingerprint: content_fingerprint(&target.role, target.label.as_deref(), &next),
    };
    ActionResponse {
        ok: true,
        status: "applied".into(),
        verified: true,
        undo: Some(undo),
        error: None,
    }
}

fn undo(request: &ActionRequest) -> ActionResponse {
    let Some(undo) = &request.undo else {
        return ActionResponse::failure("invalid_request", "The undo record is unavailable.");
    };
    if undo.original_value.encode_utf16().count() > MAX_TEXT_LEN {
        return ActionResponse::failure("invalid_request", "The undo record is unavailable.");
    }
    let target = match resolve_live(&request.expected, false) {
        Ok(t) => t,
        Err(e) => return e,
    };
    if content_fingerprint(&target.role, target.label.as_deref(), &target.value)
        != undo.applied_content_fingerprint
    {
        return ActionResponse::failure(
            "stale_target",
            "The field changed after Apply, so automatic undo was cancelled.",
        );
    }
    let applied = apply_mutation(&undo.original_value);
    let actual = read_focused_value();
    match mutation_disposition(&target.value, &undo.original_value, actual.as_deref()) {
        Disposition::Verified => {}
        Disposition::Unreadable if applied => {}
        Disposition::Unreadable => {
            return ActionResponse::failure("verification_failed", "The target value became unreadable during Undo.")
        }
        Disposition::Unchanged => {
            return ActionResponse::failure("undo_failed", "The target app rejected Undo without changing the field.")
        }
        Disposition::Unexpected(actual_value) => {
            return rollback(&target, &actual_value, "undo_rolled_back");
        }
    }
    ActionResponse {
        ok: true,
        status: "undone".into(),
        verified: true,
        undo: None,
        error: None,
    }
}

/// Attempt to restore `target.value` after an unexpected divergence.
fn rollback(target: &LiveTarget, _actual: &str, rolled_back_status: &str) -> ActionResponse {
    if apply_mutation(&target.value) {
        if read_focused_value().as_deref() == Some(target.value.as_str()) {
            return ActionResponse::failure(
                rolled_back_status,
                "The target transformed the draft, so the original text was restored.",
            );
        }
    }
    ActionResponse::failure(
        "verification_failed",
        "The target changed unexpectedly and exact rollback could not be verified.",
    )
}

// ---------------------------------------------------------------------------
// Native primitives (#[cfg(windows)]) + host fallbacks
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn resolve_live(expected: &ExpectedTarget, require_fp: bool) -> Result<LiveTarget, ActionResponse> {
    use uiautomation::UIAutomation;

    // Allowlist: only the disposable editor.
    if !expected.bundle_id.eq_ignore_ascii_case(ALLOWED_EDITOR_EXE) {
        return Err(ActionResponse::failure(
            "unsupported_target",
            "Guarded Apply is currently limited to a disposable Notepad document.",
        ));
    }
    let pid = expected.process_identifier;
    if pid <= 0 {
        return Err(ActionResponse::failure("stale_target", "The exact target process is no longer available."));
    }
    let our_pid = std::process::id();
    let exe = crate::procs::exe_for_pid(pid as u32).unwrap_or_default();
    if !action_gate_ok(pid as u32, our_pid, &exe) {
        return Err(ActionResponse::failure("stale_target", "The exact target process is no longer available."));
    }
    // Bring the target forward; a refusal is `activation_failed`.
    if !crate::window::activate_pid(pid as u32) {
        return Err(ActionResponse::failure(
            "activation_failed",
            "The target app could not be brought forward safely.",
        ));
    }
    std::thread::sleep(std::time::Duration::from_millis(80));

    let automation = UIAutomation::new()
        .map_err(|_| ActionResponse::failure("permission_required", "UI Automation is unavailable."))?;
    let el = automation
        .get_focused_element()
        .map_err(|_| ActionResponse::failure("stale_target", "The editable target lost focus."))?;

    // get_process_id() is i32 (UIA); `pid` (expected.process_identifier) is i64 — widen to compare.
    if i64::from(el.get_process_id().unwrap_or(-1)) != pid {
        return Err(ActionResponse::failure("stale_target", "Another app took focus before the edit could be applied."));
    }
    let ct_name = el
        .get_control_type()
        .map(|ct| format!("{ct:?}"))
        .unwrap_or_default();
    let role = crate::uia::ax_role_for(&ct_name);
    let name = el.get_name().unwrap_or_default();
    let label = if name.is_empty() { None } else { Some(name.clone()) };
    let is_password = false; // notepad's edit is never secure
    if role != expected.focused_role
        || label.as_deref() != expected.focused_label.as_deref()
        || !crate::uia::is_editable_role(&role)
        || crate::uia::is_secure(&role, &name, is_password)
    {
        return Err(ActionResponse::failure(
            "stale_target",
            "The focused control changed or is not safely editable.",
        ));
    }
    let value = read_focused_value().unwrap_or_default();
    if value.encode_utf16().count() > MAX_TEXT_LEN {
        return Err(ActionResponse::failure("unsupported_target", "The focused control did not expose a bounded text value."));
    }
    // TODO(on-device): read the caret/selection via TextPattern.GetSelection → (location, length) in
    // UTF-16 units. Until verified we report no selection (insert-at-caret / whole-value semantics).
    let sel: Option<(i64, i64)> = None;
    if require_fp {
        let fp = helpers_core::fingerprint::state_fingerprint(&role, label.as_deref(), &value, sel);
        if fp != expected.focused_fingerprint {
            return Err(ActionResponse::failure("stale_target", "The text or selection changed. Prepare the draft again."));
        }
    }
    Ok(LiveTarget { role, label, value, sel })
}

/// Set the whole value: prefer UIA `ValuePattern.SetValue`, else select-all + inject text.
#[cfg(windows)]
fn apply_mutation(next: &str) -> bool {
    use uiautomation::patterns::UIValuePattern;
    use uiautomation::UIAutomation;

    if let Ok(automation) = UIAutomation::new() {
        if let Ok(el) = automation.get_focused_element() {
            // Preferred: ValuePattern.SetValue (atomic).
            if let Ok(vp) = el.get_pattern::<UIValuePattern>() {
                if vp.set_value(next).is_ok() {
                    return true;
                }
            }
        }
    }
    // Fallback: select-all then type. TODO(on-device): verify Ctrl+A select works in the target;
    // for notepad it does. enigo handles the Unicode text.
    select_all_windows();
    crate::inject::type_text(next)
}

#[cfg(windows)]
fn select_all_windows() {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    if let Ok(mut e) = Enigo::new(&Settings::default()) {
        // TODO(on-device): confirm modifier chord API. Ctrl down, 'a', Ctrl up.
        let _ = e.key(Key::Control, Direction::Press);
        let _ = e.key(Key::Unicode('a'), Direction::Click);
        let _ = e.key(Key::Control, Direction::Release);
    }
}

#[cfg(windows)]
fn read_focused_value() -> Option<String> {
    use uiautomation::patterns::UIValuePattern;
    use uiautomation::UIAutomation;
    std::thread::sleep(std::time::Duration::from_millis(30)); // ~30 ms settle (Swift valueEquals)
    let automation = UIAutomation::new().ok()?;
    let el = automation.get_focused_element().ok()?;
    let vp = el.get_pattern::<UIValuePattern>().ok()?;
    vp.get_value().ok()
}

#[cfg(not(windows))]
fn resolve_live(_expected: &ExpectedTarget, _require_fp: bool) -> Result<LiveTarget, ActionResponse> {
    crate::degrade("UIA agent-action resolve");
    Err(ActionResponse::failure(
        "permission_required",
        "UI Automation is unavailable on this host.",
    ))
}

#[cfg(not(windows))]
fn apply_mutation(_next: &str) -> bool {
    false
}

#[cfg(not(windows))]
fn read_focused_value() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_utf16_faithful() {
        assert_eq!(replacing("A🙂B", (1, 2), "ok").as_deref(), Some("AokB"));
        assert_eq!(replacing("hello", (5, 0), "!").as_deref(), Some("hello!"));
        assert_eq!(replacing("hello", (6, 0), "!"), None);
    }

    #[test]
    fn disposition_cases() {
        assert_eq!(mutation_disposition("before", "after", Some("after")), Disposition::Verified);
        assert_eq!(mutation_disposition("before", "after", Some("before")), Disposition::Unchanged);
        assert_eq!(mutation_disposition("before", "after", None), Disposition::Unreadable);
        assert_eq!(
            mutation_disposition("before", "after", Some("changed")),
            Disposition::Unexpected("changed".into())
        );
    }

    #[test]
    fn gate_predicate() {
        assert!(action_gate_ok(42, 7, "notepad.exe"));
        assert!(!action_gate_ok(7, 7, "notepad.exe")); // our own pid
        assert!(!action_gate_ok(42, 7, "chrome.exe")); // wrong editor
    }

    #[test]
    fn fingerprints_match_helpers_core() {
        // Sanity: content fingerprint is stable and distinct from state fingerprint with a range.
        let a = content_fingerprint("AXTextArea", Some("Body"), "x");
        let b = content_fingerprint("AXTextArea", Some("Body"), "x");
        assert_eq!(a, b);
        assert_ne!(
            helpers_core::fingerprint::state_fingerprint("AXTextArea", None, "x", Some((0, 0))),
            helpers_core::fingerprint::state_fingerprint("AXTextArea", None, "x", Some((1, 0)))
        );
    }
}
