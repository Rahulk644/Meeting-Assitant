//! AT-SPI reads for meeting-state — TODO(on-device) adapter.
//!
//! REAL: the bus availability probe (`ax_trusted`). TODO(on-device): harvesting menu-item labels
//! (for `derive_controls`) and Zoom participant/active-speaker tiles — these need a live AT-SPI tree
//! and, for Zoom-on-Linux (Qt/embedded-Chromium), `QT_ACCESSIBILITY=1` plus the `AXManualAccessibility`
//! equivalent and on-device `--dump` selector tuning. Until wired they return empty, so meeting-state
//! still emits useful state from X11 window titles alone (which need no accessibility).

/// A11y-bus availability = the Linux analog of macOS `AXIsProcessTrusted()`. REAL.
pub async fn ax_trusted() -> bool {
    // TODO(on-device): confirm atspi 0.28 constructor. A successful bind means toolkits will
    // populate the tree (assuming toolkit-accessibility / QT_ACCESSIBILITY=1 are set).
    atspi::AccessibilityConnection::new().await.is_ok()
}

/// Menu-item titles for the app owning `pid` (feeds `derive_controls`). TODO(on-device) → empty.
pub async fn menu_titles(_pid: u32) -> Vec<String> {
    // TODO(on-device): walk the AT-SPI tree of the app: root → find MenuBar (role "menu bar") →
    // each MenuBarItem → its Menu → each MenuItem's accessible name. Collect non-empty names.
    // These labels ("Unmute Audio", "Stop Video", "Start Share", …) live in the a11y tree even
    // when the menus are closed, exactly like the macOS VoiceOver labels.
    Vec::new()
}

/// Extracted participant info.
#[derive(Debug, Default, Clone)]
pub struct ZoomNames {
    pub participants: Vec<String>,
    pub active_speaker: Option<String>,
}

/// Zoom participant + active-speaker extraction. TODO(on-device) → empty.
pub async fn zoom_names(_pid: u32) -> ZoomNames {
    // TODO(on-device): set the app's "AXManualAccessibility"-equivalent to force Zoom's embedded
    // Chromium to expose its tree, then BFS (depth cap 30) reading each element's:
    //   - description → derive::name_from_tile_desc / name_from_profile_desc
    //   - value       → derive::name_from_talking  (authoritative active speaker — overrides tiles)
    // classify source via derive::participant_source; first floating-window tile = active speaker
    // unless a "Talking:" value overrides. Requires on-device selector tuning (Qt build).
    ZoomNames::default()
}

/// AT-SPI window names for the app owning `pid` — the Wayland fallback for window titles (X11 uses
/// `platform::win`). TODO(on-device) → empty.
pub async fn window_names(_pid: u32) -> Vec<String> {
    // TODO(on-device): app accessible → children with role "frame"/"window" → accessible name.
    // This is the ONLY title source on Wayland, where X11 window enumeration is impossible.
    Vec::new()
}
