//! Display-server detection — REAL, pure env logic (no live server required, fully unit-tested).
//!
//! One half of the 2×2 runtime world. Every helper calls [`detect`] at startup and branches on the
//! result. Wayland's two hard blocks are encoded as capability flags here so callers never guess:
//! - [`DisplayServer::can_inject_global_input`] — false on Wayland (XTEST is blocked; injection must
//!   go through libei / the RemoteDesktop portal / uinput, all of which require user consent).
//! - [`DisplayServer::can_enumerate_windows`] — false on Wayland (no client can list other clients'
//!   windows; `_NET_CLIENT_LIST` does not exist).
//!
//! Detection precedence mirrors the freedesktop convention: `XDG_SESSION_TYPE` is authoritative when
//! set to `wayland`/`x11`; otherwise `WAYLAND_DISPLAY` implies Wayland and `DISPLAY` implies X11.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayServer {
    X11,
    Wayland,
    /// Neither socket/var present — headless or a TTY. Helpers degrade to "no GUI" behavior.
    Headless,
}

impl DisplayServer {
    /// X11 permits synthetic global input via the XTEST extension; Wayland does not.
    pub fn can_inject_global_input(self) -> bool {
        matches!(self, DisplayServer::X11)
    }

    /// X11 exposes the window list via `_NET_CLIENT_LIST`; Wayland exposes nothing cross-client.
    pub fn can_enumerate_windows(self) -> bool {
        matches!(self, DisplayServer::X11)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DisplayServer::X11 => "x11",
            DisplayServer::Wayland => "wayland",
            DisplayServer::Headless => "headless",
        }
    }
}

impl fmt::Display for DisplayServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Detect from the process environment. See module docs for precedence.
pub fn detect() -> DisplayServer {
    detect_from(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("DISPLAY").ok().as_deref(),
    )
}

/// Pure core of [`detect`], split out so it is unit-testable without touching the real environment.
pub fn detect_from(
    xdg_session_type: Option<&str>,
    wayland_display: Option<&str>,
    display: Option<&str>,
) -> DisplayServer {
    match xdg_session_type.map(str::trim) {
        Some("wayland") => return DisplayServer::Wayland,
        Some("x11") => return DisplayServer::X11,
        _ => {}
    }
    // XDG_SESSION_TYPE unset or "tty"/"unspecified": fall back to the socket env vars.
    if wayland_display.map(|s| !s.is_empty()).unwrap_or(false) {
        return DisplayServer::Wayland;
    }
    if display.map(|s| !s.is_empty()).unwrap_or(false) {
        return DisplayServer::X11;
    }
    DisplayServer::Headless
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_session_type_wins() {
        assert_eq!(detect_from(Some("wayland"), Some(":0"), Some(":0")), DisplayServer::Wayland);
        assert_eq!(detect_from(Some("x11"), Some("wayland-0"), None), DisplayServer::X11);
    }

    #[test]
    fn falls_back_to_socket_vars() {
        assert_eq!(detect_from(None, Some("wayland-0"), None), DisplayServer::Wayland);
        assert_eq!(detect_from(Some("tty"), None, Some(":0")), DisplayServer::X11);
        assert_eq!(detect_from(None, None, None), DisplayServer::Headless);
        assert_eq!(detect_from(None, Some(""), Some("")), DisplayServer::Headless);
    }

    #[test]
    fn capability_flags() {
        assert!(DisplayServer::X11.can_inject_global_input());
        assert!(DisplayServer::X11.can_enumerate_windows());
        assert!(!DisplayServer::Wayland.can_inject_global_input());
        assert!(!DisplayServer::Wayland.can_enumerate_windows());
    }
}
