# Windows implementation handoff

This document is the starting packet for a maintainer working on a physical Windows machine. It records the
shared contracts, first build sequence, adapter boundaries, and proof required to make Windows claims without
asking that maintainer to rediscover the macOS implementation.

Nothing in this document is Windows verification. The current development machine is a Mac, the current
native helpers are macOS-only, and Windows remains planned work. A macOS cross-compile is not a substitute
for a native Windows build, installer, permission flow, real microphone, loopback capture, target insertion,
or meeting-platform acceptance.

## What is ready to reuse

The product is deliberately split into one shared core plus platform adapters:

- `desktop/src-tauri/src/meeting_detection.rs` is OS-free. It owns the two-matching-observation start,
  three-miss end, source-family controls, stable identity, and metadata-refresh policy.
- `POST /api/meeting/observation` is the bounded adapter-to-core contract. Platform code reports evidence;
  it does not decide that a meeting started or ended.
- the local meeting lifecycle, SSE transcript feed, meeting library, corrections, scratchpad, notes, Ask,
  and grounded Live Assist live above the platform boundary;
- the production Web UI is shared through Tauri/WebView; Windows does not need a second UI implementation;
  and
- [`OSS_COMPONENT_STRATEGY.md`](OSS_COMPONENT_STRATEGY.md) defines the `AudioSource`,
  `MeetingEvidenceSource`, `TranscriptEngine`, and `Diarizer` seams and the qualification rule for upstream
  components.

The following are not portable and must stay behind target-specific adapters:

- Core Audio process taps and macOS audio-process enumeration;
- Accessibility/AppKit window and active-speaker queries;
- the bare-modifier event tap, focused-target insertion, AppKit overlay, and macOS permissions;
- ScreenCaptureKit video capture; and
- Apple code signing, entitlements, DMG packaging, and notarization.

Do not translate Swift line by line. Preserve the bounded input/output contracts and implement the Windows
API behavior directly.

## Windows workstation

Use a supported Windows 11 x64 machine first. Add ARM64 only after x64 works. Install:

1. Git and Node.js LTS.
2. Microsoft C++ Build Tools with **Desktop development with C++** selected.
3. Microsoft Edge WebView2 Evergreen Runtime if the machine does not already have it.
4. Rust stable with the `x86_64-pc-windows-msvc` host/toolchain.
5. The repository dependencies with `npm ci` in `app/` and `desktop/`.

These prerequisites follow the current
[official Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/). Build the installer on
Windows; Tauri documents that MSI creation is Windows-only, and native testing is required regardless of
whether a non-Windows host can cross-compile an executable.

Keep model files, native outputs, recordings, transcripts, acceptance evidence, `.exe`/`.msi`/installer
artifacts, and local app data outside Git.

## First Windows task: X-00

The first owner should make the existing shared core build on Windows before implementing audio:

1. create target-specific Cargo dependency sections so Apple frameworks are never selected on Windows;
2. put current macOS startup, shortcut, target, overlay, permission, and helper-spawn code behind explicit
   `cfg(target_os = "macos")` modules;
3. add capability-reporting Windows stubs for unsupported platform adapters; an unavailable capability must
   fail closed rather than silently succeed;
4. compile and run all OS-free unit and route-contract tests natively on Windows;
5. keep the production navigation contract unchanged: Dictation, Meetings **Beta**, and one disabled
   **Agents — Coming soon** label; and
6. record failures and exact tool versions in the Windows task/CI result, not in durable project-state docs.

Minimum first commands, adjusted only for repository scripts that have been made cross-platform:

