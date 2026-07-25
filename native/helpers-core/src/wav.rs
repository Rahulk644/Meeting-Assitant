//! 16-bit PCM WAV writer — a direct port of the Swift `writeWav16`. Produces a 44-byte canonical
//! RIFF header + interleaved little-endian `i16` samples. Used for the per-utterance temp files
//! meeting-tap POSTs to the Whisper server (16 kHz mono).

use std::io::{self, Write};

/// Encode mono/stereo `f32` samples in `[-1.0, 1.0]` to an in-memory 16-bit PCM WAV.
pub fn encode_wav16(samples: &[f32], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    write_header(&mut out, data_len as u32, sample_rate, channels).expect("Vec write is infallible");
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        // Truncate toward zero (Rust `as i16`), matching Swift `Int16(f * 32767)` exactly — the Swift
        // does NOT round, so rounding here would make the WAV differ by up to 1 LSB per sample.
        let v = (clamped * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Write the 44-byte canonical PCM WAV header for `data_len` bytes of sample payload.
pub fn write_header<W: Write>(
    w: &mut W,
    data_len: u32,
    sample_rate: u32,
    channels: u16,
) -> io::Result<()> {
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * channels as u32 * (bits_per_sample as u32 / 8);
    let block_align = channels * (bits_per_sample / 8);

    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVE")?;
    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?; // PCM fmt chunk size
    w.write_all(&1u16.to_le_bytes())?; // audio format = PCM
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&bits_per_sample.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_44_bytes_and_riff() {
        let wav = encode_wav16(&[0.0, 0.5, -0.5], 16000, 1);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        // 44-byte header + 3 samples * 2 bytes.
        assert_eq!(wav.len(), 44 + 6);
    }

    #[test]
    fn sample_scaling() {
        let wav = encode_wav16(&[1.0], 16000, 1);
        let sample = i16::from_le_bytes([wav[44], wav[45]]);
        assert_eq!(sample, 32767);
    }
}
