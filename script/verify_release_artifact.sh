#!/usr/bin/env bash
# Inspect the exact read-only DMG a recipient will install.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG="${1:?usage: verify_release_artifact.sh /path/to/release.dmg [signing-identity]}"
SIGN_IDENTITY="${2:-${SIGN_IDENTITY:-My Assistant Dev}}"
MOUNT="$(mktemp -d)"
ATTACHED=false

cleanup() {
  if [[ "$ATTACHED" == true ]]; then
    hdiutil detach "$MOUNT" >/dev/null 2>&1 || hdiutil detach "$MOUNT" -force >/dev/null 2>&1 || true
  fi
  rmdir "$MOUNT" >/dev/null 2>&1 || true
}
trap cleanup EXIT

[[ -f "$DMG" ]] || { echo "DMG not found: $DMG" >&2; exit 1; }
hdiutil verify "$DMG" >/dev/null
codesign --verify --strict "$DMG"
if [[ "$SIGN_IDENTITY" != "-" ]]; then
  DETAILS="$(codesign -dv --verbose=4 "$DMG" 2>&1)"
  [[ "$DETAILS" == *"Authority=$SIGN_IDENTITY"* ]] || {
    echo "Unexpected DMG signing authority." >&2
    exit 1
  }
fi

hdiutil attach -readonly -nobrowse -mountpoint "$MOUNT" "$DMG" >/dev/null
ATTACHED=true
[[ -L "$MOUNT/Applications" && "$(readlink "$MOUNT/Applications")" == "/Applications" ]]
[[ -f "$MOUNT/1 — START HERE — INSTALL.txt" ]]
"$ROOT/script/verify_app_bundle.sh" "$MOUNT/My Assistant.app" "$SIGN_IDENTITY"

echo "Verified release DMG: $DMG"
