# Native helper protocol — the cross-platform contract

The desktop app (Tauri + Rust core) drives five **native helper binaries**. Each helper is a
small standalone executable spawned by the core. macOS ships Swift implementations; Windows and
Linux ship Rust implementations under [`native/windows`](./windows) and [`native/linux`](./linux).

**A helper on any OS is a drop-in replacement as long as it honours this contract** — same argv,
same stdin/stdout framing, same exit codes, same JSON field names. This file is the source of
truth; it was reverse-engineered from the macOS Swift helpers *and* the Rust spawn/parse sites in
`desktop/src-tauri/src/{lib,local_server,dictation_context}.rs`.

Nothing below is macOS-specific except where a row is explicitly called out as "macOS concept →
map per platform".

---

## Cross-cutting rules (all five helpers)

| Concern | Contract |
| --- | --- |
| Parent liveness | Read `ASSISTANT_PARENT_PID` from the environment. Poll it; when the parent dies, stop cleanly and exit. macOS: `kill(ppid, 0)` treating `EPERM` as alive. Windows: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `WaitForSingleObject(h,0)`. Linux: `kill(ppid, 0)` (`nix`). |
| Auth token | Control-plane HTTP uses `Authorization: Bearer <token>`, token from `ASSISTANT_GUI_TOKEN` / `ASSISTANT_STT_TOKEN` (env overrides the `--gui-token` / `--stt-token` flags). |
| Shutdown | The core sends a graceful stop first, then a hard kill after ~500 ms. macOS/Linux: `SIGTERM` → `SIGKILL`. **Windows has no SIGTERM**: the core closes the child's **stdin** (and/or sends `CTRL_BREAK` to a `CREATE_NEW_PROCESS_GROUP` child); the helper must treat stdin-EOF / console-ctrl as "finalize and exit". This matters for `meeting-tap` (must write the `.mp4` moov atom). |
| Encoding | stdout is UTF-8. JSON keys are camelCase and **stable** — reproduce verbatim. Number timestamps are epoch-milliseconds. |
| stderr | Human logs / diagnostics only. The core surfaces stderr **only on non-zero exit** for the one-shot helpers. |
| `--selftest` | Every helper supports `--selftest`: run pure-logic checks, print one line, exit `0` on pass / `1` on fail. Used as a build smoke test. |
| Resource layout | The core builds helper paths as `<resources>/<subdir>/<stem>` and appends `.exe` on Windows. See `native_host` in the core. |

---

## 1. `meeting-tap` — system + mic audio, optional H.264 video

Long-running capture daemon. Detects an in-progress call, captures the *other* participants'
audio (permission-free on macOS via Core Audio process taps), segments on silence (energy VAD),
transcribes whole utterances via an HTTP Whisper server, and POSTs results to the core's loopback
control server. Optionally records screen video.

**Argv**
```
meeting-tap <guiPort=8787> <whisperURL=http://127.0.0.1:8091/inference> [flags]
  --app auto|all|<id>        # which app to scope audio to (default auto)
  --mic on|off               # also capture the microphone
  --me <name>                # display name for "self" captions
  --gate on|off              # energy VAD gate
  --video off|always|share   # screen recording mode
  --rec-dir <dir>            # .mp4 output dir (default data/meetings/rec)
  --temp-dir <dir>           # per-utterance temp WAVs
  --gui-token <t> --stt-token <t>
  --rms --mic-rms --min-conf --expressions --max-seg <n>
meeting-tap --list-apps       # print capturable apps, one per line, then exit 0
meeting-tap --selftest
```
Positional 1 is `guiPort` only if it does not start with `--`. Env: `ASSISTANT_GUI_TOKEN`,
`ASSISTANT_STT_TOKEN`, `ASSISTANT_PARENT_PID`.

**I/O — this helper does NOT use a stdout line protocol.** Its control channel is HTTP:
- **Pull** config every 2 s: `GET http://127.0.0.1:<guiPort>/api/state` → reads
  `modules.meetings`, `sharing`, `videoMode`, `translate`, `language`, `manual`, `autoDetect`,
  `browserMeet`, `vocab`.
