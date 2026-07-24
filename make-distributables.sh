#!/usr/bin/env bash
# make-distributables.sh — one command, run on a Mac, to produce per-OS distributables in dist/.
#
# SCOPE:
#   • macOS  → a full installable app:  dist/macos/My Assistant*.dmg
#   • Linux  → a full installable app:  dist/linux/My Assistant*.deb   (built in Docker)
#   • Windows→ cross-built helper .exes: dist/windows/*-helpers-windows-*.zip
#              (a full .msi needs a Windows runner — tauri can't cross-bundle WiX/WebView2 from Linux;
#               see the CI 'windows-helpers' job)
#
# The Tauri core was ported off macOS-only APIs (MIGRATION.md), so it now compiles on Linux. The Linux
# .deb is built entirely in a Docker container (Rust + Node + tauri-cli + WebKitGTK), bundling the app
# plus all five native helper binaries (meeting-tap, transcribe-server, meeting-state, dictation-target,
# llama-server) freshly compiled for Linux. The .deb targets this Docker host's CPU arch — on Apple
# Silicon that is arm64; set DOCKER_DEFAULT_PLATFORM=linux/amd64 to build x86_64 (emulated, slower).
#
# Each platform is independent: one failing does not abort the others. If Docker is unavailable the
# cross builds are skipped with instructions, and .github/workflows/ci.yml builds them on per-OS runners.
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

DO_MACOS=1 DO_LINUX=1 DO_WINDOWS=1 HELPERS_ONLY=0
for a in "$@"; do case "$a" in
  --macos-only) DO_LINUX=0 DO_WINDOWS=0 ;;
  --linux-only) DO_MACOS=0 DO_WINDOWS=0 ;;
  --windows-only) DO_MACOS=0 DO_LINUX=0 ;;
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

# NB: macOS has no `timeout` command, so don't use it here — `docker info` returns promptly whether
# the daemon is up or down.
docker_ok() { command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; }

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
# Cross-platform builds via Docker. Linux produces a full .deb; Windows cross-builds the helper .exes
# (a full .msi needs a real Windows runner — tauri can't cross-bundle WiX/WebView2 from Linux).
# ---------------------------------------------------------------------------
LINUX_IMG="meeting-assistant-linux-builder"

# The five helper binaries whose macOS versions must be swapped for Linux ones during the .deb build
# and restored after (all are gitignored, reproducible artifacts).
LINUX_HELPERS=(stt/meeting-tap stt/transcribe-server/transcribe-server meeting-state/meeting-state \
               dictation-target/dictation-target polish-runtime/llama-server)

build_linux() {
  say "Linux app distributable (.deb, via Docker)"
  if ! docker_ok; then
    skip "Linux: Docker not usable — start Docker Desktop, or CI builds it on ubuntu-latest"; return
  fi
  # Build the builder image on first use (Rust + Node + tauri-cli + all system -dev packages).
  if ! docker image inspect "$LINUX_IMG" >/dev/null 2>&1; then
    echo "Building the Linux builder image (first run, a few minutes)…"
    docker build -f "$ROOT/native/docker/linux-builder.Dockerfile" -t "$LINUX_IMG" "$ROOT/native/docker" \
      || { fail "Linux: builder image build failed"; return; }
  fi
  # Swap macOS helper binaries for Linux ones for the duration of the build, then restore.
  local bak="$ROOT/.dist-macbak"; mkdir -p "$bak"
  local i p
  for i in "${!LINUX_HELPERS[@]}"; do p="${LINUX_HELPERS[$i]}"; [ -f "$ROOT/$p" ] && cp "$ROOT/$p" "$bak/h$i"; done
  docker run --rm -u "$(id -u):$(id -g)" -e HOME=/tmp \
    -v "$ROOT":/src -w /src "$LINUX_IMG" bash /src/native/docker/build-linux-app.sh
  local rc=$?
  for i in "${!LINUX_HELPERS[@]}"; do p="${LINUX_HELPERS[$i]}"; [ -f "$bak/h$i" ] && cp "$bak/h$i" "$ROOT/$p"; done
  rm -rf "$bak"
  if [ "$rc" -ne 0 ]; then fail "Linux: .deb build failed (rc=$rc)"; return; fi
  local out="$DIST/linux"; mkdir -p "$out"
  local deb
  deb="$(find "$ROOT/.target-linux/release/bundle/deb" -name '*.deb' 2>/dev/null | head -1)"
  if [ -n "$deb" ]; then
    cp "$deb" "$out/"
    ok "Linux: $(basename "$deb") → dist/linux/ (built for this Docker host's arch; set DOCKER_DEFAULT_PLATFORM=linux/amd64 for x86_64)"
  else
    fail "Linux: no .deb produced"
  fi
}

