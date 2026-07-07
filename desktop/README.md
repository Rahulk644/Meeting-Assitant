# Assistant — desktop shell (Tauri)

Wraps the Meetings UI in a native, cross-platform window (Rust + the OS WebView), adding what a browser
tab can't: a **menu-bar tray** and a **screen-share-invisible** window (`content-protected` →
`NSWindow.sharingType = .none` on macOS, `WDA_EXCLUDEFROMCAPTURE` on Windows) — the foundation for the
in-call Copilot overlay.

The window loads the app from the local server (`http://localhost:8790`), so the capture + AI stack is
shared with the browser build — no logic is duplicated.

## Run (dev)

The capture stack (whisper-server + Swift helpers) needs your Terminal's Screen-Recording grant, so
start it the usual way first, then launch the shell:

```bash
./run.sh                    # repo root — starts whisper + the GUI server on :8790
cd desktop && npm run dev   # native window (Rust compiles on first run, ~1 min)
```

Tray → **Stealth** toggles screen-share invisibility (off by default). Tray → **Quit** exits.

Prereqs: [Rust](https://rustup.rs) and Node. Run `npm install` in `desktop/` once for the Tauri CLI.

## Status

- [x] Native window hosting the UI · menu-bar tray · stealth (content protection)
- [ ] App starts the whole stack as sidecars (today `run.sh` still runs capture)
- [ ] Migrate the Screen-Recording grant onto the signed `.app` (dev rebuilds churn the TCC grant)
- [ ] Global shortcut · launch-at-login · packaged installers · the invisible Copilot overlay