- **Push** events: `POST http://127.0.0.1:<guiPort>/ingest` with `Authorization: Bearer <GUI_TOKEN>`.
  Three JSON message types:
  ```json
  {"type":"caption","speaker":"Speaker","text":"…","self":false,"ts":1690000000000}
  {"type":"recording","file":"1690000000000.mp4","startedAt":1690000000000}
  {"type":"state","source":"coreaudio","state":{"inMeeting":true,"app":"zoom","meetingTitle":"…","title":"…"}}
  ```
  `source:"coreaudio"` marks the state **authoritative** — only this helper may start/end a meeting.
  On other platforms keep `source:"coreaudio"` (the core keys on that string) even though the audio
  source differs; treat it as "authoritative audio-derived state".

**`--list-apps` output**: one app per line. macOS: `bundleId\tappName`. Windows: `exeName\twindowTitle`.
Linux: `nodeId\tapplicationName`. The `--app <id>` selector matches whatever identifier this prints.

**Files**: per-utterance temp WAV (16 kHz mono PCM16, deleted after transcribe); `<recDir>/<epochMs>.mp4`
(H.264, ≤4 Mbps, 10 fps, keyframe ~10 s, fragmented for crash-safety); sibling `<epochMs>.jpg` first-frame thumbnail (≤480 px, q≈0.76).

**Signals**: trap graceful-stop → finalize the in-progress `.mp4` → exit after ≤3 s grace. Also exit if the parent dies.

**macOS concept → map per platform**: Core Audio process taps (call detection + per-app capture) →
Windows WASAPI Audio Session API + process loopback; Linux PipeWire node link / Pulse monitor source.
Bundle-ID app table → exe-name/AUMID (Windows) or `application.process.binary` (Linux).

---

## 2. `dictation-target` — inject dictated text into the focused field

**Argv**
```
dictation-target             # one-shot: classify focused field, emit Result JSON, exit
dictation-target --session   # classify, emit Result, then run a stdin command loop
dictation-target --selftest
```

**One-shot / first line (always)** — the classification `Result`:
```json
{"kind":"naturalLanguage","secure":false,"startsSentence":true,"liveEditable":true}
```
`kind ∈ {secure, unknown, codeOrTerminal, naturalLanguage}`.

**`--session` stdin** — one JSON command per line:
```json
{"op":"preview","text":"…"}   {"op":"commit","text":"…"}   {"op":"cancel"}
```
**`--session` stdout** — one JSON response per line after each command:
```json
{"ok":false,"active":true,"error":"cursorChanged","unsafeFallback":true}
```
Error tokens: `invalidCommand`, `targetChanged`, `provisionalTextChanged`, `cursorChanged`,
`previewEditRejected`, `previewEditNotObserved`, `provisionalTextUnavailable`, `finalEditRejected`,
`finalEditNotObserved`, `previewRemovalRejected`, `unknownCommand`. `preview` text ≤4000 chars,
`commit` ≤20000. Process ends on stdin EOF (remove any outstanding provisional span first).

**macOS concept → map**: AX focused element + `AXSelectedTextRange` + Unicode CGEvent keystrokes →
Windows UIA `GetFocusedElement`/`TextPattern` + `SendInput(KEYEVENTF_UNICODE)` (secure = `IsPassword`);
Linux AT-SPI focused element + XTEST/enigo (X11) or clipboard-paste (Wayland). Set `liveEditable=false`
conservatively when a stable caret cannot be guaranteed (typical on Wayland).

---

## 3. `agent-context` — read-only accessibility snapshot

**Argv**: `agent-context --once` (emit one `Snapshot`, exit 0) | `--selftest`. stdin unused.
Host enforces a 3 s timeout and rejects >256 KB stdout.

**Snapshot** (one JSON object + newline, sorted keys):
```json
{"ok":true,"status":"ready","snapshotId":"…","capturedAt":1690000000000,"source":"frontmost_app",
 "appName":"TextEdit","bundleId":"com.apple.TextEdit","processIdentifier":4242,"windowTitle":"Untitled",
 "focusedRole":"AXTextArea","focusedLabel":"Body","focusedFingerprint":"<64-hex>",
 "selectionLocation":0,"selectionLength":0,"selectedText":null,"editable":true,
 "elementCount":12,"truncated":false,"elements":[…],"privacy":["read_only","on_demand",
 "secure_fields_redacted","no_screenshot","no_network","no_persistence"],"error":null}
```
`status ∈ {ready, permission_required, no_target, no_window}`. Each `Element`:
`{id, role, subrole, label, value, frame{x,y,width,height}, enabled, focused, editable, secure, actions[]}`.
Bounds: `maxElements 180`, `maxDepth 12`, `maxChildrenPerElement 80`, `captureBudget 0.75s`. Secure
fields redacted (value nulled). `focusedFingerprint` = SHA-256 over `v1\u{1f}role\u{1f}label\u{1f}value[\u{1f}loc:len]`.

