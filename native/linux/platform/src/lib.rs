//! # platform — Linux runtime detection + shared helper spine
//!
//! REAL (verifiable without a Linux desktop):
//! - [`display`] — X11 vs Wayland detection from `XDG_SESSION_TYPE` / `WAYLAND_DISPLAY` /
//!   `DISPLAY`. Pure env logic, unit-tested.
//! - [`audio`] — PulseAudio vs PipeWire probe from env + well-known socket paths. Pure logic.
//! - [`roles`] — AT-SPI role string → the `AX*` tokens the core expects. Pure table, unit-tested.
//! - [`control`] (feature `control`) — the loopback HTTP control-plane client (pull config, POST
//!   caption / recording / state, whisper multipart upload). Real `reqwest` calls; the *shapes* are
//!   frozen by `helpers_core::protocol` and `native/PROTOCOL.md`.
//! - [`proc`] (feature `procenum`) — meeting-app process enumeration via `sysinfo`. Real.
//!
//! TODO(on-device) surfaces are confined to [`inject`] (Wayland injection needs a real
//! libei/portal/uinput session) and [`win`] (X11 property reads need a live X server). Both are
//! written with the correct API sequence and always return a contract-valid fallback.
//!
//! THE CRUX is the 2×2 runtime world: {X11, Wayland} × {PulseAudio, PipeWire}. Every helper probes
//! it at startup via [`display::detect`] + [`audio::detect`] and branches. Wayland's hard blocks
//! (no global input injection, no window enumeration) are called out at every site that hits them.

pub mod audio;
pub mod display;
pub mod roles;
pub mod text;

#[cfg(feature = "control")]
pub mod control;

#[cfg(feature = "procenum")]
pub mod proc;

#[cfg(feature = "x11")]
pub mod win;

#[cfg(feature = "inject")]
pub mod inject;

/// The best-guess local user display name, used by meeting-tap as the "self"/mic speaker label
/// (the Linux analog of macOS `NSFullUserName()`). Falls back to `$USER`, then `"Me"`.
pub fn full_user_name() -> String {
    // getpwuid(getuid()).pw_gecos is the classic "full name" field; fall back to the login name.
    if let Some(name) = passwd_gecos() {
        let trimmed = name.split(',').next().unwrap_or("").trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    std::env::var("USER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Me".to_string())
}

fn passwd_gecos() -> Option<String> {
    // SAFETY: getpwuid returns a pointer into a static buffer; we copy the C string out
    // immediately and never retain the pointer. getuid() is always safe.
    unsafe {
        let uid = libc::getuid();
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return None;
        }
        let gecos = (*pw).pw_gecos;
        if gecos.is_null() {
            return None;
        }
        let c = std::ffi::CStr::from_ptr(gecos);
        Some(c.to_string_lossy().into_owned())
    }
}
