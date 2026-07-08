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
TS_BIN="$ROOT/stt/transcribe-server/transcribe-server"       # primary: transcribe.cpp (faster, lighter, cleaner)
TS_MODEL="$ROOT/stt/whisper/whisper-large-v3-turbo-Q8_0.gguf"
MODEL="$ROOT/stt/whisper/ggml-large-v3-turbo-q8_0.bin"       # whisper.cpp fallback (GGML)
WHISPER_BIN="$(command -v whisper-server || true)"

[ -x "$ROOT/stt/meeting-tap" ] || { echo "Not built. Run ./setup.sh first."; exit 1; }
command -v node >/dev/null    || { echo "node is required — brew install node"; exit 1; }
if   [ -x "$TS_BIN" ] && [ -f "$TS_MODEL" ]; then :
elif [ -n "$WHISPER_BIN" ] && [ -f "$MODEL" ]; then :
else echo "No STT engine. Run ./setup.sh first (builds transcribe-server), or: brew install whisper-cpp"; exit 1; fi

PIDS=()
cleanup() { echo; echo "stopping…"; kill "${PIDS[@]}" 2>/dev/null; pkill -f 'app/server.js' 2>/dev/null; exit 0; }
trap cleanup INT TERM

# 1) STT server — loads the model once, stays hot on the GPU. Prefer transcribe.cpp; fall back to whisper.cpp
#    (no Silero VAD — it degraded clean speech; whole-utterance capture + conf gate handle noise).
if ! curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$WHISPER_PORT/" 2>/dev/null; then
  if [ -x "$TS_BIN" ] && [ -f "$TS_MODEL" ]; then
    echo "starting transcribe-server (transcribe.cpp, large-v3-turbo)…"
    "$TS_BIN" -m "$TS_MODEL" --host 127.0.0.1 --port "$WHISPER_PORT" >/tmp/mt-whisper.log 2>&1 &
  else
    echo "starting whisper-server (fallback, large-v3-turbo, ~10s)…"
    "$WHISPER_BIN" -m "$MODEL" --host 127.0.0.1 --port "$WHISPER_PORT" -t 4 --suppress-nst >/tmp/mt-whisper.log 2>&1 &
  fi
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
