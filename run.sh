#!/usr/bin/env bash
# Live on-device meeting transcription.
# Run from YOUR Terminal (it holds the Screen-Recording + Accessibility grants):
#     ./run.sh                       →  open http://localhost:8790 in a browser ; Ctrl-C to stop.
#     ./run.sh --app all             →  capture all system audio (e.g. Google Meet in a browser)
#     ./run.sh --mic off             →  don't transcribe your own mic
#
# Captures YOU (mic) + the other participants (meeting-app audio) → Whisper (Metal GPU) → live GUI.
# Auto-scopes to a running Zoom/Teams/Webex/FaceTime window so background audio isn't transcribed.
# Capture is GATED: the mic + system taps run ONLY while authoritative call evidence is present
# (`--gate on`, default) and are released otherwise. Browser/chat calls require both active audio and
# narrowly scoped meeting-window evidence. Use `--gate off` only for an intentional manual capture.
# No captions toggle, multilingual, fully on-device. Flags after ./run.sh pass through to the capturer.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUI_PORT="${GUI_PORT:-8790}"
WHISPER_PORT="${WHISPER_PORT:-8091}"
TS_BIN="$ROOT/stt/transcribe-server/transcribe-server"       # primary: transcribe.cpp (faster, lighter, cleaner)
TS_MODEL="$ROOT/stt/whisper/whisper-large-v3-turbo-Q8_0.gguf"
MODEL="$ROOT/stt/whisper/ggml-large-v3-turbo-q8_0.bin"       # whisper.cpp fallback (GGML)
WHISPER_BIN="$(command -v whisper-server || true)"
SESSION_TOKEN="$(/usr/bin/openssl rand -hex 32)"
export ASSISTANT_GUI_TOKEN="$SESSION_TOKEN" ASSISTANT_STT_TOKEN="$SESSION_TOKEN"
PRIVATE_TMP="$ROOT/data/tmp"
mkdir -p "$PRIVATE_TMP"
chmod 700 "$PRIVATE_TMP"

[ -x "$ROOT/stt/meeting-tap" ] || { echo "Not built. Run ./setup.sh first."; exit 1; }
command -v node >/dev/null    || { echo "node is required — brew install node"; exit 1; }
if   [ -x "$TS_BIN" ] && [ -f "$TS_MODEL" ]; then :
elif [ -n "$WHISPER_BIN" ] && [ -f "$MODEL" ]; then :
else echo "No STT engine. Run ./setup.sh first (builds transcribe-server), or: brew install whisper-cpp"; exit 1; fi

PIDS=()
cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if ((${#PIDS[@]})); then
    echo
    echo "stopping…"
    kill "${PIDS[@]}" 2>/dev/null || true
    wait "${PIDS[@]}" 2>/dev/null || true
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

port_in_use() { /usr/bin/nc -z 127.0.0.1 "$1" >/dev/null 2>&1; }
wait_for_service() {
  local port="$1" label="$2" log_file="$3" child_pid="$4"
  for ((attempt = 0; attempt < 90; attempt++)); do
    if curl -fsS -o /dev/null --max-time 2 -H "Authorization: Bearer $SESSION_TOKEN" "http://127.0.0.1:$port/" 2>/dev/null; then
      return 0
    fi
    if ! kill -0 "$child_pid" 2>/dev/null; then
      echo "$label exited before becoming ready. See $log_file" >&2
      return 1
    fi
    sleep 1
  done
  echo "$label did not become ready on port $port. See $log_file" >&2
  return 1
}

if port_in_use "$WHISPER_PORT"; then
  echo "Port $WHISPER_PORT is already in use. Stop the other transcription session or set WHISPER_PORT." >&2
  exit 1
fi
if port_in_use "$GUI_PORT"; then
  echo "Port $GUI_PORT is already in use. Stop the other UI session or set GUI_PORT." >&2
  exit 1
fi

# 1) STT server — loads the model once, stays hot on the GPU. Prefer transcribe.cpp; fall back to whisper.cpp
#    (no Silero VAD — it degraded clean speech; whole-utterance capture + conf gate handle noise).
STT_LOG="$PRIVATE_TMP/transcribe.log"
if [ -x "$TS_BIN" ] && [ -f "$TS_MODEL" ]; then
  echo "starting transcribe-server (transcribe.cpp, large-v3-turbo)…"
  "$TS_BIN" -m "$TS_MODEL" --host 127.0.0.1 --port "$WHISPER_PORT" >"$STT_LOG" 2>&1 &
else
  echo "starting whisper-server (fallback, large-v3-turbo, ~10s)…"
  STT_LOG="$PRIVATE_TMP/whisper.log"
  "$WHISPER_BIN" -m "$MODEL" --host 127.0.0.1 --port "$WHISPER_PORT" -t 4 --suppress-nst >"$STT_LOG" 2>&1 &
fi
STT_PID="$!"
PIDS+=("$STT_PID")

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
GUI_LOG="$PRIVATE_TMP/gui.log"
PORT="$GUI_PORT" node "$ROOT/app/server.js" >"$GUI_LOG" 2>&1 &
GUI_PID="$!"
PIDS+=("$GUI_PID")

wait_for_service "$WHISPER_PORT" "Transcription service" "$STT_LOG" "$STT_PID"
wait_for_service "$GUI_PORT" "Developer UI" "$GUI_LOG" "$GUI_PID"

echo
echo "  ✅  Open  http://127.0.0.1:$GUI_PORT/?bootstrap=$SESSION_TOKEN"
echo "  🎙  Transcribing you + the other participants live. Ctrl-C to stop."
echo

# 3) capturer — direct child under THIS Terminal's Screen-Recording grant. Keep this shell alive so its EXIT
#    trap owns and reaps every process; exec would orphan the two background services when capture stops.
#    "$@" forwards flags like --app / --mic to meeting-tap.
"$ROOT/stt/meeting-tap" "$GUI_PORT" "http://127.0.0.1:$WHISPER_PORT/inference" \
  --rec-dir "$ROOT/data/meetings/rec" --temp-dir "$PRIVATE_TMP" "$@" &
CAPTURE_PID="$!"
PIDS+=("$CAPTURE_PID")
wait "$CAPTURE_PID"
