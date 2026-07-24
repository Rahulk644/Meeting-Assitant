#!/usr/bin/env bash
# Build the native helper binaries for the CURRENT platform.
#
# There is no cross-compilation here: each OS builds its own helpers on its own machine (or CI
# runner), because they link OS-native libraries (CoreAudio/ScreenCaptureKit on macOS, WASAPI/UIA on
# Windows, PipeWire·Pulse/AT-SPI/X11 on Linux). Run this on each target OS, or let the CI matrix
# (.github/workflows/ci.yml) run all three. See native/PROTOCOL.md and ../MIGRATION.md.
#
# Usage:  native/build-all.sh            # build + test for this OS
#         native/build-all.sh --check    # compile-check only (no release build)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NATIVE="$ROOT/native"
MODE="${1:-build}"
OS="$(uname -s)"

say() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# 1) Shared crate — pure Rust, builds and tests on every OS.
say "helpers-core (shared, all platforms)"
( cd "$NATIVE/helpers-core" && cargo test )
if [ "$MODE" = build ]; then ( cd "$NATIVE/helpers-core" && cargo build --release ); fi

case "$OS" in
  Darwin)
    say "macOS helpers (Swift)"
    echo "The macOS app's helpers are the Swift tools in stt/, meeting-state/, dictation-target/."
    echo "Build the full native set (incl. model runtimes) with: ./script/build_native_resources.sh"
    echo "Building just the fast Swift helper subset here:"
    xcrun swiftc -O "$ROOT/meeting-state/meeting-state.swift" -o "$ROOT/meeting-state/meeting-state" \
      -framework AppKit -framework ApplicationServices
    xcrun swiftc -O "$ROOT/dictation-target/dictation-target.swift" -o "$ROOT/dictation-target/dictation-target" \
      -framework AppKit -framework ApplicationServices
    xcrun swiftc -O "$ROOT/stt/meeting-tap.swift" -o "$ROOT/stt/meeting-tap" \
      -framework CoreAudio -framework AudioToolbox -framework AVFoundation \
      -framework ScreenCaptureKit -framework CoreGraphics -framework CoreImage \
      -framework AppKit -framework ApplicationServices
    "$ROOT/meeting-state/meeting-state" --selftest
    "$ROOT/dictation-target/dictation-target" --selftest
    "$ROOT/stt/meeting-tap" --selftest
    echo "✓ macOS Swift helpers built + selftested"
    say "note"
    echo "Windows/Linux helper binaries are NOT built here — run this script on those OSes (or CI)."
    echo "The full Windows/Linux *application* additionally needs the core de-macOS work (MIGRATION.md Phase 1–3)."
    ;;

  Linux)
    say "Linux helpers (native/linux → x86_64-unknown-linux-gnu)"
    echo "Requires the -dev packages listed in native/linux/README.md"
    echo "(libpulse-dev libpipewire-0.3-dev clang gstreamer1.0 plugins libxcb/xtst-dev at-spi2-core)."
    if [ "$MODE" = check ]; then
      ( cd "$NATIVE/linux" && cargo check --workspace )
    else
      ( cd "$NATIVE/linux" && cargo build --release --workspace && cargo test --workspace )
      for b in meeting-tap dictation-target agent-context agent-action meeting-state; do
        "$NATIVE/linux/target/release/$b" --selftest && echo "  ✓ $b --selftest"
      done
    fi
    echo "✓ Linux helpers in native/linux/target/release/"
    ;;

  MINGW* | MSYS* | CYGWIN*)
    say "Windows helpers"
    echo "Use the PowerShell harness on Windows: native/windows/build.ps1"
    echo "(cargo build --release --target x86_64-pc-windows-msvc)"
    ;;

  *)
    echo "Unsupported host OS: $OS — build helpers on macOS, Windows, or Linux." >&2
    exit 1
    ;;
esac

say "done"
echo "Per-OS artifacts stage into the resource layout native_host expects; see ../MIGRATION.md Phase 3."
