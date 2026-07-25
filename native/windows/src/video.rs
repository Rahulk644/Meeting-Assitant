//! Screen recording — H.264 `.mp4` (+ first-frame `.jpg` thumbnail) via the `windows-capture`
//! crate, the Windows analogue of the Swift ScreenCaptureKit + AVAssetWriter path.
//!
//! **Status: TODO(on-device).** The `windows-capture` 2.x `GraphicsCaptureApiHandler` / `Settings`
//! / `VideoEncoder` surface cannot be verified without a Windows toolchain, and getting the moov
//! atom finalised correctly on console-ctrl / stdin-EOF is the whole point (a truncated mp4 is
//! useless). Rather than emit confident-but-unverified capture code, [`Recorder::start`] currently
//! returns `None` (no video), which is fully contract-valid: video is optional and `meeting-tap`
//! simply announces no `recording` event. The intended real sequence is documented inline so it can
//! be finished on a device.

/// A live screen recording. When `start` returns `Some`, the caller announces
/// `{"type":"recording","file":...}` and calls [`Recorder::finish`] on stop to write the moov atom.
pub struct Recorder {
    pub file: String,      // "<epochMs>.mp4" — safe basename only
    pub started_at: u64,   // epoch ms
    #[allow(dead_code)]
    rec_dir: String,
}

impl Recorder {
    /// Begin recording into `<rec_dir>/<epochMs>.mp4`. Returns `None` when video is unavailable
    /// (current default) or init fails.
    pub fn start(rec_dir: &str) -> Option<Recorder> {
        let _ = std::fs::create_dir_all(rec_dir);
        let started_at = helpers_core::epoch_ms();
        let file = format!("{started_at}.mp4");

        // TODO(on-device): wire windows-capture. Intended sequence (windows-capture 2.x):
        //   use windows_capture::{capture::GraphicsCaptureApiHandler, settings::{Settings, ...},
        //       encoder::{VideoEncoder, VideoSettingsBuilder, ...}, monitor::Monitor};
        //   - Monitor::primary() as the capture item.
        //   - VideoEncoder with H.264, ~4 Mbps average bitrate, 10 fps, keyframe every ~10 s.
        //   - A handler that, on each frame, appends to the encoder and, on the FIRST frame, writes a
        //     downscaled (≤480 px) JPEG sibling "<epochMs>.jpg" at q≈0.76.
        //   - On stop (console-ctrl / stdin-EOF), call encoder.finish() so the moov atom is written —
        //     this MUST run even on abrupt shutdown, which is why meeting-tap traps SetConsoleCtrlHandler
        //     AND stdin-EOF and calls Capture::stop() → Recorder::finish() before exit.
        // Returning None keeps the binary well-formed with audio-only capture until this is verified.
        #[cfg(windows)]
        {
            crate::log(&format!(
                ">> video recording requested → {file} — TODO(on-device): windows-capture wiring; audio-only for now"
            ));
        }
        #[cfg(not(windows))]
        {
            crate::degrade("windows-capture video recording");
        }
        let _ = (file, started_at, rec_dir);
        None
    }

    /// Finalise the mp4 (write the moov atom). Consumes the recorder.
    pub fn finish(self) {
        // TODO(on-device): call the windows-capture encoder's finish()/stop() here.
        crate::log(&format!(">> finalizing recording {} (TODO on-device)", self.file));
    }
}
