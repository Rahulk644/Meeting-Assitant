//! X11 window enumeration + titles via EWMH — REAL API sequence with `x11rb`. Feature `x11`.
//!
//! On X11 this is fully functional: `_NET_ACTIVE_WINDOW`, `_NET_CLIENT_LIST`, `_NET_WM_NAME`,
//! `_NET_WM_PID`. It needs a live X server to actually return data, so the reads are marked
//! TODO(on-device) for their *runtime* result, but the API sequence and types are real and correct;
//! every function returns a safe empty/None fallback if the server is unreachable.
//!
//! WAYLAND LIMITATION (hard, called out here because it is the single biggest capability loss):
//! there is **no equivalent** — no Wayland client may enumerate or read other clients' windows or
//! titles. Under a Wayland session [`connect`] will typically fail (or, via XWayland, see only
//! XWayland clients, missing native Wayland apps entirely). Callers MUST fall back to AT-SPI window
//! names on Wayland and must not treat an empty result here as "no meeting".

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, Window};
use x11rb::rust_connection::RustConnection;

/// A live X11 connection + root window, or `None` under Wayland/headless.
pub struct X11 {
    conn: RustConnection,
    root: Window,
}

impl X11 {
    /// Connect to `$DISPLAY`. Returns `None` on any failure (Wayland-only session, no server, …).
    pub fn connect() -> Option<X11> {
        // TODO(on-device): actual reachability depends on a running X server; on a pure Wayland
        // session this fails and we correctly degrade to AT-SPI-based titles upstream.
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen_num)?.root;
        Some(X11 { conn, root })
    }

    fn atom(&self, name: &[u8]) -> Option<u32> {
        self.conn.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom)
    }

    /// Read the UTF-8 (`_NET_WM_NAME`, preferred) or legacy (`WM_NAME`) title of a window.
    pub fn window_title(&self, w: Window) -> Option<String> {
        if let Some(net_name) = self.atom(b"_NET_WM_NAME") {
            if let Some(s) = self.read_string_prop(w, net_name) {
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
        // Legacy WM_NAME (Latin-1/STRING).
        self.read_string_prop(w, u32::from(AtomEnum::WM_NAME))
    }

    fn read_string_prop(&self, w: Window, prop: u32) -> Option<String> {
        let reply = self
            .conn
            .get_property(false, w, prop, AtomEnum::ANY, 0, u32::MAX)
            .ok()?
            .reply()
            .ok()?;
        if reply.value.is_empty() {
            return None;
        }
        Some(String::from_utf8_lossy(&reply.value).into_owned())
    }

    /// The focused/active window per `_NET_ACTIVE_WINDOW` (set by EWMH-compliant WMs).
    pub fn active_window(&self) -> Option<Window> {
        let prop = self.atom(b"_NET_ACTIVE_WINDOW")?;
        let reply = self
            .conn
            .get_property(false, self.root, prop, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        reply.value32().and_then(|mut it| it.next()).filter(|&w| w != 0)
    }

    /// Title of the active window, or `None`.
    pub fn active_window_title(&self) -> Option<String> {
        self.window_title(self.active_window()?)
    }

    /// The `_NET_WM_PID` of a window (the process that owns it), if the app set it.
    pub fn window_pid(&self, w: Window) -> Option<u32> {
        let prop = self.atom(b"_NET_WM_PID")?;
        let reply = self
            .conn
            .get_property(false, w, prop, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        reply.value32().and_then(|mut it| it.next())
    }

    /// All top-level client windows per `_NET_CLIENT_LIST` (managed windows, EWMH).
    pub fn client_windows(&self) -> Vec<Window> {
        let Some(prop) = self.atom(b"_NET_CLIENT_LIST") else {
            return Vec::new();
        };
        let Ok(cookie) = self.conn.get_property(false, self.root, prop, AtomEnum::WINDOW, 0, u32::MAX)
        else {
            return Vec::new();
        };
        let Ok(reply) = cookie.reply() else {
            return Vec::new();
        };
        reply.value32().map(|it| it.collect()).unwrap_or_default()
    }

    /// Titles of every managed client window (best-effort; skips windows with no title).
    pub fn all_window_titles(&self) -> Vec<String> {
        self.client_windows()
            .into_iter()
            .filter_map(|w| self.window_title(w))
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// Titles of managed windows owned by `pid` (via `_NET_WM_PID`). Empty if the WM/app doesn't
    /// export pids — callers should fall back to [`all_window_titles`](Self::all_window_titles).
    pub fn window_titles_for_pid(&self, pid: u32) -> Vec<String> {
        self.client_windows()
            .into_iter()
            .filter(|&w| self.window_pid(w) == Some(pid))
            .filter_map(|w| self.window_title(w))
            .filter(|t| !t.is_empty())
            .collect()
    }
}
