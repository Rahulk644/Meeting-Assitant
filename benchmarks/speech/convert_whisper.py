#!/usr/bin/env python3
"""Reproduce a cataloged Whisper fine-tune GGUF conversion outside the repository."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_CACHE = Path(os.environ.get("ASSISTANT_BENCH_CACHE", "~/.cache/my-assistant-bench")).expanduser()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify(path: Path, expected: dict) -> None:
    if not path.is_file():
        raise ValueError(f"missing {path}")
    if path.stat().st_size != expected["size"]:
        raise ValueError(f"size mismatch for {path}")
    if sha256(path) != expected["sha256"]:
        raise ValueError(f"SHA-256 mismatch for {path}")


def hf_download(repo: str, revision: str, destination: Path, files: list[str]) -> None:
    subprocess.run(
        [
            "hf",
            "download",
            repo,
            *files,
            "--revision",
            revision,
            "--local-dir",
            str(destination),
        ],
        check=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("model_id")
    parser.add_argument("--catalog", type=Path, default=HERE / "catalog.json")
    parser.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--clean-source", action="store_true", help="remove source weights after success")
    args = parser.parse_args()

    catalog = json.loads(args.catalog.read_text())
    model = next((item for item in catalog["models"] if item["id"] == args.model_id), None)
    if model is None:
        parser.error(f"unknown model {args.model_id!r}")
    conversion = model.get("conversion", {})
    if conversion.get("family") != "whisper" or "artifact" not in model:
        parser.error(f"{args.model_id} is not a reproducible Whisper conversion")
    minimum = int(conversion.get("minimumFreeBytes", 0))
    cache = args.cache.expanduser().resolve()
    cache.mkdir(parents=True, exist_ok=True)
    artifact = model["artifact"]
    output_dir = cache / "models" / model["id"]
    destination = output_dir / artifact["file"]
    if destination.exists():
        verify(destination, artifact)
        print(destination)
        return 0
    if shutil.disk_usage(cache).free < minimum:
        parser.error(f"conversion needs at least {minimum / 1_000_000_000:.1f} GB free")
    if not shutil.which("hf") or not shutil.which("uv"):
        parser.error("the `hf` and `uv` commands are required")

    source = model["source"]
    source_dir = cache / "sources" / model["id"]
    source_dir.mkdir(parents=True, exist_ok=True)
    output_dir.mkdir(parents=True, exist_ok=True)
    source_files = [
        "config.json",
        "generation_config.json",
        source["file"],
        "tokenizer.json",
        "tokenizer_config.json",
    ]
    if "preprocessor" not in model:
        source_files.append("preprocessor_config.json")
    hf_download(source["repo"], source["revision"], source_dir, source_files)
    verify(source_dir / source["file"], source)

    if preprocessor := model.get("preprocessor"):
        preprocessor_dir = cache / "sources" / f"{model['id']}-preprocessor"
        preprocessor_dir.mkdir(parents=True, exist_ok=True)
        hf_download(
            preprocessor["repo"],
            preprocessor["revision"],
            preprocessor_dir,
            [preprocessor["file"]],
        )
        source_preprocessor = preprocessor_dir / preprocessor["file"]
        verify(source_preprocessor, preprocessor)
        shutil.copyfile(source_preprocessor, source_dir / "preprocessor_config.json")

    native_root = ROOT / "stt/transcribe-server/.transcribe.cpp"
    converter = native_root / "scripts/convert-whisper.py"
    quantizer = native_root / "build/bin/transcribe-quantize"
    if not converter.is_file() or not os.access(quantizer, os.X_OK):
        parser.error("run ./stt/transcribe-server/build.sh before conversion")

    reference = output_dir / f"{model['id']}-reference.gguf"
    try:
        subprocess.run(
            [
                "uv",
                "run",
                "--project",
                str(native_root / "scripts/envs/whisper"),
                str(converter),
                str(source_dir),
                str(reference),
                "--repo-id",
                source["repo"],
                "--variant",
                conversion["variant"],
            ],
            check=True,
        )
        subprocess.run(
            [str(quantizer), str(reference), str(destination), "--quant", conversion["quant"]],
            check=True,
        )
        verify(destination, artifact)
    finally:
        reference.unlink(missing_ok=True)
    if args.clean_source:
        (source_dir / source["file"]).unlink(missing_ok=True)
    print(destination)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
