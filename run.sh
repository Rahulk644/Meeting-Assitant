#!/usr/bin/env bash
# Live on-device meeting transcription.
# Run from YOUR Terminal (it holds the Screen-Recording + Accessibility grants):
#     ./run.sh          →  open http://localhost:8790 in a browser ; Ctrl-C to stop.
#
# Captures the OTHER participants' audio (system output) → Whisper (Metal GPU) → live GUI.
# Works on Zoom / Meet / Teams / any call. No captions toggle, multilingual, fully on-device.

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

# 2) GUI + caption relay
pkill -f 'app/server.js' 2>/dev/null
PORT="$GUI_PORT" node "$ROOT/app/server.js" --ingest >/tmp/mt-gui.log 2>&1 &
PIDS+=($!)

until curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$WHISPER_PORT/" 2>/dev/null; do sleep 1; done
until curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$GUI_PORT/"     2>/dev/null; do sleep 1; done

echo
echo "  ✅  Open  http://localhost:$GUI_PORT"
echo "  🎙  Transcribing the other participants live. Ctrl-C to stop."
echo

# 3) capturer — foreground, under THIS Terminal's Screen-Recording grant
exec "$ROOT/stt/meeting-tap" "$GUI_PORT" "http://127.0.0.1:$WHISPER_PORT/inference"
