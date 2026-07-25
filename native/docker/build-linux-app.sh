#!/usr/bin/env bash
# Runs INSIDE the linux-builder container (see native/docker/linux-builder.Dockerfile). Builds every
# Linux helper binary, stages them at the resource paths tauri.conf.json bundles, and produces the
# .deb. Tolerant: any helper that fails to build is stubbed so a .deb is always produced (that helper's
# feature is just degraded). The host wrapper in make-distributables.sh backs up/restores the macOS
# helper binaries around this.
set -uo pipefail
# Repo root relative to this script, so it works both in the Docker mount (/src) and on a CI runner.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
# Force a writable, repo-local CARGO_HOME: the rust Docker image pre-sets CARGO_HOME=/usr/local/cargo
# (root-owned), which a mapped non-root uid cannot write. A repo-local dir works both in Docker and CI.
export CARGO_HOME="$ROOT/.cargo-docker"

stub() { printf '#!/bin/sh\nexit 1\n' > "$1"; chmod +x "$1"; echo "  ⚠ STUBBED $1 (feature degraded)"; }

echo "== [1/4] C++ model runtimes (Linux, CPU backend) =="
# The build scripts self-heal a cmake build dir configured for a different source path (host /Users vs
# container /src), so a repo previously built on macOS reconfigures cleanly for Linux here.
if GGML_BACKEND=cpu bash stt/transcribe-server/build.sh; then echo "  ✓ transcribe-server"; else stub stt/transcribe-server/transcribe-server; fi
if GGML_BACKEND=cpu bash script/build_polish_runtime.sh; then echo "  ✓ llama-server"; else stub polish-runtime/llama-server; fi

echo "== [2/4] native/linux Rust helpers =="
if ( cd native/linux && CARGO_TARGET_DIR="$ROOT/.target-linux-helpers" cargo build --release --workspace ); then
  HB="$ROOT/.target-linux-helpers/release"
  cp "$HB/meeting-tap"        stt/meeting-tap
  cp "$HB/meeting-state"      meeting-state/meeting-state
  cp "$HB/dictation-target"   dictation-target/dictation-target
  chmod +x stt/meeting-tap meeting-state/meeting-state dictation-target/dictation-target
  echo "  ✓ native helpers staged"
else
  echo "  native/linux build failed → stubbing the three spawned helpers"
  stub stt/meeting-tap; stub meeting-state/meeting-state; stub dictation-target/dictation-target
fi

echo "== [3/4] resource helper arch check =="
for p in stt/meeting-tap stt/transcribe-server/transcribe-server meeting-state/meeting-state \
         dictation-target/dictation-target polish-runtime/llama-server; do
  printf '  %-45s ' "$p"; file "$p" | sed 's/.*: //'
done

echo "== [4/4] bundle .deb =="
cd desktop/src-tauri
CARGO_TARGET_DIR="$ROOT/.target-linux" cargo tauri build --bundles deb
