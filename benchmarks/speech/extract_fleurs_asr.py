#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "pyarrow==21.0.0",
#   "soundfile==0.13.1",
# ]
# ///
"""Extract a deterministic human-reference English ASR set from pinned FLEURS audio."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import random
from pathlib import Path

import pyarrow.parquet as pq
import soundfile as sf


DATASET_ID = "google/fleurs"
DATASET_REVISION = "70bb2e84b976b7e960aa89f1c648e09c59f894dd"
ENGLISH_PARQUET_SHA256 = "7c3eeb11a9597bd52cdc1b0d637e85389fe094cfd8763913e7bf4fdf7a853959"
DEFAULT_CACHE = Path(os.environ.get("ASSISTANT_BENCH_CACHE", "~/.cache/my-assistant-bench")).expanduser()


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--parquet",
        type=Path,
        default=DEFAULT_CACHE / "datasets/fleurs/parquet-data/en_us/validation-00000-of-00001.parquet",
    )
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_CACHE / "corpora/fleurs-en-asr")
    parser.add_argument("--limit", type=int, default=24)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()

    parquet = args.parquet.expanduser().resolve()
    if hash_file(parquet) != ENGLISH_PARQUET_SHA256:
        parser.error("FLEURS English validation parquet SHA-256 mismatch")

    rows: list[tuple[int, dict]] = []
    parquet_file = pq.ParquetFile(parquet)
    row_index = 0
    for batch in parquet_file.iter_batches(columns=["id", "audio", "transcription"]):
        for row in batch.to_pylist():
            rows.append((row_index, row))
            row_index += 1
    limit = min(max(args.limit, 1), len(rows))
    selected = sorted(random.Random(args.seed).sample(range(len(rows)), limit))

    out_dir = args.out_dir.expanduser().resolve()
    clips_dir = out_dir / "clips"
    clips_dir.mkdir(parents=True, exist_ok=True)
    records: list[dict] = []
    for selected_index in selected:
        source_row, row = rows[selected_index]
        samples, sample_rate = sf.read(io.BytesIO(row["audio"]["bytes"]), dtype="float32", always_2d=False)
        if sample_rate != 16_000 or getattr(samples, "ndim", 1) != 1:
            raise ValueError(f"FLEURS row {source_row} is not 16 kHz mono")
        sentence_id = int(row["id"])
        name = f"fleurs-en-{sentence_id}-{source_row}.wav"
        sf.write(clips_dir / name, samples, sample_rate, subtype="PCM_16")
        records.append(
            {
                "id": f"fleurs-en-{sentence_id}-{source_row}",
                "audio": f"clips/{name}",
                "ref_text": row["transcription"],
                "language": "auto",
                "referenceStyle": "english",
                "referenceQuality": "human",
                "sourceSentenceId": sentence_id,
                "sourceRow": source_row,
            }
        )

    manifest = out_dir / f"fleurs-en-asr-seed{args.seed}-n{limit}.jsonl"
    header = {
        "type": "corpus_header",
        "schemaVersion": 1,
        "dataset": DATASET_ID,
        "revision": DATASET_REVISION,
        "parquetSha256": ENGLISH_PARQUET_SHA256,
        "license": "CC-BY-4.0",
        "use": "evaluation-only in this project",
        "referenceStyle": "english",
        "referenceQuality": "human",
        "selection": {"seed": args.seed, "count": limit, "totalRows": len(rows)},
    }
    with manifest.open("w", encoding="utf-8") as handle:
        handle.write(json.dumps(header, ensure_ascii=False) + "\n")
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")
    print(manifest)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
