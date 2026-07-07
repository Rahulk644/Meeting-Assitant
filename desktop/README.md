# Assistant — desktop app (Tauri)

A native, cross-platform window (Rust + the OS WebView) around the Meetings UI. A **release build is
self-contained**: it starts the whole stack itself — Node relay, `whisper-server`, the gated capturer,
and `meeting-state` — and cleans them up on quit, so the app is a **double-click, no dev commands**. It
also adds what a browser tab can't: a **menu-bar tray** and a **screen-share-invisible** window
(`set_content_protected` → `NSWindow.sharingType = .none` on macOS, `WDA_EXCLUDEFROMCAPTURE` on Windows).

The window loads the UI from the local server (`http://localhost:8790`), so the packaged app and the
browser build share one codebase.

## Build

```bash
npm install                         # once — the Tauri CLI (needs Rust: https://rustup.rs)
npx tauri build --bundles app       # → src-tauri/target/release/bundle/macos/Assistant.app
cp -R src-tauri/target/release/bundle/macos/Assistant.app /Applications/
```

Double-click it. First launch prompts for **Microphone**, **Screen Recording**, and **Accessibility** —
grant once and it sticks. This build references the `setup.sh`-installed models/binaries on *this*
machine; a portable, signed, notarized installer is on the roadmap.

## Dev

```bash
../run.sh                    # whisper + server on :8790 (dev capture rides on Terminal's Screen-Recording grant)
npm run dev                  # the native window against that server
```

In dev the app does **not** host capture (it leaves that to `run.sh`, whose Terminal grant is stable
across rebuilds); a release build hosts everything itself.

## Notes

- `src-tauri/src/lib.rs` — `start_stack()` (release-only) spawns the stack, resolving tool paths with
  `find_bin` (GUI apps lack the shell PATH); the `set_stealth` command; the tray; kills children on
  `RunEvent::Exit`. Capturer stderr → `/tmp/assistant-tap.log`.
- `src-tauri/Info.plist` — `NSMicrophoneUsageDescription` + `NSAppTransportSecurity` (needed so the
  packaged WebView can load `http://localhost:8790`).

## Status

- [x] Self-contained release build — self-starts the stack; double-click, no dev commands
- [x] Menu-bar tray · Snaply-style no-scroll **Settings** modal · **screen-share stealth** (verified)
- [ ] Portable installer — bundle models, code-sign + notarize (today: reuses this machine's `setup.sh` install)
- [ ] Global shortcut · launch-at-login · the invisible Copilot overlay
