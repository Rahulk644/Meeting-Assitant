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

# Never compile an arbitrary stale checkout from a reused release workspace. The integration files below
# are expected to be local, but the upstream commit itself must exactly match the audited pin.
ACTUAL_COMMIT="$(git -C "$SRC" rev-parse HEAD)"
if [[ "$ACTUAL_COMMIT" != "$COMMIT" ]]; then
  echo "transcribe.cpp checkout mismatch: expected $COMMIT, found $ACTUAL_COMMIT" >&2
  echo "Remove $SRC and rerun this script to fetch the pinned source." >&2
  exit 1
fi
git -C "$SRC" submodule status --recursive | awk '$1 ~ /^[-+U]/ { bad=1 } END { exit bad }' || {
  echo "transcribe.cpp submodule state does not match the pinned checkout" >&2
  exit 1
}

# The pinned upstream Qwen runtime caps output at 256 tokens, which truncates normal Hinglish utterances.
# Apply our narrow, reviewable patch fail-closed; a future upstream pin must either accept it cleanly or make
# the build stop for an explicit re-audit. Reverse-check keeps repeated local/release builds idempotent.
QWEN_PATCH="$ROOT/patches/qwen3-output-budget.patch"
if git -C "$SRC" apply --check "$QWEN_PATCH" 2>/dev/null; then
  git -C "$SRC" apply "$QWEN_PATCH"
elif git -C "$SRC" apply --reverse --check "$QWEN_PATCH" 2>/dev/null; then
  : # already applied in this checkout
else
  echo "Qwen output-budget patch no longer matches pinned transcribe.cpp" >&2
  exit 1
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
cmake -S "$SRC" -B "$SRC/build" -DCMAKE_BUILD_TYPE=Release -DTRANSCRIBE_BUILD_EXAMPLES=ON >/dev/null
cmake --build "$SRC/build" --target transcribe-server -j >/dev/null
cp "$SRC/build/bin/transcribe-server" "$ROOT/transcribe-server"
# Give the freshly linked local binary a valid code directory. Release/local app builds replace this with
# the configured stable identity and hardened runtime before the executable enters a bundle.
codesign --force --sign - "$ROOT/transcribe-server" 2>/dev/null || true
"$ROOT/transcribe-server" --selftest
echo "✓ transcribe-server → $ROOT/transcribe-server"
