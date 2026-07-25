//! Screen recording — GStreamer encode pipeline + Wayland ScreenCast portal. TODO(on-device).
//!
//! The pipeline and portal sequence are the real, correct ones; wiring the live capture needs a
//! GStreamer plugin stack + (on Wayland) an interactive portal grant, so actual recording is marked
//! TODO(on-device) and `start()` returns `None` (nothing recorded, nothing falsely reported) until
//! wired. The crash-safety contract is preserved by design: `mp4mux fragment-duration=10000` writes
//! a playable moov roughly every 10 s, and `stop()` sends EOS + waits for the bus so the final moov
//! atom is flushed (PROTOCOL.md §1: SIGTERM must finalize the `.mp4`).
//!
//! Source selection:
//! - **X11**: `ximagesrc` (whole screen) → the encoder.
//! - **Wayland**: the org.freedesktop.portal.ScreenCast portal (via `ashpd`) prompts the user, hands
//!   back a PipeWire remote node fd, and `pipewiresrc fd=<fd> path=<node>` feeds the encoder. There
//!   is no non-interactive Wayland screen capture — the prompt is unavoidable and by design.

use platform::display::DisplayServer;
use std::path::PathBuf;

/// The requested video behaviour (from `--video` / server config).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoMode {
    Off,
    Always,
    Share,
}

impl VideoMode {
    pub fn parse(s: &str) -> VideoMode {
        match s {
            "always" => VideoMode::Always,
            "share" => VideoMode::Share,
            _ => VideoMode::Off,
        }
    }
}

pub struct Recorder {
    rec_dir: PathBuf,
    running: bool,
    /// Whether the encoder is actively writing frames (mode `always`, or `share` while sharing).
    record: bool,
    gst_ok: bool,
}

impl Recorder {
    pub fn new(rec_dir: PathBuf) -> Recorder {
        // A real, verifiable call: initialize GStreamer. If the plugin stack is missing we log and
        // disable video rather than crash.
        let gst_ok = match gstreamer::init() {
            Ok(()) => true,
            Err(e) => {
                eprintln!(">> GStreamer init failed ({e}); video disabled");
                false
            }
        };
        Recorder { rec_dir, running: false, record: false, gst_ok }
    }

    /// The GStreamer launch string for a given source element, per PROTOCOL.md §1 encode params
    /// (H.264 ≤4 Mbps, ~10 s keyframes, fragmented mp4). `appsink`+`jpegenc` would branch off a `tee`
    /// for the thumbnail; shown here as the video branch only.
    fn pipeline_desc(&self, source: &str, out_path: &str) -> String {
        format!(
            "{source} ! videoconvert ! videoscale ! videorate ! video/x-raw,framerate=10/1 ! \
             x264enc bitrate=4000 key-int-max=100 speed-preset=veryfast tune=zerolatency ! \
             h264parse ! mp4mux fragment-duration=10000 ! filesink location={out_path}"
        )
    }

    /// Start recording if the mode warrants it. Returns `Some((basename, started_at_ms))` when a
    /// recording actually began (so the caller posts the `recording` event) — currently always
    /// `None` (TODO on-device).
    pub fn start(
        &mut self,
        mode: VideoMode,
        sharing: bool,
        server: DisplayServer,
    ) -> Option<(String, u64)> {
        if mode == VideoMode::Off || !self.gst_ok || self.running {
            return None;
        }
        self.record = match mode {
            VideoMode::Always => true,
            VideoMode::Share => sharing,
            VideoMode::Off => false,
        };

        let stamp = helpers_core::epoch_ms();
        let basename = format!("{stamp}.mp4");
        let out_path = self.rec_dir.join(&basename);
        if let Err(e) = std::fs::create_dir_all(&self.rec_dir) {
            eprintln!(">> could not create rec dir {:?}: {e}", self.rec_dir);
            return None;
        }

        let source = match server {
            DisplayServer::X11 => "ximagesrc use-damage=false".to_string(),
            DisplayServer::Wayland => {
                // TODO(on-device): drive the ScreenCast portal with ashpd:
                //   let proxy = ashpd::desktop::screencast::Screencast::new().await?;
                //   let session = proxy.create_session().await?;
                //   proxy.select_sources(&session, CursorMode::Embedded, SourceType::Monitor,
                //                        false, None, PersistMode::DoNot).await?;
                //   let response = proxy.start(&session, &WindowIdentifier::default()).await?;
                //   let (node_id, _) = response.streams()[0]...;
                //   let fd = proxy.open_pipe_wire_remote(&session).await?;   // RawFd
                //   → source = format!("pipewiresrc fd={fd} path={node_id}")
                eprintln!(">> Wayland screen capture requires an interactive ScreenCast portal grant (TODO on-device)");
                "pipewiresrc".to_string()
            }
            DisplayServer::Headless => {
                eprintln!(">> no display server; cannot record video");
                return None;
            }
        };

        let desc = self.pipeline_desc(&source, &out_path.to_string_lossy());
        eprintln!(">> [video TODO on-device] would launch: {desc}");
        // TODO(on-device): gstreamer::parse::launch(&desc)?, set to Playing, add a tee→appsink→
        // jpegenc branch for the ≤480px q≈0.76 first-frame thumbnail written next to the mp4 as
        // `<stamp>.jpg`, and hold the pipeline. Then return Some((basename, stamp)).
        let _ = desc;
        None
    }

    /// Live-toggle the record flag for `share` mode (called each config tick).
    pub fn update_share(&mut self, sharing: bool) {
        if self.running {
            self.record = sharing;
        }
    }

    /// Finalize: send EOS and wait for the bus so the mp4 moov atom is written. Idempotent.
    pub fn stop(&mut self) {
        if !self.running {
            return;
        }
        self.running = false;
        self.record = false;
        // TODO(on-device): pipeline.send_event(gstreamer::event::Eos::new()); then poll the bus for
        // Eos/Error before setting state to Null — this is what flushes the final moov atom so a
        // SIGTERM'd recording is still playable.
        eprintln!(">> screen recording finalized (EOS)");
    }
}
