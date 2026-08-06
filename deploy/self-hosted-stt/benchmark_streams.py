#!/usr/bin/env python3
"""Measure a self-hosted worker's named streams without storing or printing transcript text."""

from __future__ import annotations

import argparse
import json
import statistics
import threading
import time
import urllib.parse
import urllib.request
import wave
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class StreamMetrics:
    session: str
    audio_seconds: float
    request_ms: list[float] = field(default_factory=list)
    max_schedule_lag_ms: float = 0.0
    final_ms: float = 0.0


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * fraction)))
    return ordered[index]


def read_wav(path: Path) -> tuple[bytes, int]:
    with wave.open(str(path), "rb") as audio:
        if audio.getnchannels() != 1 or audio.getsampwidth() != 2 or audio.getframerate() != 16_000:
            raise ValueError(f"{path}: expected 16 kHz mono signed-16 WAV")
        if audio.getcomptype() != "NONE":
            raise ValueError(f"{path}: compressed WAV is unsupported")
        frames = audio.readframes(audio.getnframes())
        return frames, audio.getnframes()


class Client:
    def __init__(self, base_url: str, token: str, timeout: float) -> None:
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.timeout = timeout

    def post(self, path: str, session: str, body: bytes = b"", **params: str) -> tuple[dict, float]:
        query = {"session": session, **params}
        url = f"{self.base_url}{path}?{urllib.parse.urlencode(query)}"
        request = urllib.request.Request(
            url,
            data=body,
            method="POST",
            headers={
                "Authorization": f"Bearer {self.token}",
                "Content-Type": "application/octet-stream",
            },
        )
        started = time.monotonic()
        with urllib.request.urlopen(request, timeout=self.timeout) as response:
            payload = json.loads(response.read())
        return payload, (time.monotonic() - started) * 1000.0


def run_stream(
    client: Client,
    barrier: threading.Barrier,
    session: str,
    pcm: bytes,
    frames: int,
    chunk_ms: int,
    language: str,
    realtime: bool,
) -> StreamMetrics:
    client.post("/stream/begin", session, language=language)
    samples_per_chunk = 16_000 * chunk_ms // 1000
    bytes_per_chunk = samples_per_chunk * 2
    metrics = StreamMetrics(session=session, audio_seconds=frames / 16_000.0)
    barrier.wait()
    schedule_start = time.monotonic()
    for index, offset in enumerate(range(0, len(pcm), bytes_per_chunk)):
        if realtime:
            target = schedule_start + index * chunk_ms / 1000.0
            remaining = target - time.monotonic()
            if remaining > 0:
                time.sleep(remaining)
            metrics.max_schedule_lag_ms = max(
                metrics.max_schedule_lag_ms,
                max(0.0, (time.monotonic() - target) * 1000.0),
            )
        _, elapsed = client.post("/stream/feed", session, pcm[offset : offset + bytes_per_chunk])
        metrics.request_ms.append(elapsed)
    _, metrics.final_ms = client.post("/stream/finalize", session)
    return metrics


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default="http://127.0.0.1:18091")
    parser.add_argument("--token-file", type=Path, required=True)
    parser.add_argument("--wav", type=Path, action="append", required=True)
    parser.add_argument("--streams", type=int, choices=(1, 2), default=1)
    parser.add_argument("--language", default="en-US")
    parser.add_argument("--chunk-ms", type=int, choices=(560, 1120), default=560)
    parser.add_argument("--mode", choices=("max", "realtime"), default="max")
    parser.add_argument("--timeout", type=float, default=30.0)
    args = parser.parse_args()

    token = args.token_file.read_text(encoding="utf-8").strip()
    if len(token) < 32 or any(character.isspace() for character in token):
        raise ValueError("token file must contain one non-whitespace token of at least 32 characters")
    wavs = [read_wav(path) for path in args.wav]
    while len(wavs) < args.streams:
        wavs.append(wavs[-1])

    client = Client(args.base_url, token, args.timeout)
    barrier = threading.Barrier(args.streams)
    results: list[StreamMetrics | None] = [None] * args.streams
    failures: list[BaseException] = []

    def worker(index: int) -> None:
        try:
            pcm, frames = wavs[index]
            results[index] = run_stream(
                client,
                barrier,
                f"benchmark-{index + 1}",
                pcm,
                frames,
                args.chunk_ms,
                args.language,
                args.mode == "realtime",
            )
        except BaseException as error:  # surface worker failures on the main thread
            failures.append(error)
            try:
                barrier.abort()
            except threading.BrokenBarrierError:
                pass

    started = time.monotonic()
    threads = [threading.Thread(target=worker, args=(index,)) for index in range(args.streams)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    wall_seconds = time.monotonic() - started
    if failures:
        raise failures[0]

    completed = [result for result in results if result is not None]
    request_times = [value for result in completed for value in result.request_ms]
    aggregate_audio = sum(result.audio_seconds for result in completed)
    report = {
        "mode": args.mode,
        "streams": args.streams,
        "chunkMs": args.chunk_ms,
        "wallSeconds": round(wall_seconds, 3),
        "aggregateAudioSeconds": round(aggregate_audio, 3),
        "aggregateRealtimeFactor": round(wall_seconds / aggregate_audio, 4),
        "aggregateThroughputXRealtime": round(aggregate_audio / wall_seconds, 3),
        "feedRequestMs": {
            "mean": round(statistics.fmean(request_times), 2),
            "p50": round(percentile(request_times, 0.50), 2),
            "p95": round(percentile(request_times, 0.95), 2),
            "max": round(max(request_times), 2),
        },
        "maxScheduleLagMs": round(max(result.max_schedule_lag_ms for result in completed), 2),
        "finalizeMs": [round(result.final_ms, 2) for result in completed],
    }
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
