#!/usr/bin/env bash
# Fail closed when a production bundle contains stale, unsigned, wrong-architecture, or preview resources.
set -euo pipefail

APP="${1:?usage: verify_app_bundle.sh /path/to/My\ Assistant.app [signing-identity]}"
SIGN_IDENTITY="${2:-${SIGN_IDENTITY:-My Assistant Dev}}"
RESOURCES="$APP/Contents/Resources"

[[ -d "$APP" ]] || { echo "App bundle not found: $APP" >&2; exit 1; }
codesign --verify --deep --strict "$APP"

signed_runtime() {
  local executable="$1"
  [[ -x "$executable" ]] || { echo "Missing executable resource: $executable" >&2; exit 1; }
  codesign --verify --strict "$executable"
  local details
  details="$(codesign -dv --verbose=4 "$executable" 2>&1)"
  [[ "$details" == *"flags="*"runtime"* ]] || {
    echo "Hardened runtime is missing: $executable" >&2
    exit 1
  }
  if [[ "$SIGN_IDENTITY" != "-" ]]; then
    [[ "$details" == *"Authority=$SIGN_IDENTITY"* ]] || {
      echo "Unexpected signing authority: $executable" >&2
      exit 1
    }
  fi
  [[ "$(lipo -archs "$executable")" == "arm64" ]] || {
    echo "Release executable is not arm64-only: $executable" >&2
    exit 1
  }
}

signed_runtime "$APP/Contents/MacOS/app"
signed_runtime "$RESOURCES/stt/meeting-tap"
signed_runtime "$RESOURCES/stt/transcribe-server/transcribe-server"
signed_runtime "$RESOURCES/meeting-state/meeting-state"
signed_runtime "$RESOURCES/dictation-target/dictation-target"
signed_runtime "$RESOURCES/polish-runtime/llama-server"

ENTITLEMENTS="$(codesign -d --entitlements :- "$APP" 2>&1)"
[[ "$ENTITLEMENTS" == *"com.apple.security.device.audio-input"* ]] || {
  echo "The hardened app is missing its microphone entitlement." >&2
  exit 1
}

[[ ! -e "$RESOURCES/agent-context" && ! -e "$RESOURCES/agent-action" ]] || {
  echo "Agent helpers must not be packaged in the initial product." >&2
  exit 1
}
if find "$RESOURCES" -type f \( -name '*.gguf' -o -name '*.bin' \) -print -quit | grep -q .; then
  echo "Model weights must be downloaded separately, not bundled." >&2
  exit 1
fi

node - "$RESOURCES/webui/release-payload.json" <<'NODE'
const fs = require("node:fs");
const release = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
if (release.channel !== "production") throw new Error("packaged channel is not production");
if (release.capabilities?.agents !== false) throw new Error("Agents must be disabled");
if (release.capabilities?.dictation !== true || release.capabilities?.meetings !== true) {
  throw new Error("Dictation and Meetings must remain enabled");
}
NODE

echo "Verified production app bundle: $APP"
