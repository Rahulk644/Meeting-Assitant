#!/usr/bin/env python3
"""Benchmark the app's shipped transcribe-server on a pinned speech corpus."""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import platform
import re
import secrets
import socket
import subprocess
import sys
import time
import unicodedata
import wave
from datetime import datetime, timezone
from pathlib import Path


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_CACHE = Path(os.environ.get("ASSISTANT_BENCH_CACHE", "~/.cache/my-assistant-bench")).expanduser()


def script_safe_tokens(text: str) -> list[str]:
    """Preserve Indic marks while removing punctuation and normalizing whitespace."""
    normalized = unicodedata.normalize("NFC", text).casefold()
    output: list[str] = []
    for char in normalized:
        category = unicodedata.category(char)
        if category[0] in {"L", "M", "N"}:
            output.append(char)
        elif char in {"'", "’"}:
            continue
        elif char.isspace() or category[0] in {"P", "S", "Z"}:
            output.append(" ")
    return "".join(output).split()


def edit_counts(reference: list[str], hypothesis: list[str]) -> tuple[int, int, int]:
    """Return substitutions, deletions, and insertions for two token sequences."""
    rows: list[list[tuple[int, int, int, int]]] = []
    rows.append([(index, 0, index, 0) for index in range(len(hypothesis) + 1)])
    for ref_index, ref_token in enumerate(reference, start=1):
        row = [(ref_index, 0, 0, ref_index)]
        for hyp_index, hyp_token in enumerate(hypothesis, start=1):
            if ref_token == hyp_token:
                previous = rows[ref_index - 1][hyp_index - 1]
                row.append(previous)
                continue
            substitution = rows[ref_index - 1][hyp_index - 1]
            deletion = rows[ref_index - 1][hyp_index]
            insertion = row[hyp_index - 1]
            candidates = [
                (substitution[0] + 1, substitution[1] + 1, substitution[2], substitution[3]),
                (deletion[0] + 1, deletion[1], deletion[2], deletion[3] + 1),
                (insertion[0] + 1, insertion[1], insertion[2] + 1, insertion[3]),
            ]
            row.append(min(candidates, key=lambda value: (value[0], value[2], value[3], value[1])))
        rows.append(row)
    _, substitutions, insertions, deletions = rows[-1][-1]
    return substitutions, deletions, insertions


def degenerate_reason(text: str) -> str | None:
    tokens = script_safe_tokens(text)
    if not tokens:
        return "empty"
    if tokens == ["nan"]:
        return "non-finite-token"
    if len(tokens) >= 20:
        ngrams = [tuple(tokens[index : index + 4]) for index in range(len(tokens) - 3)]
        if len(set(ngrams)) / len(ngrams) < 0.2:
            return "repetition-loop"
    return None


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_catalog(path: Path) -> dict[str, dict]:
    data = json.loads(path.read_text())
    return {model["id"]: model for model in data["models"]}


def resolve_model_path(model: dict, cache: Path) -> Path:
    for raw in model.get("localPaths", []):
        candidate = Path(raw).expanduser()
        if candidate.is_file():
            return candidate
    expected = model.get("artifact", model["source"])
    return cache / "models" / model["id"] / expected["file"]


def verify_model(path: Path, model: dict) -> None:
    expected = model.get("artifact", model["source"])
    if model["status"] not in {"runnable", "runnable-local"}:
        raise ValueError(f"{model['id']} is {model['status']}")
    if not path.is_file():
        raise ValueError(f"missing model {path}; run prepare-models first")
    if path.stat().st_size != expected["size"]:
        raise ValueError(f"size mismatch for {model['id']}")
    if sha256(path) != expected["sha256"]:
        raise ValueError(f"SHA-256 mismatch for {model['id']}")


def load_manifest(path: Path, maximum: int | None) -> tuple[dict, list[dict]]:
    header: dict | None = None
    entries: list[dict] = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            record = json.loads(line)
            if record.get("type") == "corpus_header":
                header = record
            else:
                audio = (path.parent / record["audio"]).resolve()
                if not audio.is_file():
                    raise ValueError(f"missing audio for {record['id']}: {audio}")
                record["audio"] = str(audio)
                entries.append(record)
                if maximum and len(entries) >= maximum:
                    break
    if not header or not entries:
        raise ValueError("manifest needs one corpus_header and at least one sample")
    return header, entries


def wav_duration(path: Path) -> float:
    with wave.open(str(path), "rb") as audio:
        return audio.getnframes() / audio.getframerate()


