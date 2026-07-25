//! atspi target resolution — REAL permission + allowlist gates, TODO(on-device) element resolution.
//!
//! The Swift `resolveTarget` runs a 12-step revalidation (permission → allowlist → process → window
//! → title → control identity → fingerprint). Two of those gates are OS-agnostic and fully REAL
//! here: the AT-SPI bus permission check and the single-app allowlist. The remaining steps require a
//! live AT-SPI control (and, on Wayland, window geometry/stacking that is simply *not queryable* —
//! see note), so they are written as the correct sequence and marked TODO(on-device); until wired,
//! resolution fails closed with `stale_target`, which is contract-valid and safe.

use crate::exec::{EditTarget, ALLOWED_BUNDLE};
use helpers_core::protocol::{ActionResponse, ExpectedTarget};

/// Resolve the live editable target, or return the failure response to emit as-is.
///
/// `require_fingerprint` distinguishes apply (checks `state_fingerprint` == expected) from undo
/// (skips it). Both share every other gate.
pub async fn resolve_target(
    expected: &ExpectedTarget,
    _require_fingerprint: bool,
) -> Result<Box<dyn EditTarget>, ActionResponse> {
    // 1) Permission — AT-SPI bus reachable?  (REAL)
    if let Err(e) = connect().await {
        eprintln!(">> AT-SPI bus unavailable ({e})");
        return Err(ActionResponse::failure(
            "permission_required",
            "Accessibility permission is required.",
        ));
    }

    // 2) Allowlist — exactly one disposable editor (REAL, pure).
    if expected.bundle_id != ALLOWED_BUNDLE {
        return Err(ActionResponse::failure(
            "unsupported_target",
            "Guarded Apply is currently limited to a disposable GNOME Text Editor document.",
        ));
    }

    // 3) WAYLAND LIMITATION: the "window behind the dashboard" activation gate (Swift
    //    `appBehindDashboardProcessIdentifier` via CGWindowList) has NO Wayland equivalent — a client
    //    cannot query window stacking/geometry or raise another app's window. On Wayland this step
    //    can only fail closed (activation_failed / stale_target); on X11 it maps to
    //    `_NET_ACTIVE_WINDOW` + `_NET_CLIENT_LIST` + `_NET_WM_PID` (platform::win).
    //
    // 4) TODO(on-device): resolve the focused editable control and validate identity. Correct atspi
    //    sequence (needs a live control to verify):
    //      - find the app by name/pid == expected.process_identifier,
    //      - its active window (StateSet contains State::Active); compare cleaned title,
    //      - its focused element (State::Focused); map role via atspi_role_to_ax,
    //      - require role == expected.focused_role, label == expected.focused_label,
    //        State::Editable, and (apply only) state_fingerprint(role,label,value,range) ==
    //        expected.focused_fingerprint,
    //      - build an `impl EditTarget` over EditableText (set_selected_text = insert/replace over
    //        the selected range; set_whole_value = EditableText.set_text_contents; read_value_settled
    //        = sleep 30ms then Text.get_text; set_selection = Text.set_caret_offset/add_selection),
    //        converting UTF-16 offsets ↔ AT-SPI character offsets at this boundary.
    //
    //    Until then, fail closed.
    eprintln!(">> agent-action: control resolution not wired on this build; failing closed (stale_target)");
    Err(ActionResponse::failure(
        "stale_target",
        "The exact target control could not be revalidated. Prepare the draft again.",
    ))
}

async fn connect() -> Result<(), Box<dyn std::error::Error>> {
    // TODO(on-device): confirm the constructor against pinned atspi 0.28. Single real atspi symbol.
    atspi::AccessibilityConnection::new().await?;
    Ok(())
}