build_windows() {
  say "Windows helper bundle (Docker + cargo-xwin)"
  if ! docker_ok; then
    skip "Windows: Docker not usable — CI job 'windows-helpers' builds these on windows-latest"; return
  fi
  local img="messense/cargo-xwin"
  # HOME=/tmp so cargo-xwin's SDK cache ($HOME/.cache/cargo-xwin) is writable under the mapped non-root
  # uid; a repo-local CARGO_HOME for the same reason. XWIN_CACHE_DIR persists the downloaded MSVC SDK.
  if ! docker run --rm -u "$(id -u):$(id -g)" -v "$ROOT":/src -w /src \
        -e HOME=/tmp -e CARGO_HOME=/src/.cargo-xwin -e XWIN_CACHE_DIR=/src/.cargo-xwin/xwin "$img" bash -c '
      set -e
      cd native/windows && cargo xwin build --release --workspace --target x86_64-pc-windows-msvc --locked
    '; then
    fail "Windows: cross-build failed (some cfg(windows) code may need on-device fixes — MIGRATION Phase 4)"; return
  fi
  local out="$DIST/windows"; mkdir -p "$out"
  local rel="$ROOT/native/windows/target/x86_64-pc-windows-msvc/release"
  # A README so this bundle isn't mistaken for the installer — these are background helpers, not the app.
  cat > "$rel/README.txt" <<'TXT'
These five .exe files are BACKGROUND HELPERS for My Assistant, not the app itself — there is
nothing to install here and running them directly does nothing useful. They are spawned by the
main app (audio capture, text injection, accessibility, meeting detection).

To INSTALL My Assistant on Windows you want the single installer, "My Assistant_<version>_x64-setup.exe",
which bundles the app + these helpers + the WebView2 runtime. That installer must be built on Windows
(tauri's NSIS/WebView2 packaging can't be produced from macOS/Linux). Get it from the GitHub Actions
"windows-app" job artifact ("windows-installer"), or build it on a Windows PC:

    cd desktop && npm ci
    cargo tauri build --bundles nsis
    # -> desktop/src-tauri/target/release/bundle/nsis/My Assistant_<version>_x64-setup.exe
TXT
  ( cd "$rel" && zip -q "$out/meeting-assistant-helpers-windows-x86_64-$VERSION.zip" \
      meeting-tap.exe dictation-target.exe agent-context.exe agent-action.exe meeting-state.exe README.txt ) \
    && ok "Windows: helper .exe bundle → dist/windows/ (helpers only; the single installer -setup.exe comes from the windows-app CI job)" \
    || fail "Windows: packaging failed (exes missing — build may have errored)"
}

# ---------------------------------------------------------------------------
[ "$DO_MACOS" = 1 ] && build_macos
[ "$DO_LINUX" = 1 ] && build_linux
[ "$DO_WINDOWS" = 1 ] && build_windows

# ---------------------------------------------------------------------------
# Summary + machine-readable status.
# ---------------------------------------------------------------------------
{
  echo "# Distributables — build $(date -u '+%Y-%m-%d %H:%M UTC' 2>/dev/null || echo)"
  echo
  printf '%s\n' "${STATUS[@]}"
  echo
  echo "macOS (.dmg) and Linux (.deb) are full installable apps. Windows cross-builds the helper .exes"
  echo "here; its .msi installer needs a Windows runner (tauri can't cross-bundle WiX/WebView2 from"
  echo "Linux) — see the CI 'windows-helpers' job. Linux .deb is built for this Docker host's CPU arch."
} | tee "$DIST/STATUS.md"

say "dist/ contents"; find "$DIST" -type f -not -name STATUS.md | sed "s|$DIST/|  |" || true
# Non-zero exit if any platform hard-failed, so CI/callers notice.
printf '%s\n' "${STATUS[@]}" | grep -q '❌' && exit 1 || exit 0
