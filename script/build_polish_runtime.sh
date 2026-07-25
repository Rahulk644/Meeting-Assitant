#!/usr/bin/env bash
# Build the exact llama.cpp server used by Smart Polish. Models are downloaded separately at runtime.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMMIT="a935fbffe1a3d31509c325c116454ab5d56b2eb8" # upstream b9960
SOURCE="${LLAMA_SOURCE_DIR:-$ROOT/.build/llama.cpp-$COMMIT}"
BUILD="$SOURCE/build-smart-polish"

# Device-agnostic: pick the GPU backend, executable suffix, and dylib-audit tool per OS. macOS uses
# Metal; Linux/Windows honor GGML_BACKEND=cuda|vulkan|cpu (default cpu, which is always available).
OS="$(uname -s)"
EXE=""
case "$OS" in
  MINGW* | MSYS* | CYGWIN*) EXE=".exe" ;;
esac
OUTPUT="$ROOT/polish-runtime/llama-server$EXE"

backend_flags() {
  case "$OS" in
    Darwin) echo "-DGGML_METAL=ON" ;;
    *)
      case "${GGML_BACKEND:-cpu}" in
        cuda) echo "-DGGML_CUDA=ON" ;;
        vulkan) echo "-DGGML_VULKAN=ON" ;;
        *) echo "-DGGML_METAL=OFF" ;;
      esac
      ;;
  esac
}

# A self-contained static binary should link nothing from a package prefix. otool on macOS, ldd on
# Linux; skip on Windows (dumpbin is not always on PATH and the MSVC runtime is expected).
foreign_libs() {
  case "$OS" in
    Darwin) otool -L "$1" 2>/dev/null | grep -E "(/opt/homebrew|/usr/local)" || true ;;
    Linux) ldd "$1" 2>/dev/null | grep -E "(/opt/homebrew|/usr/local|/home)" || true ;;
    *) : ;;
  esac
}

mkdir -p "$ROOT/.build" "$ROOT/polish-runtime"
if [[ -x "$OUTPUT" ]] \
  && "$OUTPUT" --version 2>&1 | grep -E "(9960|a935fbf)" >/dev/null \
  && [[ -z "$(foreign_libs "$OUTPUT")" ]]; then
  echo "Pinned Smart Polish runtime is already built: $OUTPUT"
  exit 0
fi
if [[ ! -d "$SOURCE/.git" ]]; then
  TEMP="$ROOT/.build/llama.cpp-$COMMIT.tmp"
  rm -rf "$TEMP"
  echo "Fetching llama.cpp @ ${COMMIT:0:12} (shallow, pinned commit) …"
  # Shallow single-commit fetch: pulls only the pinned commit's tree and blobs, not the full history — far
  # smaller and far more resilient on a slow link than a blobless promisor clone, which fetches blobs lazily
  # afterward and dies mid-checkout on a dropped connection. Force HTTP/1.1 to sidestep the "HTTP/2 stream
  # not closed cleanly: CANCEL" resets GitHub throws on flaky networks, and retry before giving up.
  git init -q "$TEMP"
  git -C "$TEMP" remote add origin https://github.com/ggml-org/llama.cpp.git
  attempt=1
  until git -C "$TEMP" -c http.version=HTTP/1.1 fetch -q --depth 1 origin "$COMMIT"; do
    if (( attempt >= 3 )); then
      echo "Failed to fetch pinned llama.cpp commit after $attempt attempts" >&2
      rm -rf "$TEMP"
      exit 1
    fi
    echo "Fetch attempt $attempt failed; retrying in $((attempt * 5))s …" >&2
    sleep $((attempt * 5))
    attempt=$((attempt + 1))
  done
  git -C "$TEMP" checkout -q --detach FETCH_HEAD
  mv "$TEMP" "$SOURCE"
fi

[[ "$(git -C "$SOURCE" rev-parse HEAD)" == "$COMMIT" ]] || {
  echo "llama.cpp source is not at the pinned Smart Polish commit" >&2
  exit 1
}

# Self-heal a build dir configured for a different source path (e.g. built in a container at /src then
# reused on the host, or vice-versa) — cmake refuses to reuse such a cache and errors out.
if [ -f "$BUILD/CMakeCache.txt" ] && ! grep -q "CMAKE_HOME_DIRECTORY:INTERNAL=$SOURCE$" "$BUILD/CMakeCache.txt"; then
  echo "Clearing cmake build dir configured for a different source path"
  rm -rf "$BUILD"
fi
cmake -S "$SOURCE" -B "$BUILD" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF \
  -DLLAMA_BUILD_SERVER=ON \
  -DLLAMA_CURL=OFF \
  -DLLAMA_OPENSSL=OFF \
  $(backend_flags) \
  -DGGML_NATIVE=OFF
JOBS="$(sysctl -n hw.logicalcpu 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || echo 4)"
cmake --build "$BUILD" --config Release --target llama-server --parallel "$JOBS"

install -m 755 "$BUILD/bin/llama-server$EXE" "$OUTPUT"
"$OUTPUT" --version 2>&1 | grep -E "(9960|a935fbf)" >/dev/null
if [[ -n "$(foreign_libs "$OUTPUT")" ]]; then
  echo "Smart Polish runtime has a non-system dynamic dependency" >&2
  exit 1
fi
echo "Built pinned Smart Polish runtime: $OUTPUT"
