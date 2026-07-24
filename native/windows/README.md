# Windows native helpers

Rust implementations of the five native helper binaries the Tauri desktop app spawns, for
`x86_64-pc-windows-msvc`. They are drop-in replacements for the macOS Swift helpers: **same argv,
same stdin/stdout framing, same exit codes, same JSON field names**. The authoritative contract is
[`../PROTOCOL.md`](../PROTOCOL.md); the shared, OS-agnostic logic lives in
[`../helpers-core`](../helpers-core) (a path dependency, its public API frozen).

This is a **standalone cargo workspace** (note the empty `[workspace]` table in `Cargo.toml`) — it is
deliberately not a member of the mac app's cargo workspace.

## What's here

One package, one `platform` library + five `[[bin]]` targets:

| Binary | PROTOCOL § | Purpose |
| --- | --- | --- |
| `meeting-tap` | 1 | System + mic audio → whisper → captions; optional H.264 video. HTTP control plane. |
| `dictation-target` | 2 | Classify the focused field; inject dictated text (`--session`). |
| `agent-context` | 3 | One bounded, read-only UI Automation snapshot. |
| `agent-action` | 4 | One guarded UI mutation (apply/undo) on `notepad.exe`. |
| `meeting-state` | 5 | Track meeting-app state; stream NDJSON. |

## Build

Native (on a Windows box with the MSVC toolchain + Windows SDK):

```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --target x86_64-pc-windows-msvc --release
```

Outputs land in `target/x86_64-pc-windows-msvc/release/{meeting-tap,dictation-target,agent-context,agent-action,meeting-state}.exe`.

Cross-compile from macOS/Linux with [`cargo-xwin`](https://github.com/rust-cross/cargo-xwin) (pulls the
MSVC CRT + Windows SDK headers, no Windows box needed to link):

```bash
cargo install cargo-xwin
rustup target add x86_64-pc-windows-msvc
cargo xwin build --target x86_64-pc-windows-msvc --release
```

Smoke-test each binary without hardware — the `--selftest` pure-logic checks are real:

```powershell
foreach ($b in "meeting-tap","dictation-target","agent-context","agent-action","meeting-state") {
  .\target\x86_64-pc-windows-msvc\release\$b.exe --selftest
}
```

## Dependencies

| Crate | Version | Used for |
| --- | --- | --- |
| `helpers-core` | path | Frozen shared core: protocol structs, `Segmenter`, `wav`, fingerprints, watchdog, args. |
| `ureq` | 2 | Blocking control-plane HTTP: config pull, `/ingest` POST, whisper multipart. |
| `serde` / `serde_json` | 1 | Wire (de)serialization; sorted-key output for the snapshot/action helpers. |
| `uuid` | 1 (v4) | `snapshotId` / temp-WAV names. |
| `windows` | 0.62 | COM + Win32: WASAPI audio sessions, `EnumWindows`, Toolhelp32, `SendInput`, console-ctrl. |
| `wasapi` | 0.23 | Per-application process loopback + mic capture. |
| `windows-capture` | 2 | Screen capture → H.264 mp4 + thumbnail. |
| `uiautomation` | 0.24 | UI Automation reads/writes (context/action/dictation/state). |
| `enigo` | 0.6 | Unicode text injection (`KEYEVENTF_UNICODE`). |
| `arboard` | 3 | Clipboard fallback for paste-based injection. |

The `windows`/`wasapi`/`windows-capture`/`uiautomation`/`enigo`/`arboard` crates are **target-gated to
Windows**, so `cargo check` on a non-Windows host only builds the portable spine + the
`#[cfg(not(windows))]` fallbacks.

## Packaging as Tauri sidecars

The app resolves helper paths through `desktop/src-tauri/src/native_host.rs` as
`<resource_dir>/<subdir>/<stem>` and appends `.exe` on Windows. **You must ship the helpers as
`bundle.resources`, not `externalBin`** — see the caveat below.

**Resource files (what the mac build already does).** Mirror the macOS `resources` map in
`tauri.conf.json`, pointing at the built Windows exes, e.g.:

