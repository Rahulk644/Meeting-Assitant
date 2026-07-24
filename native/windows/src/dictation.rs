//! `dictation-target --session` — the preview/commit/cancel state machine over stdin/stdout.
//!
//! The **protocol logic is portable and real**: command parsing, the exact error-token taxonomy
//! (`helpers_core::protocol::session_error`), the ≤4000/≤20000 length caps, one JSON response per
//! line, and provisional-span cleanup on stdin-EOF. The **edit + observation primitives** are
//! `#[cfg(windows)]` (raw `SendInput` via [`crate::inject`] + UIA reacquire/read); each has a
//! `#[cfg(not(windows))]` fallback. Because the classifier only reports `liveEditable:true` on a
//! device with a UIA text/value pattern, the command loop only actually runs on Windows — off-device
//! the helper emits the classification and exits (contract-valid).
//!
//! On-device caveat (marked below): the strict provisional-text / cursor reconciliation that macOS
//! gets from `AXStringForRange` needs UIA `TextPattern` range reads to be equally strict. Where a
//! range read is unavailable we fall back to injecting-and-optimistically-accepting, which is why
//! those spots are TODO(on-device).

use std::io::{BufRead, Write};

use helpers_core::protocol::{session_error as E, SessionResponse, COMMIT_TEXT_MAX, PREVIEW_TEXT_MAX};

#[cfg(windows)]
use crate::inject::{self, InjectOutcome};
use crate::uia::FocusIdentity;

fn emit(resp: &SessionResponse) {
    if let Ok(s) = serde_json::to_string(resp) {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = writeln!(lock, "{s}");
        let _ = lock.flush();
    }
}

fn resp(ok: bool, active: bool, error: Option<&str>, unsafe_fallback: bool) -> SessionResponse {
    SessionResponse {
        ok,
        active,
        error: error.map(|e| e.to_string()),
        unsafe_fallback,
    }
}

/// Run the stdin command loop for a live-editable target. Emits one response per command line;
/// cleans up any outstanding provisional span on EOF.
pub fn run_session(id: FocusIdentity) {
    let mut current = String::new();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                emit(&resp(false, !current.is_empty(), Some(E::INVALID_COMMAND), !current.is_empty()));
                continue;
            }
        };
        let op = value.get("op").and_then(|o| o.as_str());
        let text = value.get("text").and_then(|t| t.as_str()).unwrap_or("");

        match op {
            Some("preview") => {
                let next: String = text.chars().take(PREVIEW_TEXT_MAX).collect();
                match apply_span(&id, current.chars().count(), &next) {
                    SpanResult::TargetChanged => {
                        emit(&resp(false, !current.is_empty(), Some(E::TARGET_CHANGED), true));
                    }
                    SpanResult::ProvisionalChanged => {
                        emit(&resp(false, !current.is_empty(), Some(E::PROVISIONAL_TEXT_CHANGED), true));
                    }
                    SpanResult::Rejected { unsafe_fallback } => {
                        emit(&resp(false, !current.is_empty(), Some(E::PREVIEW_EDIT_REJECTED), unsafe_fallback));
                    }
                    SpanResult::NotObserved => {
                        emit(&resp(false, !current.is_empty(), Some(E::PREVIEW_EDIT_NOT_OBSERVED), !current.is_empty()));
                    }
                    SpanResult::Ok => {
                        current = next;
                        emit(&resp(true, !current.is_empty(), None, false));
                    }
                }
            }
            Some("commit") => {
                let final_text: String = text.chars().take(COMMIT_TEXT_MAX).collect();
                match apply_span(&id, current.chars().count(), &final_text) {
                    SpanResult::TargetChanged | SpanResult::ProvisionalChanged => {
                        emit(&resp(false, !current.is_empty(), Some(E::PROVISIONAL_TEXT_UNAVAILABLE), true));
                    }
                    SpanResult::Rejected { .. } => {
                        emit(&resp(false, !current.is_empty(), Some(E::FINAL_EDIT_REJECTED), true));
                    }
                    SpanResult::NotObserved => {
                        emit(&resp(false, !current.is_empty(), Some(E::FINAL_EDIT_NOT_OBSERVED), true));
                    }
                    SpanResult::Ok => {
                        emit(&resp(true, false, None, false));
                    }
                }
                return;
            }
            Some("cancel") => {
                let removed = match remove_span(&id, current.chars().count()) {
                    SpanResult::Ok => true,
                    _ => current.is_empty(),
                };
                emit(&resp(
                    removed,
                    !removed && !current.is_empty(),
                    if removed { None } else { Some(E::PREVIEW_REMOVAL_REJECTED) },
                    !removed && !current.is_empty(),
                ));
                return;
            }
            Some(_) => {
                emit(&resp(false, !current.is_empty(), Some(E::UNKNOWN_COMMAND), !current.is_empty()));
            }
            None => {
                emit(&resp(false, !current.is_empty(), Some(E::INVALID_COMMAND), !current.is_empty()));
            }
        }
    }

    // stdin EOF: remove any outstanding provisional span first (PROTOCOL §2).
    if !current.is_empty() {
        let _ = remove_span(&id, current.chars().count());
    }
}

