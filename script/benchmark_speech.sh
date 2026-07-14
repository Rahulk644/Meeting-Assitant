#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_DIR="$ROOT_DIR/benchmarks/speech"
CACHE_DIR="${ASSISTANT_BENCH_CACHE:-$HOME/.cache/my-assistant-bench}"
DATASET_DIR="$CACHE_DIR/datasets/coshe-500"
CORPUS_DIR="$CACHE_DIR/corpora/coshe-500"

usage() {
  echo "usage: $0 prepare-models MODEL... | prepare-data | convert-srota | run MODEL,MODEL... | test" >&2
}

command="${1:-}"
if [[ -z "$command" ]]; then
  usage
  exit 2
fi
shift

case "$command" in
  prepare-models)
    if [[ "$#" -eq 0 ]]; then usage; exit 2; fi
    python3 "$BENCH_DIR/prepare_models.py" --cache "$CACHE_DIR" "$@"
    ;;
  prepare-data)
    mkdir -p "$DATASET_DIR"
    hf download Trelis/CoSHE-500 \
      data/eval-00000-of-00001.parquet README.md \
      --repo-type dataset \
      --revision 3da116bac98bfe5349f9b454d95a17a43483e1e8 \
      --local-dir "$DATASET_DIR"
    uv run "$BENCH_DIR/extract_coshe.py" \
      --parquet "$DATASET_DIR/data/eval-00000-of-00001.parquet" \
      --out-dir "$CORPUS_DIR"
    ;;
  convert-srota)
    source_dir="$CACHE_DIR/sources/srota"
    output_dir="$CACHE_DIR/models/srota-hinglish-q6"
    bf16="$output_dir/srota-hinglish-BF16.gguf"
    q6="$output_dir/srota-hinglish-Q6_K.gguf"
    mkdir -p "$source_dir" "$output_dir"
    hf download moorlee/qwen3-asr-0.6b-hinglish \
      --revision 188ae2b61f932feb998e3b3c7a868e927a467031 \
      --local-dir "$source_dir" \
      --exclude 'figures/*' --exclude 'predictions_*' --exclude 'trainer_state.json'
    [[ "$(shasum -a 256 "$source_dir/model.safetensors" | awk '{print $1}')" == \
      "87dfcb5780623483746dddf5121ec0f324e7bf40b1938628b0102d868eb330a0" ]]
    uv run --project "$ROOT_DIR/stt/transcribe-server/.transcribe.cpp/scripts/envs/qwen3_asr" \
      "$ROOT_DIR/stt/transcribe-server/.transcribe.cpp/scripts/convert-qwen3_asr.py" \
      "$source_dir" "$bf16" \
      --repo-id moorlee/qwen3-asr-0.6b-hinglish \
      --variant qwen3-asr-0.6b
    cmake -S "$ROOT_DIR/stt/transcribe-server/.transcribe.cpp" \
      -B "$ROOT_DIR/stt/transcribe-server/.transcribe.cpp/build" \
      -DTRANSCRIBE_BUILD_TOOLS=ON -DTRANSCRIBE_BUILD_EXAMPLES=ON -DGGML_METAL=ON
    cmake --build "$ROOT_DIR/stt/transcribe-server/.transcribe.cpp/build" \
      --target transcribe-quantize -j 4
    "$ROOT_DIR/stt/transcribe-server/.transcribe.cpp/build/bin/transcribe-quantize" \
      "$bf16" "$q6" --quant Q6_K
    [[ "$(stat -f '%z' "$q6")" == "690417824" ]]
    [[ "$(shasum -a 256 "$q6" | awk '{print $1}')" == \
      "b0e6b1b81bfb2e278431eae81d8ec3fb4fbb7cae26c801f10bd31ec80cd03ac5" ]]
    rm -f "$bf16"
    echo "$q6"
    ;;
  run)
    models="${1:-}"
    if [[ -z "$models" ]]; then usage; exit 2; fi
    manifest="${MANIFEST:-$CORPUS_DIR/coshe-500-seed42-n24.jsonl}"
    python3 "$BENCH_DIR/run.py" --models "$models" --manifest "$manifest" --cache "$CACHE_DIR"
    ;;
  test)
    python3 -m unittest "$BENCH_DIR/test_run.py"
    ;;
  *)
    usage
    exit 2
    ;;
esac
