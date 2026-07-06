# meeting-state

A single-file Swift helper that reads live meeting UI state from native meeting apps (Zoom, Teams)
via the macOS **Accessibility API**, and emits newline-delimited JSON for a parent process.

**Role in this project:** it supplies the *speaker names* and *mute / share / in-meeting* signals that
enrich the transcript. The live caption **text** comes from on-device Whisper (see `../stt`), not here.

## Reads reliably today

`inMeeting`, `selfMuted`, `videoOn`, `sharing` — derived from the meeting app's **menu-bar toggle
labels** (`Unmute Audio` / `Stop Share` …). Stable across app versions and readable while backgrounded.

## Speaker names

`participants` and `activeSpeaker` are read from Zoom's video-tile element **descriptions**, shaped
like `"Rahul Khatri, Computer audio muted, Video off"` (and profile buttons `"View <Name>'s profile"`).
Zoom's always-on-top **floating video window** follows the active speaker in speaker view — and in a
1-on-1 it always shows the other person — so the name in that window's tile is who's talking. We set
`AXManualAccessibility = true` first to unlock Zoom's embedded UI, then harvest names (`zoomNames()`).

**Accuracy:** reliable for 1-on-1 (the floating window shows the remote person); in large gallery-view
calls the active-speaker pick is best-effort. Names update at the watch `--interval` (default 1s).

## Build & run

```bash
swiftc -O meeting-state.swift -o meeting-state

./meeting-state             # stream state as NDJSON on change
./meeting-state --once      # one snapshot, then exit
./meeting-state --dump      # dump a live meeting app's AX tree (build name selectors from this)
./meeting-state --selftest  # validate the derive logic (exit 0/1)
```

Requires **Accessibility** permission on the launching process (e.g. your Terminal).

## Emits

```json
{"type":"state","ts":1234567890,"state":{"app":"zoom","inMeeting":true,"selfMuted":false,"sharing":false,"participants":[],"activeSpeaker":null}}
```

License: MIT.
