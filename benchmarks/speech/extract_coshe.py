#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "numpy>=2.0",
#   "pyarrow>=18.0",
#   "soundfile>=0.13",
# ]
# ///
"""Extract a deterministic CoSHE-500 evaluation subset into private benchmark cache."""

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


DATASET_ID = "Trelis/CoSHE-500"
DATASET_REVISION = "3da116bac98bfe5349f9b454d95a17a43483e1e8"
PARQUET_SHA256 = "b5201501cde4134b2bcb70151f2b97aa0daf8b0c117b9e6c1dc494c4c230c926"
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
        default=DEFAULT_CACHE / "datasets/coshe-500/data/eval-00000-of-00001.parquet",
    )
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_CACHE / "corpora/coshe-500")
    parser.add_argument("--limit", type=int, default=24, help="0 extracts all 500 clips")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--skip-hash", action="store_true", help="Development only")
    args = parser.parse_args()

    parquet = args.parquet.expanduser().resolve()
    if not parquet.is_file():
        parser.error(f"missing {parquet}; run script/benchmark_speech.sh prepare-data")
    if not args.skip_hash and hash_file(parquet) != PARQUET_SHA256:
        parser.error("CoSHE parquet SHA-256 mismatch")

    parquet_file = pq.ParquetFile(parquet)
    total = parquet_file.metadata.num_rows
    limit = total if args.limit == 0 else min(max(args.limit, 1), total)
    selected = set(random.Random(args.seed).sample(range(total), limit))
    out_dir = args.out_dir.expanduser().resolve()
    clips_dir = out_dir / "clips"
    clips_dir.mkdir(parents=True, exist_ok=True)

    records: list[dict] = []
    row_index = 0
    for batch in parquet_file.iter_batches(batch_size=32, columns=["audio", "transcription"]):
        for row in batch.to_pylist():
            current = row_index
            row_index += 1
            if current not in selected:
                continue
            encoded = row["audio"]["bytes"]
            samples, sample_rate = sf.read(io.BytesIO(encoded), dtype="float32", always_2d=False)
            if sample_rate != 16000 or getattr(samples, "ndim", 1) != 1:
                raise ValueError(f"row {current} is not 16 kHz mono")
            clip_name = f"coshe-{current:04d}.wav"
            clip_path = clips_dir / clip_name
            sf.write(clip_path, samples, sample_rate, subtype="PCM_16")
            records.append(
                {
                    "id": f"coshe-{current:04d}",
                    "audio": f"clips/{clip_name}",
                    "ref_text": row["transcription"],
                    "language": "auto",
                    "referenceStyle": "mixed",
                    "sourceRow": current,
                }
            )

    records.sort(key=lambda record: record["sourceRow"])
    manifest = out_dir / f"coshe-500-seed{args.seed}-n{limit}.jsonl"
    header = {
        "type": "corpus_header",
        "schemaVersion": 1,
        "dataset": DATASET_ID,
        "revision": DATASET_REVISION,
        "parquetSha256": PARQUET_SHA256,
        "license": "CC-BY-NC-4.0",
        "use": "evaluation-only; never training",
        "referenceStyle": "mixed",
        "selection": {"seed": args.seed, "count": limit, "totalRows": total},
    }
    with manifest.open("w", encoding="utf-8") as handle:
        handle.write(json.dumps(header, ensure_ascii=False) + "\n")
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")
    print(manifest)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
