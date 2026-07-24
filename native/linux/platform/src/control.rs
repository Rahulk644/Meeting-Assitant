//! Loopback HTTP control plane + whisper upload — REAL `reqwest` (blocking). Feature `control`.
//!
//! This is the portable spine meeting-tap POSTs through, and it is byte-for-byte the same contract
//! as the macOS Swift helper (`native/PROTOCOL.md` §1). Everything here is verifiable without a
//! Linux desktop — the only Linux-specific parts of meeting-tap are audio/video capture, which feed
//! into `helpers_core::segmenter` → [`ControlPlane::transcribe`] → [`ControlPlane::post_caption`].
//!
//! Endpoints (all loopback, no TLS backend compiled in):
//! - `GET  http://127.0.0.1:<guiPort>/api/state`  → [`helpers_core::protocol::PulledConfig`]
//! - `POST http://127.0.0.1:<guiPort>/ingest`      ← caption / recording / state JSON
//! - `POST <whisperURL>` (multipart)               → transcript + mean-word-probability confidence
//!
//! `PulledConfig::manual` is the Swift tri-state string (`"on"|"auto"|"off"|"paused"`), read via the
//! `manual_force_on()` / `manual_force_off()` / `manual_is_auto()` helpers on `PulledConfig`. (An
//! earlier revision typed it as `bool`, which silently failed the whole config deserialize on every
//! poll — see meeting-tap's gate for the corrected tri-state handling.)

use std::time::Duration;

use helpers_core::protocol::{AudioStateEvent, Caption, PulledConfig, RecordingStarted};
use serde::Deserialize;

/// A configured client bound to one GUI port + token. Cheap to clone (wraps an `Arc` client).
pub struct ControlPlane {
    client: reqwest::blocking::Client,
    api_state_url: String,
    ingest_url: String,
    gui_token: String,
}

impl ControlPlane {
    /// `gui_port` is kept as a string to mirror the Swift (which never parses it to an int).
    pub fn new(gui_port: &str, gui_token: impl Into<String>) -> Self {
        let client = reqwest::blocking::Client::builder()
            .build()
            .expect("reqwest client builds with no TLS backend and default settings");
        ControlPlane {
            client,
            api_state_url: format!("http://127.0.0.1:{gui_port}/api/state"),
            ingest_url: format!("http://127.0.0.1:{gui_port}/ingest"),
            gui_token: gui_token.into(),
        }
    }

    fn bearer(&self, rb: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        if self.gui_token.is_empty() {
            rb
        } else {
            rb.header("Authorization", format!("Bearer {}", self.gui_token))
        }
    }

    /// `GET /api/state` with a 2 s timeout (the Swift `URLSession` timeout). `None` on any error,
    /// so a transient GUI outage just skips one config tick.
    pub fn pull_config(&self) -> Option<PulledConfig> {
        let req = self
            .bearer(self.client.get(&self.api_state_url))
            .timeout(Duration::from_secs(2));
        let resp = req.send().ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json::<PulledConfig>().ok()
    }

    /// `POST /ingest` a caption. Errors are swallowed (best-effort like the Swift fire-and-forget).
    pub fn post_caption(&self, caption: &Caption) {
        self.post_ingest(caption);
    }

    pub fn post_recording(&self, rec: &RecordingStarted) {
        self.post_ingest(rec);
    }

    pub fn post_state(&self, ev: &AudioStateEvent) {
        self.post_ingest(ev);
    }

    fn post_ingest<T: serde::Serialize>(&self, body: &T) {
        let req = self
            .bearer(self.client.post(&self.ingest_url))
            .timeout(Duration::from_secs(4))
            .json(body);
        // Fire-and-forget: log on failure, never block the audio path.
        if let Err(e) = req.send() {
            eprintln!(">> ingest POST failed: {e}");
        }
    }