def multipart(audio_path: Path, language: str) -> tuple[str, bytes]:
    boundary = f"assistant-{secrets.token_hex(12)}"
    chunks: list[bytes] = []

    def field(name: str, value: bytes, filename: str | None = None, content_type: str | None = None) -> None:
        chunks.append(f"--{boundary}\r\n".encode())
        disposition = f'Content-Disposition: form-data; name="{name}"'
        if filename:
            disposition += f'; filename="{filename}"'
        chunks.append((disposition + "\r\n").encode())
        if content_type:
            chunks.append(f"Content-Type: {content_type}\r\n".encode())
        chunks.append(b"\r\n")
        chunks.append(value)
        chunks.append(b"\r\n")

    field("file", audio_path.read_bytes(), audio_path.name, "audio/wav")
    field("language", language.encode())
    field("translate", b"false")
    chunks.append(f"--{boundary}--\r\n".encode())
    return f"multipart/form-data; boundary={boundary}", b"".join(chunks)


def request(port: int, token: str, method: str, path: str, body: bytes | None = None, content_type: str | None = None) -> bytes:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=180)
    headers = {"Authorization": f"Bearer {token}"}
    if content_type:
        headers["Content-Type"] = content_type
    connection.request(method, path, body=body, headers=headers)
    response = connection.getresponse()
    payload = response.read()
    connection.close()
    if response.status != 200:
        raise RuntimeError(f"HTTP {response.status}: {payload.decode(errors='replace')}")
    return payload


def rss_kib(pid: int) -> int:
    output = subprocess.check_output(["/bin/ps", "-o", "rss=", "-p", str(pid)], text=True).strip()
    return int(output or 0)


def ensure_port_free(port: int) -> None:
    with socket.socket() as probe:
        if probe.connect_ex(("127.0.0.1", port)) == 0:
            raise ValueError(f"port {port} is already occupied")


def wait_ready(process: subprocess.Popen, port: int, token: str, timeout: float = 90.0) -> float:
    started = time.perf_counter()
    while time.perf_counter() - started < timeout:
        if process.poll() is not None:
            raise RuntimeError(f"transcribe-server exited with {process.returncode}")
        try:
            request(port, token, "GET", "/")
            return time.perf_counter() - started
        except (ConnectionError, OSError, RuntimeError):
            time.sleep(0.1)
    raise RuntimeError("transcribe-server readiness timeout")


def host_info() -> dict:
    memory = subprocess.run(["/usr/sbin/sysctl", "-n", "hw.memsize"], capture_output=True, text=True)
    pin = (ROOT / "stt/transcribe-server/PINNED_COMMIT").read_text().strip()
    return {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "memoryBytes": int(memory.stdout.strip()) if memory.returncode == 0 else None,
        "transcribeCommit": pin,
    }


