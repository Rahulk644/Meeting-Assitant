#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
APP_NAME="My Assistant"
PROCESS_NAME="app"
BUNDLE_ID="com.myassistant.desktop"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESKTOP_DIR="$ROOT_DIR/desktop"
APP_BUNDLE="$DESKTOP_DIR/src-tauri/target/release/bundle/macos/$APP_NAME.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/app"

pkill -f "^$APP_BINARY$" >/dev/null 2>&1 || true
pkill -f "^$APP_BUNDLE/Contents/Resources/stt/meeting-tap " >/dev/null 2>&1 || true
pkill -f "^$APP_BUNDLE/Contents/Resources/stt/transcribe-server/transcribe-server " >/dev/null 2>&1 || true
pkill -f "^$APP_BUNDLE/Contents/Resources/meeting-state/meeting-state$" >/dev/null 2>&1 || true

(
  cd "$DESKTOP_DIR"
  npm run build
)

if [[ ! -d "$APP_BUNDLE" || ! -x "$APP_BINARY" ]]; then
  echo "built app bundle not found at: $APP_BUNDLE" >&2
  exit 1
fi

open_app() {
  /usr/bin/open -n "$APP_BUNDLE"
}

case "$MODE" in
  run)
    open_app
    ;;
  --debug|debug)
    lldb -- "$APP_BINARY"
    ;;
  --logs|logs)
    open_app
    /usr/bin/log stream --info --style compact --predicate "process == \"$PROCESS_NAME\""
    ;;
  --telemetry|telemetry)
    open_app
    /usr/bin/log stream --info --style compact --predicate "subsystem == \"$BUNDLE_ID\""
    ;;
  --verify|verify)
    open_app
    for _ in {1..20}; do
      if pgrep -f "^$APP_BINARY$" >/dev/null && [[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8790/api/release 2>/dev/null)" == "401" ]]; then break; fi
      sleep 0.5
    done
    pgrep -f "^$APP_BINARY$" >/dev/null
    [[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8790/api/release 2>/dev/null)" == "401" ]]
    ;;
  *)
    echo "usage: $0 [run|--debug|--logs|--telemetry|--verify]" >&2
    exit 2
    ;;
esac
