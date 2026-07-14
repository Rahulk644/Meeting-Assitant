#!/usr/bin/env bash
# Reproducible Apple-Silicon beta release pipeline using the project's stable self-signed identity.
# Current macOS recipients approve it once through Privacy & Security → Open Anyway.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIGN_IDENTITY="${SIGN_IDENTITY:-My Assistant Dev}"
APP="$ROOT/desktop/src-tauri/target/release/bundle/macos/My Assistant.app"

pkill -f "^$APP/Contents/MacOS/app$" >/dev/null 2>&1 || true
pkill -f "^$APP/Contents/Resources/stt/meeting-tap " >/dev/null 2>&1 || true
pkill -f "^$APP/Contents/Resources/meeting-state/meeting-state$" >/dev/null 2>&1 || true

security find-certificate -c "$SIGN_IDENTITY" >/dev/null 2>&1 || {
  echo "Signing certificate not found: $SIGN_IDENTITY" >&2
  exit 1
}

(cd "$ROOT/app" && npm test)
(cd "$ROOT/desktop/src-tauri" && cargo test --locked)

xcrun swiftc -O -target arm64-apple-macos14.4 "$ROOT/stt/meeting-tap.swift" -o "$ROOT/stt/meeting-tap" \
  -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia -framework CoreAudio -framework CoreImage \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$ROOT/stt/Info.plist"
xcrun swiftc -O "$ROOT/meeting-state/meeting-state.swift" -o "$ROOT/meeting-state/meeting-state"
"$ROOT/stt/meeting-tap" --selftest
"$ROOT/meeting-state/meeting-state" --selftest
"$ROOT/stt/transcribe-server/build.sh"

# Remove local/debug symbols before signing. This reduces the shipped footprint and avoids exposing a
# convenient symbol map while preserving the dynamic symbols each native executable needs to run.
/usr/bin/strip -S -x "$ROOT/stt/meeting-tap"
/usr/bin/strip -S -x "$ROOT/meeting-state/meeting-state"
/usr/bin/strip -S -x "$ROOT/stt/transcribe-server/transcribe-server"

codesign --force --options runtime --identifier com.myassistant.desktop --sign "$SIGN_IDENTITY" "$ROOT/stt/meeting-tap"
codesign --force --options runtime --identifier com.myassistant.desktop --sign "$SIGN_IDENTITY" "$ROOT/meeting-state/meeting-state"
codesign --force --options runtime --identifier com.myassistant.desktop.transcribe-server --sign "$SIGN_IDENTITY" "$ROOT/stt/transcribe-server/transcribe-server"

(cd "$ROOT/desktop" && npm run build)
codesign --verify --deep --strict "$APP"
codesign --verify --strict "$APP/Contents/Resources/stt/meeting-tap"
codesign --verify --strict "$APP/Contents/Resources/stt/transcribe-server/transcribe-server"
codesign --verify --strict "$APP/Contents/Resources/meeting-state/meeting-state"

SIGN_IDENTITY="$SIGN_IDENTITY" "$ROOT/desktop/make-dmg.sh"
DMG="$(find "$ROOT/desktop/src-tauri/target/release/bundle/dmg" -maxdepth 1 -name 'My_Assistant_*-beta.1_arm64.dmg' -print | sort | tail -n 1)"
[[ -f "$DMG" ]]
codesign --verify "$DMG"
hdiutil verify "$DMG" >/dev/null

if [[ -n "${NOTARY_KEYCHAIN_PROFILE:-}" ]]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_KEYCHAIN_PROFILE" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
fi

echo "Release artifact: $DMG"
stat -f 'Size: %z bytes' "$DMG"
shasum -a 256 "$DMG"
