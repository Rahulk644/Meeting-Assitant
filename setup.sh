#!/usr/bin/env bash
# One-time setup: build the transcription engine + model, the post-call LLM, and the native helpers.
set -e
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODEL_DIR="$ROOT/stt/whisper"
GGUF="$MODEL_DIR/whisper-large-v3-turbo-Q8_0.gguf"
GGUF_URL="https://huggingface.co/handy-computer/whisper-large-v3-turbo-gguf/resolve/main/whisper-large-v3-turbo-Q8_0.gguf"
GGUF_SHA256="d5e65f2b0828802ae2c231673d31982cebe3a778c95d9494a9f3efee6bd17448"

command -v swiftc >/dev/null || { echo "Xcode Command Line Tools required: xcode-select --install"; exit 1; }
command -v brew   >/dev/null || { echo "Homebrew required: https://brew.sh"; exit 1; }
command -v cmake  >/dev/null || brew install cmake   # transcribe.cpp builds with CMake
command -v git    >/dev/null || { echo "git required"; exit 1; }

echo "1/3  Transcription engine — transcribe.cpp (built from source, Metal GPU) + large-v3-turbo q8 model, ~886 MB…"
mkdir -p "$MODEL_DIR"
[ -f "$GGUF" ] || curl -L --fail --progress-bar -o "$GGUF" "$GGUF_URL"   # GGUF for transcribe.cpp
ACTUAL_GGUF_SHA256="$(shasum -a 256 "$GGUF" | awk '{print $1}')"
[[ "$ACTUAL_GGUF_SHA256" == "$GGUF_SHA256" ]] || {
  echo "Speech-model checksum mismatch; refusing to load $GGUF" >&2
  rm -f "$GGUF"
  exit 1
}
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
# macOS 14.4 target: meeting-tap uses Core Audio process taps (14.2+) so audio-only meetings capture the
# other participants via "System Audio Recording Only" (no Screen Recording). Needs the 14.4+ SDK — update
# Command Line Tools if `xcrun --show-sdk-version` prints < 14.
( cd "$ROOT/stt" && swiftc -O -target arm64-apple-macos14.4 meeting-tap.swift -o meeting-tap \
    -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia -framework CoreAudio -framework CoreImage \
    -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker Info.plist )
( cd "$ROOT/meeting-state" && swiftc -O meeting-state.swift -o meeting-state )
( cd "$ROOT/agent-context" && swiftc -O agent-context.swift -o agent-context )
# TCC keys Screen-Recording / Mic / Accessibility grants on a process's SIGNING IDENTIFIER + code
# requirement — NOT on who its parent is. These helpers are spawned children of the app, but each is its
# own binary with its own identity, so the app's grant (identifier "com.myassistant.desktop") does NOT
# cover them: the capturer would get "user declined" and re-prompt on every launch. Two things fix it,
# together: (1) sign with the SAME cert the app uses ("My Assistant Dev"), and (2) force the SAME signing
# identifier as the app (APP_ID) — so each helper satisfies the app's Designated Requirement and RIDES the
# app's single, already-granted permission. No separate toggle, no re-prompt. (swiftc's default ad-hoc
# signature can't carry a stable identity at all, which is why it must be re-signed.) ALWAYS do this after
# rebuilding the helpers.
SIGN_ID="My Assistant Dev"
APP_ID="com.myassistant.desktop"        # must match tauri.conf.json "identifier"
if security find-certificate -c "$SIGN_ID" >/dev/null 2>&1; then
  codesign --force --options runtime --identifier "$APP_ID" --sign "$SIGN_ID" "$ROOT/stt/meeting-tap"
  codesign --force --options runtime --identifier "$APP_ID" --sign "$SIGN_ID" "$ROOT/meeting-state/meeting-state"
  codesign --force --options runtime --identifier "$APP_ID" --sign "$SIGN_ID" "$ROOT/agent-context/agent-context"
  echo "     signed native helpers as '$APP_ID' with '$SIGN_ID' — they ride the app's TCC grants (no re-prompt)."
else
  echo "     ⚠  Signing identity '$SIGN_ID' not found in your keychain. Falling back to ad-hoc —"
  echo "        macOS will RE-PROMPT for Screen Recording / Mic / Accessibility on every launch."
  echo "        Create the self-signed cert (Keychain Access → Certificate Assistant, name it exactly"
  echo "        '$SIGN_ID', type 'Code Signing') and re-run this script to make the prompts stop."
  codesign --force --options runtime --identifier "$APP_ID" --sign - "$ROOT/stt/meeting-tap"
  codesign --force --options runtime --identifier "$APP_ID" --sign - "$ROOT/meeting-state/meeting-state"
  codesign --force --options runtime --identifier "$APP_ID" --sign - "$ROOT/agent-context/agent-context"
fi

cat <<'EOF'

✅ Setup complete.

One-time macOS permissions (grant to your Terminal, in System Settings → Privacy & Security):
  • Screen & System Audio Recording   (to hear the other participants)
  • Accessibility                     (for the speaker-names layer, when enabled)

Then, from your Terminal:   ./run.sh   →   open http://localhost:8790
EOF
