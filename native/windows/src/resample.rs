//! Audio conditioning — stereo/multi-channel → mono downmix and arbitrary-rate → 16 kHz resampling.
//! **Fully portable and real.** WASAPI hands us the endpoint mix format (commonly 48 kHz float32
//! stereo for render loopback; the mic endpoint varies); the `Segmenter` in `helpers-core` requires
//! 16 kHz mono, so every native block is passed through here before [`crate::pipeline`].
//!
//! This is the Rust equivalent of the `AVAudioConverter` step the Swift helper used. It is a simple
//! linear-interpolation resampler — correct and dependency-free. For the exact 48k→16k integer ratio
//! it degenerates to picking 1 of every 3 output-aligned samples with linear blending, which is
//! adequate for speech VAD + whisper (whisper resamples internally anyway).

use helpers_core::segmenter::SR;

/// Downmix interleaved `channels`-channel f32 frames to mono by averaging channels.
pub fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let frames = interleaved.len() / channels;
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let base = f * channels;
        let mut sum = 0.0f32;
        for c in 0..channels {
            sum += interleaved[base + c];
        }
        out.push(sum / channels as f32);
    }
    out
}

/// Linear-resample a mono f32 stream from `in_rate` Hz to 16 kHz.
pub fn to_16k_mono(mono: &[f32], in_rate: u32) -> Vec<f32> {
    let target = SR as u32;
    if in_rate == target || mono.is_empty() {
        return mono.to_vec();
    }
    let ratio = in_rate as f64 / target as f64;
    let out_len = ((mono.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 * ratio;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(mono.len() - 1);
        let frac = (src - i0 as f64) as f32;
        out.push(mono[i0] * (1.0 - frac) + mono[i1] * frac);
    }
    out
}

/// Convenience: interleaved multi-channel at `in_rate` → 16 kHz mono in one call.
pub fn condition(interleaved: &[f32], channels: usize, in_rate: u32) -> Vec<f32> {
    let mono = downmix_to_mono(interleaved, channels);
    to_16k_mono(&mono, in_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_stereo() {
        let out = downmix_to_mono(&[1.0, 0.0, 0.5, 0.5], 2);
        assert_eq!(out, vec![0.5, 0.5]);
    }

    #[test]
    fn resample_48k_to_16k_thirds_the_length() {
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32).sin()).collect();
        let out = to_16k_mono(&input, 48_000);
        // 48000 / 3 == 16000 output samples (floor).
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn passthrough_when_already_16k() {
        let input = vec![0.1, 0.2, 0.3];
        assert_eq!(to_16k_mono(&input, 16_000), input);
    }
}