/// Result of replacing the provisional span with new text. Several variants are only constructed on
/// the Windows edit path; the host fallback returns `TargetChanged`.
#[allow(dead_code)]
enum SpanResult {
    Ok,
    TargetChanged,
    ProvisionalChanged,
    Rejected { unsafe_fallback: bool },
    NotObserved,
}

/// Replace the current `prev_len`-char provisional with `next`: reacquire focus, delete the old span,
/// type the new text, verify it landed.
#[cfg(windows)]
fn apply_span(id: &FocusIdentity, prev_len: usize, next: &str) -> SpanResult {
    if !focus_matches(id) {
        return SpanResult::TargetChanged;
    }
    // TODO(on-device): before mutating, read the current provisional span and caret via UIA
    // TextPattern and compare (⇒ ProvisionalChanged / CursorChanged) exactly as the Swift session
    // does with AXStringForRange. Until that read is verified, we proceed to inject.
    if prev_len > 0 {
        match inject::send_backspace(prev_len) {
            InjectOutcome::Ok => {}
            InjectOutcome::UipiBlocked => return SpanResult::Rejected { unsafe_fallback: true },
            InjectOutcome::Failed => return SpanResult::Rejected { unsafe_fallback: false },
        }
    }
    if !next.is_empty() {
        match inject::send_unicode(next) {
            InjectOutcome::Ok => {}
            InjectOutcome::UipiBlocked => return SpanResult::Rejected { unsafe_fallback: true },
            InjectOutcome::Failed => return SpanResult::Rejected { unsafe_fallback: false },
        }
    }
    // Best-effort observation: if we can read the field, require the new text to be present.
    match read_focused_value(id) {
        Some(v) if next.is_empty() || v.contains(next) => SpanResult::Ok,
        Some(_) => SpanResult::NotObserved,
        None => SpanResult::Ok, // read unavailable ⇒ optimistic accept (TODO on-device: strict verify)
    }
}

#[cfg(windows)]
fn remove_span(id: &FocusIdentity, prev_len: usize) -> SpanResult {
    if prev_len == 0 {
        return SpanResult::Ok;
    }
    if !focus_matches(id) {
        return SpanResult::TargetChanged;
    }
    match inject::send_backspace(prev_len) {
        InjectOutcome::Ok => SpanResult::Ok,
        InjectOutcome::UipiBlocked => SpanResult::Rejected { unsafe_fallback: true },
        InjectOutcome::Failed => SpanResult::Rejected { unsafe_fallback: false },
    }
}

/// Is the originally-classified control still the focused element?
#[cfg(windows)]
fn focus_matches(id: &FocusIdentity) -> bool {
    use uiautomation::UIAutomation;
    let Ok(automation) = UIAutomation::new() else {
        return false;
    };
    let Ok(el) = automation.get_focused_element() else {
        return false;
    };
    let pid = el.get_process_id().unwrap_or(-1);
    if pid != id.pid {
        return false;
    }
    let ct_name = el
        .get_control_type()
        .map(|ct| format!("{ct:?}"))
        .unwrap_or_default();
    if crate::uia::ax_role_for(&ct_name) != id.ax_role {
        return false;
    }
    // Name may legitimately be empty; only reject on a definite mismatch.
    let name = el.get_name().unwrap_or_default();
    name.is_empty() || id.name.is_empty() || name == id.name
}

#[cfg(windows)]
fn read_focused_value(id: &FocusIdentity) -> Option<String> {
    use uiautomation::patterns::UIValuePattern;
    use uiautomation::UIAutomation;
    let automation = UIAutomation::new().ok()?;
    let el = automation.get_focused_element().ok()?;
    if el.get_process_id().unwrap_or(-1) != id.pid {
        return None;
    }
    let vp = el.get_pattern::<UIValuePattern>().ok()?;
    vp.get_value().ok()
}

// --- non-windows fallbacks (never reached: liveEditable is false off-device) ---

#[cfg(not(windows))]
fn apply_span(_id: &FocusIdentity, _prev_len: usize, _next: &str) -> SpanResult {
    SpanResult::TargetChanged
}

#[cfg(not(windows))]
fn remove_span(_id: &FocusIdentity, _prev_len: usize) -> SpanResult {
    SpanResult::TargetChanged
}
