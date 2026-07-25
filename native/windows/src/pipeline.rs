//! The utterance pipeline — `Segmenter` → temp WAV → whisper → caption POST. **Fully portable and
//! real**; this is the exact downstream path of the Swift `Segmenter.flush` → `process` →
//! `postCaption`, and the single most important thing to get right against `helpers-core`.
//!
//! A native capture thread feeds 16 kHz-mono blocks into [`SttPipeline::feed`]. When the shared
//! [`Segmenter`](helpers_core::segmenter::Segmenter) closes an utterance on a natural pause, we spawn
//! a short-lived worker (the Swift `procQ`) that: writes the per-utterance temp WAV (16 kHz mono
//! PCM16), transcribes it, drops it below the confidence floor, cleans the text, and POSTs a
//! `caption` to `/ingest`. The temp file is deleted after transcribe.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use helpers_core::epoch_ms;
use helpers_core::protocol::{Caption, CaptionTag};
use helpers_core::segmenter::{Segmenter, Utterance};
use helpers_core::wav::encode_wav16;

use crate::net::{self, WhisperParams};
use crate::text;

/// The server-pulled half of the transcription config; updated live by the config tick.
#[derive(Clone)]
pub struct DynConfig {
    pub language: String,
    pub translate: bool,
    pub vocab: String,
    pub min_conf: f32,
    pub expressions: bool,
}

impl Default for DynConfig {
    fn default() -> Self {
        DynConfig {
            language: "auto".into(),
            translate: false,
            vocab: String::new(),
            min_conf: 0.5,
            expressions: false,
        }
    }
}

/// Immutable, thread-shareable half of the pipeline. Cloned (by `Arc`) into each transcription
/// worker so the capture thread never blocks on whisper.
pub struct Shared {
    pub whisper_url: String,
    pub stt_token: String,
    pub gui_port: String,
    pub gui_token: String,
    pub temp_dir: PathBuf,
    pub dyn_cfg: Arc<Mutex<DynConfig>>,
}

impl Shared {
    fn transcribe_and_post(&self, u: Utterance, speaker: String, is_self: bool, tag: &str) {
        let cfg = self.dyn_cfg.lock().map(|c| c.clone()).unwrap_or_default();
        let secs = u.seconds();
        let wav_bytes = encode_wav16(&u.samples, 16_000, 1);

        // Per-utterance temp WAV (16 kHz mono PCM16), deleted after transcribe — matches PROTOCOL.
        let path = self
            .temp_dir
            .join(format!("meeting-{tag}-{}-{}.wav", u.index, uuid::Uuid::new_v4().simple()));
        let _ = std::fs::write(&path, &wav_bytes);

        let params = WhisperParams {
            language: &cfg.language,
            translate: cfg.translate,
            vocab: &cfg.vocab,
            token: &self.stt_token,
        };
        let result = net::transcribe(&self.whisper_url, &wav_bytes, &params);
        let _ = std::fs::remove_file(&path);

        let Some((raw, conf)) = result else {
            return;
        };
        if conf < cfg.min_conf {
            crate::log(&format!("{tag} seg#{} {secs:.1}s dropped: confidence {conf:.2} < {:.2}", u.index, cfg.min_conf));
            return;
        }
        let clean = if cfg.expressions {
            text::keep_expressions(&raw)
        } else {
            text::clean(&raw)
        };
        if clean.chars().count() <= 1 {
            return;
        }
        crate::log(&format!("{tag} seg#{} {secs:.1}s accepted ({} chars)", u.index, clean.chars().count()));
        let caption = Caption {
            kind: CaptionTag::Caption,
            speaker,
            text: clean,
            is_self,
            ts: epoch_ms(),
        };
        net::post_ingest(&self.gui_port, &self.gui_token, &caption);
    }
}

/// One energy-VAD segmenter bound to one audio source (system or mic). Not thread-safe: use one per
/// source, on that source's capture thread.
pub struct SttPipeline {
    seg: Segmenter,
    tag: String,
    shared: Arc<Shared>,
}

impl SttPipeline {
    pub fn new(seg: Segmenter, tag: impl Into<String>, shared: Arc<Shared>) -> Self {
        SttPipeline {
            seg,
            tag: tag.into(),
            shared,
        }
    }

    /// Feed a block of 16 kHz mono samples. On a completed utterance, hands transcription to a
    /// worker thread so audio capture keeps flowing.
    pub fn feed(&mut self, samples: &[f32]) {
        if let Some(u) = self.seg.feed(samples) {
            let shared = self.shared.clone();
            let speaker = self.seg.speaker.clone();
            let is_self = self.seg.is_self;
            let tag = self.tag.clone();
            std::thread::spawn(move || {
                shared.transcribe_and_post(u, speaker, is_self, &tag);
            });
        }
    }

    /// Reset VAD state between calls (keeps capacity), mirroring the Swift `reset()`.
    pub fn reset(&mut self) {
        self.seg.reset();
    }
}
