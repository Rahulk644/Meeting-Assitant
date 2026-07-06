#!/usr/bin/env bash
# One-time setup: fetch the Whisper engine + model and build the native helpers.
set -e
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODEL_DIR="$ROOT/stt/whisper"
MODEL="$MODEL_DIR/ggml-large-v3-turbo-q8_0.bin"
MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin"

command -v swiftc >/dev/null || { echo "Xcode Command Line Tools required: xcode-select --install"; exit 1; }
command -v brew   >/dev/null || { echo "Homebrew required: https://brew.sh"; exit 1; }

echo "1/3  Whisper engine (whisper-server via Homebrew)…"
command -v whisper-server >/dev/null || brew install whisper-cpp

echo "2/3  Whisper model — large-v3-turbo q8_0, ~830 MB…"
mkdir -p "$MODEL_DIR"
[ -f "$MODEL" ] || curl -L --fail --progress-bar -o "$MODEL" "$MODEL_URL"

echo "3/3  Building native helpers (Swift)…"
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
