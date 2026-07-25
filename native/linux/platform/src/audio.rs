//! Audio-server detection — REAL, pure logic (env + well-known socket paths).
//!
//! The other half of the 2×2 runtime world. Crucial nuance the design hinges on: **`libpulse`
//! works on both PulseAudio and PipeWire**, because PipeWire ships `pipewire-pulse`, a drop-in
//! PulseAudio server. So meeting-tap builds its capture on the Pulse API unconditionally and only
//! needs the *native* PipeWire client for the optional per-app node-link scoping (`--app <id>`).
//!
//! This module reports which server is actually backing the Pulse socket so callers can log it and
//! decide whether native per-app scoping ([`AudioServer::supports_native_app_scoping`]) is even
//! possible. Detection is best-effort from env + socket presence; it never opens a connection here.

use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioServer {
    /// Classic PulseAudio daemon.
    PulseAudio,
    /// PipeWire (with pipewire-pulse providing the Pulse-compatible socket).
    PipeWire,
    /// A Pulse-compatible socket exists but we could not tell which daemon backs it. Treat as
    /// Pulse-capable (capture works) but without confirmed native PipeWire scoping.
    PulseCompatible,
    /// No audio server socket found. meeting-tap will fail to open a monitor and log it.
    None,
}

impl AudioServer {
    /// Native PipeWire node-link scoping (`--app <nodeId>`) is only possible on a real PipeWire
    /// graph. Everywhere else `--app auto|<id>` must degrade to the global monitor and log it.
    pub fn supports_native_app_scoping(self) -> bool {
        matches!(self, AudioServer::PipeWire)
    }

    /// True when a Pulse-compatible capture path (the load-bearing one) is available.
    pub fn has_pulse_capture(self) -> bool {
        !matches!(self, AudioServer::None)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AudioServer::PulseAudio => "pulseaudio",
            AudioServer::PipeWire => "pipewire",
            AudioServer::PulseCompatible => "pulse-compatible",
            AudioServer::None => "none",
        }
    }
}

impl fmt::Display for AudioServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Runtime dir where the audio sockets live (`XDG_RUNTIME_DIR`, default `/run/user/<uid>`).
fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    // SAFETY: getuid is always safe.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}"))
}

/// Detect the backing audio server. Order: a live PipeWire socket alongside the Pulse socket means
/// PipeWire is the real daemon; a Pulse socket with no PipeWire socket means classic PulseAudio.
pub fn detect() -> AudioServer {
    let rd = runtime_dir();
    let pipewire_sock = rd.join("pipewire-0").exists();
    // pipewire-pulse and classic PulseAudio both expose this native-protocol socket.
    let pulse_sock = rd.join("pulse").join("native").exists() || env_pulse_server_set();

    match (pipewire_sock, pulse_sock) {
        (true, _) => AudioServer::PipeWire,
        (false, true) => AudioServer::PulseAudio,
        (false, false) => {
            // Some setups only advertise via env; be conservative.
            if env_pulse_server_set() {
                AudioServer::PulseCompatible
            } else {
                AudioServer::None
            }
        }
    }
}

fn env_pulse_server_set() -> bool {
    std::env::var("PULSE_SERVER").map(|s| !s.is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoping_capabilities() {
        assert!(AudioServer::PipeWire.supports_native_app_scoping());
        assert!(!AudioServer::PulseAudio.supports_native_app_scoping());
        assert!(AudioServer::PulseAudio.has_pulse_capture());
        assert!(AudioServer::PipeWire.has_pulse_capture());
        assert!(!AudioServer::None.has_pulse_capture());
    }

    #[test]
    fn str_labels_stable() {
        assert_eq!(AudioServer::PipeWire.as_str(), "pipewire");
        assert_eq!(AudioServer::PulseAudio.as_str(), "pulseaudio");
    }
}
