//! # helpers-core
//!
//! The OS-agnostic heart shared by the Windows and Linux native helper binaries. Everything in
//! here is pure logic (or a thin, target-gated syscall) and behaves identically on every platform,
//! so the per-OS crates only implement the genuinely platform-specific pieces: audio capture, text
//! injection, and accessibility reads.
//!
//! The wire contract these types encode is documented in [`native/PROTOCOL.md`](../../PROTOCOL.md)
//! and was reverse-engineered from the macOS Swift helpers plus the Rust core that spawns them.
//!
//! Modules:
//! - [`protocol`] — every JSON struct on the wire (camelCase, verbatim field names).
//! - [`args`]     — the `opt(name, def)` + positional argv conventions all helpers share.
//! - [`watchdog`] — `ASSISTANT_PARENT_PID` liveness (`kill(pid,0)` / `OpenProcess`).
//! - [`segmenter`]— energy-VAD utterance segmentation (the load-bearing shared audio logic).
//! - [`wav`]      — 16-bit PCM WAV writer, byte-identical to the Swift `writeWav16`.
//! - [`fingerprint`] — SHA-256 content/state fingerprints used by agent-context / agent-action.

pub mod args;
pub mod fingerprint;
pub mod protocol;
pub mod segmenter;
pub mod wav;
pub mod watchdog;

/// Epoch-milliseconds, the timestamp unit used everywhere on the wire.
pub fn epoch_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
