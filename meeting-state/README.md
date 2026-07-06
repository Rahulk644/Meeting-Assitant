# meeting-state

A single-file Swift helper that reads live meeting UI state from native meeting apps (Zoom, Teams)
via the macOS **Accessibility API**, and emits newline-delimited JSON for a parent process.

**Role in this project:** it supplies the *speaker names* and *mute / share / in-meeting* signals that
enrich the transcript. The live caption **text** comes from on-device Whisper (see `../stt`), not here.

## Reads reliably today

`inMeeting`, `selfMuted`, `videoOn`, `sharing` — derived from the meeting app's **menu-bar toggle
labels** (`Unmute Audio` / `Stop Share` …). Stable across app versions and readable while backgrounded.

## Work in progress: speaker names

Zoom renders its meeting UI in embedded Chromium, which is opaque to Accessibility **until you set
`AXManualAccessibility = true`** on the process (done in `--dump` / `--probe`). Once unlocked, the
active-speaker name + participant list live in the speaker-tile / participants panel. Build the
selectors from a live `--dump`, then set `state.activeSpeaker` in the watch loop (see the `TODO(names)`).

## Build & run

```bash
swiftc -O meeting-state.swift -o meeting-state

./meeting-state             # stream state as NDJSON on change
./meeting-state --once      # one snapshot, then exit
./meeting-state --dump      # dump a live meeting app's AX tree (build name selectors from this)
./meeting-state --probe     # print text nodes as they appear while you speak
./meeting-state --selftest  # validate the derive logic (exit 0/1)
```

Requires **Accessibility** permission on the launching process (e.g. your Terminal).

## Emits

```json
{"type":"state","ts":1234567890,"state":{"app":"zoom","inMeeting":true,"selfMuted":false,"sharing":false,"participants":[],"activeSpeaker":null}}
```

License: MIT.
