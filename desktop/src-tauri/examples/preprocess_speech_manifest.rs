use app_lib::speech_audio::{resample_16k, vad_extract_speech};
use serde_json::Value;
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

fn read_mono_wav(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path).map_err(|error| error.to_string())?;
    let spec = reader.spec();
    if spec.channels != 1 {
        return Err(format!("{} is not mono", path.display()));
    }
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?,
        hound::SampleFormat::Int => {
            let scale = 2_f32.powi(i32::from(spec.bits_per_sample.saturating_sub(1)));
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / scale))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        }
    };
    Ok((samples, spec.sample_rate))
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|error| error.to_string())?;
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        writer
            .write_sample(value)
            .map_err(|error| error.to_string())?;
    }
    writer.finalize().map_err(|error| error.to_string())
}

fn main() -> Result<(), String> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or("usage: preprocess_speech_manifest INPUT.jsonl OUTPUT_DIR")?,
    );
    let output_dir = PathBuf::from(
        args.next()
            .ok_or("usage: preprocess_speech_manifest INPUT.jsonl OUTPUT_DIR")?,
    );
    if args.next().is_some() {
        return Err("usage: preprocess_speech_manifest INPUT.jsonl OUTPUT_DIR".into());
    }

    let input = input.canonicalize().map_err(|error| error.to_string())?;
    let input_dir = input.parent().ok_or("input manifest has no parent")?;
    let clips_dir = output_dir.join("clips");
    fs::create_dir_all(&clips_dir).map_err(|error| error.to_string())?;
    let output_manifest =
        output_dir.join(input.file_name().ok_or("input manifest has no filename")?);
    let reader = BufReader::new(File::open(&input).map_err(|error| error.to_string())?);
    let mut writer =
        BufWriter::new(File::create(&output_manifest).map_err(|error| error.to_string())?);

    for line in reader.lines() {
        let mut record: Value = serde_json::from_str(&line.map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        if record.get("type").and_then(Value::as_str) == Some("corpus_header") {
            record["audioPipeline"] = Value::String("product-dictation-vad-v1".into());
            writeln!(writer, "{record}").map_err(|error| error.to_string())?;
            continue;
        }

        let relative = record
            .get("audio")
            .and_then(Value::as_str)
            .ok_or("sample is missing audio")?;
        let source = input_dir.join(relative);
        let (samples, sample_rate) = read_mono_wav(&source)?;
        let resampled = resample_16k(&samples, sample_rate);
        let speech = vad_extract_speech(&resampled);
        let filename = source.file_name().ok_or("audio path has no filename")?;
        let destination = clips_dir.join(filename);
        write_wav(&destination, &speech)?;
        record["audio"] = Value::String(format!("clips/{}", filename.to_string_lossy()));
        record["audioPipeline"] = Value::String("product-dictation-vad-v1".into());
        record["sourceAudioSamples"] = Value::from(samples.len() as u64);
        record["retainedAudioSamples"] = Value::from(speech.len() as u64);
        record["retainedAudioRatio"] = Value::from(if samples.is_empty() {
            0.0
        } else {
            speech.len() as f64 / samples.len() as f64
        });
        writeln!(writer, "{record}").map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    println!("{}", output_manifest.display());
    Ok(())
}
