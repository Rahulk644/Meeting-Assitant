#!/usr/bin/env python3
"""Run resumable, repeated speech benchmarks with the model's production audio policy."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import shutil
import statistics
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

import run as benchmark


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_CACHE = Path(os.environ.get("ASSISTANT_BENCH_CACHE", "~/.cache/my-assistant-bench")).expanduser()
POLICIES = {"resample-only", "vad-gate", "vad-compact"}


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def preprocessing_policy(model: dict) -> str:
    policy = model.get("preprocessing", {}).get("policy")
    if policy not in POLICIES:
        raise ValueError(f"{model['id']} has no supported preprocessing.policy")
    return policy


def model_receipt(model: dict) -> dict:
    expected = model.get("artifact", model["source"])
    receipt = {
        "id": model["id"],
        "license": model.get("license"),
        "source": {
            key: model["source"].get(key)
            for key in ("repo", "revision", "file", "size", "sha256")
            if model["source"].get(key) is not None
        },
        "runtimeArtifact": {
            key: expected.get(key)
            for key in ("file", "size", "sha256", "converterCommit", "quant")
            if expected.get(key) is not None
        },
    }
    return receipt


def prepare_manifest(
    source: Path,
    policy: str,
    cache: Path,
    preprocessor: Path,
) -> Path:
    if policy not in POLICIES:
        raise ValueError(f"unsupported preprocessing policy {policy!r}")
    source_hash = file_sha256(source)
    preprocessor_hash = file_sha256(preprocessor)
    output_dir = (
        cache
        / "corpora"
        / "preprocessed"
        / f"{source.stem}-{source_hash[:12]}-{policy}-{preprocessor_hash[:12]}"
    )
    output = output_dir / source.name
    receipt = output_dir / "preprocessing-receipt.json"
    expected_receipt = {
        "schemaVersion": 1,
        "sourceManifest": str(source),
        "sourceManifestSha256": source_hash,
        "preprocessorSha256": preprocessor_hash,
        "policy": policy,
    }
    if output.is_file() and receipt.is_file():
        current = json.loads(receipt.read_text())
        if all(current.get(key) == value for key, value in expected_receipt.items()):
            return output

    if output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True)
    subprocess.run(
        [str(preprocessor), str(source), str(output_dir), "--policy", policy],
        check=True,
    )
    if not output.is_file():
        raise RuntimeError(f"preprocessor did not create {output}")
    expected_receipt["createdAt"] = datetime.now(timezone.utc).isoformat()
    receipt.write_text(json.dumps(expected_receipt, indent=2) + "\n")
    return output


def numeric_stats(values: list[float]) -> dict:
    return {
        "min": round(min(values), 6),
        "median": round(statistics.median(values), 6),
        "max": round(max(values), 6),
        "mean": round(statistics.fmean(values), 6),
        "populationStdDev": round(statistics.pstdev(values), 6),
    }


def summarize_model(model_id: str, reports: list[dict], policy: str) -> dict:
    runs = [report["models"][0] for report in reports]
    hypothesis_by_id: dict[str, set[str]] = {}
    for run in runs:
        for sample in run["results"]:
            hypothesis_by_id.setdefault(sample["id"], set()).add(sample["hypText"])
    return {
        "id": model_id,
        "label": runs[0]["label"],
        "preprocessingPolicy": policy,
        "referenceStyle": runs[0]["referenceStyle"],
        "referenceQuality": runs[0]["referenceQuality"],
        "scoreComparable": all(run["scoreComparable"] for run in runs),
        "samplesPerRun": runs[0]["samples"],
        "trials": len(runs),
        "scriptSafeWer": numeric_stats([run["scriptSafeWer"] for run in runs]),
        "characterF2": numeric_stats([run["characterF2"] for run in runs]),
        "rtf": numeric_stats([run["rtf"] for run in runs]),
        "peakRssMiB": numeric_stats([run["peakRssMiB"] for run in runs]),
        "truncatedSamples": numeric_stats([float(run["truncatedSamples"]) for run in runs]),
        "degenerateSamples": numeric_stats([float(run["degenerateSamples"]) for run in runs]),
        "silenceSamples": runs[0].get("silenceSamples", 0),
        "silenceFalsePositiveSamples": numeric_stats(
            [float(run.get("silenceFalsePositiveSamples", 0)) for run in runs]
        ),
        "hypothesisDisagreementSamples": sum(
            len(hypotheses) > 1 for hypotheses in hypothesis_by_id.values()
        ),
        "reportFiles": [Path(report["_path"]).name for report in reports],
    }


def valid_completed_report(path: Path, model_id: str, manifest: Path, expected_samples: int) -> dict | None:
    if not path.is_file():
        return None
    try:
        report = json.loads(path.read_text())
        run = report["models"][0]
    except (json.JSONDecodeError, KeyError, IndexError, TypeError):
        return None
    if (
        run.get("id") != model_id
        or run.get("samples") != expected_samples
        or Path(report.get("manifest", "")).resolve() != manifest.resolve()
    ):
        return None
    report["_path"] = str(path)
    return report


def preserve_trial_log(report: dict, destination: Path) -> None:
    """Pin the model log before the next trial overwrites its cache-level filename."""
    source_value = report["models"][0].get("logPath", "")
    source = Path(source_value).expanduser() if source_value else None
    if source and source.is_file():
        shutil.copy2(source, destination)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", required=True, help="Comma-separated IDs from catalog.json")
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--catalog", type=Path, default=HERE / "catalog.json")
    parser.add_argument(
        "--server", type=Path, default=ROOT / "stt/transcribe-server/transcribe-server"
    )
    parser.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--out-dir", type=Path)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--cooldown-seconds", type=float, default=5.0)
    parser.add_argument("--raw", action="store_true", help="Bypass model-specific product preprocessing")
    parser.add_argument(
        "--allow-smoke",
        action="store_true",
        help="Allow fewer than 500 samples or three repeats; never use for promotion evidence",
    )
    args = parser.parse_args()

    source_manifest = args.manifest.expanduser().resolve()
    catalog_path = args.catalog.expanduser().resolve()
    cache = args.cache.expanduser().resolve()
    server = args.server.expanduser().resolve()
    catalog = benchmark.load_catalog(catalog_path)
    requested = [value.strip() for value in args.models.split(",") if value.strip()]
    if len(requested) != len(set(requested)):
        parser.error("--models contains duplicates")
    for model_id in requested:
        if model_id not in catalog:
            parser.error(f"unknown model {model_id!r}")
        benchmark.verify_model(benchmark.resolve_model_path(catalog[model_id], cache), catalog[model_id])

    _, entries = benchmark.load_manifest(source_manifest, None)
    if not args.allow_smoke and len(entries) != 500:
        parser.error(f"promotion matrix requires exactly 500 samples, got {len(entries)}")
    if not args.allow_smoke and args.repeats < 3:
        parser.error("promotion matrix requires at least three repeats")
    if args.repeats < 1:
        parser.error("--repeats must be positive")
    if args.cooldown_seconds < 0 or args.cooldown_seconds > 60:
        parser.error("--cooldown-seconds must be between 0 and 60")

    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    out_dir = (
        args.out_dir.expanduser().resolve()
        if args.out_dir
        else cache / "reports" / f"matrix-{source_manifest.stem}-{stamp}"
    )
    out_dir.mkdir(parents=True, exist_ok=True)

    preprocessor = ROOT / "desktop/src-tauri/target/release/examples/preprocess_speech_manifest"
    if not args.raw:
        subprocess.run(
            [
                "cargo",
                "build",
                "--locked",
                "--release",
                "--manifest-path",
                str(ROOT / "desktop/src-tauri/Cargo.toml"),
                "--example",
                "preprocess_speech_manifest",
            ],
            check=True,
        )
        if not preprocessor.is_file():
            raise RuntimeError(f"missing built preprocessor {preprocessor}")

    manifests: dict[str, Path] = {}
    policies: dict[str, str] = {}
    pipeline_receipts: dict[str, dict] = {}
    for model_id in requested:
        policy = "raw-input" if args.raw else preprocessing_policy(catalog[model_id])
        policies[model_id] = policy
        manifests[model_id] = (
            source_manifest
            if args.raw
            else prepare_manifest(source_manifest, policy, cache, preprocessor)
        )
        pipeline_receipts[model_id] = {
            "policy": policy,
            "manifestFile": manifests[model_id].name,
            "manifestSha256": file_sha256(manifests[model_id]),
            "preprocessorSha256": None if args.raw else file_sha256(preprocessor),
        }

    reports_by_model: dict[str, list[dict]] = {model_id: [] for model_id in requested}
    trial_orders: list[dict] = []
    for trial in range(1, args.repeats + 1):
        order = requested.copy()
        random.Random(args.seed + trial).shuffle(order)
        trial_orders.append({"trial": trial, "models": order})
        print(f"\ntrial {trial}/{args.repeats}: {', '.join(order)}", flush=True)
        for position, model_id in enumerate(order):
            manifest = manifests[model_id]
            output = out_dir / f"trial-{trial:02d}-{model_id}.json"
            completed = valid_completed_report(output, model_id, manifest, len(entries))
            if completed:
                print(f"resume: {output.name}", flush=True)
                reports_by_model[model_id].append(completed)
                continue
            command = [
                sys.executable,
                str(HERE / "run.py"),
                "--models",
                model_id,
                "--manifest",
                str(manifest),
                "--catalog",
                str(catalog_path),
                "--server",
                str(server),
                "--cache",
                str(cache),
                "--out",
                str(output),
            ]
            subprocess.run(command, check=True)
            completed = valid_completed_report(output, model_id, manifest, len(entries))
            if not completed:
                raise RuntimeError(f"invalid completed report {output}")
            preserve_trial_log(completed, output.with_suffix(".log"))
            reports_by_model[model_id].append(completed)
            has_more_runs = trial < args.repeats or position + 1 < len(order)
            if args.cooldown_seconds and has_more_runs:
                time.sleep(args.cooldown_seconds)

    summary = {
        "schemaVersion": 1,
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "sourceManifestFile": source_manifest.name,
        "sourceManifestSha256": file_sha256(source_manifest),
        "catalogFile": catalog_path.name,
        "catalogSha256": file_sha256(catalog_path),
        "serverFile": server.name,
        "serverSha256": file_sha256(server),
        "samplesPerRun": len(entries),
        "repeats": args.repeats,
        "orderSeed": args.seed,
        "trialOrders": trial_orders,
        "pipeline": "raw-control" if args.raw else "model-specific-product",
        "pipelineReceipts": pipeline_receipts,
        "modelReceipts": [model_receipt(catalog[model_id]) for model_id in requested],
        "promotionEligibleRunShape": len(entries) == 500 and args.repeats >= 3,
        "host": benchmark.host_info(),
        "models": [
            summarize_model(model_id, reports_by_model[model_id], policies[model_id])
            for model_id in requested
        ],
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
    print(f"\nsummary: {summary_path}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
