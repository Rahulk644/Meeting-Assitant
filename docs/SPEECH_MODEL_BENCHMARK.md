# Hinglish speech-model benchmark

**Status:** engineering smoke result, not a release-quality model claim

**Measured:** 14 July 2026 on an Apple M1 Mac with 8 GB memory and macOS 26.5

**Machine-readable evidence:**
[`benchmarks/speech/results/2026-07-14-m1-smoke.json`](../benchmarks/speech/results/2026-07-14-m1-smoke.json)

## Decision

Keep Whisper Large v3 Turbo Q8 as the shipped default for now. Qwen3-ASR 0.6B Base Q6 is the strongest
mixed-script Hinglish candidate in this smoke test, but it truncated 8 of 24 clips at the native runtime's
current output-token limit. Do not expose a model picker or silently change Automatic until that limit is
fixed and the candidate passes the full and noisy-audio gates.

- **Advance:** Qwen3-ASR Base Q6, after the truncation/chunking fix. It scored 30.85% script-safe WER.
- **Do not use forced Hindi for the current Whisper:** it regressed from 59.92% to 75.23% WER and produced
  many repetition loops. Automatic language detection remains safer for Hinglish.
- **Do not promote Srota as the Hinglish default:** Q6 scored 46.73% WER with substantially more deletions.
  A BF16 control scored 46.96%, so quantization or conversion did not cause the regression.
- **Do not rank Apex yet:** it emits Roman Hinglish while CoSHE uses natural mixed Devanagari/Latin
  references. Its raw WER is not comparable. It needs a human-reviewed Roman-reference corpus.
- **Defer Trelis preview conversion:** its source weights are about 6.2 GB before conversion. The current
  machine did not have a safe amount of free disk for source, intermediate, and quantized copies together.

## Measurements

The 24 deterministic clips contain 614.04 seconds of natural conversational Hinglish. Each model received
one warm-up request and then ran through the exact pinned `transcribe-server` used by the product.

| Candidate | Comparable WER | Speed vs real time | Peak server RSS | Truncated | Degenerate | Outcome |
|---|---:|---:|---:|---:|---:|---|
| Whisper Turbo Q8, auto | 59.92% | 8.49x | 1,038.0 MiB | 0/24 | 1/24 | shipped baseline |
| Whisper Turbo Q8, forced Hindi | 75.23% | 6.61x | 962.2 MiB | 0/24 | 10/24 | reject configuration |
| Apex Hinglish Q8 | not comparable | 9.41x | 938.4 MiB | 0/24 | 1/24 | build Roman test set |
| Qwen3-ASR Base Q6 | **30.85%** | 8.77x | 1,186.7 MiB | **8/24** | 0/24 | best candidate; blocked |
| Srota Hinglish Q6 | 46.73% | **10.47x** | 1,216.9 MiB | 5/24 | 0/24 | do not promote |
| Srota Hinglish BF16 control | 46.96% | 7.01x | 2,406.6 MiB | 6/24 | 0/24 | diagnostic only |

WER preserves Indic combining marks, does not transliterate, and is comparable only when output and
reference script styles match. RSS is the native server process, not total app memory. The very first Metal
launch took about 12.9 seconds while pipelines compiled; subsequent warm starts were about 0.5–1.4 seconds.

## Production defects exposed and fixed

The benchmark found two native-wrapper problems rather than hiding them in the scorer:

1. `transcribe.cpp` can return a valid partial transcript with `TRANSCRIBE_ERR_OUTPUT_TRUNCATED`. The wrapper
   previously discarded that text and returned an empty result. It now keeps the partial transcript and marks
   the response `"truncated": true`.
2. A byte-limited partial result can end inside a UTF-8 code point. JSON escaping now validates UTF-8,
   preserves valid Hindi text, and replaces malformed trailing bytes with U+FFFD rather than returning
   invalid JSON.

The native build now runs a self-test covering Hindi, JSON metacharacters, and malformed trailing UTF-8.
These source fixes require a fresh app/DMG build before they exist in a distributable artifact.

## What this benchmark does not prove

CoSHE is conversational but is not a controlled noise suite. This result does not yet answer café, wind,
traffic, fan, room echo, overlapping speaker, or microphone-distance performance. It also does not represent
all Indian accents, age groups, devices, or regional code-switching patterns. The CC-BY-NC-4.0 dataset is
evaluation-only and must never be used for training or included in a release.

## Promotion gate and next engineering work

1. Raise or segment the Qwen output budget without losing words; require zero truncations on this set.
2. Run all candidates on the full 500-clip pinned CoSHE set, three times, and report variance.
3. Build a consented product corpus with clean and controlled café, traffic, wind, fan, echo, overlap,
   silence, short dictation, and long meeting fixtures.
4. Add a human-reviewed Roman Hinglish reference track before scoring Apex or another Roman-output model.
5. Test adaptive noise cleanup as a separate A/B layer; never force denoising on clean audio without proof.
6. Verify model and dataset licensing, provenance, runtime compatibility, and memory on 8/16/32 GB Macs.
7. Only then implement the signed model catalog, resumable downloader, hardware routing, removal, rollback,
   and user-facing Automatic / On this Mac / Cloud Speech picker.

## Reproduce

Use [`benchmarks/speech/README.md`](../benchmarks/speech/README.md). Models, audio, logs, and transcript-bearing
reports stay under `~/.cache/my-assistant-bench` by default; only aggregate, non-transcript evidence belongs
in Git.
