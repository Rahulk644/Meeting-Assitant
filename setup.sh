#!/usr/bin/env bash
# One-time setup: fetch the Whisper engine + model and build the native helpers.
set -e
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODEL_DIR="$ROOT/stt/whisper"
MODEL="$MODEL_DIR/ggml-large-v3-turbo-q8_0.bin"
MODEL_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin"
VAD="$MODEL_DIR/ggml-silero-v5.1.2.bin"
VAD_URL="https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin"

command -v swiftc >/dev/null || { echo "Xcode Command Line Tools required: xcode-select --install"; exit 1; }
command -v brew   >/dev/null || { echo "Homebrew required: https://brew.sh"; exit 1; }

echo "1/4  Speech engine (whisper-server via Homebrew)…"
command -v whisper-server >/dev/null || brew install whisper-cpp

echo "2/4  Whisper model — large-v3-turbo q8_0, ~830 MB…"
mkdir -p "$MODEL_DIR"
[ -f "$MODEL" ] || curl -L --fail --progress-bar -o "$MODEL" "$MODEL_URL"
[ -f "$VAD" ]   || curl -L --fail --progress-bar -o "$VAD"   "$VAD_URL"   # Silero VAD — anti-hallucination pre-filter

echo "3/4  Post-call AI — LiteRT-LM + Gemma 4 E2B (text-only; effective-2B ≈ 1 GB RAM, 2.4 GB disk)…"
command -v uv >/dev/null || brew install uv
LITERT="$(command -v litert-lm || echo "$HOME/.local/bin/litert-lm")"
[ -x "$LITERT" ] || uv tool install litert-lm
"$LITERT" list 2>/dev/null | grep -q gemma4-e2b || \
  "$LITERT" import --from-huggingface-repo litert-community/gemma-4-E2B-it-litert-lm gemma-4-E2B-it.litertlm gemma4-e2b

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
