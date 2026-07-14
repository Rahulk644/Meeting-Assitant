use std::sync::Mutex;

// Downsample captured mono f32 to 16 kHz (the speech models' and Silero's rate) with a box average over
// each source window. This is the production dictation path and is public only so the benchmark can exercise
// the exact same implementation instead of maintaining an approximate copy.
pub fn resample_16k(samples: &[f32], rate: u32) -> Vec<f32> {
    if rate <= 16_000 || rate == 0 {
        return samples.to_vec();
    }
    let ratio = rate as f64 / 16_000.0;
    let out_len = (samples.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let start = (i as f64 * ratio) as usize;
        let end = (((i + 1) as f64 * ratio) as usize).clamp(start + 1, samples.len());
        let window = &samples[start..end];
        out.push(window.iter().sum::<f32>() / window.len() as f32);
    }
    out
}

// Keep only speech regions so silence and room tone cannot trigger fluent hallucination loops. On detector
// initialization failure the original samples are retained; losing a user's speech is the worse failure.
static VAD: Mutex<Option<voice_activity_detector::VoiceActivityDetector>> = Mutex::new(None);

pub fn vad_extract_speech(samples16k: &[f32]) -> Vec<f32> {
    use voice_activity_detector::{IteratorExt, VoiceActivityDetector};

    let mut guard = VAD.lock().unwrap_or_else(|error| error.into_inner());
    if guard.is_none() {
        match VoiceActivityDetector::builder()
            .sample_rate(16_000i64)
            .chunk_size(512usize)
            .build()
        {
            Ok(detector) => *guard = Some(detector),
            Err(_) => return samples16k.to_vec(),
        }
    }
    let detector = guard.as_mut().expect("VAD was initialized above");
    detector.reset();
    let mut output = Vec::with_capacity(samples16k.len());
    for chunk in samples16k
        .iter()
        .copied()
        .label(&mut *detector, 0.5f32, 3usize)
    {
        if chunk.is_speech() {
            output.extend(chunk.iter().copied());
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_khz_audio_is_not_resampled() {
        let samples = vec![-0.5, 0.0, 0.5];
        assert_eq!(resample_16k(&samples, 16_000), samples);
    }

    #[test]
    fn forty_eight_khz_uses_three_sample_box_average() {
        let samples = vec![0.0, 0.3, 0.6, -0.6, -0.3, 0.0];
        assert_eq!(resample_16k(&samples, 48_000), vec![0.3, -0.3]);
    }
}
