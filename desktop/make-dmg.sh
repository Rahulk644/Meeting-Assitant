#!/usr/bin/env bash
# Build a portable, drag-to-install .dmg from the built "My Assistant.app".
#
# Why this exists: tauri's own DMG step (bundle_dmg.sh, a create-dmg fork) always runs an AppleScript to
# style the Finder window, which fails in any non-interactive / no-automation context (CI, an agent shell,
# SSH) with a bare "failed to run bundle_dmg.sh". Plain hdiutil needs no Finder and produces a DMG that
# installs identically. So `npm run build` bundles just the .app (bundle.targets = ["app"]), and this makes
# the installer. Result: a self-contained .dmg — drag My Assistant to Applications.
#
# Speech models are not inside the app (about 44 MB–1.1 GB each); first launch asks the user to choose one.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/desktop/src-tauri/target/release/bundle/macos/My Assistant.app"
OUT_DIR="$ROOT/desktop/src-tauri/target/release/bundle/dmg"
VERSION="$(/usr/bin/plutil -extract CFBundleShortVersionString raw "$APP/Contents/Info.plist" 2>/dev/null || echo 0.1.0)"
RELEASE_LABEL="${RELEASE_LABEL:-beta.2}"
DMG="$OUT_DIR/My_Assistant_${VERSION}-${RELEASE_LABEL}_arm64.dmg"
IDENTITY="${SIGN_IDENTITY:-My Assistant Dev}"

[ -d "$APP" ] || { echo "No .app at: $APP" >&2; echo "Run 'cd desktop && npm run build' first." >&2; exit 1; }
codesign --verify --deep --strict "$APP"

# Clear any stale half-mounted images from a previous failed run (they wedge hdiutil).
for v in /Volumes/dmg.*; do [ -d "$v" ] && hdiutil detach "$v" -force >/dev/null 2>&1 || true; done
rm -f "$OUT_DIR"/rw.*.dmg "$DMG"
mkdir -p "$OUT_DIR"

STAGE="$(mktemp -d)"; trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"   # drag-to-install target
cp "$ROOT/desktop/DMG-README.txt" "$STAGE/1 — START HERE — INSTALL.txt"

hdiutil create -volname "My Assistant" -srcfolder "$STAGE" -ov -format UDZO -fs HFS+ "$DMG" >/dev/null
codesign --force --sign "$IDENTITY" "$DMG" >/dev/null 2>&1 || echo "note: DMG left unsigned (identity '$IDENTITY' not found)"

echo "✅ built $(basename "$DMG")"
ls -lah "$DMG"