**macOS concept → map**: AX roles (`AXTextField`/`AXTextArea`/…) → UIA control types / AT-SPI roles via a
mapping table so the emitted `role` strings stay the `AX*` tokens the core expects.

---

## 4. `agent-action` — perform one guarded UI mutation

**Argv**: `agent-action --request` (read one `ActionRequest` JSON from stdin ≤256 KB, emit one
`ActionResponse`, exit 0) | `--selftest`. Host timeout 5 s.

**ActionRequest**:
```json
{"operation":"apply","action":"draft_text","text":"…",
 "expected":{"bundleId":"…","processIdentifier":42,"windowTitle":"…","focusedRole":"…","focusedLabel":"…","focusedFingerprint":"<64-hex>"},
 "undo":null}
```
`operation ∈ {apply, undo}`, `action ∈ {draft_text, replace_selection}`.

**ActionResponse**:
```json
{"ok":true,"status":"applied","verified":true,"undo":{…},"error":null}
```
`status ∈ {applied, undone, permission_required, unsupported_target, stale_target, activation_failed,
invalid_request, apply_failed, undo_failed, apply_rolled_back, undo_rolled_back, verification_failed}`.
Core requires `ok==true && verified==true`, then stores the returned `undo` record. Every op
revalidates process→app→window→control→fingerprint, applies, verifies after a ~30 ms settle, and
rolls back on unexpected divergence. Gated to one disposable editor (macOS `com.apple.TextEdit`;
Windows `notepad.exe`; Linux `org.gnome.TextEditor`).

---

## 5. `meeting-state` — track meeting-app state

**Argv**
```
meeting-state                 # watch: stream state JSON lines on every change (default)
meeting-state --once          # one snapshot line, exit
meeting-state --dump          # dump the a11y tree to STDERR (selector authoring), exit
meeting-state --selftest
meeting-state --interval <sec> --depth <n>
```
stdout is **unbuffered** (flush after each line). Env: `ASSISTANT_PARENT_PID`.

**Emitted lines** (NDJSON):
```json
{"type":"state","source":"accessibility","ts":1690000000000,"state":{"app":"zoom","title":"…","inMeeting":true,
 "selfMuted":true,"videoOn":false,"sharing":false,"participants":["…"],"participantEvidence":[…],
 "activeSpeaker":"…","activeSpeakerEvidence":{…},"ignoredParticipantEvidence":[]}}
{"type":"status","source":"accessibility","axTrusted":false,"msg":"…"}
```
`inMeeting` here is **advisory** — the core strips it (only `coreaudio` from `meeting-tap` is
authoritative) and merges the rest. On macOS it sleeps until woken by `meeting-tap`'s meeting-activity
signal and fails closed when the parent dies or the heartbeat goes stale.

> **Status (Windows/Linux):** the wake signal is **not yet wired** — the core does not pipe
> `meeting-tap`'s activity to `meeting-state`'s stdin, and neither platform helper subscribes to a
> D-Bus/named-event bus yet. Both currently **poll continuously** (a wasted-CPU gap, not a functional
> break). Wiring the wake (core-mediated `tap→core→state-stdin`, or a private D-Bus/named-event IPC)
> plus the stale-heartbeat fail-closed is tracked in `MIGRATION.md`.

**macOS concept → map**: `NSWorkspace.runningApplications` + bundle IDs → process enumeration by
exe/cmdline; menu-bar toggle labels ("Unmute Audio" ⇒ muted) → UIA toolbar buttons (Windows) / AT-SPI
menu items (Linux); `DistributedNotificationCenter` wake signal → core-mediated (tap→core→state stdin)
or a private D-Bus / named-event IPC (see status note above).

---

## Status legend used across `native/`

- **portable** — OS-agnostic logic, shared in `helpers-core`, identical on every platform.
- **implemented** — real per-OS code calling the right API.
- **TODO(on-device)** — correct API/crate identified; needs a real target toolchain + hardware to finish and verify. Never silently degraded — always logged.
