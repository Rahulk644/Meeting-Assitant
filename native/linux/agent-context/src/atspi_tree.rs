//! atspi-backed target discovery — the TODO(on-device) adapter.
//!
//! REAL here: the AT-SPI bus connection attempt (the permission/availability gate) and the X11
//! active-window cross-check. The deep tree query (registry → apps → active window → focused
//! element → children) is written as the correct sequence in comments with real atspi types, but its
//! live result needs a running AT-SPI registry + a real app, so it is marked TODO(on-device) and
//! currently degrades to a contract-valid `no_window`. The binary is always well-formed.
//!
//! AT-SPI gating (PROTOCOL.md §3 "gated on a11y being enabled"): toolkits only populate the
//! accessibility tree when `toolkit-accessibility` (GTK) / `QT_ACCESSIBILITY=1` (Qt) is on. When the
//! bus is unreachable we emit `permission_required`; when it is reachable but the tree is empty we
//! emit `no_window` — never a crash.

use crate::ax::{error_snapshot, ReadyTarget};
use helpers_core::protocol::Snapshot;

/// Discovery outcome: either a ready target to walk, or a finished error snapshot to emit as-is.
pub enum Discovery {
    Ready(ReadyTarget),
    Error(Snapshot),
}

/// Connect to the AT-SPI bus and resolve the active window + focused element.
pub async fn discover(captured_at: u64) -> Discovery {
    // 1) Bus availability = our "permission" gate. If the a11y bus is not reachable, the toolkit
    //    accessibility is effectively off/denied.
    if let Err(e) = connect().await {
        eprintln!(">> AT-SPI bus unavailable ({e}); is toolkit-accessibility / QT_ACCESSIBILITY=1 set?");
        return Discovery::Error(error_snapshot(
            "permission_required",
            "none",
            captured_at,
            None,
            None,
            None,
            "Accessibility (AT-SPI) is not available. Enable toolkit accessibility.",
        ));
    }

    // 2) TODO(on-device): resolve the active application + its active window + focused element.
    //    Correct atspi sequence (needs a live registry to verify method names/return shapes):
    //
    //      let registry = conn.root_accessible_on_registry().await?;   // the desktop root
    //      let apps = registry.get_children().await?;                  // one child per app
    //      // pick the app whose window carries StateSet containing State::Active:
    //      for app in apps {
    //          let app_proxy = AccessibleProxy::from(app);
    //          for win in app_proxy.get_children().await? {
    //              let states = win.get_state().await?;                // StateSet
    //              if states.contains(State::Active) { /* active window */ }
    //          }
    //      }
    //      // focused element = descend following State::Focused, or query the app's
    //      // "focus" via the AT-SPI focus event cache.
    //      // Map each node's Role via `role.name()` → platform::roles::atspi_role_to_ax(...).
    //      // Wrap each proxy in an `impl ax::AxNode` (title = get_name(), value/selection via the
    //      // Text/EditableText interfaces, actions via the Action interface, frame via Component).
    //
    //    Until that is completed on a real session, degrade gracefully. Optionally enrich the log
    //    with the X11 active-window title so operators can see *something* was in front.
    let hint = x11_active_title_hint();
    if let Some(t) = &hint {
        eprintln!(">> (x11 cross-check) active window title: {t:?}");
    }

    // Keep a reference to the intended flow so future work has a compile-checked skeleton.
    let _unused: Option<ReadyTarget> = None;

    Discovery::Error(error_snapshot(
        "no_window",
        "frontmost_app",
        captured_at,
        None,
        None,
        None,
        "The target app did not expose an Accessibility window.",
    ))
}

/// Real bus connection attempt. Kept in its own fn so the error is easy to map to the gate.
async fn connect() -> Result<(), Box<dyn std::error::Error>> {
    // TODO(on-device): confirm the exact constructor name against the pinned atspi 0.28 — the crate
    // exposes an `AccessibilityConnection` with an async `new()` that binds to the a11y bus. This is
    // the single real atspi symbol referenced; everything downstream of a successful bind is the
    // TODO(on-device) tree walk.
    atspi::AccessibilityConnection::new().await?;
    Ok(())
}

/// Best-effort X11 `_NET_ACTIVE_WINDOW` title, purely as a diagnostic cross-check (empty on Wayland).
fn x11_active_title_hint() -> Option<String> {
    let server = platform::display::detect();
    if !server.can_enumerate_windows() {
        return None;
    }
    platform::win::X11::connect().and_then(|x| x.active_window_title())
}
