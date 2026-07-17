#!/usr/bin/env bash
# Atomically install the exact app contained in a verified release DMG, then keep one launchable bundle.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG="${1:?usage: install_verified_dmg.sh /path/to/My_Assistant.dmg}"
SIGN_IDENTITY="${SIGN_IDENTITY:-My Assistant Dev}"
INSTALL_DIR="${APP_INSTALL_DIR:-/Applications}"
DEST="$INSTALL_DIR/My Assistant.app"
BUILD_APP="$ROOT/desktop/src-tauri/target/release/bundle/macos/My Assistant.app"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
MOUNT="$(mktemp -d)"
STAGED="$INSTALL_DIR/.My Assistant.installing.$$.app"
PREVIOUS="$INSTALL_DIR/.My Assistant.previous.$$.app"
ATTACHED=false
ORIGINAL_MOVED=false
NEW_INSTALLED=false
COMMITTED=false

delete_generated_bundle() {
  local path="$1"
  case "$path" in
    "$INSTALL_DIR"/.My\ Assistant.installing.*.app|\
    "$INSTALL_DIR"/.My\ Assistant.previous.*.app|\
    "$BUILD_APP")
      [[ ! -e "$path" ]] || find "$path" -depth -delete
      ;;
    *)
      echo "Refusing to remove unexpected path: $path" >&2
      return 1
      ;;
  esac
}

stop_installed_app() {
  local patterns=(
    "^$DEST/Contents/MacOS/app$"
    "^$DEST/Contents/Resources/stt/meeting-tap "
    "^$DEST/Contents/Resources/stt/transcribe-server/transcribe-server "
    "^$DEST/Contents/Resources/meeting-state/meeting-state$"
    "^$DEST/Contents/Resources/dictation-target/dictation-target "
    "^$DEST/Contents/Resources/polish-runtime/llama-server "
  )
  local pattern
  for pattern in "${patterns[@]}"; do pkill -f "$pattern" >/dev/null 2>&1 || true; done
}

cleanup() {
  if [[ "$NEW_INSTALLED" == true && "$COMMITTED" != true ]]; then
    stop_installed_app
    if [[ -d "$DEST" ]]; then
      local failed="$INSTALL_DIR/.My Assistant.installing.$$.app"
      mv "$DEST" "$failed"
      delete_generated_bundle "$failed" || true
    fi
    if [[ "$ORIGINAL_MOVED" == true && -d "$PREVIOUS" ]]; then mv "$PREVIOUS" "$DEST"; fi
  fi
  delete_generated_bundle "$STAGED" || true
  if [[ "$ATTACHED" == true ]]; then
    hdiutil detach "$MOUNT" >/dev/null 2>&1 || hdiutil detach "$MOUNT" -force >/dev/null 2>&1 || true
  fi
  rmdir "$MOUNT" >/dev/null 2>&1 || true
}
trap cleanup EXIT

"$ROOT/script/verify_release_artifact.sh" "$DMG" "$SIGN_IDENTITY"
hdiutil attach -readonly -nobrowse -mountpoint "$MOUNT" "$DMG" >/dev/null
ATTACHED=true
SOURCE="$MOUNT/My Assistant.app"

/usr/bin/ditto "$SOURCE" "$STAGED"
"$ROOT/script/verify_app_bundle.sh" "$STAGED" "$SIGN_IDENTITY"
diff -qr "$SOURCE" "$STAGED" >/dev/null

stop_installed_app
if [[ -d "$DEST" ]]; then
  mv "$DEST" "$PREVIOUS"
  ORIGINAL_MOVED=true
fi
mv "$STAGED" "$DEST"
NEW_INSTALLED=true
"$ROOT/script/verify_app_bundle.sh" "$DEST" "$SIGN_IDENTITY"
diff -qr "$SOURCE" "$DEST" >/dev/null

"$LSREGISTER" -u "$BUILD_APP" >/dev/null 2>&1 || true
"$LSREGISTER" -gc >/dev/null 2>&1 || true
"$LSREGISTER" -f "$DEST" >/dev/null 2>&1 || true
/usr/bin/open -n "$DEST"

ready=false
for _ in {1..40}; do
  if pgrep -f "^$DEST/Contents/MacOS/app$" >/dev/null 2>&1 \
    && [[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8790/api/release 2>/dev/null || true)" == "401" ]]; then
    ready=true
    break
  fi
  sleep 0.5
done
[[ "$ready" == true ]] || { echo "Installed app did not pass its launch/authentication smoke test." >&2; exit 1; }

COMMITTED=true
delete_generated_bundle "$PREVIOUS"
if [[ "$BUILD_APP" != "$DEST" ]]; then
  delete_generated_bundle "$BUILD_APP"
fi
"$LSREGISTER" -gc >/dev/null 2>&1 || true
"$LSREGISTER" -f "$DEST" >/dev/null 2>&1 || true

echo "Installed and verified: $DEST"
