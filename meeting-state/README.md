# meeting-state

A single-file Swift helper that reads live meeting UI state from the audio-active native meeting,
browser, or communication app via the macOS **Accessibility API**, and emits newline-delimited JSON
for a parent process.

**Role in this project:** it supplies the *speaker names* and *mute / share* signals that enrich the
transcript. The live caption **text** comes from the selected on-device speech model (see `../stt`), not here. Core Audio in
`meeting-tap` is the only authority for automatic meeting start/end. Dedicated conferencing apps use
active per-app audio; browser/chat platforms require audio plus narrowly scoped meeting-window evidence.

In watch mode this helper is **meeting-gated**. It performs no Accessibility scans while meeting apps are
idle. `meeting-tap` publishes an internal distributed notification with the exact audio-active app; only
then does this helper install `AXObserver` and begin a meeting-only reconciliation timer. The event is
internal plumbing, not a user-visible Notification Center alert.

## Reads reliably today

`title`, `selfMuted`, `videoOn`, `sharing` (plus a non-authoritative `inMeeting` hint). The title is taken
only from the active meeting window and generic UI chrome is rejected. Controls are derived from the meeting
app's **menu-bar toggle labels** (`Unmute Audio` / `Stop Share` …); coverage varies by platform and version.

## Speaker names

`participants` and `activeSpeaker` are read from Zoom's video-tile element **descriptions**, shaped
like `"Rahul Khatri, Computer audio muted, Video off"` (and profile buttons `"View <Name>'s profile"`).
Zoom's always-on-top **floating video window** follows the active speaker in speaker view — and in a
1-on-1 it always shows the other person — so the name in that window's tile is who's talking. We set
`AXManualAccessibility = true` first to unlock Zoom's embedded UI, then harvest names (`zoomNames()`).

**Accuracy:** reliable for 1-on-1 (the floating window shows the remote person); in large gallery-view
calls the active-speaker pick is best-effort. Names update at the watch `--interval` (default 1s).

Every accepted name includes `participantEvidence` with its AX source, element role, and containing window
context. `activeSpeakerEvidence` distinguishes participant-list, video-tile, floating-tile, and active-speaker
label evidence. Tile-shaped Zoom interface text is retained only in `ignoredParticipantEvidence`; it never
enters the participant roster.

## Build & run

```sh
# Canonical production build from the repository root
./script/build_native_resources.sh

./meeting-state             # wait for Core Audio activity, then stream AX state as NDJSON on change
./meeting-state --once      # one snapshot, then exit
./meeting-state --dump      # dump a live meeting app's AX tree (build name selectors from this)
./meeting-state --selftest  # validate the derive logic (exit 0/1)
```

An isolated direct run requires **Accessibility** permission on the launching process. Production invokes
the helper only from the signed app bundle.

## Emits

```json
{"type":"state","source":"accessibility","ts":1234567890,"state":{"app":"zoom","title":"Customer review","inMeeting":true,"selfMuted":false,"sharing":false,"participants":[],"activeSpeaker":null}}
```

License: MIT.
