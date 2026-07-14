#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "numpy==2.3.2",
#   "soundfile==0.13.1",
# ]
# ///
"""Create deterministic synthetic wind or babble ASR stress-test corpora."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np
import soundfile as sf


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--condition", choices=("wind", "babble"), required=True)
    parser.add_argument("--snr-db", type=float, default=5.0)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--max-samples", type=int)
    return parser.parse_args()


def load_manifest(path: Path, maximum: int | None) -> tuple[dict, list[dict]]:
    header: dict | None = None
    samples: list[dict] = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            record = json.loads(line)
            if record.get("type") == "corpus_header":
                header = record
            elif maximum is None or len(samples) < maximum:
                samples.append(record)
    if header is None or not samples:
        raise ValueError("manifest must contain one corpus_header and at least one sample")
    return header, samples


def read_mono(path: Path) -> tuple[np.ndarray, int]:
    audio, sample_rate = sf.read(path, dtype="float32", always_2d=False)
    if audio.ndim != 1:
        raise ValueError(f"{path} is not mono")
    return np.asarray(audio, dtype=np.float32), sample_rate


def repeat_to_length(audio: np.ndarray, length: int, offset: int) -> np.ndarray:
    if audio.size == 0:
        return np.zeros(length, dtype=np.float32)
    needed = length + offset
    repeated = np.tile(audio, math.ceil(needed / audio.size))
    return repeated[offset : offset + length]


def wind_noise(length: int, sample_rate: int, rng: np.random.Generator) -> np.ndarray:
    white = rng.standard_normal(length + 511).astype(np.float32)
    # A short moving average emphasizes low frequencies. Slow modulation creates gusts.
    low = np.convolve(white, np.ones(512, dtype=np.float32) / 512, mode="valid")[:length]
    seconds = np.arange(length, dtype=np.float32) / sample_rate
    phase = float(rng.uniform(0, 2 * np.pi))
    gusts = 0.35 + 0.65 * np.square(np.sin(2 * np.pi * 0.17 * seconds + phase))
    return np.asarray(low * gusts, dtype=np.float32)


def babble_noise(
    index: int,
    target_length: int,
    sample_rate: int,
    corpus_audio: list[tuple[np.ndarray, int]],
    rng: np.random.Generator,
) -> np.ndarray:
    candidates = [candidate for candidate in range(len(corpus_audio)) if candidate != index]
    if not candidates:
        raise ValueError("babble generation requires at least two source clips")
    selected = rng.choice(candidates, size=min(3, len(candidates)), replace=False)
    layers: list[np.ndarray] = []
    for candidate in selected:
        audio, source_rate = corpus_audio[int(candidate)]
        if source_rate != sample_rate:
            raise ValueError("all source clips must have the same sample rate")
        offset = int(rng.integers(0, max(audio.size, 1)))
        layers.append(repeat_to_length(audio, target_length, offset))
    return np.mean(np.stack(layers), axis=0, dtype=np.float32)


def mix_at_snr(signal: np.ndarray, noise: np.ndarray, snr_db: float) -> tuple[np.ndarray, float]:
    signal_rms = float(np.sqrt(np.mean(np.square(signal), dtype=np.float64)))
    noise_rms = float(np.sqrt(np.mean(np.square(noise), dtype=np.float64)))
    if signal_rms == 0 or noise_rms == 0:
        raise ValueError("cannot mix silent signal or noise")
    target_noise_rms = signal_rms / (10 ** (snr_db / 20))
    mixed = signal + noise * (target_noise_rms / noise_rms)
    peak = float(np.max(np.abs(mixed)))
    gain = min(1.0, 0.98 / peak) if peak else 1.0
    return np.asarray(mixed * gain, dtype=np.float32), gain


def main() -> None:
    args = parse_args()
    manifest = args.manifest.expanduser().resolve()
    header, records = load_manifest(manifest, args.max_samples)
    input_dir = manifest.parent
    args.output_dir.mkdir(parents=True, exist_ok=True)
    clips_dir = args.output_dir / "clips"
    clips_dir.mkdir(exist_ok=True)

    corpus_audio = [read_mono((input_dir / record["audio"]).resolve()) for record in records]
    rng = np.random.default_rng(args.seed)
    output_name = f"{manifest.stem}-{args.condition}-{args.snr_db:g}db.jsonl"
    output_manifest = args.output_dir / output_name

    output_header = dict(header)
    output_header.update(
        {
            "syntheticNoise": True,
            "noiseCondition": args.condition,
            "snrDb": args.snr_db,
            "noiseSeed": args.seed,
            "noiseMethod": "deterministic-engineering-stress-v1",
            "noiseCaveat": "Synthetic stress test only; not evidence for real cafe or outdoor speech.",
            "selection": {**header.get("selection", {}), "count": len(records)},
        }
    )

    with output_manifest.open("w", encoding="utf-8") as handle:
        handle.write(json.dumps(output_header, ensure_ascii=False) + "\n")
        for index, (record, (signal, sample_rate)) in enumerate(zip(records, corpus_audio, strict=True)):
            if args.condition == "wind":
                noise = wind_noise(signal.size, sample_rate, rng)
            else:
                noise = babble_noise(index, signal.size, sample_rate, corpus_audio, rng)
            mixed, gain = mix_at_snr(signal, noise, args.snr_db)
            filename = f"{record['id']}-{args.condition}-{args.snr_db:g}db.wav"
            destination = clips_dir / filename
            sf.write(destination, mixed, sample_rate, subtype="PCM_16")
            output_record = dict(record)
            output_record.update(
                {
                    "audio": f"clips/{filename}",
                    "syntheticNoise": True,
                    "noiseCondition": args.condition,
                    "snrDb": args.snr_db,
                    "noiseMixGain": gain,
                    "sourceAudioSha256": hashlib.sha256(
                        (input_dir / record["audio"]).resolve().read_bytes()
                    ).hexdigest(),
                }
            )
            handle.write(json.dumps(output_record, ensure_ascii=False) + "\n")

    print(output_manifest)


if __name__ == "__main__":
    main()
