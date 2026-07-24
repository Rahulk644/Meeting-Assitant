# Linux native helpers

Rust implementations of the five native helper binaries the Meeting-Assistant desktop app drives on
Linux. They are drop-in replacements for the macOS Swift helpers and honour the exact contract in
[`../PROTOCOL.md`](../PROTOCOL.md): same argv, same stdin/stdout framing, same JSON field names, same
exit codes. The OS-agnostic logic is shared via [`../helpers-core`](../helpers-core) (frozen) and a
local `platform` crate; only the genuinely platform-specific pieces (audio/video capture, input
injection, accessibility reads) live in the binaries.

Target triple: **`x86_64-unknown-linux-gnu`**.

> This is a **standalone cargo workspace** — deliberately *not* a member of the macOS app's workspace
> (the empty-membered `[workspace]` in `Cargo.toml` makes this directory its own root). It builds only
> on Linux and links Linux-only system libraries.

## The 2×2 runtime world

Every helper probes two axes at startup and branches on them (`platform::display` + `platform::audio`):

| Axis | Detected from | Why it matters |
| --- | --- | --- |
| **Display**: X11 vs Wayland | `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY`, `DISPLAY` | Wayland **blocks** global input injection (no XTEST) and **blocks** window enumeration. |
| **Audio**: PulseAudio vs PipeWire | sockets in `XDG_RUNTIME_DIR`, `PULSE_SERVER` | `libpulse` works on **both** (PipeWire ships `pipewire-pulse`), so capture is built on Pulse; native PipeWire is only needed for per-app node scoping. |

Wayland limitations are called out at every site that hits them, and every degrade is logged — never
silent.

## Build

```sh
# from native/linux/
cargo build --target x86_64-unknown-linux-gnu --release
# binaries land in target/x86_64-unknown-linux-gnu/release/{meeting-tap,dictation-target,agent-context,agent-action,meeting-state}

# unit tests (the verifiable spine — runs on any host, no display/audio needed):
cargo test

# smoke test each helper:
./target/.../release/meeting-tap --selftest        # → "SELFTEST OK", exit 0
./target/.../release/dictation-target --selftest    # → "dictation-target selftest ok", exit 0
./target/.../release/agent-context --selftest       # → "agent-context self-test passed", exit 0
./target/.../release/agent-action --selftest        # → "agent-action self-test passed", exit 0
./target/.../release/meeting-state --selftest       # → "SELFTEST OK", exit 0
```

### Required `-dev` packages (build/link time)

Debian/Ubuntu names (Fedora equivalents in parentheses):

| Package | For |
| --- | --- |
| `libpulse-dev` (`pulseaudio-libs-devel`) | meeting-tap audio capture (`libpulse` / `libpulse-simple`) |
| `libpipewire-0.3-dev` (`pipewire-devel`) | meeting-tap per-app scoping (feature `pipewire-scope`) + Wayland video src |
| `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev` (`gstreamer1-devel`, `gstreamer1-plugins-base-devel`) | meeting-tap video encode |
| `libxcb1-dev`, `libxcb-randr0-dev` (`libxcb-devel`) | x11rb window enumeration |
| `libxtst-dev` (`libXtst-devel`) | enigo XTEST injection (X11) |
| `libatspi2.0-dev` / `at-spi2-core` (`at-spi2-core-devel`) | AT-SPI accessibility (agent-context/action, dictation, meeting-state) |
| `libdbus-1-dev` (`dbus-devel`) | zbus / AT-SPI / portals |
| `clang`, `pkg-config` | `-sys` crate bindgen + discovery |

GStreamer runtime plugins to install for actual recording: `gstreamer1.0-plugins-good`,
`-plugins-bad` (for `x264enc`), `-plugins-base`, and `pipewire` (for `pipewiresrc` on Wayland).

### Runtime dependencies

- **PulseAudio or PipeWire** running (`pipewire` + `pipewire-pulse` recommended).
- **at-spi2 registry** running (standard on GNOME/KDE) for the accessibility helpers.
- **An EWMH-compliant window manager** for X11 window titles (`_NET_CLIENT_LIST` / `_NET_WM_NAME`).
- For video on Wayland: the **xdg-desktop-portal** + a backend (`xdg-desktop-portal-gnome` / `-kde` / `-wlr`).

## Permission model

