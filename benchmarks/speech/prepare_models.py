#!/usr/bin/env python3
"""Download and verify runnable speech benchmark models outside the repository."""

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
DEFAULT_CACHE = Path(os.environ.get("ASSISTANT_BENCH_CACHE", "~/.cache/my-assistant-bench")).expanduser()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_catalog(path: Path) -> dict[str, dict]:
    data = json.loads(path.read_text())
    return {model["id"]: model for model in data["models"]}


def artifact(model: dict) -> dict:
    return model.get("artifact", model["source"])


def expected_path(model: dict, cache: Path) -> Path:
    return cache / "models" / model["id"] / artifact(model)["file"]


def verify(path: Path, expected: dict) -> None:
    if not path.is_file():
        raise ValueError(f"model is missing: {path}")
    size = path.stat().st_size
    if size != expected["size"]:
        raise ValueError(f"size mismatch for {path}: expected {expected['size']}, got {size}")
    actual = sha256(path)
    if actual != expected["sha256"]:
        raise ValueError(f"SHA-256 mismatch for {path}: expected {expected['sha256']}, got {actual}")


def find_local(model: dict) -> Path | None:
    for raw in model.get("localPaths", []):
        candidate = Path(raw).expanduser()
        if candidate.is_file():
            return candidate
    return None


def prepare(model: dict, cache: Path) -> Path:
    if model["status"] not in {"runnable", "runnable-local"}:
        needed = model.get("conversion", {}).get("minimumFreeBytes")
        suffix = f"; conversion needs at least {needed / 1_000_000_000:.1f} GB free" if needed else ""
        raise ValueError(f"{model['id']} is {model['status']}{suffix}")

    local = find_local(model)
    if local:
        verify(local, artifact(model))
        return local

    if not shutil.which("hf"):
        raise ValueError("the Hugging Face `hf` CLI is not installed")
    destination = expected_path(model, cache)
    if not destination.exists():
        downloadable = artifact(model)
        if "repo" not in downloadable:
            raise ValueError(f"{model['id']} is a local conversion; run script/benchmark_speech.sh convert-srota")
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [
                "hf",
                "download",
                downloadable["repo"],
                downloadable["file"],
                "--revision",
                downloadable["revision"],
                "--local-dir",
                str(destination.parent),
            ],
            check=True,
        )
    verify(destination, artifact(model))
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("models", nargs="+", help="Model IDs from catalog.json")
    parser.add_argument("--catalog", type=Path, default=HERE / "catalog.json")
    parser.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    args = parser.parse_args()

    catalog = load_catalog(args.catalog)
    failed = False
    for model_id in args.models:
        model = catalog.get(model_id)
        if not model:
            print(f"error: unknown model {model_id!r}", file=sys.stderr)
            failed = True
            continue
        try:
            path = prepare(model, args.cache.expanduser())
            print(f"{model_id}\t{path}")
        except (OSError, ValueError, subprocess.CalledProcessError) as error:
            print(f"error: {model_id}: {error}", file=sys.stderr)
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
