# Build the Windows native helper binaries. Run on Windows (or a Windows CI runner) with the MSVC
# toolchain + Rust installed. Cross-building from macOS/Linux is possible with cargo-xwin, but the
# native build here is the reference path. See ../PROTOCOL.md and ../../MIGRATION.md.
#
#   pwsh native/windows/build.ps1            # build + test
#   pwsh native/windows/build.ps1 -Check     # compile-check only
param([switch]$Check)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$target = "x86_64-pc-windows-msvc"

Write-Host "==> helpers-core (shared)" -ForegroundColor Cyan
Push-Location (Join-Path $here "..\helpers-core")
cargo test
Pop-Location

Write-Host "==> Windows helpers (native/windows -> $target)" -ForegroundColor Cyan
Push-Location $here
if ($Check) {
    cargo check --workspace --target $target
} else {
    cargo build --release --workspace --target $target
    cargo test --workspace
    $rel = Join-Path $here "target\$target\release"
    foreach ($b in @("meeting-tap", "dictation-target", "agent-context", "agent-action", "meeting-state")) {
        & (Join-Path $rel "$b.exe") --selftest
        Write-Host "  OK $b.exe --selftest"
    }
    Write-Host "Artifacts: $rel\*.exe"
}
Pop-Location

Write-Host "==> done" -ForegroundColor Cyan
Write-Host "Stage the .exe helpers into tauri.conf.json bundle.resources at <subdir>/<stem>.exe"
Write-Host "(NOT externalBin — native_host resolves under resource_dir; see ../windows/README.md)."
