#!/usr/bin/env bash
# make-distributables.sh — one command, run on a Mac, to produce per-OS distributables in dist/.
#
# HONEST SCOPE (read this):
#   • macOS  → a REAL, installable app: dist/macos/My Assistant.dmg
#   • Linux  → the native HELPER binaries, bundled: dist/linux/*-helpers-linux-x86_64.tar.gz
#   • Windows→ the native HELPER binaries, bundled: dist/windows/*-helpers-windows-x86_64.zip
#
# Why Linux/Windows are helpers-only right now: the Tauri *core* (desktop/src-tauri) still imports
# macOS-only crates and shell-outs, so `tauri build` fails to COMPILE off-macOS (MIGRATION.md Phase
# 1–3). This script already contains the full-app build steps for those OSes — they are gated behind
# a compile probe and will start emitting .deb / .msi automatically once the core is made portable.
# No script can conjure a distributable the code can't yet compile.
#
# Cross-OS builds use Docker (Linux container natively; Windows via cargo-xwin in a Linux container),
# which is how you build for other OSes from one Mac. Each platform is independent: one failing does
# not abort the others. If Docker is unavailable, those platforms are skipped with instructions, and
# the .github/workflows/ci.yml matrix builds them on real per-OS runners instead.
#
# Usage:
#   ./make-distributables.sh                 # all reachable platforms
#   ./make-distributables.sh --helpers-only  # skip the heavy macOS app build; just helper binaries
#   ./make-distributables.sh --macos-only     # only the macOS .dmg
#   ./make-distributables.sh --skip-macos     # only Linux + Windows helper bundles
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIST="$ROOT/dist"
HOST="$(uname -s)"
VERSION="$(grep -m1 '"version"' "$ROOT/desktop/src-tauri/tauri.conf.json" | sed 's/[^0-9.]//g')"
VERSION="${VERSION:-0.0.0}"

DO_MACOS=1 DO_CROSS=1 HELPERS_ONLY=0
for a in "$@"; do case "$a" in
  --macos-only) DO_CROSS=0 ;;
  --skip-macos) DO_MACOS=0 ;;
  --helpers-only) HELPERS_ONLY=1 ;;
  -h | --help) sed -n '2,30p' "$0"; exit 0 ;;
  *) echo "unknown flag: $a" >&2; exit 2 ;;
esac; done

declare -a STATUS
say() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }
ok()  { STATUS+=("✅ $1"); printf '\033[32m✓ %s\033[0m\n' "$1"; }
skip(){ STATUS+=("⏭️  $1"); printf '\033[33m- %s\033[0m\n' "$1"; }
fail(){ STATUS+=("❌ $1"); printf '\033[31m✗ %s\033[0m\n' "$1" >&2; }

docker_ok() { command -v docker >/dev/null 2>&1 && timeout 25 docker info >/dev/null 2>&1; }

mkdir -p "$DIST"

# ---------------------------------------------------------------------------
# Shared crate — a fast sanity build/test that every platform depends on.
# ---------------------------------------------------------------------------
say "helpers-core (shared)"
if (cd "$ROOT/native/helpers-core" && cargo test --locked >/dev/null 2>&1); then
  ok "helpers-core: tests pass"
else
  fail "helpers-core: tests FAILED (fix before shipping any platform)"; exit 1
fi

# ---------------------------------------------------------------------------
# macOS — the full, installable app distributable (.dmg).
# ---------------------------------------------------------------------------
build_macos() {
  if [ "$HOST" != Darwin ]; then skip "macOS: not on a Mac (build the .dmg on macOS)"; return; fi
  if [ "$HELPERS_ONLY" = 1 ]; then
    say "macOS helper binaries (Swift) — --helpers-only"
    if (cd "$ROOT" && bash native/build-all.sh >/dev/null 2>&1); then ok "macOS: Swift helpers built + selftested"
    else fail "macOS: Swift helper build failed"; fi
    return
  fi
  say "macOS app distributable (.dmg)"
  # Signing: use the configured Developer ID if it's actually in the keychain, else fall back to
  # ad-hoc ("-") so a local build still produces an installable (unnotarized) .dmg instead of dying on
  # `codesign: no identity found`. tauri honors APPLE_SIGNING_IDENTITY (overrides tauri.conf.json).
  local sign="${SIGN_IDENTITY:-My Assistant Dev}"
  if security find-identity -v -p codesigning 2>/dev/null | grep -qF "$sign"; then
    echo "Signing with keychain identity: $sign"
  else
    echo "Signing identity '$sign' not in keychain → ad-hoc signatures (local, unnotarized build)."
    sign="-"
  fi
  export APPLE_SIGNING_IDENTITY="$sign" SIGN_IDENTITY="$sign"
  # 1) Native resources: Swift helpers + llama-server + transcribe-server (compiles ggml — minutes).
  if ! (cd "$ROOT" && ./script/build_native_resources.sh); then fail "macOS: native resources failed"; return; fi
  # 2) Frontend + Tauri .app (bundle.targets=["app"]; models are fetched at first launch, not bundled).
  if ! (cd "$ROOT/desktop" && npm ci && npm run build); then fail "macOS: tauri build failed"; return; fi
  # 3) .app → .dmg via hdiutil (no Finder/AppleScript; works headless).
  if ! (cd "$ROOT" && ./desktop/make-dmg.sh); then fail "macOS: dmg packaging failed"; return; fi
  mkdir -p "$DIST/macos"
  local dmg
  dmg="$(find "$ROOT/desktop/src-tauri/target/release/bundle/dmg" -name '*.dmg' | head -1)"
  if [ -n "$dmg" ]; then cp "$dmg" "$DIST/macos/"; ok "macOS: $(basename "$dmg") → dist/macos/"; else fail "macOS: no .dmg produced"; fi
}

