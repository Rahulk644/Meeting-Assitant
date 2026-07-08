# Assistant — desktop app (Tauri)

A native, cross-platform window (Rust + the OS WebView) around the Meetings UI. A **release build is
self-contained**: it starts the stack itself — the Node relay, the gated capturer, and `meeting-state` —
and cleans them up on quit, so the app is a **double-click, no dev commands**. The transcription server
(`transcribe-server`, our transcribe.cpp wrapper) is *not* started here: the Node server spawns it on
demand when a call begins and unloads it when idle, so it never holds the model in RAM between calls. The app also adds what a browser tab can't: a **menu-bar tray** (with an
audio-wave recording indicator) and a **screen-share-invisible** window (`set_content_protected` →
`NSWindow.sharingType = .none` on macOS, `WDA_EXCLUDEFROMCAPTURE` on Windows).

The window loads the UI from the local server (`http://localhost:8790`), so the packaged app and the
browser build share one codebase.

## Build

```bash
npm install                         # once — the Tauri CLI (needs Rust: https://rustup.rs)
npx tauri build --bundles app       # → src-tauri/target/release/bundle/macos/Assistant.app
codesign --force --deep --sign - \
  src-tauri/target/release/bundle/macos/Assistant.app   # ← repair the signature, or macOS re-prompts every launch
cp -R src-tauri/target/release/bundle/macos/Assistant.app /Applications/
```

Double-click it. First launch prompts for **Microphone**, **Screen Recording**, and **Accessibility** —
grant each once. The `codesign` step matters: Tauri's ad-hoc "linker-signed" output is an *invalid*
signature, so without it macOS can't verify the app and re-asks for permissions on every launch;
re-signing makes it valid → grants stick (for *this* build — a rebuild changes the fingerprint and you
re-grant once). This build references the `setup.sh`-installed models/binaries on *this* machine; a
portable, Developer-ID-signed + notarized installer is on the roadmap.

## Dev

```bash
../run.sh                    # whisper + server on :8790 (dev capture rides on Terminal's Screen-Recording grant)
npm run dev                  # the native window against that server
```

In dev the app does **not** host capture (it leaves that to `run.sh`, whose Terminal grant is stable
across rebuilds) and lets the Node server own `meeting-state`; a release build hosts everything itself.

## Notes

- `src-tauri/src/lib.rs`:
  - `start_stack()` (release-only) spawns Node + the capturer, resolving tool paths with `find_bin`
    (GUI apps lack the shell PATH). Node spawns the transcription server (`transcribe-server`) on demand, not here.
  - `start_meeting_state()` spawns `meeting-state` **directly** (not via Node) so the app's Accessibility
    grant covers it — a grandchild spawned through Node is not trusted — and forwards its NDJSON to
    `/ingest` via `curl`.
  - `set_stealth` (screen-share invisibility) and `set_tray_recording` (animates the menu-bar audio-wave
    while a call records) commands; the tray; children killed on `RunEvent::Exit`. Capturer stderr →
    `/tmp/assistant-tap.log`.
  - `on_window_event` → the red **X hides** the window (app stays in the tray; reopen via tray → *Show*),
    so closing it never tears down capture mid-call. Quit is **Cmd-Q** or the tray's *Quit Assistant*.
- `src-tauri/Info.plist` — `NSMicrophoneUsageDescription` + `NSAppTransportSecurity` (needed so the
  packaged WebView can load `http://localhost:8790`).

## Status

- [x] Self-contained release build — self-starts the stack; double-click, no dev commands
- [x] Menu-bar tray + audio-wave recording indicator · no-scroll **Settings** modal · **screen-share stealth** (verified)
- [ ] Portable installer — bundle models, Developer-ID sign + notarize (today: reuses this machine's `setup.sh` install)
- [ ] Global shortcut · launch-at-login · the invisible Copilot overlay
