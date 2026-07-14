# Speech model benchmark

This harness evaluates candidate speech models through the same native `transcribe-server` binary shipped
by My Assistant. It measures transcript accuracy, model startup, hot inference speed, and process RSS on the
machine running the test. Models and evaluation audio remain outside the repository under
`~/.cache/my-assistant-bench` by default.

Build the pinned native server once before benchmarking: `./stt/transcribe-server/build.sh`.

## Reproducibility and data boundaries

- `catalog.json` pins every runnable weight by Hugging Face revision, byte size, and SHA-256.
- CoSHE-500 is **CC-BY-NC-4.0, evaluation-only**. It must never be used for training or redistributed in a
  release artifact. The extractor records the dataset revision and source-row selection in the manifest.
- CoSHE references use natural mixed Devanagari/Latin script. Its WER is not comparable for a model designed
  to emit all-Roman text. Such runs remain useful for latency and qualitative review, and the report marks
  their score `scoreComparable: false`.
- The scorer preserves Devanagari combining marks. It does not use Whisper's normalizer, transliterate, or
  normalize numbers.
- Private recordings should be placed only in the benchmark cache. Reports can contain transcripts and are
  therefore local/private by default.

## Quick start

```sh
# Verify/download the two initial candidates and the already-installed baseline.
./script/benchmark_speech.sh prepare-models \
  whisper-turbo-q8 apex-hinglish-q8 qwen3-asr-base-q6

# Download the pinned public evaluation parquet and extract 24 deterministic clips.
./script/benchmark_speech.sh prepare-data

# Reproduce the pinned Srota Q6 conversion (large temporary files; needs 6 GB free).
./script/benchmark_speech.sh convert-srota

# Run all three models through the shipped native server.
./script/benchmark_speech.sh run \
  whisper-turbo-q8,apex-hinglish-q8,qwen3-asr-base-q6,srota-hinglish-q6

# Fast unit checks for script-safe scoring.
./script/benchmark_speech.sh test
```

`ASSISTANT_BENCH_CACHE=/another/private/path` changes the cache root. Pass `MANIFEST=/path/to/file.jsonl`
to the wrapper to evaluate a different corpus. A compatible manifest begins with one `corpus_header` JSON
record and then uses `id`, `audio`, `ref_text`, `language`, and `referenceStyle` per sample.

## Promotion gate

A model is not eligible for the product catalog until it:

1. passes the full 500-clip mixed-script evaluation or an equivalent style-matched Roman corpus;
2. passes consented clean, café, traffic, wind, fan, echo, overlapping-speaker, and silence fixtures;
3. records accuracy, hallucination rate, startup, real-time factor, peak memory, energy impact, and recovery;
4. has verified licensing and provenance for both upstream and converted weights; and
5. loads through the pinned production runtime with no benchmark-only code path.

The 24-clip default is an engineering smoke benchmark, not a publishable model-quality claim.
The first aggregate result and its product decision are documented in
[`docs/SPEECH_MODEL_BENCHMARK.md`](../../docs/SPEECH_MODEL_BENCHMARK.md). Raw reports contain transcripts and
must remain in the private cache; commit only aggregate evidence under `results/`.