    /// Multipart upload to the whisper server. Mirrors the Swift `curl` form exactly:
    /// `file`, `temperature=0`, `response_format=verbose_json`, `language`, and — only when set —
    /// `translate=true` and `prompt=<vocab>`. Returns the transcript text + mean word probability
    /// (confidence), or `None` on transport/parse failure.
    pub fn transcribe(
        &self,
        whisper_url: &str,
        stt_token: &str,
        wav_bytes: Vec<u8>,
        filename: &str,
        language: &str,
        translate: bool,
        vocab: &str,
    ) -> Option<Transcription> {
        let part = reqwest::blocking::multipart::Part::bytes(wav_bytes)
            .file_name(filename.to_string())
            .mime_str("audio/wav")
            .ok()?;
        let mut form = reqwest::blocking::multipart::Form::new()
            .part("file", part)
            .text("temperature", "0")
            .text("response_format", "verbose_json")
            .text("language", language.to_string());
        if translate {
            form = form.text("translate", "true");
        }
        if !vocab.is_empty() {
            form = form.text("prompt", vocab.to_string());
        }

        let mut rb = self
            .client
            .post(whisper_url)
            .timeout(Duration::from_secs(120))
            .multipart(form);
        if !stt_token.is_empty() {
            rb = rb.header("Authorization", format!("Bearer {stt_token}"));
        }
        let resp = rb.send().ok()?;
        if !resp.status().is_success() {
            eprintln!(">> whisper returned HTTP {}", resp.status());
            return None;
        }
        let parsed: WhisperResponse = resp.json().ok()?;
        Some(Transcription::from(parsed))
    }
}

/// A completed transcription: the full text and a mean-word-probability confidence in `[0,1]`.
#[derive(Debug, Clone)]
pub struct Transcription {
    pub text: String,
    pub confidence: f64,
}

impl From<WhisperResponse> for Transcription {
    fn from(r: WhisperResponse) -> Self {
        // Confidence = mean of every segment word probability. No word probs ⇒ 1.0 (don't penalize),
        // exactly as the Swift does.
        let mut probs: Vec<f64> = Vec::new();
        for seg in &r.segments {
            for w in &seg.words {
                if let Some(p) = w.probability {
                    probs.push(p);
                }
            }
        }
        let confidence = if probs.is_empty() {
            1.0
        } else {
            probs.iter().sum::<f64>() / probs.len() as f64
        };
        Transcription {
            text: r.text.unwrap_or_default(),
            confidence,
        }
    }
}

// whisper.cpp / faster-whisper `verbose_json` shape (only the fields we need).
#[derive(Debug, Deserialize)]
pub struct WhisperResponse {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub segments: Vec<WhisperSegment>,
}

#[derive(Debug, Deserialize)]
pub struct WhisperSegment {
    #[serde(default)]
    pub words: Vec<WhisperWord>,
}

#[derive(Debug, Deserialize)]
pub struct WhisperWord {
    #[serde(default)]
    pub probability: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_is_mean_word_probability() {
        let json = r#"{"text":"hello world","segments":[
            {"words":[{"probability":0.9},{"probability":0.7}]},
            {"words":[{"probability":0.8}]}]}"#;
        let r: WhisperResponse = serde_json::from_str(json).unwrap();
        let t = Transcription::from(r);
        assert_eq!(t.text, "hello world");
        assert!((t.confidence - 0.8).abs() < 1e-9);
    }

    #[test]
    fn no_word_probs_is_full_confidence() {
        let json = r#"{"text":"x","segments":[{"words":[]}]}"#;
        let r: WhisperResponse = serde_json::from_str(json).unwrap();
        assert_eq!(Transcription::from(r).confidence, 1.0);
    }

    #[test]
    fn urls_built_from_port() {
        let cp = ControlPlane::new("8787", "");
        assert_eq!(cp.api_state_url, "http://127.0.0.1:8787/api/state");
        assert_eq!(cp.ingest_url, "http://127.0.0.1:8787/ingest");
    }
}
