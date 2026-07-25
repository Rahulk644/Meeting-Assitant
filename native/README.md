# `native/` — cross-platform native helpers

This app is a Tauri (Rust + web) desktop application. The GUI, the in-process HTTP server, dictation
polish, and model management already run in portable Rust. The parts that are inherently OS-native —
capturing meeting audio, injecting dictated text, reading the accessibility tree — live in five small
**helper binaries** the core spawns. macOS ships Swift helpers (in the repo's top-level `stt/`,
`dictation-target/`, `agent-context/`, `agent-action/`, `meeting-state/`); Windows and Linux ship
Rust helpers here.

```
native/
  PROTOCOL.md        ← the contract every helper (any OS) must honour. START HERE.
  helpers-core/      ← shared, OS-agnostic Rust crate (protocol structs, argv, VAD, WAV,
                       parent-watchdog, fingerprints). Compiled + unit-tested on every host.
  windows/           ← Windows helper workspace (Rust: WASAPI / UI Automation / SendInput).
  linux/             ← Linux helper workspace   (Rust: PipeWire·Pulse / AT-SPI / XTEST·portal).
```

macOS is unchanged — its Swift helpers remain the reference implementation and the source of the
contract. Windows/Linux are additive.

## Why a "parallel native helpers" architecture

Each helper is a standalone executable that talks to the core over a stable wire protocol (argv +
stdin/stdout JSON or loopback HTTP; see [`PROTOCOL.md`](./PROTOCOL.md)). The core doesn't care what
language or OS API a helper uses — only that it speaks the protocol. So each OS implements the same
five helpers against its own native APIs, and the core selects the right binary at runtime via the
`native_host` module (`desktop/src-tauri/src/native_host.rs`), which adds the `.exe` suffix on
Windows and otherwise keeps the resource layout identical.

The genuinely portable logic — argv parsing, the energy-VAD segmenter, 16 kHz WAV encoding, the
parent-liveness watchdog, the SHA-256 control fingerprints, and every JSON struct on the wire — is
written **once** in [`helpers-core`](./helpers-core) and shared by both platform crates, so the
per-OS code is only the irreducibly native part.

## The five helpers

| Helper | Job | macOS | Windows | Linux |
| --- | --- | --- | --- | --- |
| `meeting-tap` | capture meeting + mic audio, optional video | CoreAudio taps / ScreenCaptureKit | WASAPI process loopback / windows-capture | PipeWire·Pulse monitor / portal screencast |
| `dictation-target` | inject dictated text into the focused field | AX + CGEvent | UI Automation + SendInput | AT-SPI + XTEST (X11) / clipboard-paste (Wayland) |
| `agent-context` | read-only accessibility snapshot | ApplicationServices AX | UI Automation | AT-SPI |
| `agent-action` | one guarded, verified UI mutation | ApplicationServices AX | UI Automation patterns | AT-SPI EditableText |
| `meeting-state` | detect meeting app + live call state | AX + NSWorkspace | process enum + UIA | process enum + X11 titles / AT-SPI |

## Building

- **macOS**: unchanged — `./setup.sh` (Swift helpers via `script/build_native_resources.sh`).
- **Windows**: see [`windows/README.md`](./windows/README.md) — `cargo build --release` (natively or
  cross-compiled with `cargo-xwin`), staged as Tauri `externalBin` sidecars.
- **Linux**: see [`linux/README.md`](./linux/README.md) — `cargo build --release` after installing the
  system `-dev` packages; packaged as `.deb`/`.rpm`/AppImage.

`helpers-core` is verified on any host:

```sh
cd native/helpers-core && cargo test
```

## Honesty about status

The Windows and Linux crates were authored and reviewed on macOS, so the OS-native calls (WASAPI COM,
UI Automation cache walks, PipeWire node linking, AT-SPI trees, Wayland input injection) are written
against the correct APIs but **have not been compiled or run on their target OS**. Anything requiring
a real target toolchain or hardware to finish is marked `// TODO(on-device)` at its call site and
degrades to a contract-valid fallback (logged, never silent). See
[`../MIGRATION.md`](../MIGRATION.md) for the full status matrix and the remaining core work.
