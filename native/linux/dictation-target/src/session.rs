//! `--session` preview/commit/cancel state machine — REAL, unit-tested. The full error taxonomy,
//! `active`/`unsafeFallback` truth tables, tracked-range accounting, and terminal-op semantics run
//! against the [`SessionOps`] trait, so they are verifiable with a mock. The concrete atspi+injector
//! ops are the TODO(on-device) part (`atspi_focus.rs` + `platform::inject`).
//!
//! Contract: `native/PROTOCOL.md` §2 + the Swift `runSession`. Offsets/lengths here are AT-SPI
//! character offsets (the Linux-native unit); the invariant preserved is "trackedRange covers
//! exactly the provisional text and the caret sits at its end."

use helpers_core::protocol::{
    session_error as err, SessionCommand, SessionOp, SessionResponse, COMMIT_TEXT_MAX,
    PREVIEW_TEXT_MAX,
};

/// The live editing operations the session drives. On Linux: `replace_range` = AT-SPI select-span +
/// `platform::inject` type/backspace; the reads are AT-SPI `Text` queries.
pub trait SessionOps {
    /// Re-resolve the focused element and confirm it still matches the captured identity.
    fn focus_matches(&self) -> bool;
    /// Text currently occupying `range` (AT-SPI char offsets), or `None` if unreadable.
    fn string_for_range(&self, range: (i64, i64)) -> Option<String>;
    /// Current caret/selection.
    fn selection_range(&self) -> Option<(i64, i64)>;
    /// Select `range` then inject `text` (replacing the selection; empty ⇒ delete). Returns whether
    /// the write was dispatched.
    fn replace_range(&mut self, range: (i64, i64), text: &str) -> bool;
    /// Poll until `range` holds `text` with the caret collapsed at `cursor`, or timeout. Returns
    /// whether it was observed.
    fn wait_for_tracked(&self, range: (i64, i64), text: &str, cursor: (i64, i64), timeout_ms: u64) -> bool;
    /// Set the caret/selection (best-effort cursor restore on commit).
    fn set_selection(&mut self, range: (i64, i64)) -> bool;
}

