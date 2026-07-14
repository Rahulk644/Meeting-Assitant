# My Assistant — desktop app (Tauri)

A native macOS window (Rust + the OS WebView) around the Meetings UI. A **release build is self-contained**:
the authenticated UI/API server runs inside the Rust app process, which starts the gated capturer and
`meeting-state` directly and cleans them up on quit. No Node runtime or Node process is shipped. The native
transcription server starts only when speech needs transcription and unloads after the configured idle
period, so it never holds the model in RAM indefinitely. The app also adds what a browser tab can't: a **menu-bar tray** (with an
audio-wave recording indicator) and a **screen-share-invisible** window (`set_content_protected` →
`NSWindow.sharingType = .none` on macOS, `WDA_EXCLUDEFROMCAPTURE` on Windows).

The window loads the UI from an app-owned loopback server (`http://127.0.0.1:8790`). A random per-launch
secret protects every route, and the WebView exchanges a one-time bootstrap URL for an HttpOnly cookie.

## Build

```bash
npm install                         # once — the Tauri CLI (needs Rust: https://rustup.rs)
../setup.sh                         # builds + cert-signs the native helpers (its 3/3 step; see below)
npx tauri build --bundles app       # → src-tauri/target/release/bundle/macos/My Assistant.app
cp -R "src-tauri/target/release/bundle/macos/My Assistant.app" /Applications/
```

For a locally built copy, double-click it. For a DMG received from another person, drag it to Applications
and try to open it once. On current macOS, dismiss the unverified-app warning, then use **System Settings →
Privacy & Security → Open Anyway**, authenticate, and confirm **Open**. Older versions may offer the same
approval through Control-click → **Open**. First use asks only for permissions needed by the feature being
used: Microphone for dictation/meetings, System Audio for other participants, Accessibility for paste,
meeting-window evidence, and speaker names, and Screen Recording only when video capture is enabled.

**Why the signing matters (the TCC trap).** macOS keys each permission grant on the requesting binary's
*code requirement*. For **ad-hoc** code (swiftc's / Tauri's default "linker-signed" output) there is no
identity to pin, so TCC pins the raw **cdhash** — which changes on every rebuild, so the grant silently
stops matching and the app re-prompts *forever* (the Settings toggle stays "on" but is dead). The cure is
a **stable certificate**: `tauri.conf.json` sets `bundle.macOS.signingIdentity: "My Assistant Dev"`, and
`setup.sh` signs the two spawned helpers (`meeting-tap`, `meeting-state`) **with the same cert *and* the
app's identifier** (`com.myassistant.desktop`). That makes every binary's requirement
`identifier "com.myassistant.desktop" and certificate leaf = …` — **cdhash-independent**, so grants
survive rebuilds *and* the helpers ride the app's single grant instead of each demanding their own.

> If you previously ran an **ad-hoc** build, its dead cdhash-pinned grant will block the new one. Clear it
> once: `tccutil reset ScreenCapture com.myassistant.desktop` (repeat for `Microphone`, `Accessibility`),
> then relaunch and grant in System Settings. After that it sticks across all future rebuilds.

The DMG is portable and contains all application code and native helpers. Speech models (about 44 MB–1.1 GB
each) are not bundled; the app opens the model library on first use, verifies the selected artifact's exact
size and SHA-256, and stores it under the current user's Application Support directory.

## Dev

```bash
../run.sh                    # optional developer-only browser harness
npm run dev                  # native debug app with the in-process Rust server
```

`run.sh` retains the legacy Node browser harness for UI development only. It is not copied into the app or
DMG. Native debug and release builds use the same Rust server architecture.

## Notes

- `src-tauri/src/lib.rs`:
  - `local_server` starts before any WebView and owns port 8790. If the port is already occupied, startup
    fails closed instead of loading content from the other process.
  - `start_stack()` (release-only) starts the capturer; Rust starts `transcribe-server` on demand.
  - `start_meeting_state()` spawns `meeting-state` directly and forwards its NDJSON to the authenticated
    `/ingest` route without a subprocess.
  - `set_stealth` (screen-share invisibility) and `set_tray_recording` (animates the menu-bar audio-wave
    while a call records) commands; the tray; children killed on `RunEvent::Exit`. Helper output is not
    written to world-readable `/tmp` logs.
  - `on_window_event` → the red **X hides** the window (app stays in the tray; reopen via tray → *Show*),
    so closing it never tears down capture mid-call. Quit is **Cmd-Q** or the tray's *Quit Assistant*.
- `src-tauri/Info.plist` — `NSMicrophoneUsageDescription` + narrowly scoped local-network transport support
  for the packaged WebView's `http://127.0.0.1:8790` origin.

## Status

- [x] Self-contained release build — self-starts the stack; double-click, no dev commands
- [x] Menu-bar tray + audio-wave recording indicator · full-page **Settings** module · **screen-share stealth** (verified)
- [x] Portable no-paid-account DMG — native helpers bundled; explicit current-macOS Open Anyway instructions
- [x] Global dictation shortcut + click-through overlay
- [ ] Launch at login
