#!/usr/bin/env bash
# Build transcribe-server: a small HTTP wrapper around handy-computer/transcribe.cpp (pinned commit) that
# holds a Whisper GGUF model hot and serves POST /inference — a faster, lighter drop-in for whisper-server
# (≈half the RAM, ≈2x the speed on Apple Silicon, cleaner output). Called by ../../setup.sh.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
COMMIT="$(cat "$ROOT/PINNED_COMMIT")"
SRC="$ROOT/.transcribe.cpp"
REPO="https://github.com/handy-computer/transcribe.cpp"

if [ ! -e "$SRC/.git" ]; then
  echo "Fetching transcribe.cpp @ ${COMMIT:0:12} …"
  rm -rf "$SRC"; mkdir -p "$SRC"
  git -C "$SRC" init -q
  git -C "$SRC" remote add origin "$REPO"
  git -C "$SRC" fetch -q --depth 1 origin "$COMMIT"
  git -C "$SRC" checkout -q FETCH_HEAD
  git -C "$SRC" submodule update -q --init --recursive --depth 1
fi

# Drop our server in as an example target (its build already links the transcribe lib + ggml/Metal).
mkdir -p "$SRC/examples/server"
cp "$ROOT/transcribe-server.cpp" "$ROOT/httplib.h" "$SRC/examples/server/"
cat > "$SRC/examples/server/CMakeLists.txt" <<'CM'
add_executable(transcribe-server transcribe-server.cpp)
target_include_directories(transcribe-server PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})
find_package(Threads REQUIRED)
target_link_libraries(transcribe-server PRIVATE transcribe Threads::Threads)
CM
grep -q "add_subdirectory(server)" "$SRC/examples/CMakeLists.txt" || echo "add_subdirectory(server)" >> "$SRC/examples/CMakeLists.txt"

echo "Building transcribe-server (Metal) — first build compiles ggml, ~2–4 min …"
cmake -S "$SRC" -B "$SRC/build" -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "$SRC/build" --target transcribe-server -j >/dev/null
cp "$SRC/build/bin/transcribe-server" "$ROOT/transcribe-server"
codesign --force --sign - "$ROOT/transcribe-server" 2>/dev/null || true  # stable ad-hoc sig so TCC grants stick
echo "✓ transcribe-server → $ROOT/transcribe-server"
