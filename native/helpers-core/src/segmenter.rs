//! Energy-VAD utterance segmenter — a faithful port of the Swift `Segmenter` in
//! `stt/meeting-tap.swift:233`. This is the load-bearing shared audio logic: both the system-audio
//! and microphone streams feed a `Segmenter`, which cuts on a natural pause (silence hangover) and
//! emits whole utterances. The per-OS crate wires each emitted [`Utterance`] to WAV → Whisper →
//! caption POST; that downstream path is identical on every platform.
//!
//! Constants match the Swift defaults exactly:
//! - `SR = 16000` Hz (all audio is resampled to 16 kHz mono before feeding).
//! - `SILENCE_HANGOVER = 0.7 s` ends an utterance.
//! - `MIN_SEG = 0.5 s` — blips shorter than this are dropped.
//! - `MAX_SEG = 25 s` — a safety cap (the real boundary is the pause).
//! - System gate `--rms` default `0.006`; mic gate `--mic-rms` default `0.02`.

pub const SR: f64 = 16000.0;
pub const SILENCE_HANGOVER: f64 = 0.7;
pub const MIN_SEG: f64 = 0.5;
pub const DEFAULT_MAX_SEG: f64 = 25.0;
pub const DEFAULT_SPEECH_RMS: f32 = 0.006;
pub const DEFAULT_MIC_RMS: f32 = 0.02;

/// One completed utterance ready to transcribe.
#[derive(Debug, Clone)]
pub struct Utterance {
    pub index: u64,
    pub samples: Vec<f32>,
}

impl Utterance {
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / SR
    }
}

/// Streaming energy-VAD state machine. Not thread-safe; use one per audio source.
pub struct Segmenter {
    pub speaker: String,
    pub is_self: bool,
    pub tag: String,
    threshold: f32,
    max_seg: f64,
    seg: Vec<f32>,
    in_speech: bool,
    silent_dur: f64,
    seg_idx: u64,
}

impl Segmenter {
    pub fn new(speaker: impl Into<String>, is_self: bool, tag: impl Into<String>, threshold: f32) -> Self {
        Segmenter {
            speaker: speaker.into(),
            is_self,
            tag: tag.into(),
            threshold,
            max_seg: DEFAULT_MAX_SEG,
            seg: Vec::new(),
            in_speech: false,
            silent_dur: 0.0,
            seg_idx: 0,
        }
    }

    pub fn with_max_seg(mut self, max_seg: f64) -> Self {
        self.max_seg = max_seg;
        self
    }

    /// Reset between calls (keeps capacity), matching Swift `reset()`.
    pub fn reset(&mut self) {
        self.seg.clear();
        self.in_speech = false;
        self.silent_dur = 0.0;
    }

    /// Feed a block of 16 kHz mono samples. Returns any utterance that completed on this block
    /// (at most one, since a completed segment resets state). Mirrors Swift `feed` + `flush`.
    pub fn feed(&mut self, s: &[f32]) -> Option<Utterance> {
        if s.is_empty() {
            return None;
        }
        let dur = s.len() as f64 / SR;
        let rms = (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();

        if rms > self.threshold {
            if !self.in_speech {
                self.in_speech = true;
                self.seg.clear();
            }
            self.seg.extend_from_slice(s);
            self.silent_dur = 0.0;
        } else if self.in_speech {
            self.seg.extend_from_slice(s);
            self.silent_dur += dur;
            if self.silent_dur >= SILENCE_HANGOVER {
                return self.flush();
            }
        }
        if self.in_speech && self.seg.len() as f64 / SR >= self.max_seg {
            return self.flush();
        }
        None
    }

    /// End the current utterance. Returns it only if it clears `MIN_SEG` (else it's a dropped blip).
    fn flush(&mut self) -> Option<Utterance> {
        let samples = std::mem::take(&mut self.seg);
        self.in_speech = false;
        self.silent_dur = 0.0;
        if samples.len() as f64 / SR < MIN_SEG {
            return None;
        }
        self.seg_idx += 1;
        Some(Utterance {
            index: self.seg_idx,
            samples,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(rms: f32, secs: f64) -> Vec<f32> {
        vec![rms; (secs * SR) as usize]
    }

    #[test]
    fn silence_before_speech_emits_nothing() {
        let mut seg = Segmenter::new("Speaker", false, "sys", DEFAULT_SPEECH_RMS);
        assert!(seg.feed(&block(0.0, 1.0)).is_none());
    }

    #[test]
    fn speech_then_hangover_emits_one_utterance() {
        let mut seg = Segmenter::new("Speaker", false, "sys", DEFAULT_SPEECH_RMS);
        // 1s loud speech, well above the 0.006 gate.
        assert!(seg.feed(&block(0.2, 1.0)).is_none());
        // 0.7s of silence ends it.
        let out = seg.feed(&block(0.0, SILENCE_HANGOVER));
        let u = out.expect("utterance should flush after hangover");
        assert_eq!(u.index, 1);
        assert!(u.seconds() >= 1.0);
    }

    #[test]
    fn hangover_silence_is_included_in_the_segment() {
        // Faithful to Swift `feed`: while in-speech, the trailing silence that ends the utterance is
        // appended to the segment (meeting-tap.swift:256). So even a short 0.2s blip flushes a
        // segment of blip + 0.7s hangover ≈ 0.9s, which clears MIN_SEG (0.5s). MIN_SEG only drops a
        // flush shorter than 0.5s, which a hangover-terminated segment can never be.
        let mut seg = Segmenter::new("Speaker", false, "sys", DEFAULT_SPEECH_RMS);
        seg.feed(&block(0.2, 0.2));
        let u = seg.feed(&block(0.0, SILENCE_HANGOVER)).expect("blip + hangover flushes");
        assert!(u.seconds() >= MIN_SEG);
    }

    #[test]
    fn short_flush_below_min_seg_is_dropped() {
        // Exercise the MIN_SEG guard directly with the MAX_SEG cap: a 0.3s continuous segment capped
        // at 0.2s would be < MIN_SEG and must be dropped.
        let mut seg = Segmenter::new("Speaker", false, "sys", DEFAULT_SPEECH_RMS).with_max_seg(0.2);
        // 0.3s continuous loud audio: crosses the 0.2s cap on the first block, but 0.3s ≥ MIN_SEG,
        // so it DOES emit. To force a sub-MIN_SEG flush we need max_seg < MIN_SEG.
        let mut tiny = Segmenter::new("Speaker", false, "sys", DEFAULT_SPEECH_RMS).with_max_seg(0.1);
        let _ = seg.feed(&block(0.2, 0.3));
        assert!(tiny.feed(&block(0.2, 0.12)).is_none(), "0.12s < MIN_SEG 0.5 is dropped");
    }

    #[test]
    fn max_seg_cap_forces_flush() {
        let mut seg = Segmenter::new("Speaker", false, "sys", DEFAULT_SPEECH_RMS).with_max_seg(2.0);
        // Continuous loud audio past the 2s cap flushes without any silence.
        let out = seg.feed(&block(0.2, 2.5));
        assert!(out.is_some());
    }
}
