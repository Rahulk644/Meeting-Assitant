#!/usr/bin/env bash
# One-time setup: build the transcription engine + model, the post-call LLM, and the native helpers.
set -e
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODEL_DIR="$ROOT/stt/whisper"
GGUF="$MODEL_DIR/whisper-large-v3-turbo-Q8_0.gguf"
GGUF_URL="https://huggingface.co/handy-computer/whisper-large-v3-turbo-gguf/resolve/main/whisper-large-v3-turbo-Q8_0.gguf"

command -v swiftc >/dev/null || { echo "Xcode Command Line Tools required: xcode-select --install"; exit 1; }
command -v brew   >/dev/null || { echo "Homebrew required: https://brew.sh"; exit 1; }
command -v cmake  >/dev/null || brew install cmake   # transcribe.cpp builds with CMake
command -v git    >/dev/null || { echo "git required"; exit 1; }

echo "1/3  Transcription engine — transcribe.cpp (built from source, Metal GPU) + large-v3-turbo q8 model, ~886 MB…"
mkdir -p "$MODEL_DIR"
[ -f "$GGUF" ] || curl -L --fail --progress-bar -o "$GGUF" "$GGUF_URL"   # GGUF for transcribe.cpp
"$ROOT/stt/transcribe-server/build.sh"                                    # → stt/transcribe-server/transcribe-server
# Fallback engine (optional, auto-used only if present): whisper.cpp —
#   brew install whisper-cpp && curl -L -o "$MODEL_DIR/ggml-large-v3-turbo-q8_0.bin" \
#     https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin

echo "2/3  Post-call AI — LiteRT-LM + Gemma 4 E2B (text-only; effective-2B ≈ 1 GB RAM, 2.4 GB disk)…"
command -v uv >/dev/null || brew install uv
LITERT="$(command -v litert-lm || echo "$HOME/.local/bin/litert-lm")"
[ -x "$LITERT" ] || uv tool install litert-lm
"$LITERT" list 2>/dev/null | grep -q gemma4-e2b || \
  "$LITERT" import --from-huggingface-repo litert-community/gemma-4-E2B-it-litert-lm gemma-4-E2B-it.litertlm gemma4-e2b

echo "3/3  Building native helpers (Swift)…"
( cd "$ROOT/stt" && swiftc -O -target arm64-apple-macos13.0 meeting-tap.swift -o meeting-tap \
    -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia \
    -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker Info.plist )
( cd "$ROOT/meeting-state" && swiftc -O meeting-state.swift -o meeting-state )
# swiftc emits a "linker-signed" ad-hoc signature, which macOS will NOT persist a TCC grant to — the app
# would re-prompt for Screen Recording / Mic / Accessibility on every launch. Re-sign both helpers with a
# PROPER ad-hoc signature (flags 0x2, not 0x20002) so grants stick. ALWAYS do this after rebuilding them.
codesign --force --sign - "$ROOT/stt/meeting-tap"
codesign --force --sign - "$ROOT/meeting-state/meeting-state"

cat <<'EOF'

✅ Setup complete.

One-time macOS permissions (grant to your Terminal, in System Settings → Privacy & Security):
  • Screen & System Audio Recording   (to hear the other participants)
  • Accessibility                     (for the speaker-names layer, when enabled)

Then, from your Terminal:   ./run.sh   →   open http://localhost:8790
EOF
