#!/usr/bin/env bash
# Live on-device meeting transcription.
# Run from YOUR Terminal (it holds the Screen-Recording + Accessibility grants):
#     ./run.sh                       →  open http://localhost:8790 in a browser ; Ctrl-C to stop.
#     ./run.sh --app all             →  capture all system audio (e.g. Google Meet in a browser)
#     ./run.sh --mic off             →  don't transcribe your own mic
#
# Captures YOU (mic) + the other participants (meeting-app audio) → Whisper (Metal GPU) → live GUI.
# Auto-scopes to a running Zoom/Teams/Webex/FaceTime window so background audio isn't transcribed.
# Capture is GATED: the mic + system taps run ONLY while you're actually in a call (--gate on, default)
# and are released otherwise — so your mic isn't held when you're idle. Pass --gate off to always
# capture (e.g. Google Meet, which the Accessibility layer can't detect).
# No captions toggle, multilingual, fully on-device. Flags after ./run.sh pass through to the capturer.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUI_PORT="${GUI_PORT:-8790}"
WHISPER_PORT="${WHISPER_PORT:-8091}"
MODEL="$ROOT/stt/whisper/ggml-large-v3-turbo-q8_0.bin"
WHISPER_BIN="$(command -v whisper-server || true)"

[ -x "$ROOT/stt/meeting-tap" ] || { echo "Not built. Run ./setup.sh first."; exit 1; }
[ -f "$MODEL" ]               || { echo "Model missing. Run ./setup.sh first."; exit 1; }
[ -n "$WHISPER_BIN" ]         || { echo "whisper-server not found — run: brew install whisper-cpp"; exit 1; }
command -v node >/dev/null    || { echo "node is required — brew install node"; exit 1; }

PIDS=()
cleanup() { echo; echo "stopping…"; kill "${PIDS[@]}" 2>/dev/null; pkill -f 'app/server.js' 2>/dev/null; exit 0; }
trap cleanup INT TERM

# 1) whisper-server — loads the model once, stays hot on the GPU
if ! curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$WHISPER_PORT/" 2>/dev/null; then
  echo "starting whisper-server (loading large-v3-turbo, ~10s)…"
  VAD="$ROOT/stt/whisper/ggml-silero-v5.1.2.bin"; [ -f "$VAD" ] && VADARGS="--vad --vad-model $VAD" || VADARGS=""
  "$WHISPER_BIN" -m "$MODEL" --host 127.0.0.1 --port "$WHISPER_PORT" -t 4 --suppress-nst $VADARGS >/tmp/mt-whisper.log 2>&1 &
  PIDS+=($!)
fi

# 1.5) local LLM (optional) — LiteRT-LM (Gemma 4 E2B, text-only, ~1 GB RAM). Imported by ./setup.sh.
#      NOT started here: app/server.js spawns it ON DEMAND (first summary/ask) so it doesn't compete
#      with Whisper for RAM during a call, and unloads it when idle. We just export the hints it needs.
#      Skipped silently if not set up (the AI panel then shows a "set one up" hint).
export LLM_PORT="${LLM_PORT:-9379}"
LITERT_BIN="$(command -v litert-lm || echo "$HOME/.local/bin/litert-lm")"
if [ -x "$LITERT_BIN" ] && "$LITERT_BIN" list 2>/dev/null | grep -q "gemma4-e2b"; then
  export LITERT_BIN LLM_MODEL="gemma4-e2b" LLM_BASE="http://127.0.0.1:$LLM_PORT/v1"
fi

# 2) GUI + caption relay
pkill -f 'app/server.js' 2>/dev/null
PORT="$GUI_PORT" node "$ROOT/app/server.js" >/tmp/mt-gui.log 2>&1 &
PIDS+=($!)

until curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$WHISPER_PORT/" 2>/dev/null; do sleep 1; done
until curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$GUI_PORT/"     2>/dev/null; do sleep 1; done

echo
echo "  ✅  Open  http://localhost:$GUI_PORT"
echo "  🎙  Transcribing you + the other participants live. Ctrl-C to stop."
echo

# 3) capturer — foreground, under THIS Terminal's Screen-Recording grant.
#    "$@" forwards flags like --app / --mic to meeting-tap.
exec "$ROOT/stt/meeting-tap" "$GUI_PORT" "http://127.0.0.1:$WHISPER_PORT/inference" --rec-dir "$ROOT/data/meetings/rec" "$@"
