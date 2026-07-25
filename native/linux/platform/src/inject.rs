//! Text injection — X11 is REAL (XTEST via enigo); Wayland is the documented degrade. Feature
//! `inject`.
//!
//! `dictation-target` needs to (a) type arbitrary Unicode and (b) send single Backspace/paste
//! keystrokes over whatever field AT-SPI has selected. The mechanism forks hard on the display
//! server:
//!
//! - **X11**: `enigo.text()` synthesizes layout-independent Unicode through the XTEST extension —
//!   the faithful analog of the macOS `CGEvent` + `keyboardSetUnicodeString` path. Real and
//!   complete.
//!
//! - **WAYLAND — HARD BLOCK**: the compositor refuses synthetic global input (no XTEST). There is
//!   no portable "type this string" call. The PRIMARY Wayland path is therefore
//!   **clipboard-set + paste** (Ctrl+V / Shift+Insert). But even the paste keystroke is itself
//!   injected input, so it only works if enigo can reach a permitted backend:
//!     * `libei` (GNOME 45+/KDE) via the RemoteDesktop portal — user consents once, or
//!     * a `uinput` virtual device — requires the user be in the `input` group / a udev rule.
//!   When neither is available the keystroke silently no-ops; we set the clipboard anyway and report
//!   failure so the caller degrades (`liveEditable=false`, `unsafeFallback=true`). All of this is
//!   marked TODO(on-device) because it cannot be verified without a real Wayland session + consent.

use crate::display::DisplayServer;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

pub struct Injector {
    enigo: Enigo,
    server: DisplayServer,
    clipboard_paste: bool,
}

#[derive(Debug)]
pub enum InjectError {
    /// enigo could not initialize a backend (no X server, or no permitted Wayland input path).
    NoBackend(String),
}

impl std::fmt::Display for InjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InjectError::NoBackend(s) => write!(f, "no input-injection backend available: {s}"),
        }
    }
}

impl Injector {
    pub fn new(server: DisplayServer) -> Result<Injector, InjectError> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| InjectError::NoBackend(format!("{e:?}")))?;
        Ok(Injector {
            enigo,
            server,
            // On Wayland the only viable "type" path is clipboard + paste; on X11 we type directly.
            clipboard_paste: matches!(server, DisplayServer::Wayland),
        })
    }

    /// Whether a stable caret / precise per-keystroke editing can be guaranteed. False on Wayland,
    /// where injection is coarse (clipboard paste) — dictation-target keys `liveEditable` off this.
    pub fn can_guarantee_caret(&self) -> bool {
        self.server.can_inject_global_input()
    }

    /// Type `text` at the current caret / over the current selection. Returns whether the input was
    /// dispatched. On Wayland this replaces the selection via clipboard paste.
    pub fn type_text(&mut self, text: &str) -> bool {
        if self.clipboard_paste {
            return self.paste_via_clipboard(text);
        }
        // X11: real Unicode injection through XTEST.
        match self.enigo.text(text) {
            Ok(()) => true,
            Err(e) => {
                eprintln!(">> enigo.text failed: {e:?}");
                false
            }
        }
    }

    /// Send exactly one Backspace (deletes the AT-SPI-selected span in a single press, matching the
    /// Swift `postDelete()`). On Wayland this depends on a permitted injection backend.
    pub fn press_backspace(&mut self) -> bool {
        self.click(Key::Backspace)
    }

    fn click(&mut self, key: Key) -> bool {
        match self.enigo.key(key, Direction::Click) {
            Ok(()) => true,
            Err(e) => {
                eprintln!(">> enigo.key({key:?}) failed: {e:?}");
                false
            }
        }
    }

    /// Wayland primary path: set the clipboard, then paste. Returns true only if the paste keystroke
    /// was accepted (clipboard is always set as a fallback the user can paste manually).
    fn paste_via_clipboard(&mut self, text: &str) -> bool {
        // TODO(on-device): arboard's Wayland backend uses wlr-data-control / the clipboard portal;
        // on some compositors the copied selection is dropped when this short-lived process exits
        // (Wayland clipboard ownership is per-client). If that bites, switch to holding a
        // wl-clipboard-rs `Source` alive across the paste, or shell out to `wl-copy`.
        match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.to_string())) {
            Ok(()) => {}
            Err(e) => {
                eprintln!(">> clipboard set failed: {e:?}");
                return false;
            }
        }
        self.paste()
    }

    /// Ctrl+V. Falls back to Shift+Insert if Ctrl+V is rejected (some terminals).
    pub fn paste(&mut self) -> bool {
        if self.chord(Key::Control, Key::Unicode('v')) {
            return true;
        }
        eprintln!(">> Ctrl+V rejected; trying Shift+Insert");
        self.shift_insert()
    }

    // `Key::Insert` is present on the Linux target (`enigo` gates it to non-macOS unix + windows) but
    // absent on macOS, so gate the fallback to keep the crate cross-compilable for host validation.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn shift_insert(&mut self) -> bool {
        self.chord(Key::Shift, Key::Insert)
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    fn shift_insert(&mut self) -> bool {
        false
    }

    fn chord(&mut self, modifier: Key, key: Key) -> bool {
        // TODO(on-device): on Wayland every one of these key events must traverse libei/uinput; if
        // the backend is absent enigo returns Err and we report false (caller degrades).
        let ok_down = self.enigo.key(modifier, Direction::Press).is_ok();
        let ok_key = self.enigo.key(key, Direction::Click).is_ok();
        let ok_up = self.enigo.key(modifier, Direction::Release).is_ok();
        ok_down && ok_key && ok_up
    }
}
