# Cross-platform migration — status & plan

> ## ✅ Linux desktop app now builds, installs, and runs
> The Tauri **core was ported off macOS-only APIs and now compiles on Linux**. A full Linux `.deb` is
> produced end-to-end by `./make-distributables.sh --linux-only` (built entirely in Docker):
> - `dpkg`/`apt` installs it cleanly; the launcher's dynamic libraries all resolve (runnable);
> - the app binary **and all five native helpers** (`meeting-tap`, `transcribe-server`, `meeting-state`,
>   `dictation-target`, `llama-server`) are freshly compiled **Linux ELF** binaries and pass their
>   `--selftest`s (STT `transcribe-server` included).
>
> **What made it work:** cfg-gating the macOS-only core (see the audit-fixed items below) plus three
> Linux-build fixes — `RunEvent::Reopen` gated to macOS; the C++ ggml runtimes built with
> `-DGGML_NATIVE=OFF` off macOS (the aarch64 FP16 NEON path won't inline without `+fp16`); and the C++
> build scripts self-heal a cmake cache keyed to a different source path (host `/Users` vs container `/src`).
>
> **Arch:** on Apple Silicon, Docker builds a `linux/arm64` `.deb`; set `DOCKER_DEFAULT_PLATFORM=linux/amd64`
> (or use the `ubuntu-latest` CI job) for `x86_64`.
>
> **Windows:** the ported core + all 5 helpers **cross-compile in Docker** via `cargo-xwin` (verified:
> `app.exe` builds with 0 errors), and tauri runs its whole bundling pipeline there. But the final
> `makensis` step **segfaults on the Linux NSIS build** with tauri's generated script (the makensis
> itself is fine — trivial scripts and the tauri plugin both compile). On Apple Silicon, wine + the
> Windows makensis would need x86 emulation, so it's impractical locally. The **single `-setup.exe`
> installer therefore comes from the `windows-app` CI job** (native Windows makensis) — which builds it
> with `--config tauri.windows.conf.json` (maps helper resources to `.exe` dests so `native_host` finds
> them) and uploads it as the `windows-installer` artifact. The local `make-distributables.sh` emits the
> cross-built helper `.exe`s only.

This document tracks making the app (today: macOS-only) run on **Windows** and **Linux**. It is the
honest single source of truth for what is done, what compiles, what is verified, and what still needs
on-device work. It is deliberately candid about the boundary between "written & reviewed" and
"proven on the target OS", because much of the native surface cannot be compiled or exercised on a
macOS CI box.

The app is a Tauri (Rust + web) desktop app. The portable layers (GUI, in-process HTTP server,
dictation polish, model management) already run in cross-platform Rust. The OS-native work lives in
five helper binaries the core spawns. The strategy — chosen by the maintainer — is **parallel native
helpers**: each OS implements the same five helpers against its own native APIs, behind one shared
wire contract, and the core picks the right binary at runtime.

See [`native/PROTOCOL.md`](./native/PROTOCOL.md) for the contract and [`native/README.md`](./native/README.md)
for the architecture.

---

## What is done and verified (on a macOS host)

| Piece | Location | Status |
| --- | --- | --- |
| Wire contract, documented | `native/PROTOCOL.md` | ✅ Complete — reverse-engineered from the Swift helpers + the Rust spawn/parse sites |
| Shared portable crate | `native/helpers-core/` | ✅ **Compiles + 20 unit tests pass.** Protocol structs (incl. `manual` tri-state regression tests), argv parser, VAD segmenter, WAV encoder, parent-watchdog, SHA-256 fingerprints |
| Platform-selection seam | `desktop/src-tauri/src/native_host.rs` | ✅ **Core lib type-checks clean + 2 unit tests pass.** All 7 helper-spawn sites routed through it (`.exe` on Windows) |
| Windows helper workspace | `native/windows/` | ✅ **Portable spine compiles + 22 tests pass on host**; native paths structured, 37 `TODO(on-device)` markers |
| Linux helper workspace | `native/linux/` | ✅ **Host-buildable crates compile + 44 tests pass**; native paths structured, Wayland limits flagged at every site |
| GGML build backend selection | `script/build_polish_runtime.sh`, `stt/transcribe-server/build.sh` | ✅ OS-aware: Metal on macOS, `GGML_BACKEND=cuda\|vulkan\|cpu` elsewhere; `ldd`/`otool`; `.exe`; codesign gated to macOS |

