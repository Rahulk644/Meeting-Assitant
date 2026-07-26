# Browser meeting metadata bridge

This directory is the extension half of a future, optional Chrome metadata bridge. It is deliberately
separate from capture and from the native-host process:

```text
meeting page content script
        ↓ bounded page observation
extension service worker
        ↓ Chrome native messaging
separately installed native host
        ↓ owner-only local IPC
Rust meeting detector
```

The local desktop app does not require this extension. Core Audio plus bounded macOS meeting-window
evidence remains the shipped browser-call path. The extension may later improve exact tab identity and
meeting metadata; it must never become the owner of meeting lifecycle, audio capture, transcript storage,
tokens, or live-assist execution.

## Security contract

- Content scripts run only on allowlisted Meet, Zoom, and Teams meeting URL shapes.
- Messages contain a platform key, a per-browser-session tab identity, a bounded page title, and active/
  inactive state. They contain no URL, meeting code, audio, transcript, cookies, page text, or credentials.
- `protocol.js` rejects unknown fields, versions, types, platforms, and malformed identities before native
  messaging.
- The native host will be a separately signed executable with a Chrome `allowed_origins` manifest. Its only
  output is the existing bounded `MeetingObservation` contract over owner-only local IPC.
- Losing the extension or native host degrades metadata only. It cannot stop local capture or corrupt the
  meeting record.

`native-host-manifest.template.json` is intentionally not installable until packaging substitutes the
signed host path and stable extension ID. The production release matrix must remain unchanged until that
installer and physical Chrome acceptance suite exist.