| Capability | X11 | Wayland |
| --- | --- | --- |
| System-audio capture | Pulse monitor source (no prompt) | same via pipewire-pulse (no prompt) |
| Screen video | `ximagesrc` (no prompt) | **interactive** ScreenCast portal grant (unavoidable) |
| Text injection | XTEST (no prompt) | needs **libei** (RemoteDesktop portal grant) **or** `uinput` (user in the `input` group + a udev rule) — clipboard-paste is the primary path |
| Accessibility reads | AT-SPI, gated on toolkit a11y | same |

Accessibility must be **enabled** for the a11y helpers to see anything:
- GTK: `gsettings set org.gnome.desktop.interface toolkit-accessibility true` (or `GTK_MODULES=gail:atk-bridge`).
- Qt (incl. **Zoom on Linux**): export **`QT_ACCESSIBILITY=1`** (and `QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1`).

When a11y is off, helpers degrade gracefully: agent-context emits `permission_required`/`no_window`,
dictation-target classifies `secure`/`unknown`, meeting-state emits an `axTrusted:false` status line
and still reports X11 window-title state (which is a11y-independent).

## Packaging

Ship the five binaries under the app's resources dir (`<resources>/<subdir>/<stem>` per the core's
`native_host` layout). Recommended formats: **`.deb` / `.rpm`** (declare the `-dev`→runtime library
deps), or **AppImage** bundling the GStreamer plugins. **Avoid Flatpak/Snap for the helpers** — their
sandboxes block the very things these tools need (global input injection, AT-SPI cross-app reads,
process enumeration, arbitrary loopback HTTP). The main app may be sandboxed; the helpers should not.

## Per-helper × display-server status

Legend: **real** = implemented + verifiable against helpers-core; **X11** / **Wayland** note the
capability; **TODO(on-device)** = correct API sequence written, needs a real Linux desktop + libs to
finish/verify (always degrades gracefully + logs).

| Helper | Portable spine (real, tested) | X11 | Wayland |
| --- | --- | --- | --- |
| **meeting-tap** | args, 2 s config pull, gate logic, segmenter→WAV→whisper→caption/state, `source:"coreaudio"`, detection helpers, `--list-apps`, `--selftest`, SIGTERM finalize, parent-watchdog | Pulse monitor capture (TODO: default-monitor resolution); `ximagesrc` video (TODO wire); window-evidence detection (TODO) | same audio via pipewire-pulse; **video needs ScreenCast portal grant** (TODO); per-app scoping needs native PipeWire (feature `pipewire-scope`, TODO) |
| **dictation-target** | classification (kind/secure/startsSentence), full session state machine + error taxonomy, Result-first output, `--selftest` | injection **real** (XTEST); focus read TODO | `liveEditable=false` (no stable caret); clipboard-paste primary, needs libei/uinput; focus read TODO |
| **agent-context** | BFS budgets, secure redaction, label/fingerprint, 4 status outcomes, sorted-key JSON, `--selftest` | `_NET_ACTIVE_WINDOW` cross-check; AT-SPI tree walk TODO | AT-SPI tree walk TODO (no X11 cross-check) |
| **agent-action** | UTF-16 splice, MutationDisposition SM, verify/rollback, undo payload, allowlist + permission gates, `--selftest` | AT-SPI EditableText apply + window-behind gate TODO | **activation gate has no equivalent** — fails closed; apply TODO |
| **meeting-state** | control derivation, name/title extraction, process enum, unbuffered NDJSON, ≥2 s interval clamp, parent fail-closed, `--selftest` | window titles **real** (x11rb); AT-SPI menu/participant reads TODO | **no window enumeration** — titles fall back to AT-SPI window names; wake via zbus signal TODO |

### What "TODO(on-device)" means here

The load-bearing, verifiable spine (arg parsing, the whole helpers-core protocol, fingerprints,
`--selftest`, the segmenter→whisper→caption pipeline, the X11/Wayland + Pulse/PipeWire probe, and all
the pure per-helper brains) is **real and unit-tested** — `cargo test` exercises it on any host. The
deep native calls (Pulse/PipeWire linking, GStreamer pipelines, AT-SPI tree walks, Wayland injection)
are written with the correct API sequence and real crate types but need a Linux desktop + the system
libraries + hardware to finish and verify; each is marked `// TODO(on-device):` at its site and always
returns a contract-valid fallback so the binary is well-formed and degrades loudly rather than lying.