"Verified on host" for the platform crates means: the portable Rust (argv, protocol wiring,
segmenter, fingerprints, `--selftest`, and the OS-native fallbacks) compiles and its unit tests pass
on macOS, and each helper's fallback path emits **contract-valid** JSON. It does **not** mean the
`#[cfg(windows)]` / Linux-native code has been built or run on its target OS.

---

## Per-helper status matrix

Legend: **ref** = reference implementation (macOS Swift, shipping). **spine** = portable logic done &
tested. **native** = OS-native calls written against the correct API, `TODO(on-device)` = needs the
target toolchain/hardware to finish & verify.

| Helper | macOS | Windows | Linux (X11) | Linux (Wayland) |
| --- | --- | --- | --- | --- |
| `meeting-tap` audio | ref | spine ✅ · WASAPI capture `TODO(on-device)` | spine ✅ · Pulse monitor `TODO(on-device)` | spine ✅ · same |
| `meeting-tap` video | ref | windows-capture `TODO(on-device)` | ximagesrc `TODO(on-device)` | portal screencast (prompts) `TODO(on-device)` |
| `dictation-target` | ref | spine ✅ · SendInput/UIA `TODO(on-device)` | spine ✅ · XTEST inject ✅* | ⚠️ clipboard-paste only; no universal injector |
| `agent-context` | ref | spine ✅ · UIA cache-walk `TODO(on-device)` | spine ✅ · AT-SPI `TODO(on-device)` | ⚠️ AT-SPI only; no window list |
| `agent-action` | ref | spine ✅ · UIA patterns `TODO(on-device)` | spine ✅ · AT-SPI `TODO(on-device)` | ⚠️ no window raise; fail-closed |
| `meeting-state` | ref | spine ✅ · procs ✅ · UIA `TODO(on-device)` | spine ✅ · X11 titles ✅* · AT-SPI `TODO` | ⚠️ no window enumeration |

`*` X11 window titles and XTEST injection use pure-Rust `x11rb`/`enigo` and are structurally complete;
they still want a real X session to confirm behavior.

**The two hard platform truths** (not fixable in code, must be designed around):
1. **Windows has no SIGTERM.** The core must finalize `meeting-tap`'s `.mp4` by closing the child's
   stdin (or sending `CTRL_BREAK`), not `TerminateProcess`. `native_host` + the Windows `shutdown.rs`
   implement the helper side; **the core's `kill_children` still SIGTERMs** — see the checklist below.
2. **Wayland forbids global input injection and window enumeration.** `dictation-target` degrades to
   clipboard-paste, `meeting-state` loses title-based detection, and `agent-action` cannot raise
   windows. These are OS policy, not bugs; the helpers detect the session type and report reduced
   capability via `liveEditable:false` / `unsafeFallback:true` rather than failing silently.

---

## Remaining work — the core is still macOS-bound

The helper *binaries* are cross-platform, but the **Rust core does not yet compile on Windows/Linux**.
Two files import macOS-only crates unconditionally, and ~30 sites shell out to macOS binaries. This is
the next phase; it's mechanical but must be done and compiled per-OS. Source: the de-macOS inventory.

