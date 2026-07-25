//! Control-plane HTTP — the Rust/ureq port of the Swift `postJSON` / `fetchConfig` / `transcribe`
//! paths in `meeting-tap.swift`. **Fully portable and real** (no Windows APIs), so this is part of
//! the verifiable spine.
//!
//! Three jobs:
//! 1. `pull_config` — `GET http://127.0.0.1:<port>/api/state` → [`PulledConfig`] (every ~2 s).
//! 2. `post_ingest` — `POST .../ingest` with `Authorization: Bearer <GUI_TOKEN>` (caption/recording/state).
//! 3. `transcribe`  — `POST <whisperURL>` multipart/form-data of one utterance WAV → (text, meanConf).
//!
//! ureq is blocking; callers run it on the 2 s config tick or on a short-lived per-utterance thread,
//! exactly mirroring the Swift `URLSession` + `procQ` split.

use std::time::Duration;

use helpers_core::protocol::PulledConfig;
use serde::Serialize;

/// Whisper transcription request knobs (the dynamic, server-pulled half).
pub struct WhisperParams<'a> {
    pub language: &'a str, // detection language, default "auto"
    pub translate: bool,   // output translation (off for the shipped Turbo model)
    pub vocab: &'a str,    // optional prompt/vocabulary bias
    pub token: &'a str,    // ASSISTANT_STT_TOKEN (may be empty)
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout(Duration::from_secs(15)) // whisper on a whole utterance can take a few seconds
        .build()
}

/// `GET /api/state`. Returns `None` on any error (unreachable server, bad JSON) so a partial or
/// missing response never breaks the helper — identical to the Swift "keep last config" behaviour.
pub fn pull_config(port: &str, gui_token: &str) -> Option<PulledConfig> {
    let url = format!("http://127.0.0.1:{port}/api/state");
    let mut req = agent().get(&url);
    if !gui_token.is_empty() {
        req = req.set("Authorization", &format!("Bearer {gui_token}"));
    }
    match req.call() {
        Ok(resp) => resp.into_json::<PulledConfig>().ok(),
        Err(_) => None,
    }
}

/// `POST /ingest` with a JSON body and the GUI bearer token. Fire-and-forget: errors are logged, not
/// propagated (the Swift `dataTask(...).resume()` is likewise best-effort).
pub fn post_ingest<T: Serialize>(port: &str, gui_token: &str, body: &T) {
    let url = format!("http://127.0.0.1:{port}/ingest");
    let mut req = agent()
        .post(&url)
        .set("Content-Type", "application/json");
    if !gui_token.is_empty() {
        req = req.set("Authorization", &format!("Bearer {gui_token}"));
    }
    if let Err(e) = req.send_json(serde_json::to_value(body).unwrap_or(serde_json::Value::Null)) {
        // A transient loopback error is normal during startup/shutdown; don't spam.
        crate::log(&format!("ingest POST failed: {e}"));
    }
}

/// Transcribe one 16 kHz-mono WAV. Returns `(text, meanWordConfidence)` on success.
///
/// Mirrors the Swift `transcribe`: multipart form with `file`, `temperature=0`,
/// `response_format=verbose_json`, `language`, optional `translate=true`, optional `prompt=<vocab>`.
/// Confidence is the mean of `segments[].words[].probability`; no word probs ⇒ 1.0 (don't penalise).
pub fn transcribe(whisper_url: &str, wav: &[u8], p: &WhisperParams) -> Option<(String, f32)> {
    let boundary = format!("----assistant{}", uuid::Uuid::new_v4().simple());
    let mut fields: Vec<(String, String)> = vec![
        ("temperature".into(), "0".into()),
        ("response_format".into(), "verbose_json".into()),
        ("language".into(), p.language.to_string()),
    ];
    if p.translate {
        fields.push(("translate".into(), "true".into()));
    }
    if !p.vocab.is_empty() {
        fields.push(("prompt".into(), p.vocab.to_string()));
    }
    let body = multipart_body(&boundary, &fields, "file", "audio.wav", "audio/wav", wav);

    let mut req = agent()
        .post(whisper_url)
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        );
    if !p.token.is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", p.token));
    }
    let text = match req.send_bytes(&body) {
        Ok(resp) => resp.into_string().ok()?,
        Err(e) => {
            crate::log(&format!("whisper POST failed: {e}"));
            return None;
        }
    };
    parse_whisper(&text)
}

/// Parse a whisper `verbose_json` response body into `(text, meanConfidence)`.
pub fn parse_whisper(body: &str) -> Option<(String, f32)> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let mut probs: Vec<f64> = Vec::new();
    if let Some(segs) = v.get("segments").and_then(|s| s.as_array()) {
        for seg in segs {
            if let Some(words) = seg.get("words").and_then(|w| w.as_array()) {
                for w in words {
                    if let Some(p) = w.get("probability").and_then(|p| p.as_f64()) {
                        probs.push(p);
                    }
                }
            }
        }
    }
    let conf = if probs.is_empty() {
        1.0
    } else {
        (probs.iter().sum::<f64>() / probs.len() as f64) as f32
    };
    Some((text, conf))
}

/// Build a `multipart/form-data` body: text fields, then one binary file part.
fn multipart_body(
    boundary: &str,
    fields: &[(String, String)],
    file_field: &str,
    filename: &str,
    file_ct: &str,
    file_bytes: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(file_bytes.len() + 512);
    let mut push = |s: &str| out.extend_from_slice(s.as_bytes());
    for (name, value) in fields {
        push(&format!("--{boundary}\r\n"));
        push(&format!(
            "Content-Disposition: form-data; name=\"{name}\"\r\n\r\n"
        ));
        push(&format!("{value}\r\n"));
    }
    push(&format!("--{boundary}\r\n"));
    push(&format!(
        "Content-Disposition: form-data; name=\"{file_field}\"; filename=\"{filename}\"\r\n"
    ));
    push(&format!("Content-Type: {file_ct}\r\n\r\n"));
    out.extend_from_slice(file_bytes);
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_mean_confidence() {
        let body = r#"{"text":"hello world","segments":[{"words":[
            {"word":"hello","probability":0.9},{"word":"world","probability":0.7}]}]}"#;
        let (t, c) = parse_whisper(body).unwrap();
        assert_eq!(t, "hello world");
        assert!((c - 0.8).abs() < 1e-5);
    }

    #[test]
    fn whisper_no_word_probs_is_full_confidence() {
        let (_t, c) = parse_whisper(r#"{"text":"hi"}"#).unwrap();
        assert_eq!(c, 1.0);
    }

    #[test]
    fn multipart_has_boundaries_and_file() {
        let f = vec![("temperature".to_string(), "0".to_string())];
        let body = multipart_body("BND", &f, "file", "a.wav", "audio/wav", b"RIFF");
        let s = String::from_utf8_lossy(&body);
        assert!(s.contains("--BND\r\n"));
        assert!(s.contains("name=\"temperature\""));
        assert!(s.contains("filename=\"a.wav\""));
        assert!(s.contains("--BND--\r\n"));
        assert!(s.contains("RIFF"));
    }
}