fn char_len(s: &str) -> i64 {
    s.chars().count() as i64
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn resp(ok: bool, active: bool, error: Option<&str>, unsafe_fallback: bool) -> SessionResponse {
    SessionResponse {
        ok,
        active,
        error: error.map(|s| s.to_string()),
        unsafe_fallback,
    }
}

/// Run the command loop. `insertion_location` is the caret captured at classify time. `lines` yields
/// raw stdin lines (so a non-decodable line becomes `invalidCommand`). `emit` receives one response
/// per command. Returns when a terminal op (commit/cancel) runs or `lines` is exhausted (EOF),
/// stripping any outstanding provisional span on EOF.
pub fn run_session(
    insertion_location: i64,
    ops: &mut dyn SessionOps,
    lines: impl Iterator<Item = String>,
    mut emit: impl FnMut(&SessionResponse),
) {
    let mut current = String::new();

    for line in lines {
        let tracked = (insertion_location, char_len(&current));
        let expected_cursor = (tracked.0 + tracked.1, 0);
        let active = !current.is_empty();

        let command: Option<SessionCommand> = serde_json::from_str(&line).ok();
        let Some(command) = command else {
            emit(&resp(false, active, Some(err::INVALID_COMMAND), active));
            continue;
        };

        match command.op {
            SessionOp::Preview => {
                let next = truncate_chars(command.text.as_deref().unwrap_or(""), PREVIEW_TEXT_MAX);
                if let Some(r) = preview(ops, &mut current, tracked, expected_cursor, insertion_location, &next) {
                    emit(&r);
                }
            }
            SessionOp::Commit => {
                let final_text = truncate_chars(command.text.as_deref().unwrap_or(""), COMMIT_TEXT_MAX);
                let r = commit(ops, &current, tracked, expected_cursor, insertion_location, &final_text);
                emit(&r);
                return; // terminal
            }
            SessionOp::Cancel => {
                let r = cancel(ops, &current, tracked);
                emit(&r);
                return; // terminal
            }
        }
    }

    // EOF: strip any outstanding provisional span.
    if !current.is_empty() {
        let tracked = (insertion_location, char_len(&current));
        let _ = remove_tracked(ops, tracked, &current);
    }
}

/// Returns `Some(response)` to emit; mutates `current` on success. Non-terminal.
fn preview(
    ops: &mut dyn SessionOps,
    current: &mut String,
    tracked: (i64, i64),
    expected_cursor: (i64, i64),
    insertion_location: i64,
    next: &str,
) -> Option<SessionResponse> {
    let active = !current.is_empty();
    if !ops.focus_matches() {
        return Some(resp(false, active, Some(err::TARGET_CHANGED), true));
    }
    if ops.string_for_range(tracked).as_deref() != Some(current.as_str()) {
        return Some(resp(false, active, Some(err::PROVISIONAL_TEXT_CHANGED), true));
    }
    if ops.selection_range() != Some(expected_cursor) {
        return Some(resp(false, active, Some(err::CURSOR_CHANGED), true));
    }
    if !ops.replace_range(tracked, next) {
        // Note: unsafeFallback is false here (Swift) — the edit was rejected before anything changed.
        return Some(resp(false, active, Some(err::PREVIEW_EDIT_REJECTED), false));
    }
    let next_range = (insertion_location, char_len(next));
    let next_cursor = (next_range.0 + next_range.1, 0);
    if !ops.wait_for_tracked(next_range, next, next_cursor, 350) {
        return Some(resp(false, active, Some(err::PREVIEW_EDIT_NOT_OBSERVED), active));
    }
    *current = next.to_string();
    let now_active = !current.is_empty();
    Some(resp(true, now_active, None, false))
}

fn commit(
    ops: &mut dyn SessionOps,
    current: &str,
    tracked: (i64, i64),
    expected_cursor: (i64, i64),
    insertion_location: i64,
    final_text: &str,
) -> SessionResponse {
    let active = !current.is_empty();
    if !ops.focus_matches() || ops.string_for_range(tracked).as_deref() != Some(current) {
        return resp(false, active, Some(err::PROVISIONAL_TEXT_UNAVAILABLE), true);
    }
    let previous_selection = ops.selection_range();
    if !ops.replace_range(tracked, final_text) {
        return resp(false, active, Some(err::FINAL_EDIT_REJECTED), true);
    }
    let final_range = (insertion_location, char_len(final_text));
    let final_cursor = (final_range.0 + final_range.1, 0);
    if !ops.wait_for_tracked(final_range, final_text, final_cursor, 600) {
        return resp(false, active, Some(err::FINAL_EDIT_NOT_OBSERVED), true);
    }
    // Cursor restore: only if the user's saved selection wasn't just the provisional-end caret.
    if let Some(prev) = previous_selection {
        if prev != expected_cursor {
            let adjusted = adjusted_selection(prev, tracked, char_len(final_text));
            let _ = ops.set_selection(adjusted);
        }
    }
    resp(true, false, None, false)
}

fn cancel(ops: &mut dyn SessionOps, current: &str, tracked: (i64, i64)) -> SessionResponse {
    let removed = remove_tracked(ops, tracked, current);
    let active = !removed && !current.is_empty();
    let error = if removed { None } else { Some(err::PREVIEW_REMOVAL_REJECTED) };
    resp(removed, active, error, active)
}

/// Strip the provisional span. `true` if there was nothing to remove or the removal was observed.
fn remove_tracked(ops: &mut dyn SessionOps, range: (i64, i64), text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    if !ops.focus_matches() || ops.string_for_range(range).as_deref() != Some(text) {
        return false;
    }
    if !ops.replace_range(range, "") {
        return false;
    }
    let collapsed = (range.0, 0);
    ops.wait_for_tracked(collapsed, "", collapsed, 350)
}

/// Remap a saved selection across a length change at `range` (Swift `adjustedSelection`).
fn adjusted_selection(sel: (i64, i64), range: (i64, i64), replacement_len: i64) -> (i64, i64) {
    let old_end = range.0 + range.1;
    let delta = replacement_len - range.1;
    let map = |p: i64| -> i64 {
        let mapped = if p <= range.0 {
            p
        } else if p >= old_end {
            p + delta
        } else {
            range.0 + replacement_len
        };
        mapped.max(0)
    };
    let start = map(sel.0);
    let end = map(sel.0 + sel.1);
    (start, (end - start).max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cooperative mock text field that honors replace_range/select and always observes edits.
    struct Mock {
        value: String,
        selection: Option<(i64, i64)>,
    }

    impl Mock {
        fn new(insertion: i64) -> Self {
            // Seed `insertion` chars of pre-existing text so the caret at `insertion` is valid.
            let value = "x".repeat(insertion.max(0) as usize);
            Mock { value, selection: Some((insertion, 0)) }
        }
        fn substr(&self, range: (i64, i64)) -> Option<String> {
            let chars: Vec<char> = self.value.chars().collect();
            let (l, len) = (range.0 as usize, range.1 as usize);
            if l + len <= chars.len() {
                Some(chars[l..l + len].iter().collect())
            } else {
                None
            }
        }
    }

    impl SessionOps for Mock {
        fn focus_matches(&self) -> bool {
            true
        }
        fn string_for_range(&self, range: (i64, i64)) -> Option<String> {
            self.substr(range)
        }
        fn selection_range(&self) -> Option<(i64, i64)> {
            self.selection
        }
        fn replace_range(&mut self, range: (i64, i64), text: &str) -> bool {
            let chars: Vec<char> = self.value.chars().collect();
            let (l, len) = (range.0 as usize, range.1 as usize);
            if l + len > chars.len() {
                return false;
            }
            let mut out: String = chars[..l].iter().collect();
            out.push_str(text);
            out.extend(&chars[l + len..]);
            self.value = out;
            self.selection = Some((range.0 + text.chars().count() as i64, 0));
            true
        }
        fn wait_for_tracked(&self, range: (i64, i64), text: &str, cursor: (i64, i64), _t: u64) -> bool {
            self.substr(range).as_deref() == Some(text) && self.selection == Some(cursor)
        }
        fn set_selection(&mut self, range: (i64, i64)) -> bool {
            self.selection = Some(range);
            true
        }
    }

    fn run(lines: &[&str], insertion: i64) -> Vec<SessionResponse> {
        let mut ops = Mock::new(insertion);
        let mut out = Vec::new();
        run_session(
            insertion,
            &mut ops,
            lines.iter().map(|s| s.to_string()),
            |r| out.push(r.clone()),
        );
        out
    }

    #[test]
    fn invalid_command_keeps_session_alive() {
        let out = run(&["not json", "{\"op\":\"cancel\"}"], 0);
        assert_eq!(out[0].error.as_deref(), Some(err::INVALID_COMMAND));
        assert!(!out[0].active); // current empty
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn preview_then_commit_flow() {
        let out = run(
            &[r#"{"op":"preview","text":"hello"}"#, r#"{"op":"commit","text":"hello world"}"#],
            0,
        );
        assert!(out[0].ok && out[0].active); // preview provisional present
        assert_eq!(out[0].error, None);
        assert!(out[1].ok && !out[1].active); // commit ends session, active false
    }

    #[test]
    fn empty_preview_clears_active() {
        let out = run(&[r#"{"op":"preview","text":"hi"}"#, r#"{"op":"preview","text":""}"#], 0);
        assert!(out[0].active);
        assert!(out[1].ok && !out[1].active); // provisional cleared
    }

    #[test]
    fn unknown_op_reports_unknown_command() {
        // SessionOp only decodes preview/commit/cancel; an unknown op fails to decode → invalidCommand.
        let out = run(&[r#"{"op":"bogus","text":"x"}"#], 0);
        assert_eq!(out[0].error.as_deref(), Some(err::INVALID_COMMAND));
    }

    #[test]
    fn cancel_removes_provisional() {
        let out = run(&[r#"{"op":"preview","text":"draft"}"#, r#"{"op":"cancel"}"#], 3);
        assert!(out[0].ok);
        assert!(out[1].ok && !out[1].active); // removed
    }
}
