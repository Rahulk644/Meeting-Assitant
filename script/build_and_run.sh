#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
APP_NAME="My Assistant"
PROCESS_NAME="app"
BUNDLE_ID="com.myassistant.desktop"
SIGN_IDENTITY="${SIGN_IDENTITY:-My Assistant Dev}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESKTOP_DIR="$ROOT_DIR/desktop"
BUILD_BUNDLE="$DESKTOP_DIR/src-tauri/target/release/bundle/macos/$APP_NAME.app"
BUILD_BINARY="$BUILD_BUNDLE/Contents/MacOS/app"
INSTALL_DIR="${APP_INSTALL_DIR:-/Applications}"
APP_BUNDLE="$INSTALL_DIR/$APP_NAME.app"
APP_BINARY="$APP_BUNDLE/Contents/MacOS/app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

stop_bundle() {
  local bundle="$1"
  local patterns=(
    "^$bundle/Contents/MacOS/app$"
    "^$bundle/Contents/Resources/stt/meeting-tap "
    "^$bundle/Contents/Resources/stt/transcribe-server/transcribe-server "
    "^$bundle/Contents/Resources/meeting-state/meeting-state$"
    "^$bundle/Contents/Resources/polish-runtime/llama-server "
  )
  local pattern
  for pattern in "${patterns[@]}"; do
    pkill -f "$pattern" >/dev/null 2>&1 || true
  done
  for _ in {1..20}; do
    local running=false
    for pattern in "${patterns[@]}"; do
      if pgrep -f "$pattern" >/dev/null 2>&1; then running=true; break; fi
    done
    [[ "$running" == false ]] && return
    sleep 0.1
  done
  for pattern in "${patterns[@]}"; do
    pkill -9 -f "$pattern" >/dev/null 2>&1 || true
  done
}

# Stop both the canonical installation and any previously launched build artifact. Helpers are included so
# a killed development build cannot leave the mic, speech port, or Smart Polish model running by itself.
stop_bundle "$APP_BUNDLE"
[[ "$BUILD_BUNDLE" != "$APP_BUNDLE" ]] && stop_bundle "$BUILD_BUNDLE"

SIGN_IDENTITY="$SIGN_IDENTITY" "$ROOT_DIR/script/build_native_resources.sh"

(
  cd "$DESKTOP_DIR"
  npm run build
)

if [[ ! -d "$BUILD_BUNDLE" || ! -x "$BUILD_BINARY" ]]; then
  echo "built app bundle not found at: $BUILD_BUNDLE" >&2
  exit 1
fi
"$ROOT_DIR/script/verify_app_bundle.sh" "$BUILD_BUNDLE" "$SIGN_IDENTITY"

install_build() {
  local staged="$INSTALL_DIR/.$APP_NAME.installing.$$.app"
  local previous="$INSTALL_DIR/.$APP_NAME.previous.$$.app"
  rm -rf "$staged" "$previous"
  /usr/bin/ditto "$BUILD_BUNDLE" "$staged"
  codesign --verify --deep --strict "$staged"

  if [[ -d "$APP_BUNDLE" ]]; then
    mv "$APP_BUNDLE" "$previous"
  fi
  if ! mv "$staged" "$APP_BUNDLE"; then
    [[ -d "$previous" ]] && mv "$previous" "$APP_BUNDLE"
    exit 1
  fi
  if ! codesign --verify --deep --strict "$APP_BUNDLE"; then
    rm -rf "$APP_BUNDLE"
    [[ -d "$previous" ]] && mv "$previous" "$APP_BUNDLE"
    exit 1
  fi
  rm -rf "$previous"

  # First unregister the transient bundle. `cleanup_build_artifact` removes it after byte-verifying the
  # canonical installation, because Spotlight can otherwise discover and re-register a build-tree .app.
  "$LSREGISTER" -u "$BUILD_BUNDLE" >/dev/null 2>&1 || true
  "$LSREGISTER" -gc >/dev/null 2>&1 || true
  "$LSREGISTER" -f "$APP_BUNDLE" >/dev/null 2>&1 || true
}

install_build

if ! cmp -s "$BUILD_BINARY" "$APP_BINARY"; then
  echo "installed binary does not match the verified build" >&2
  exit 1
fi

cleanup_build_artifact() {
  if [[ "$BUILD_BUNDLE" != "$APP_BUNDLE" && -d "$BUILD_BUNDLE" ]]; then
    "$LSREGISTER" -u "$BUILD_BUNDLE" >/dev/null 2>&1 || true
    rm -rf "$BUILD_BUNDLE"
  fi
  "$LSREGISTER" -gc >/dev/null 2>&1 || true
  "$LSREGISTER" -f "$APP_BUNDLE" >/dev/null 2>&1 || true
}

# Keep exactly one launchable My Assistant bundle on the Mac. Release packaging invokes `npm run build`
# directly and creates a fresh bundle; the everyday install/run workflow needs only /Applications.
cleanup_build_artifact

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
    ! pgrep -f "^$BUILD_BINARY$" >/dev/null
    [[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8790/api/release 2>/dev/null)" == "401" ]]
    ;;
  *)
    echo "usage: $0 [run|--debug|--logs|--telemetry|--verify]" >&2
    exit 2
    ;;
esac
