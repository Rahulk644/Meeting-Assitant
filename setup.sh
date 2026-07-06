#!/usr/bin/env bash
# One-time setup: fetch the Whisper engine + model and build the native helpers.
set -e
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODEL_DIR="$ROOT/stt/whisper"
MODEL="$MODEL_DIR/ggml-large-v3-turbo-q8_0.bin"
MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin"

command -v swiftc >/dev/null || { echo "Xcode Command Line Tools required: xcode-select --install"; exit 1; }
command -v brew   >/dev/null || { echo "Homebrew required: https://brew.sh"; exit 1; }

echo "1/4  Speech + LLM engines (whisper-server, llama-server via Homebrew)…"
command -v whisper-server >/dev/null || brew install whisper-cpp
command -v llama-server   >/dev/null || brew install llama.cpp

echo "2/4  Whisper model — large-v3-turbo q8_0, ~830 MB…"
mkdir -p "$MODEL_DIR"
[ -f "$MODEL" ] || curl -L --fail --progress-bar -o "$MODEL" "$MODEL_URL"

echo "3/4  Post-call AI model — Gemma 4 E2B q4_0 (~3.1 GB, on-device summaries & 'ask')…"
LLM_DIR="$ROOT/stt/llm"; mkdir -p "$LLM_DIR"
LLM_MODEL="$LLM_DIR/gemma-4-E2B_q4_0-it.gguf"
[ -f "$LLM_MODEL" ] || curl -L --fail --progress-bar -o "$LLM_MODEL" \
  "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/resolve/main/gemma-4-E2B_q4_0-it.gguf"

echo "4/4  Building native helpers (Swift)…"
( cd "$ROOT/stt" && swiftc -O -target arm64-apple-macos13.0 meeting-tap.swift -o meeting-tap \
    -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia \
    -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker Info.plist )
( cd "$ROOT/meeting-state" && swiftc -O meeting-state.swift -o meeting-state )

cat <<'EOF'

✅ Setup complete.

One-time macOS permissions (grant to your Terminal, in System Settings → Privacy & Security):
  • Screen & System Audio Recording   (to hear the other participants)
  • Accessibility                     (for the speaker-names layer, when enabled)

Then, from your Terminal:   ./run.sh   →   open http://localhost:8790
EOF
