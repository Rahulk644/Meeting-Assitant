#!/usr/bin/env bash
# Build every native executable copied into the production app bundle.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIGN_IDENTITY="${SIGN_IDENTITY:-}"
APP_ID="com.myassistant.desktop"
ENTITLEMENTS="$ROOT/desktop/src-tauri/Entitlements.plist"

# Command Line Tools can temporarily contain a newer swiftc beside an older default SDK Swift module
# interface (for example Swift 6.3.3 with the macOS 26.5 SDK built by 6.3.2). That pair cannot compile any
# Swift source. Prefer the installed 15.4 compatibility SDK when available: it contains every API used by
# these macOS 14.4-targeted helpers and also keeps accidental adoption of newer-only APIs out of the build.
# Maintainers can select another verified SDK explicitly without changing the system-wide xcode-select path.
DEFAULT_SWIFT_SDK="$(xcrun --sdk macosx --show-sdk-path)"
COMPAT_SWIFT_SDK="$(dirname "$DEFAULT_SWIFT_SDK")/MacOSX15.4.sdk"
ASSISTANT_SWIFT_SDK="${ASSISTANT_SWIFT_SDK:-$DEFAULT_SWIFT_SDK}"
if [[ "$ASSISTANT_SWIFT_SDK" == "$DEFAULT_SWIFT_SDK" && -d "$COMPAT_SWIFT_SDK" ]]; then
  ASSISTANT_SWIFT_SDK="$COMPAT_SWIFT_SDK"
fi
[[ -d "$ASSISTANT_SWIFT_SDK" ]] || {
  echo "Swift SDK not found: $ASSISTANT_SWIFT_SDK" >&2
  exit 1
}
export CLANG_MODULE_CACHE_PATH="$ROOT/desktop/src-tauri/target/swift-module-cache"
export SWIFT_MODULECACHE_PATH="$CLANG_MODULE_CACHE_PATH"
mkdir -p "$CLANG_MODULE_CACHE_PATH"

xcrun swiftc -sdk "$ASSISTANT_SWIFT_SDK" -O -target arm64-apple-macos14.4 "$ROOT/stt/meeting-tap.swift" \
  -o "$ROOT/stt/meeting-tap" \
  -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia \
  -framework CoreAudio -framework CoreImage \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$ROOT/stt/Info.plist"
xcrun swiftc -sdk "$ASSISTANT_SWIFT_SDK" -O -target arm64-apple-macos14.4 \
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