```jsonc
"resources": {
  "../../native/windows/target/x86_64-pc-windows-msvc/release/meeting-tap.exe": "stt/meeting-tap.exe",
  "../../native/windows/target/x86_64-pc-windows-msvc/release/meeting-state.exe": "meeting-state/meeting-state.exe",
  "../../native/windows/target/x86_64-pc-windows-msvc/release/dictation-target.exe": "dictation-target/dictation-target.exe"
}
```
(agent-context / agent-action are dormant research helpers — not packaged in production; see the status
table above and `script/build_agent_helpers.sh`.)

> ⚠️ **Do NOT use `bundle.externalBin` for these helpers.** Tauri copies `externalBin` sidecars next
> to the **main executable**, but `native_host` resolves helpers under `resource_dir()` — so a sidecar
> would never be found at runtime. Only the `resources` map (which lands files under `resource_dir()`
> at the exact `<subdir>/<stem>.exe` paths `native_host` builds) works with the current spawn code.

## Authenticode signing

Windows SmartScreen/Defender will flag unsigned helpers, and a helper that synthesizes input
(`SendInput`) or reads other apps' UI (UI Automation) is exactly the kind of binary users are warned
about. Sign every `.exe` (and the installer) with an **Authenticode** cert — ideally an EV/OV
code-signing cert so SmartScreen reputation accrues:

```powershell
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a meeting-tap.exe
signtool verify /pa meeting-tap.exe
```

Sign in CI after the release build, before bundling. No special manifest/UAC elevation is required —
the helpers run at the user's integrity level (UI Automation into elevated windows is refused by
UIPI, which `dictation-target` reports as `unsafeFallback:true`).

## Per-helper status

Legend (from PROTOCOL): **portable** = OS-agnostic shared logic · **implemented** = real Win32/COM
call sequence · **TODO(on-device)** = correct API identified, needs a Windows box + hardware to finish
and verify (never silently degraded — always logged).

| Helper | Spine (portable) | Native reads/writes | Notes |
| --- | --- | --- | --- |
| `meeting-tap` | ✅ args, config pull, `coreaudio` state + caption POSTs, `Segmenter→wav→whisper→caption`, `--selftest`, `--list-apps`, watchdog, console-ctrl + stdin-EOF shutdown | ✅ WASAPI Audio-Session **detection** (`windows` crate); ⚠️ **TODO(on-device)** WASAPI process-loopback + mic **capture** (`wasapi` 0.23) and windows-capture **video** | Detection is high-confidence; sample streaming + mp4 finalize need a device. |
| `dictation-target` | ✅ classification `Result`, session taxonomy + caps + framing, EOF cleanup | ⚠️ **TODO(on-device)** UIA classify (`IsPassword`, TextPattern) + `SendInput` UIPI detection + provisional reconciliation | Off-device reports `unknown`/`liveEditable:false`. |
| `agent-context` | ✅ arg dispatch, sorted-key JSON, `AX*` role map, secure redaction, fingerprints, privacy tags, degraded snapshots | ⚠️ **TODO(on-device)** UIA BFS + **CacheRequest** batching (0.75 s budget) + bounding rects | Off-device degrades to `no_window`. |
| `agent-action` | ✅ stdin 256 KB cap, request parse, validation, `replacing` splice, `mutation_disposition`, fingerprints, undo record, `--selftest` | ⚠️ **TODO(on-device)** resolve live control, activation, `ValuePattern.SetValue`/select+inject, ~30 ms verify | Gated to `notepad.exe`; off-device returns `permission_required`. |
| `meeting-state` | ✅ arg dispatch, unbuffered NDJSON, control derivation + name filtering, `--selftest`, watchdog | ✅ process enum (Toolhelp32) + audio-session in-call signal; ⚠️ **TODO(on-device)** UIA toolbar/participant harvesting | Wake signal is core-mediated (documented in the source). |

### Host-verification caveat

This crate was authored on macOS; it **cannot be compiled for `x86_64-pc-windows-msvc` here** (no MSVC
toolchain/SDK). The portable spine + `#[cfg(not(windows))]` fallbacks are structured to `cargo check`
on the host, but the Windows-native code paths (`#[cfg(windows)]`) are **unverified on-host** and each
uncertain call is marked `// TODO(on-device):` at the exact line needing hardware. First device build
should expect to reconcile the `windows` 0.62 / `wasapi` 0.23 / `uiautomation` 0.24 / `enigo` 0.6 /
`windows-capture` 2 API surfaces against these call sequences.