```powershell
cd app
npm test

cd ..\desktop
npm ci

cd src-tauri
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The task is complete only on a Windows host or Windows CI runner. A Mac-side `cargo check --target ...` may
find obvious dependency leaks, but it cannot close X-00.

## Adapter work after X-00

Implement each adapter behind the same shared product contracts:

| Adapter | Windows implementation direction | First proof |
|---|---|---|
| `AudioSource` | Qualify FlexAudio first; otherwise use WASAPI mic, loopback, and process-loopback adapters | Separate mic/system chunks, device changes, long-call stability, resource baseline |
| `MeetingEvidenceSource` | WASAPI audio-session/process evidence plus bounded UI Automation window semantics | Zoom native and Google Meet in Chrome do not false-start from an open app or media tab |
| Shortcut | RegisterHotKey/raw input or another narrowly scoped native implementation | Works with the app closed/backgrounded; conflicts and permission failures are visible |
| Focused target | UI Automation with a fail-closed clipboard fallback | Inserts only into the still-owned editable target; secure fields are excluded |
| Overlay | Native/Tauri Windows overlay with no focus theft | Correct monitor, scaling, full-screen, and screen-share behavior |
| Permissions | Windows privacy/capability reporting | Microphone and capture denial lead to one accurate recovery action |
| Video | Optional Windows Graphics Capture adapter | Separately enabled and never required for audio-only meetings |

FlexAudio currently advertises one Rust API for microphone, system loopback, per-process capture, hot-swap,
and device notifications across Windows, macOS, and Linux. It remains a young `0.x` component and requires a
newer Rust toolchain than this repository currently declares, so qualify it behind `AudioSource`; do not
make it a prerequisite for X-00.

## Shared fixtures to preserve

The Windows implementation must use the existing observation schema rather than introduce Windows concepts
into product policy:

```json
{
  "identity": "stable-native-target-id",
  "app": "Google Chrome",
  "title": "Weekly review - Google Meet",
  "bundle": "windows-process-or-adapter-id",
  "family": "browser"
}
```

Required shared behavior:

- two matching observations produce one start transition;
- three missing observations produce one end transition;
- identity switching does not split a stable call on a single sample;
- desktop and browser/huddle sources obey their independent enable switches;
- browser and multipurpose communication apps require both audio and meeting-window evidence;
- manual Start remains available when automatic detection is unsupported;
- calendar context may enrich metadata but never starts recording;
- unsupported participant/chat/video evidence is reported as unavailable, not as an empty success; and
- no platform handle, PID, HWND, UI Automation object, WASAPI type, or Apple type enters the stored meeting
  schema.

The Windows capture adapter must also publish the existing bounded `capture-status` contract: a per-session
monotonic sequence plus `starting|ready|failed|off|idle` microphone and meeting-audio channels. The shared
Rust `MeetingUiState` derives Starting, Recording, Needs attention, Pausing, Paused, Finalizing, and Idle.
Readiness means that each WASAPI source opened successfully, not that a participant spoke or a non-silent
buffer arrived. A silent remote participant cannot keep the meeting in Starting or discard the local
speaker's opening minutes. Once system audio has produced buffers, loss of that buffer heartbeat is a
dropout: report bounded degraded detail, tear down only the failed source, retry it, and keep the same
meeting/session identity. Repeated failures must escalate to a supervised helper-process recycle with
bounded backoff rather than an unbounded crash loop.

The Windows overlay, tray, and permission UI project that state; they do not build a parallel lifecycle.
The overlay should match the Mac projection: a tiny movable branded control and an adaptive vertical
conversation card with meeting/capture state, up to five speaker-grouped transcript bubbles, direct
grounded last-minute catch-up, and proportionate Pause/Resume/Stop controls. It must not persist another
transcript copy.

Add platform-neutral JSON fixtures when the Rust and developer-harness route tests need to prove identical
behavior. Do not build a second lifecycle state machine in the Windows adapter.

## Native acceptance sequence

Run the narrowest useful matrix first:

1. Windows 11 x64 clean build and app launch.
2. Manual meeting capture with mic plus system audio.
3. Zoom native automatic detection and end.
4. Google Meet in Chrome automatic detection and end.
5. Headset, built-in or USB mic, speaker playback, output-device change, and input-device change.
6. Pause/resume/stop, assistant crash/relaunch, meeting-app crash, and a 45-minute soak.
7. Ten-minute silent-remote regression plus a real WASAPI/device dropout and automatic source recovery.
8. Focused-target Dictation in representative native and browser editors.
9. Signed installer, clean install, update/rollback, and uninstall with no private data loss.
10. Only then expand Teams, Slack Huddles, Discord, browsers, devices, and hardware tiers.

Use two consenting people for meeting acceptance. Synthetic audio, a prerecorded loop, a Mac-side test, or a
helper self-test is not Windows platform proof.

## Done and hand-back

The first Windows owner returns:

- a narrow branch/commit with target-gated dependencies and the X-00 native build;
- the exact Windows/toolchain versions and redacted test output;
- a capability table listing working, unsupported, and blocked adapters;
- measured idle/capture/model-loaded CPU, RSS, process count, and disk footprint;
- failed acceptance cases linked to minimal reproducible issues; and
- no private audio, transcripts, identities, credentials, app data, models, or installer artifacts in Git.

The owner should not promote Windows in `PRODUCT_RELEASE_MATRIX.md` until the exact installer and physical
matrix pass. Windows work items and dependencies are tracked as X-00 through X-04 in
[`IMPLEMENTATION_ROADMAP.md`](IMPLEMENTATION_ROADMAP.md).
