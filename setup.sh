#!/usr/bin/env bash
# One-time developer setup for local models and production native resources.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODEL_DIR="$ROOT/stt/whisper"
GGUF="$MODEL_DIR/whisper-large-v3-turbo-Q8_0.gguf"
GGUF_PART="$GGUF.part"
GGUF_URL="https://huggingface.co/handy-computer/whisper-large-v3-turbo-gguf/resolve/d222c9f621c1128299248f2ded4d8a1820519780/whisper-large-v3-turbo-Q8_0.gguf"
GGUF_SHA256="d5e65f2b0828802ae2c231673d31982cebe3a778c95d9494a9f3efee6bd17448"
SIGN_IDENTITY="${SIGN_IDENTITY:-My Assistant Dev}"

command -v swiftc >/dev/null || { echo "Xcode Command Line Tools required: xcode-select --install"; exit 1; }
command -v brew >/dev/null || { echo "Homebrew required: https://brew.sh"; exit 1; }
command -v cmake >/dev/null || brew install cmake
command -v git >/dev/null || { echo "git is required"; exit 1; }

echo "Preparing the default on-device speech model…"
mkdir -p "$MODEL_DIR"
if [[ -f "$GGUF" ]] && [[ "$(shasum -a 256 "$GGUF" | awk '{print $1}')" != "$GGUF_SHA256" ]]; then
  rm -f "$GGUF"
fi
if [[ ! -f "$GGUF" ]]; then
  curl --location --fail --continue-at - --progress-bar --output "$GGUF_PART" "$GGUF_URL"
  [[ "$(shasum -a 256 "$GGUF_PART" | awk '{print $1}')" == "$GGUF_SHA256" ]] || {
    echo "Speech-model checksum mismatch; refusing to install it." >&2
    rm -f "$GGUF_PART"
    exit 1
  }
  chmod 600 "$GGUF_PART"
  mv "$GGUF_PART" "$GGUF"
fi

if [[ "${INSTALL_MEETING_INTELLIGENCE:-0}" == "1" ]]; then
  echo "Installing the optional local meeting-Intelligence runtime…"
  command -v uv >/dev/null || brew install uv
  LITERT="$(command -v litert-lm || echo "$HOME/.local/bin/litert-lm")"
  [[ -x "$LITERT" ]] || uv tool install litert-lm
  "$LITERT" list 2>/dev/null | grep -q gemma4-e2b || \
    "$LITERT" import --from-huggingface-repo \
      litert-community/gemma-4-E2B-it-litert-lm gemma-4-E2B-it.litertlm gemma4-e2b
else
  echo "Skipping optional meeting Intelligence. Set INSTALL_MEETING_INTELLIGENCE=1 to install it."
fi

echo "Building native production resources…"
if security find-certificate -c "$SIGN_IDENTITY" >/dev/null 2>&1; then
  SIGN_IDENTITY="$SIGN_IDENTITY" "$ROOT/script/build_native_resources.sh"
else
  echo "Signing identity '$SIGN_IDENTITY' was not found; native helpers will use ad-hoc signatures." >&2
  echo "Create a stable code-signing certificate before producing a distributable DMG." >&2
  SIGN_IDENTITY="-" "$ROOT/script/build_native_resources.sh"
fi

cat <<'EOF'

Setup complete.

The default Right Command shortcut needs Input Monitoring. Dictation also needs Microphone and
Accessibility; Meetings additionally requests System Audio Recording, and optional video requests Screen
Recording. macOS prompts on first use and the app links to each Privacy & Security pane.

Run ./script/build_and_run.sh --verify for the packaged desktop app, or ./run.sh for the developer harness.
EOF
