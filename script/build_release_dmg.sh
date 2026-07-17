#!/usr/bin/env bash
# Repeatable Apple-Silicon beta release pipeline using the project's stable self-signed identity.
# Current macOS recipients approve it once through Privacy & Security → Open Anyway.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIGN_IDENTITY="${SIGN_IDENTITY:-My Assistant Dev}"
RELEASE_LABEL="${RELEASE_LABEL:-beta.4}"
APP="$ROOT/desktop/src-tauri/target/release/bundle/macos/My Assistant.app"
ENTITLEMENTS="$ROOT/desktop/src-tauri/Entitlements.plist"

# A release artifact is always production, even when the invoking shell was previously used for an internal
# preview. This prevents an ambient environment variable from exposing unfinished surfaces in a DMG.
export ASSISTANT_RELEASE_CHANNEL=production

pkill -f "^$APP/Contents/MacOS/app$" >/dev/null 2>&1 || true
pkill -f "^$APP/Contents/Resources/stt/meeting-tap " >/dev/null 2>&1 || true
pkill -f "^$APP/Contents/Resources/meeting-state/meeting-state$" >/dev/null 2>&1 || true
pkill -f "^$APP/Contents/Resources/polish-runtime/llama-server " >/dev/null 2>&1 || true

security find-certificate -c "$SIGN_IDENTITY" >/dev/null 2>&1 || {
  echo "Signing certificate not found: $SIGN_IDENTITY" >&2
  exit 1
}
command -v cargo-audit >/dev/null 2>&1 || {
  echo "cargo-audit is required for a release: cargo install cargo-audit --version 0.22.2 --locked" >&2
  exit 1
}

# Release preflight covers the product surfaces that Cargo and Node do not know about. Keep this in the
# packaging path so a corpus/acceptance runner regression or malformed release metadata cannot ship merely
# because the app itself compiles.
bash -n "$ROOT"/script/*.sh "$ROOT/desktop/make-dmg.sh" "$ROOT/setup.sh" "$ROOT/run.sh"
python3 -m py_compile "$ROOT"/benchmarks/speech/*.py "$ROOT"/acceptance/*.py
python3 -m json.tool "$ROOT/benchmarks/speech/catalog.json" >/dev/null
python3 -m json.tool "$ROOT/acceptance/cases.json" >/dev/null
"$ROOT/script/benchmark_speech.sh" test

(cd "$ROOT/app" && npm test)
(cd "$ROOT/desktop" && npm audit)
(
  cd "$ROOT/desktop/src-tauri"
  cargo fmt --check
  cargo clippy --locked --all-targets -- -D warnings
  cargo check --locked
  cargo test --locked
  cargo audit
)

SIGN_IDENTITY="" "$ROOT/script/build_native_resources.sh"

# Remove local/debug symbols before signing. This reduces the shipped footprint and avoids exposing a
# convenient symbol map while preserving the dynamic symbols each native executable needs to run.
/usr/bin/strip -S -x "$ROOT/stt/meeting-tap"
/usr/bin/strip -S -x "$ROOT/meeting-state/meeting-state"
/usr/bin/strip -S -x "$ROOT/dictation-target/dictation-target"
/usr/bin/strip -S -x "$ROOT/polish-runtime/llama-server"
/usr/bin/strip -S -x "$ROOT/stt/transcribe-server/transcribe-server"

codesign --force --options runtime --entitlements "$ENTITLEMENTS" --identifier com.myassistant.desktop --sign "$SIGN_IDENTITY" "$ROOT/stt/meeting-tap"
codesign --force --options runtime --identifier com.myassistant.desktop --sign "$SIGN_IDENTITY" "$ROOT/meeting-state/meeting-state"
codesign --force --options runtime --identifier com.myassistant.desktop --sign "$SIGN_IDENTITY" "$ROOT/dictation-target/dictation-target"
codesign --force --options runtime --identifier com.myassistant.desktop.polish-runtime --sign "$SIGN_IDENTITY" "$ROOT/polish-runtime/llama-server"
codesign --force --options runtime --identifier com.myassistant.desktop.transcribe-server --sign "$SIGN_IDENTITY" "$ROOT/stt/transcribe-server/transcribe-server"

(cd "$ROOT/desktop" && npm run build)
"$ROOT/script/verify_app_bundle.sh" "$APP" "$SIGN_IDENTITY"

SIGN_IDENTITY="$SIGN_IDENTITY" RELEASE_LABEL="$RELEASE_LABEL" "$ROOT/desktop/make-dmg.sh"
DMG="$(find "$ROOT/desktop/src-tauri/target/release/bundle/dmg" -maxdepth 1 -name "My_Assistant_*-${RELEASE_LABEL}_arm64.dmg" -print | sort | tail -n 1)"
[[ -f "$DMG" ]]
"$ROOT/script/verify_release_artifact.sh" "$DMG" "$SIGN_IDENTITY"

if [[ -n "${NOTARY_KEYCHAIN_PROFILE:-}" ]]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_KEYCHAIN_PROFILE" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  "$ROOT/script/verify_release_artifact.sh" "$DMG" "$SIGN_IDENTITY"
fi

echo "Release artifact: $DMG"
stat -f 'Size: %z bytes' "$DMG"
shasum -a 256 "$DMG"
