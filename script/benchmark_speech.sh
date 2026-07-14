#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_DIR="$ROOT_DIR/benchmarks/speech"
CACHE_DIR="${ASSISTANT_BENCH_CACHE:-$HOME/.cache/my-assistant-bench}"
DATASET_DIR="$CACHE_DIR/datasets/coshe-500"
CORPUS_DIR="$CACHE_DIR/corpora/coshe-500"
FULL_CORPUS_DIR="$CACHE_DIR/corpora/coshe-500-full"

usage() {
  echo "usage: $0 prepare-models MODEL... | prepare-data | prepare-full-data | prepare-english-data | prepare-translation-data | prepare-noise-data | create-roman-pack [OUT] | capture-real ARGS... | create-real-pack [OUT] | convert-whisper MODEL | convert-srota | run MODEL,MODEL... | run-matrix MODEL,MODEL... | test" >&2
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
  prepare-full-data)
    mkdir -p "$DATASET_DIR"
    hf download Trelis/CoSHE-500 \
      data/eval-00000-of-00001.parquet README.md \
      --repo-type dataset \
      --revision 3da116bac98bfe5349f9b454d95a17a43483e1e8 \
      --local-dir "$DATASET_DIR"
    uv run "$BENCH_DIR/extract_coshe.py" \
      --parquet "$DATASET_DIR/data/eval-00000-of-00001.parquet" \
      --out-dir "$FULL_CORPUS_DIR" \
      --limit 0
    ;;
  prepare-translation-data)
    fleurs_dir="$CACHE_DIR/datasets/fleurs"
    mkdir -p "$fleurs_dir"
    hf download google/fleurs \
      parquet-data/hi_in/validation-00000-of-00001.parquet data/hi_in/dev.tsv data/en_us/dev.tsv README.md \
      --repo-type dataset \
      --revision 70bb2e84b976b7e960aa89f1c648e09c59f894dd \
      --local-dir "$fleurs_dir"
    uv run "$BENCH_DIR/extract_fleurs_translation.py" \
      --parquet "$fleurs_dir/parquet-data/hi_in/validation-00000-of-00001.parquet" \
      --english-tsv "$fleurs_dir/data/en_us/dev.tsv" \
      --out-dir "$CACHE_DIR/corpora/fleurs-hi-en"
    ;;
  prepare-english-data)
    fleurs_dir="$CACHE_DIR/datasets/fleurs"
    mkdir -p "$fleurs_dir"
    hf download google/fleurs \
      parquet-data/en_us/validation-00000-of-00001.parquet README.md \
      --repo-type dataset \
      --revision 70bb2e84b976b7e960aa89f1c648e09c59f894dd \
      --local-dir "$fleurs_dir"
    uv run "$BENCH_DIR/extract_fleurs_asr.py" \
      --parquet "$fleurs_dir/parquet-data/en_us/validation-00000-of-00001.parquet" \
      --out-dir "$CACHE_DIR/corpora/fleurs-en-asr"
    ;;
  prepare-noise-data)
    source_manifest="$CORPUS_DIR/coshe-500-seed42-n24.jsonl"
    if [[ ! -f "$source_manifest" ]]; then
      echo "missing $source_manifest; run prepare-data first" >&2
      exit 1
    fi
    for condition in wind babble; do
      noisy_dir="$CACHE_DIR/corpora/coshe-${condition}-5db"
      product_dir="$CACHE_DIR/corpora/coshe-${condition}-5db-product-vad"
      uv run "$BENCH_DIR/add_noise.py" "$source_manifest" "$noisy_dir" \
        --condition "$condition" --snr-db 5 --seed 42 --max-samples 12
      cargo run --quiet --manifest-path "$ROOT_DIR/desktop/src-tauri/Cargo.toml" \
        --example preprocess_speech_manifest -- \
        "$noisy_dir/coshe-500-seed42-n24-${condition}-5db.jsonl" "$product_dir"
    done
    ;;
  create-roman-pack)
    source_manifest="$FULL_CORPUS_DIR/coshe-500-seed42-n500.jsonl"
    if [[ ! -f "$source_manifest" ]]; then
      echo "missing $source_manifest; run prepare-full-data first" >&2
      exit 1
    fi
    out="${1:-$CACHE_DIR/annotation-packs/human-roman-coshe-500}"
    python3 "$BENCH_DIR/corpus.py" create-roman-pack \
      --source-manifest "$source_manifest" --out-dir "$out" --count 500 --seed 42
    echo "$out/annotate.html"
    ;;
  capture-real)
    python3 "$BENCH_DIR/capture_real_noise.py" "$@"
    ;;
  create-real-pack)
    captures="$CACHE_DIR/corpora/real-noise-captures/captures.jsonl"
    if [[ ! -f "$captures" ]]; then
      echo "missing $captures; use capture-real first" >&2
      exit 1
    fi
    out="${1:-$CACHE_DIR/annotation-packs/real-noise}"
    python3 "$BENCH_DIR/corpus.py" create-real-pack --captures "$captures" --out-dir "$out"
    echo "$out/annotate.html"
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
  convert-whisper)
    model="${1:-}"
    if [[ -z "$model" ]]; then usage; exit 2; fi
    python3 "$BENCH_DIR/convert_whisper.py" "$model" --cache "$CACHE_DIR"
    ;;
  run)
    models="${1:-}"
    if [[ -z "$models" ]]; then usage; exit 2; fi
    manifest="${MANIFEST:-$CORPUS_DIR/coshe-500-seed42-n24.jsonl}"
    python3 "$BENCH_DIR/run.py" --models "$models" --manifest "$manifest" --cache "$CACHE_DIR"
    ;;
  run-matrix)
    models="${1:-}"
    if [[ -z "$models" ]]; then usage; exit 2; fi
    manifest="${MANIFEST:-$FULL_CORPUS_DIR/coshe-500-seed42-n500.jsonl}"
    if [[ ! -f "$manifest" ]]; then
      echo "missing $manifest; run prepare-full-data first" >&2
      exit 1
    fi
    python3 "$BENCH_DIR/run_matrix.py" \
      --models "$models" --manifest "$manifest" --cache "$CACHE_DIR"
    ;;
  test)
    python3 -m unittest discover -s "$BENCH_DIR" -p 'test_*.py'
    python3 -m unittest discover -s "$ROOT_DIR/acceptance" -p 'test_*.py'
    ;;
  *)
    usage
    exit 2
    ;;
esac