def model_benchmark(model: dict, model_path: Path, entries: list[dict], server: Path, port: int, cache: Path, warmup: bool) -> dict:
    ensure_port_free(port)
    token = secrets.token_urlsafe(32)
    log_dir = cache / "logs"
    log_dir.mkdir(parents=True, exist_ok=True)
    log_path = log_dir / f"{model['id']}.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen(
            [str(server), "-m", str(model_path), "--host", "127.0.0.1", "--port", str(port)],
            env={**os.environ, "ASSISTANT_STT_TOKEN": token},
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        try:
            startup_seconds = wait_ready(process, port, token)
            loaded_rss = rss_kib(process.pid)
            if warmup:
                first = entries[0]
                decode_language = model.get("decode", {}).get("language", first.get("language", "auto"))
                content_type, body = multipart(Path(first["audio"]), decode_language)
                request(port, token, "POST", "/inference", body, content_type)

            results: list[dict] = []
            peak_rss = loaded_rss
            for index, entry in enumerate(entries, start=1):
                audio_path = Path(entry["audio"])
                duration = wav_duration(audio_path)
                decode_language = model.get("decode", {}).get("language", entry.get("language", "auto"))
                content_type, body = multipart(audio_path, decode_language)
                started = time.perf_counter()
                payload = request(port, token, "POST", "/inference", body, content_type)
                elapsed = time.perf_counter() - started
                response = json.loads(payload)
                transcript = response["text"]
                ref_tokens = script_safe_tokens(entry["ref_text"])
                hyp_tokens = script_safe_tokens(transcript)
                substitutions, deletions, insertions = edit_counts(ref_tokens, hyp_tokens)
                current_rss = rss_kib(process.pid)
                peak_rss = max(peak_rss, current_rss)
                results.append(
                    {
                        "id": entry["id"],
                        "sourceRow": entry.get("sourceRow"),
                        "audioSeconds": round(duration, 4),
                        "inferenceSeconds": round(elapsed, 4),
                        "rtf": round(elapsed / duration, 4),
                        "refText": entry["ref_text"],
                        "hypText": transcript,
                        "truncated": bool(response.get("truncated", False)),
                        "degenerateReason": degenerate_reason(transcript),
                        "refWords": len(ref_tokens),
                        "substitutions": substitutions,
                        "deletions": deletions,
                        "insertions": insertions,
                    }
                )
                print(f"  {index:>3}/{len(entries)} {entry['id']} {elapsed:.2f}s", flush=True)
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()

    reference_style = entries[0].get("referenceStyle", "unknown")
    totals = {
        key: sum(result[key] for result in results)
        for key in ("refWords", "substitutions", "deletions", "insertions")
    }
    total_errors = totals["substitutions"] + totals["deletions"] + totals["insertions"]
    total_audio = sum(result["audioSeconds"] for result in results)
    total_inference = sum(result["inferenceSeconds"] for result in results)
    return {
        "id": model["id"],
        "label": model["label"],
        "tier": model["tier"],
        "format": model["format"],
        "outputStyle": model["outputStyle"],
        "decode": model.get("decode", {"language": "auto"}),
        "referenceStyle": reference_style,
        "scoreComparable": model["outputStyle"] == reference_style,
        "modelPath": str(model_path),
        "modelBytes": model_path.stat().st_size,
        "startupSeconds": round(startup_seconds, 4),
        "loadedRssMiB": round(loaded_rss / 1024, 1),
        "peakRssMiB": round(peak_rss / 1024, 1),
        "samples": len(results),
        "emptySamples": sum(not result["hypText"].strip() for result in results),
        "truncatedSamples": sum(result["truncated"] for result in results),
        "degenerateSamples": sum(result["degenerateReason"] is not None for result in results),
        "audioSeconds": round(total_audio, 4),
        "inferenceSeconds": round(total_inference, 4),
        "rtf": round(total_inference / total_audio, 4),
        "speedup": round(total_audio / total_inference, 2),
        "scriptSafeWer": round(total_errors / totals["refWords"], 6) if totals["refWords"] else None,
        **totals,
        "results": results,
        "logPath": str(log_path),
    }


def print_summary(models: list[dict]) -> None:
    print("\nmodel                         WER       comparable  speed     peak RSS  trunc  degenerate")
    print("----------------------------  --------  ----------  --------  --------  -----  ----------")
    for model in models:
        wer = f"{model['scriptSafeWer'] * 100:6.2f}%" if model["scriptSafeWer"] is not None else "n/a"
        comparable = "yes" if model["scoreComparable"] else "NO"
        print(
            f"{model['id'][:28]:28}  {wer:8}  {comparable:10}  "
            f"{model['speedup']:6.1f}x  {model['peakRssMiB']:6.1f} MiB  "
            f"{model['truncatedSamples']:>5}  {model['degenerateSamples']:>10}"
        )
    print("\nWER is comparable only when model and reference output styles match.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", required=True, help="Comma-separated IDs from catalog.json")
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--catalog", type=Path, default=HERE / "catalog.json")
    parser.add_argument(
        "--server",
        type=Path,
        default=ROOT / "stt/transcribe-server/transcribe-server",
    )
    parser.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--max-samples", type=int)
    parser.add_argument("--port", type=int, default=18091)
    parser.add_argument("--no-warmup", action="store_true")
    args = parser.parse_args()

    catalog = load_catalog(args.catalog)
    requested = args.models.split(",")
    cache = args.cache.expanduser().resolve()
    server = args.server.expanduser().resolve()
    if not server.is_file() or not os.access(server, os.X_OK):
        parser.error(f"missing executable server {server}; run ./stt/transcribe-server/build.sh")
    header, entries = load_manifest(args.manifest.expanduser().resolve(), args.max_samples)

    report_models: list[dict] = []
    for model_id in requested:
        model = catalog.get(model_id)
        if not model:
            parser.error(f"unknown model {model_id!r}")
        model_path = resolve_model_path(model, cache)
        verify_model(model_path, model)
        print(f"\n{model['label']}\n  {model_path}")
        report_models.append(
            model_benchmark(model, model_path, entries, server, args.port, cache, not args.no_warmup)
        )

    report = {
        "schemaVersion": 1,
        "createdAt": datetime.now(timezone.utc).isoformat(),
        "host": host_info(),
        "corpus": header,
        "manifest": str(args.manifest.expanduser().resolve()),
        "models": report_models,
    }
    if args.out:
        output = args.out.expanduser().resolve()
    else:
        stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
        output = cache / "reports" / f"speech-{stamp}.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print_summary(report_models)
    print(f"\nreport: {output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
