#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
crate_dir="$repo_root/desktop/src-tauri"
release_target="aarch64-apple-darwin"

# RUSTSEC-2026-0235 is present only as rust_decimal's optional rkyv feature in Cargo.lock. cargo-audit scans
# lockfile package metadata rather than the activated feature graph. Refuse the exception immediately if
# rkyv 0.7 ever becomes a normal dependency of the Apple Silicon release target.
active_rkyv="$({
  cd "$crate_dir"
  cargo tree --locked --target "$release_target" --edges normal -i rkyv@0.7.46 2>/dev/null || true
})"
if [[ "$active_rkyv" == *"rkyv v0.7.46"* ]]; then
  echo "RUSTSEC-2026-0235 exception is invalid: rkyv 0.7.46 entered the release graph" >&2
  exit 1
fi

cd "$crate_dir"
cargo audit --ignore RUSTSEC-2026-0235
