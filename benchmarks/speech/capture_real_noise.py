#!/usr/bin/env python3
"""Capture or import consented field-recorded speech fixtures into the private benchmark cache."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import uuid
import wave
from datetime import datetime, timezone
from pathlib import Path


DEFAULT_OUT = Path(
    os.environ.get(
        "ASSISTANT_REAL_NOISE_DIR", "~/.cache/my-assistant-bench/corpora/real-noise-captures"
    )
).expanduser()
DEFAULT_PROMPTS = Path(__file__).with_name("field_prompts.jsonl")
CONDITIONS = ("clean", "cafe", "wind", "traffic", "fan", "echo", "overlap", "silence")
PRIVATE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{1,63}\Z")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def wav_info(path: Path) -> tuple[float, int, int, int]:
    with wave.open(str(path), "rb") as audio:
        duration = audio.getnframes() / audio.getframerate()
        return duration, audio.getframerate(), audio.getnchannels(), audio.getsampwidth() * 8


def require_ffmpeg() -> str:
    executable = shutil.which("ffmpeg")
    if not executable:
        raise ValueError("ffmpeg is required (Homebrew: brew install ffmpeg)")
    return executable


def list_devices(_: argparse.Namespace) -> None:
    subprocess.run(
        [require_ffmpeg(), "-hide_banner", "-f", "avfoundation", "-list_devices", "true", "-i", ""],
        check=False,
    )


def validate_attestation(args: argparse.Namespace) -> None:
    if not args.attest_field_recorded:
        raise ValueError(
            "pass --attest-field-recorded only when speech and background were captured together in the named real environment"
        )
    if args.condition == "silence":
        if args.speaker_id != "none" or args.consent_id != "not-applicable":
            raise ValueError("silence fixtures use --speaker-id none --consent-id not-applicable")
    elif args.speaker_id in {"", "none"} or args.consent_id in {"", "not-applicable"}:
        raise ValueError("speech fixtures require pseudonymous speaker and consent IDs")
    if not PRIVATE_ID.fullmatch(args.speaker_id) or not PRIVATE_ID.fullmatch(args.consent_id):
        raise ValueError("speaker and consent IDs use only letters, numbers, dot, underscore, or hyphen")
    if not args.location_type.strip() or not args.device.strip():
        raise ValueError("--location-type and --device are required")


def resolve_prompt(args: argparse.Namespace) -> None:
    if not args.prompt_id or args.prompt_text:
        return
    with DEFAULT_PROMPTS.open(encoding="utf-8") as handle:
        prompts = {record["id"]: record for record in map(json.loads, handle)}
    if args.prompt_id not in prompts:
        raise ValueError(f"unknown prompt ID {args.prompt_id!r} in {DEFAULT_PROMPTS}")
    args.prompt_text = prompts[args.prompt_id]["text"]


def normalize_audio(source: Path, destination: Path) -> None:
    subprocess.run(
        [
            require_ffmpeg(),
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            str(source),
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            str(destination),
        ],
        check=True,
    )


def append_record(args: argparse.Namespace, wav: Path, clip_id: str) -> None:
    duration, rate, channels, bits = wav_info(wav)
    if rate != 16_000 or channels != 1 or bits != 16:
        raise RuntimeError("normalized fixture is not 16 kHz mono PCM16")
    if duration < 0.5:
        raise RuntimeError("fixture is shorter than 0.5 seconds")
    record = {
        "id": clip_id,
        "audio": str(wav.relative_to(args.out_dir)),
        "audioSha256": sha256(wav),
        "audioSeconds": round(duration, 4),
        "sampleRate": rate,
        "channels": channels,
        "bitsPerSample": bits,
        "condition": args.condition,
        "acousticOrigin": "field-recorded",
        "speakerId": args.speaker_id,
        "consentId": args.consent_id,
        "device": args.device,
        "locationType": args.location_type.strip(),
        "promptId": args.prompt_id.strip(),
        "promptText": args.prompt_text.strip(),
        "capturedAt": datetime.now(timezone.utc).isoformat(),
        "privacy": "no precise address; pseudonymous speaker and consent IDs",
    }
    manifest = args.out_dir / "captures.jsonl"
    with manifest.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(record, ensure_ascii=False) + "\n")
    print(wav)


def capture(args: argparse.Namespace) -> None:
    resolve_prompt(args)
    validate_attestation(args)
    if args.duration < 1 or args.duration > 180:
        raise ValueError("--duration must be between 1 and 180 seconds")
    args.out_dir = args.out_dir.expanduser().resolve()
    raw_dir = args.out_dir / "audio"
    raw_dir.mkdir(parents=True, exist_ok=True)
    clip_id = f"real-{args.condition}-{uuid.uuid4().hex[:12]}"
    destination = raw_dir / f"{clip_id}.wav"
    if args.prompt_text:
        print(f"Prompt ({args.prompt_id or 'unlabelled'}): {args.prompt_text}", flush=True)
    print(f"Recording {args.duration:.1f}s of {args.condition} from AVFoundation audio device {args.device}…", flush=True)
    try:
        subprocess.run(
            [
                require_ffmpeg(),
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "avfoundation",
                "-i",
                f":{args.device}",
                "-t",
                str(args.duration),
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                str(destination),
            ],
            check=True,
        )
    except subprocess.CalledProcessError:
        destination.unlink(missing_ok=True)
        raise
    try:
        append_record(args, destination, clip_id)
    except (OSError, RuntimeError, wave.Error):
        destination.unlink(missing_ok=True)
        raise


def import_audio(args: argparse.Namespace) -> None:
    resolve_prompt(args)
    validate_attestation(args)
    source = args.audio.expanduser().resolve()
    if not source.is_file():
        raise ValueError(f"missing input audio {source}")
    args.out_dir = args.out_dir.expanduser().resolve()
    raw_dir = args.out_dir / "audio"
    raw_dir.mkdir(parents=True, exist_ok=True)
    clip_id = f"real-{args.condition}-{uuid.uuid4().hex[:12]}"
    destination = raw_dir / f"{clip_id}.wav"
    try:
        normalize_audio(source, destination)
    except subprocess.CalledProcessError:
        destination.unlink(missing_ok=True)
        raise
    try:
        append_record(args, destination, clip_id)
    except (OSError, RuntimeError, wave.Error):
        destination.unlink(missing_ok=True)
        raise


def common_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--condition", choices=CONDITIONS, required=True)
    parser.add_argument("--speaker-id", required=True, help="Pseudonym, never a name or email")
    parser.add_argument("--consent-id", required=True, help="Reference to a separately stored consent record")
    parser.add_argument("--device", required=True, help="AVFoundation device index or capture-device label")
    parser.add_argument("--location-type", required=True, help="General type only; never a precise address")
    parser.add_argument("--prompt-id", default="")
    parser.add_argument("--prompt-text", default="")
    parser.add_argument("--attest-field-recorded", action="store_true")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    devices = subparsers.add_parser("list-devices")
    devices.set_defaults(func=list_devices)
    record = subparsers.add_parser("capture")
    common_arguments(record)
    record.add_argument("--duration", type=float, default=15.0)
    record.set_defaults(func=capture)
    imported = subparsers.add_parser("import")
    common_arguments(imported)
    imported.add_argument("--audio", type=Path, required=True)
    imported.set_defaults(func=import_audio)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    args.func(args)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, wave.Error, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
