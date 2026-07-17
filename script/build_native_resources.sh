#!/usr/bin/env bash
# Build every native executable copied into the production app bundle.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIGN_IDENTITY="${SIGN_IDENTITY:-}"
APP_ID="com.myassistant.desktop"
ENTITLEMENTS="$ROOT/desktop/src-tauri/Entitlements.plist"

xcrun swiftc -O -target arm64-apple-macos14.4 "$ROOT/stt/meeting-tap.swift" \
  -o "$ROOT/stt/meeting-tap" \
  -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia \
  -framework CoreAudio -framework CoreImage \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$ROOT/stt/Info.plist"
xcrun swiftc -O -target arm64-apple-macos14.4 \
  "$ROOT/meeting-state/meeting-state.swift" -o "$ROOT/meeting-state/meeting-state"
SIGN_IDENTITY="" "$ROOT/script/build_dictation_helper.sh"
"$ROOT/script/build_polish_runtime.sh"
"$ROOT/stt/transcribe-server/build.sh"

"$ROOT/stt/meeting-tap" --selftest
"$ROOT/meeting-state/meeting-state" --selftest
"$ROOT/dictation-target/dictation-target" --selftest
"$ROOT/stt/transcribe-server/transcribe-server" --selftest

if [[ -n "$SIGN_IDENTITY" ]]; then
  if [[ "$SIGN_IDENTITY" != "-" ]]; then
    security find-certificate -c "$SIGN_IDENTITY" >/dev/null 2>&1 || {
      echo "Signing certificate not found: $SIGN_IDENTITY" >&2
      exit 1
    }
  fi
  codesign --force --options runtime --entitlements "$ENTITLEMENTS" \
    --identifier "$APP_ID" --sign "$SIGN_IDENTITY" "$ROOT/stt/meeting-tap"
  codesign --force --options runtime --identifier "$APP_ID" --sign "$SIGN_IDENTITY" \
    "$ROOT/meeting-state/meeting-state"
  codesign --force --options runtime --identifier "$APP_ID" --sign "$SIGN_IDENTITY" \
    "$ROOT/dictation-target/dictation-target"
  codesign --force --options runtime --identifier "$APP_ID.polish-runtime" --sign "$SIGN_IDENTITY" \
    "$ROOT/polish-runtime/llama-server"
  codesign --force --options runtime --identifier "$APP_ID.transcribe-server" --sign "$SIGN_IDENTITY" \
    "$ROOT/stt/transcribe-server/transcribe-server"
fi

echo "Built all production native resources."
