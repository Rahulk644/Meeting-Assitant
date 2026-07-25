//! Audio capture — REAL libpulse-simple record sequence; runtime result is TODO(on-device).
//!
//! The design's load-bearing choice: request S16LE / mono / 16 kHz from Pulse and let the server
//! resample (PipeWire's pipewire-pulse does this too), so NO manual resampler is needed — samples
//! arrive ready for `helpers_core::segmenter`. This module opens a blocking `Simple` record stream
//! and feeds each block to a `Segmenter`, forwarding completed utterances over a channel. The code
//! is real and compiles against libpulse-simple-binding; it needs a live Pulse/PipeWire server to
//! actually produce samples, and resolving the *default sink monitor* name needs the introspection
//! API — both marked TODO(on-device). On any failure the thread logs a degrade and exits cleanly
//! (the helper stays well-formed; it just produces no captions until audio is wired).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use helpers_core::segmenter::{Segmenter, Utterance};

/// One completed utterance tagged with its speaker identity, sent to the processing thread.
pub struct Captured {
    pub speaker: String,
    pub is_self: bool,
    pub tag: String,
    pub utt: Utterance,
}

/// Resolve the default sink's monitor source name (what "system audio" records from).
///
/// TODO(on-device): the robust way is the Pulse introspection API — connect a threaded mainloop,
/// `get_server_info` → `default_sink_name`, then `get_sink_info_by_name` → `monitor_source_name`.
/// Some servers also accept the special device `@DEFAULT_MONITOR@`. Returning `None` here makes the
/// Simple stream fall back to the default *source* (usually the mic), which is wrong for system
/// audio — so this MUST be completed on-device; until then system capture logs a degrade.
pub fn default_monitor_device() -> Option<String> {
    // TODO(on-device): introspect the default sink monitor. See doc above.
    None
}

/// Spawn a blocking Pulse record thread. `device = None` uses the default source (correct for the
/// mic). Feeds `seg`; sends each `Utterance` (tagged) on `tx`. Stops when `running` clears.
pub fn spawn_capture(
    device: Option<String>,
    mut seg: Segmenter,
    tx: Sender<Captured>,
    running: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use libpulse_binding::sample::{Format, Spec};
        use libpulse_binding::stream::Direction;
        use libpulse_simple_binding::Simple;

        let spec = Spec {
            format: Format::S16le,
            channels: 1,
            rate: 16000,
        };
        if !spec.is_valid() {
            eprintln!(">> [{}] invalid pulse sample spec; audio disabled", seg.tag);
            return;
        }

        // TODO(on-device): needs a live Pulse/PipeWire server. On failure we degrade to silence.
        let simple = match Simple::new(
            None,                 // default server
            "meeting-tap",        // application name
            Direction::Record,
            device.as_deref(),    // None = default source; Some(monitor) = system audio
            &seg.tag,             // stream description
            &spec,
            None,                 // default channel map
            None,                 // default buffering
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    ">> [{}] pulse record stream unavailable ({e:?}); this source is silent (TODO on-device)",
                    seg.tag
                );
                return;
            }
        };

        // 0.1 s blocks: 1600 samples * 2 bytes.
        const SAMPLES_PER_BLOCK: usize = 1600;
        let mut bytes = vec![0u8; SAMPLES_PER_BLOCK * 2];
        let mut floats = vec![0f32; SAMPLES_PER_BLOCK];

        while running.load(Ordering::Relaxed) {
            if let Err(e) = simple.read(&mut bytes) {
                eprintln!(">> [{}] pulse read error ({e:?}); stopping this source", seg.tag);
                return;
            }
            for (i, chunk) in bytes.chunks_exact(2).enumerate() {
                let s = i16::from_le_bytes([chunk[0], chunk[1]]);
                floats[i] = s as f32 / 32768.0;
            }
            if let Some(utt) = seg.feed(&floats) {
                let captured = Captured {
                    speaker: seg.speaker.clone(),
                    is_self: seg.is_self,
                    tag: seg.tag.clone(),
                    utt,
                };
                if tx.send(captured).is_err() {
                    return; // receiver gone → shut down
                }
            }
        }
    })
}

/// List capturable applications for `--list-apps`: `nodeId\tapplicationName` per line.
///
/// TODO(on-device): enumerate audio-output-capable nodes. On PipeWire, walk the graph for nodes with
/// `media.class = "Stream/Output/Audio"` and print `node.id \t application.name` (feature
/// `pipewire-scope`). On classic PulseAudio, list sink-inputs via introspection (`get_sink_input_info_list`)
/// and print `index \t application.name`. Until wired, print the always-available global monitor row.
pub fn list_apps() {
    // The robust global option is always present.
    println!("all\tSystem audio (default sink monitor)");
    eprintln!(">> per-app enumeration is TODO(on-device); only the global 'all' target is listed");
}
