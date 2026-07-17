#!/usr/bin/env bash
# Build the exact llama.cpp server used by Smart Polish. Models are downloaded separately at runtime.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMMIT="a935fbffe1a3d31509c325c116454ab5d56b2eb8" # upstream b9960
SOURCE="${LLAMA_SOURCE_DIR:-$ROOT/.build/llama.cpp-$COMMIT}"
BUILD="$SOURCE/build-smart-polish"
OUTPUT="$ROOT/polish-runtime/llama-server"

mkdir -p "$ROOT/.build" "$ROOT/polish-runtime"
if [[ -x "$OUTPUT" ]] \
  && "$OUTPUT" --version 2>&1 | grep -E "(9960|a935fbf)" >/dev/null \
  && ! otool -L "$OUTPUT" | grep -E "(/opt/homebrew|/usr/local)" >/dev/null; then
  echo "Pinned Smart Polish runtime is already built: $OUTPUT"
  exit 0
fi
if [[ ! -d "$SOURCE/.git" ]]; then
  TEMP="$ROOT/.build/llama.cpp-$COMMIT.tmp"
  rm -rf "$TEMP"
  git clone --filter=blob:none --no-checkout https://github.com/ggml-org/llama.cpp.git "$TEMP"
  git -C "$TEMP" checkout --detach "$COMMIT"
  mv "$TEMP" "$SOURCE"
fi

[[ "$(git -C "$SOURCE" rev-parse HEAD)" == "$COMMIT" ]] || {
  echo "llama.cpp source is not at the pinned Smart Polish commit" >&2
  exit 1
}

cmake -S "$SOURCE" -B "$BUILD" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF \
  -DLLAMA_BUILD_SERVER=ON \
  -DLLAMA_CURL=OFF \
  -DLLAMA_OPENSSL=OFF \
  -DGGML_METAL=ON \
  -DGGML_NATIVE=OFF
cmake --build "$BUILD" --config Release --target llama-server --parallel "$(sysctl -n hw.logicalcpu)"

install -m 755 "$BUILD/bin/llama-server" "$OUTPUT"
"$OUTPUT" --version 2>&1 | grep -E "(9960|a935fbf)" >/dev/null
if otool -L "$OUTPUT" | grep -E "(/opt/homebrew|/usr/local)" >/dev/null; then
  echo "Smart Polish runtime has a non-system dynamic dependency" >&2
  exit 1
fi
echo "Built pinned Smart Polish runtime: $OUTPUT"