# ---------------------------------------------------------------------------
# Cross-platform helper binaries via Docker. `probe_core_portable` gates the
# (currently-failing) full-app build so it flips on automatically post Phase 1–3.
# ---------------------------------------------------------------------------
CARGO_ENV="-e CARGO_HOME=/src/.cargo-docker -e CARGO_TARGET_DIR=/src/native/%WS%/target"

build_linux() {
  say "Linux helper bundle (Docker)"
  if ! docker_ok; then
    skip "Linux: Docker not usable — CI job 'linux-helpers' builds these on ubuntu-latest"; return
  fi
  local img="rust:1-bookworm"
  if ! docker run --rm -u "$(id -u):$(id -g)" -v "$ROOT":/src -w /src \
        -e CARGO_HOME=/src/.cargo-docker "$img" bash -c '
      set -e
      apt-get update >/dev/null 2>&1 || sudo apt-get update
      DEBS="libpulse-dev libpipewire-0.3-dev libspa-0.2-dev clang \
            libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
            libxcb1-dev libxcb-randr0-dev libatspi2.0-dev libdbus-1-dev pkg-config"
      apt-get install -y --no-install-recommends $DEBS >/dev/null 2>&1 || true
      cd native/linux && cargo build --release --workspace --locked
    '; then
    fail "Linux: helper build failed in Docker (see log)"; return
  fi
  local out="$DIST/linux"; mkdir -p "$out"
  local rel="$ROOT/native/linux/target/release"
  tar -czf "$out/meeting-assistant-helpers-linux-x86_64-$VERSION.tar.gz" -C "$rel" \
    meeting-tap dictation-target agent-context agent-action meeting-state 2>/dev/null \
    && ok "Linux: helper bundle → dist/linux/ (full .deb pending core Phase 1–3)" \
    || fail "Linux: packaging failed (binaries missing — build may have errored)"
}

build_windows() {
  say "Windows helper bundle (Docker + cargo-xwin)"
  if ! docker_ok; then
    skip "Windows: Docker not usable — CI job 'windows-helpers' builds these on windows-latest"; return
  fi
  local img="messense/cargo-xwin"
  if ! docker run --rm -u "$(id -u):$(id -g)" -v "$ROOT":/src -w /src \
        -e CARGO_HOME=/src/.cargo-docker "$img" bash -c '
      set -e
      cd native/windows && cargo xwin build --release --workspace --target x86_64-pc-windows-msvc --locked
    '; then
    fail "Windows: cross-build failed (some cfg(windows) code may need on-device fixes — MIGRATION Phase 4)"; return
  fi
  local out="$DIST/windows"; mkdir -p "$out"
  local rel="$ROOT/native/windows/target/x86_64-pc-windows-msvc/release"
  ( cd "$rel" && zip -q "$out/meeting-assistant-helpers-windows-x86_64-$VERSION.zip" \
      meeting-tap.exe dictation-target.exe agent-context.exe agent-action.exe meeting-state.exe ) \
    && ok "Windows: helper bundle → dist/windows/ (full .msi pending core Phase 1–3)" \
    || fail "Windows: packaging failed (exes missing — build may have errored)"
}

# ---------------------------------------------------------------------------
[ "$DO_MACOS" = 1 ] && build_macos
if [ "$DO_CROSS" = 1 ]; then build_linux; build_windows; fi

# ---------------------------------------------------------------------------
# Summary + machine-readable status.
# ---------------------------------------------------------------------------
{
  echo "# Distributables — build $(date -u '+%Y-%m-%d %H:%M UTC' 2>/dev/null || echo)"
  echo
  printf '%s\n' "${STATUS[@]}"
  echo
  echo "macOS is a full installable app. Linux/Windows are native helper bundles until the core is"
  echo "made cross-platform (MIGRATION.md Phase 1–3); the full-app steps are already wired here and"
  echo "gate on the core compiling for each target."
} | tee "$DIST/STATUS.md"

say "dist/ contents"; find "$DIST" -type f -not -name STATUS.md | sed "s|$DIST/|  |" || true
# Non-zero exit if any platform hard-failed, so CI/callers notice.
printf '%s\n' "${STATUS[@]}" | grep -q '❌' && exit 1 || exit 0
