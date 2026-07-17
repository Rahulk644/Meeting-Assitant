#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIGN_IDENTITY="${SIGN_IDENTITY:-}"
APP_ID="com.myassistant.desktop"

xcrun swiftc -O -target arm64-apple-macos14.4 "$ROOT/dictation-target/dictation-target.swift" \
  -o "$ROOT/dictation-target/dictation-target" \
  -framework AppKit -framework ApplicationServices
"$ROOT/dictation-target/dictation-target" --selftest

if [[ -n "$SIGN_IDENTITY" ]]; then
  security find-certificate -c "$SIGN_IDENTITY" >/dev/null 2>&1 || {
    echo "Signing certificate not found: $SIGN_IDENTITY" >&2
    exit 1
  }
  codesign --force --options runtime --identifier "$APP_ID" --sign "$SIGN_IDENTITY" \
    "$ROOT/dictation-target/dictation-target"
fi
