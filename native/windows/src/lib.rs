//! # platform — the Windows native-helper support library
//!
//! Shared code for the five Windows helper binaries (`meeting-tap`, `dictation-target`,
//! `agent-context`, `agent-action`, `meeting-state`). It sits on top of the frozen `helpers-core`
//! crate and adds the two things `helpers-core` deliberately leaves to each OS: the HTTP/text
//! plumbing that is portable, and the genuinely platform-specific native calls.
//!
//! ## What is REAL vs TODO(on-device)
//!
//! REAL and verifiable on any host (this is the load-bearing spine):
//! - [`net`]      — ureq control-plane client: config pull, `/ingest` POST, whisper multipart.
//! - [`resample`] — stereo→mono downmix + arbitrary-rate → 16 kHz linear resampler.
//! - [`text`]     — transcript cleaning / expression-keeping (ports the Swift regex logic).
//! - [`pipeline`] — `Segmenter` → wav → whisper → caption. The Swift `Segmenter.process` path.
//! - [`apps`]     — meeting-app exe tables + evidence classification (drives `--selftest`).
//! - [`meetingstate`] — control-label derivation + participant-name filtering (drives `--selftest`).
//! - [`shutdown`] — stdin-EOF stop watcher (portable) + `SetConsoleCtrlHandler` (Windows).
//!
//! REAL native call sequences written against the documented crate/COM APIs, but **unverifiable
//! without a Windows box** — each such function is marked `// TODO(on-device):` at the exact spot
//! that needs hardware to confirm, and every one has a `#[cfg(not(windows))]` sibling that logs a
//! degrade and returns a contract-valid fallback (`None`, empty snapshot, `no_window`, …):
//! - [`procs`]    — `CreateToolhelp32Snapshot` process enumeration + pid→exe.
//! - [`window`]   — `EnumWindows`, foreground window, `SetForegroundWindow`+`AttachThreadInput`.
//! - [`audio`]    — WASAPI Audio Session detection, process-loopback + mic capture.
//! - [`uia`]      — UI Automation snapshot / focused-field classify / meeting controls (CacheRequest).
//! - [`inject`]   — enigo Unicode injection + a raw `SendInput` UIPI probe.
//! - [`action`]   — the guarded `agent-action` apply/undo mutation flow.
//! - [`dictation`]— the `--session` preview/commit/cancel state machine.
//! - [`video`]    — `windows-capture` H.264 mp4 recorder + first-frame thumbnail.

pub mod action;
pub mod apps;
pub mod audio;
pub mod dictation;
pub mod inject;
pub mod meetingstate;
pub mod net;
pub mod pipeline;
pub mod procs;
pub mod resample;
pub mod shutdown;
pub mod text;
pub mod uia;
pub mod video;
pub mod window;

/// One-line stderr diagnostic. stderr is human logs only; the core surfaces it only on non-zero exit
/// for the one-shot helpers (see PROTOCOL.md). Keep it terse.
pub fn log(msg: &str) {
    eprintln!("{msg}");
}

/// Log a native-capability degrade exactly once-ish (best effort; no global dedupe). Used by every
/// `#[cfg(not(windows))]` fallback so a host build is honest about what it cannot do.
pub fn degrade(what: &str) {
    eprintln!(">> degrade: {what} unavailable on this host (needs x86_64-pc-windows-msvc)");
}
