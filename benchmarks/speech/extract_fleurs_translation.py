#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "pyarrow==21.0.0",
#   "soundfile==0.13.1",
# ]
# ///
"""Extract aligned Hindi speech and human English references from a pinned FLEURS validation split."""

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
PARQUET_SHA256 = "2c6286b884069103be9a8051b12dd9dfbfe1c0c3d4834ad7ecaab1a7689518c5"
ENGLISH_TSV_SHA256 = "9d57ee7e91e9d4c92edb39f6bbea668ef8dc2a3ff96eb510d5580b2ad05d17ec"
DEFAULT_CACHE = Path(os.environ.get("ASSISTANT_BENCH_CACHE", "~/.cache/my-assistant-bench")).expanduser()


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def english_references(path: Path) -> dict[int, str]:
    references: dict[int, str] = {}
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            fields = line.rstrip("\n").split("\t")
            if len(fields) != 7:
                raise ValueError(f"unexpected FLEURS TSV row with {len(fields)} fields")
            references.setdefault(int(fields[0]), fields[2])
    return references


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--parquet",
        type=Path,
        default=DEFAULT_CACHE / "datasets/fleurs/parquet-data/hi_in/validation-00000-of-00001.parquet",
    )
    parser.add_argument(
        "--english-tsv",
        type=Path,
        default=DEFAULT_CACHE / "datasets/fleurs/data/en_us/dev.tsv",
    )
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_CACHE / "corpora/fleurs-hi-en")
    parser.add_argument("--limit", type=int, default=24)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()

    parquet = args.parquet.expanduser().resolve()
    english_tsv = args.english_tsv.expanduser().resolve()
    if hash_file(parquet) != PARQUET_SHA256:
        parser.error("FLEURS Hindi validation parquet SHA-256 mismatch")
    if hash_file(english_tsv) != ENGLISH_TSV_SHA256:
        parser.error("FLEURS English dev TSV SHA-256 mismatch")

    references = english_references(english_tsv)
    rows: dict[int, tuple[int, dict]] = {}
    row_index = 0
    parquet_file = pq.ParquetFile(parquet)
    for batch in parquet_file.iter_batches(columns=["id", "audio", "transcription"]):
        for row in batch.to_pylist():
            sentence_id = int(row["id"])
            rows.setdefault(sentence_id, (row_index, row))
            row_index += 1
    available = sorted(set(rows) & set(references))
    limit = min(max(args.limit, 1), len(available))
    selected = sorted(random.Random(args.seed).sample(available, limit))

    out_dir = args.out_dir.expanduser().resolve()
    clips_dir = out_dir / "clips"
    clips_dir.mkdir(parents=True, exist_ok=True)
    records: list[dict] = []
    for sentence_id in selected:
        source_row, row = rows[sentence_id]
        samples, sample_rate = sf.read(io.BytesIO(row["audio"]["bytes"]), dtype="float32", always_2d=False)
        if sample_rate != 16_000 or getattr(samples, "ndim", 1) != 1:
            raise ValueError(f"FLEURS sentence {sentence_id} is not 16 kHz mono")
        name = f"fleurs-hi-{sentence_id}.wav"
        sf.write(clips_dir / name, samples, sample_rate, subtype="PCM_16")
        records.append(
            {
                "id": f"fleurs-hi-{sentence_id}",
                "audio": f"clips/{name}",
                "ref_text": references[sentence_id],
                "source_text": row["transcription"],
                "language": "auto",
                "referenceStyle": "english",
                "referenceQuality": "human-parallel",
                "sourceSentenceId": sentence_id,
                "sourceRow": source_row,
            }
        )

    manifest = out_dir / f"fleurs-hi-en-seed{args.seed}-n{limit}.jsonl"
    header = {
        "type": "corpus_header",
        "schemaVersion": 1,
        "dataset": DATASET_ID,
        "revision": DATASET_REVISION,
        "parquetSha256": PARQUET_SHA256,
        "englishTsvSha256": ENGLISH_TSV_SHA256,
        "license": "CC-BY-4.0",
        "use": "evaluation-only in this project",
        "referenceStyle": "english",
        "referenceQuality": "human-parallel",
        "selection": {"seed": args.seed, "count": limit, "availableSentenceIds": len(available)},
    }
    with manifest.open("w", encoding="utf-8") as handle:
        handle.write(json.dumps(header, ensure_ascii=False) + "\n")
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False) + "\n")
    print(manifest)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