### Phase 1 — make the core compile off-macOS (compile blockers)
- [ ] `local_server.rs:7–11` — `objc2*` imports are unconditional → `#[cfg(target_os="macos")]`-gate the imports and the app-icon fns (`~4245–4304`).
- [ ] `codex_runtime.rs:7–9` — `security_framework::os::macos::code_signing` unconditional → cfg-gate the whole Codex code-signing module (Codex is macOS-only today; expose behind a feature that's off elsewhere).
- [ ] `Cargo.toml` — move `objc2*`, `core-graphics`, `core-foundation*`, `macos-accessibility-client`, `security-framework` (code-signing use) into `[target.'cfg(target_os="macos")'.dependencies]`. Drop the `macos-private-api` tauri feature off-mac.

### Phase 2 — replace hardcoded macOS binaries with cross-platform crates
- [ ] Clipboard `pbcopy`/`pbpaste` (`lib.rs:863,883`) → `arboard`.
- [ ] `/usr/bin/open` (`local_server.rs:882,4004`) → `open` crate. Keep the `x-apple.systempreferences:` deep link (`3973`) macOS-only.
- [ ] `/usr/bin/shasum` (`local_server.rs:3552,3943`, `polish_runtime.rs:101`) → `sha2` in-process.
- [ ] `/usr/bin/curl` model downloads (`lib.rs:1211`, `local_server.rs:3313,3712`) → `ureq` (already a dep) with a `Range` header for resume.
- [ ] `afplay` + `/System/Library/Sounds/*.aiff` (`lib.rs:724,727`) → `rodio` + bundled sound assets.
- [ ] `/bin/kill` / `/bin/cp` (`lib.rs:158`, `local_server.rs:3081`) → std/`nix`. **This is where the no-SIGTERM-on-Windows fix lands** (`kill_children`): SIGTERM on mac/linux, close-stdin/`CTRL_BREAK` then `child.kill()` on Windows.
- [ ] Keychain via `security-framework` (`local_server.rs:1428,2317,2439`) → `keyring` (Keychain/Credential Manager/Secret Service).
- [ ] Data dirs `~/Library/Application Support`, `$HOME` (`lib.rs:79–105`) → `dirs`/`directories` (`%APPDATA%` / `$XDG_DATA_HOME`).
- [ ] CGEvent ⌘V paste (`lib.rs:898–928`) → `enigo` (⌘V on mac, Ctrl+V elsewhere).
- [ ] Unix file-mode calls (`OpenOptionsExt.mode(0o600)`, `PermissionsExt`) → cfg-gate.
- [ ] ffmpeg discovery (`local_server.rs:4171`) → generalize path list + `.exe`.

### Phase 3 — packaging & bundling
- [ ] `tauri.conf.json` `bundle.resources` currently lists the macOS helper binaries directly. Parameterize per-OS via the **`resources` map** (point at the built Windows/Linux binaries at the exact `<subdir>/<stem>[.exe]` paths `native_host` resolves). **Do NOT use `externalBin`** — Tauri places sidecars next to the main executable, but `native_host` looks under `resource_dir()`, so a sidecar would never be found at runtime.
- [ ] `tauri.conf.json` also declares `../web-dist/*` (frontend build, OS-agnostic npm) and shares the missing-resource failure mode — ensure the web build runs before `tauri build` on every OS.
- [ ] `Info.plist` / `Entitlements.plist` (and the `meeting-tap` embedded `NSMicrophoneUsageDescription` via `sectcreate`) are macOS-only and inert elsewhere — Windows/Linux need no manifest, but confirm the bundler doesn't choke on them.
- [ ] Windows: MSI/NSIS via `tauri build`; Authenticode-sign helpers + installer (no notarization).
- [ ] Linux: `.deb`/`.rpm`/AppImage; declare `Depends:` on pipewire/xdg-desktop-portal/gstreamer; ship a udev rule for `/dev/uinput` if using the Wayland uinput inject path. Avoid Flatpak for the helpers (sandbox blocks uinput/AT-SPI/process-enum).
- [ ] A Windows `build.ps1` and a Linux `build_native_resources` counterpart to `script/build_native_resources.sh` (which is macOS/swiftc-specific), staging the helper binaries into the resource layout `native_host` expects.

### Phase 4 — on-device hardening (per the `TODO(on-device)` markers)
Build each platform crate on its target OS and reconcile the crate API surfaces (`windows` 0.62 /
`wasapi` 0.23 / `uiautomation` 0.24 / `enigo` / `windows-capture` on Windows; `libpulse` / `pipewire`
/ `atspi` / `gstreamer` / `ashpd` on Linux). Then verify on real hardware: per-app audio scoping, the
mp4 finalize path, text injection into web/Electron composers, the accessibility tree shapes for
Zoom/Teams (rebuild participant selectors via `meeting-state --dump`), and — on Linux — which Wayland
input mechanism actually works on the target desktop (libei vs uinput vs portal).

---

## Adversarial audit results

Four parallel auditors reviewed the change set (core Rust, helpers-core, the Windows crate, the Linux
crate, and cross-cutting integration). The macOS shipping path was confirmed **regression-free** (all
7 native_host-resolved paths byte-identical to before; fingerprint/segmenter/serde faithful).

**Fixed & re-verified (tests green):**
- **CRITICAL — `PulledConfig.manual` typed `bool` but the server sends a string** → serde failed the
  *entire* `/api/state` parse every poll → on-device meeting-tap never captured. Now `Option<String>`
  with `manual_force_on/off/is_auto()` helpers; both platform crates updated to the tri-state gate.
  (All 3 code auditors independently flagged this.)
- **`autoDetect`/`browserMeet` defaulted `false`** vs the Swift `true` → a fresh/partial config left
  auto-capture off. `PulledConfig` now defaults both `true`.
- **Windows `watchdog::is_alive` treated `ACCESS_DENIED` as dead** → could prematurely kill the helper
  under a protected parent. Now `ACCESS_DENIED` ⇒ alive, matching the Unix `EPERM` rule.
- **WAV encode rounded** where Swift truncates → now truncates (byte-identical).
- **Windows `agent-action` `i32` vs `i64` PID compare** — a real compile break on the msvc target
  (missed by host `cargo check` because it's `#[cfg(windows)]`). Fixed to widen to `i64`.
- **`build_polish_runtime.sh`** — `backend_flags`/`foreign_libs`/`$EXE` were dead code (Metal forced,
  `otool` on Linux). Now wired: Metal on macOS, `GGML_BACKEND=cuda|vulkan|cpu` elsewhere.
- Doc corrections: the `externalBin` packaging guidance (would break runtime resolution) and the false
  "core-mediated wake" claim.

**Deferred to on-device hardening (real, but in `#[cfg(target_os)]` code that can't be built/verified
on the macOS review host — do NOT ship the affected helper before addressing):**
- **Linux `meeting-tap` opens the mic + monitor Pulse streams at process start, unconditionally**, and
  never releases them between calls — a **privacy regression** (mic-in-use indicator stays lit outside
  meetings). macOS opens/closes streams only while in a call. Must gate stream open/close on the
  capture gate before Linux ships.
- **Windows console-ctrl shutdown can lose the mp4 moov atom** — finalize runs on the main thread after
  the handler returns; if mid-`pull_config` (up to 15 s timeout) it can miss the close grace. Finalize
  inside the handler / bound the config-pull. (Latent until `windows-capture` video is wired.)
- **`agent-context`/`agent-action` selection + BFS-root are stubbed** (Windows & Linux) → snapshots are
  contract-valid but wrong-scoped, and `agent-action` only succeeds on an empty document. On-device work.
- **Fingerprint offset units** — Linux `agent-context` emits AT-SPI character offsets while
  `agent-action` expects UTF-16; astral chars would break cross-helper revalidation. Unify at wiring time.
- **`meeting-state` polls continuously** (Windows & Linux) instead of sleeping on a wake signal (see the
  wake-signal item below); Windows re-inits COM per call without `CoUninitialize`; SIGTERM finalize grace
  is short. Lower-severity logic divergences (dedicated-app priority order, over-eager `sharing`
  derivation, unclosed-bracket transcript cleaning, degraded-snapshot privacy-tag count) are noted in the
  auditor reports for on-device cleanup.

**Additional remaining work surfaced by the audit (add to the phases above):**
- [ ] Wire the **meeting-activity wake signal** (core pipes `meeting-tap` state → `meeting-state` stdin,
  or a private D-Bus/named-event bus) + stale-heartbeat fail-closed. Until then meeting-state polls 24/7.
- [ ] **Bare-modifier dictation hotkey** (right-⌘/Option) uses a macOS CGEvent Input-Monitoring event
  tap — macOS-only. Ordinary combos use the cross-platform `global-shortcut` plugin, but bare-modifier
  triggers won't work off-mac once the `core-graphics` surface is cfg-gated. Needs a per-OS equivalent.
- [ ] **`litert-lm` summarizer** is discovered only at `~/.local/bin` / Homebrew paths (no `.exe`, not
  routed through `native_host`) → always "not installed" on Windows. Add a Windows path + `.exe`.
- [ ] **Notifications** — `tauri-plugin-notification` is cross-platform but needs a Linux notification
  daemon / Windows toast app-identity to actually surface meeting-start alerts.

## Protocol divergences (benign, verified harmless by audit)

Two intentional `helpers-core` divergences from the Swift were confirmed harmless: `unsafeFallback` is
omitted-when-false (the core's `WireResponse` defaults it to `false`), and `unknownCommand` is
unreachable because `SessionOp`/`ActionOperation` are typed enums (a malformed op yields
`invalidCommand`, which the core only logs). The previously-listed `manual` bool divergence was **not**
harmless and has been fixed (see audit results above).

---

## Build quickstart

```sh
# Shared crate (any OS)
cd native/helpers-core && cargo test

# Windows helpers (native, or cross-compile from mac/linux with cargo-xwin)
cd native/windows && cargo build --release --target x86_64-pc-windows-msvc

# Linux helpers (on Linux, after installing the -dev packages in native/linux/README.md)
cd native/linux && cargo build --release --target x86_64-unknown-linux-gnu

# macOS (unchanged)
./setup.sh
```
