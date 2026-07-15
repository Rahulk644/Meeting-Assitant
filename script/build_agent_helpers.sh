#!/usr/bin/env bash
# Build the two short-lived native Accessibility helpers used by Agents.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SIGN_IDENTITY="${SIGN_IDENTITY:-}"
APP_ID="com.myassistant.desktop"

xcrun swiftc -O "$ROOT/agent-context/agent-context.swift" -o "$ROOT/agent-context/agent-context" \
  -framework AppKit -framework ApplicationServices -framework CoreGraphics -framework CryptoKit
xcrun swiftc -O "$ROOT/agent-action/agent-action.swift" -o "$ROOT/agent-action/agent-action" \
  -framework AppKit -framework ApplicationServices -framework CoreGraphics -framework CryptoKit

"$ROOT/agent-context/agent-context" --selftest
"$ROOT/agent-action/agent-action" --selftest

if [[ -n "$SIGN_IDENTITY" ]]; then
  security find-certificate -c "$SIGN_IDENTITY" >/dev/null 2>&1 || {
    echo "Signing certificate not found: $SIGN_IDENTITY" >&2
    exit 1
  }
  codesign --force --options runtime --identifier "$APP_ID" --sign "$SIGN_IDENTITY" \
    "$ROOT/agent-context/agent-context"
  codesign --force --options runtime --identifier "$APP_ID" --sign "$SIGN_IDENTITY" \
    "$ROOT/agent-action/agent-action"
fi
