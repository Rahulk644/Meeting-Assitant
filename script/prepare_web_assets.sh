#!/usr/bin/env bash
# Produce the small release WebView bundle. Source stays readable in app/ for development and tests;
# only the minified UI plus a resolved release contract is shipped in the .app.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/desktop/web-dist"
MINIFIER="$ROOT/desktop/node_modules/.bin/html-minifier-terser"
RELEASE_HTML="$(mktemp "${TMPDIR:-/tmp}/my-assistant-release-html.XXXXXX")"
trap 'rm -f "$RELEASE_HTML"' EXIT

[[ -x "$MINIFIER" ]] || {
  echo "html-minifier-terser is missing; run 'cd desktop && npm install'" >&2
  exit 1
}

rm -rf "$OUT"
mkdir -p "$OUT"

COMMON=(--collapse-whitespace --remove-comments --remove-attribute-quotes --remove-redundant-attributes \
        --remove-script-type-attributes --remove-style-link-type-attributes --sort-attributes --sort-class-name \
        --minify-css true --minify-js true)
node "$ROOT/script/prepare_release_html.cjs" "$ROOT/app/index.html" "$RELEASE_HTML"
"$MINIFIER" "${COMMON[@]}" --output "$OUT/index.html" "$RELEASE_HTML"

node -e 'const {releasePayload}=require(process.argv[1]);process.stdout.write(JSON.stringify(releasePayload(process.env.ASSISTANT_RELEASE_CHANNEL)))' \
  "$ROOT/app/release.js" > "$OUT/release-payload.json"
cp "$ROOT/app/logo.png" "$OUT/"

echo "Prepared minified WebView assets: $OUT"
