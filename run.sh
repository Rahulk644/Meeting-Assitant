#!/usr/bin/env bash
# Live on-device meeting transcription.
# Run from YOUR Terminal (it holds the Screen-Recording + Accessibility grants):
#     ./run.sh                       →  open http://localhost:8790 in a browser ; Ctrl-C to stop.
#     ./run.sh --app all             →  capture all system audio (e.g. Google Meet in a browser)
#     ./run.sh --mic off             →  don't transcribe your own mic
#
# Captures YOU (mic) + the other participants (meeting-app audio) → Whisper (Metal GPU) → live GUI.
# Auto-scopes to a running Zoom/Teams/Webex/FaceTime window so background audio isn't transcribed.
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
  "$WHISPER_BIN" -m "$MODEL" --host 127.0.0.1 --port "$WHISPER_PORT" -t 4 >/tmp/mt-whisper.log 2>&1 &
  PIDS+=($!)
fi

# 1.5) local LLM (optional) — llama-server for post-call summaries / "ask", if a model is present.
#      Downloaded by ./setup.sh into stt/llm/. Skipped silently if absent (AI features just show a hint).
LLM_PORT="${LLM_PORT:-8092}"
LLM_MODEL="$(ls "$ROOT/stt/llm/"*.gguf 2>/dev/null | head -1)"
LLAMA_BIN="$(command -v llama-server || true)"
if [ -n "$LLM_MODEL" ] && [ -n "$LLAMA_BIN" ]; then
  export LLM_BASE="http://127.0.0.1:$LLM_PORT/v1"   # app/server.js reads this
  if ! curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$LLM_PORT/health" 2>/dev/null; then
    echo "starting llama-server ($(basename "$LLM_MODEL"), Metal)…"
    "$LLAMA_BIN" -m "$LLM_MODEL" --host 127.0.0.1 --port "$LLM_PORT" -c 8192 -ngl 99 >/tmp/mt-llm.log 2>&1 &
    PIDS+=($!)
  fi
fi

# 2) GUI + caption relay
pkill -f 'app/server.js' 2>/dev/null
PORT="$GUI_PORT" node "$ROOT/app/server.js" --ingest >/tmp/mt-gui.log 2>&1 &
PIDS+=($!)

until curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$WHISPER_PORT/" 2>/dev/null; do sleep 1; done
until curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$GUI_PORT/"     2>/dev/null; do sleep 1; done

echo
echo "  ✅  Open  http://localhost:$GUI_PORT"
echo "  🎙  Transcribing you + the other participants live. Ctrl-C to stop."
echo

# 3) capturer — foreground, under THIS Terminal's Screen-Recording grant.
#    "$@" forwards flags like --app / --mic to meeting-tap.
exec "$ROOT/stt/meeting-tap" "$GUI_PORT" "http://127.0.0.1:$WHISPER_PORT/inference" "$@"
