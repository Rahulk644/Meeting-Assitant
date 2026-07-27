# macOS human acceptance program

Automated tests prove logic, security boundaries, and build integrity. They cannot prove a physical
modifier-only key, focus behavior in third-party apps, real microphone response, a second human speaker, or
multi-display and full-screen placement. Those claims require recorded human acceptance against the exact
DMG candidate.

The canonical product cases are machine-readable in [`acceptance/cases.json`](../acceptance/cases.json).
Results and evidence are private by default under `~/.cache/my-assistant-acceptance`; never commit meeting
content, audio, screenshots, access tokens, API keys, or identities. The runner pins evidence size and
SHA-256 so later changes are detectable. `blocked` is an honest incomplete result, not a pass.

## Session workflow

```sh
python3 acceptance/run.py new \
  --suite right-command --operator-id qa-01 \
  --notes "Installed from candidate DMG"

python3 acceptance/run.py walk --session /private/path/session.json

python3 acceptance/run.py record --session /private/path/session.json \
  --case RC-001 --result pass --evidence /private/path/rapid-toggle.log

python3 acceptance/run.py summary --session /private/path/session.json
```

The summary exits non-zero while a required case is pending, failed, or blocked. Record the candidate app
version and SHA-256, macOS build, Mac model, selected speech/polish models, target/platform version, and the
operator's pseudonymous ID in the private session—not in durable project documentation.

## Physical Right Command

Use the app installed from the candidate DMG, not `cargo run`, an older `/Applications` copy, another
dictation product, or a synthesized keystroke.

The suite covers:

- rapid tap-to-toggle and a new recording while an older result finishes;
- Esc cancellation and push-to-talk;
- closed-window/background apps, native and web editors, full-screen Spaces, and multiple displays;
- silence, missing models, input-device loss, sleep/wake, and 25-cycle reliability;
- Input Monitoring, Microphone, and Accessibility denial/recovery;
- clipboard modes and target drift;
- final-only versus genuine streaming models; and
- pill centering, clipping, status widths, and waveform response to measured speech level.

Privacy-safe `acceptance:right-command` breadcrumbs record physical modifier edges, generation, lifecycle
phase, and timing. They contain no audio or transcript. A live dictation that already owns provisional text
must show **Post-processing** after Stop; a final-only model shows **Transcribing** until recognition ends.

Pass RC-005 only when Right Command works while the main window is closed and another app is frontmost. Pass
RC-017 only when silence produces a visibly quiet meter, speech produces a materially larger meter, and every
pill state remains centered and fully visible.

## Smart Polish

Create a separate session:

```sh
python3 acceptance/run.py new \
  --suite smart-polish --operator-id qa-01 \
  --notes "Packaged candidate; synthetic text only"
```

The suite covers off-by-default behavior; verified download/resume/cancel/remove; 270M and selected-model
lifecycle; English, Hindi, Hinglish, protected values, literal command words, code/terminal targets, secure
fields, prompt injection, crash/timeout/occupied-port fallback, active-display focus, session-only history,
warm-up/unload, and both cleanup-timing modes.

For **While I speak**, evidence must prove that only engine-committed phrases receive mechanical cleanup,
tentative words remain revisable, and Gemma does not run before Stop. Secure-field evidence must never include
the dictated text.

## Two-person Meetings Beta

Use two consenting people in a live call. A prerecorded loop, one-person simulation, local speaker playback,
or helper self-test is not platform acceptance.

Read and follow [`MEETINGS_USER_JOURNEY.md`](MEETINGS_USER_JOURNEY.md) before the first call. It defines the
observable Idle → Detected → Recording → Paused → Finalizing → Notes journey, expected timing, and explicit
stop/failure conditions. A tester should never need source-code knowledge to decide whether the product
worked.

```sh
python3 acceptance/run.py new \
  --suite two-person-meeting \
  --platform google-meet-chrome --platform-version "exact browser / Meet version" \
  --operator-id qa-01
```

Create one session per exact platform and version. The matrix covers Zoom native, Google Meet in Chrome and
Safari, Teams native and browser, Slack Huddles, Discord, and FaceTime. It verifies:

- no false recording when the app is merely open;
- one start edge after real call evidence;
- a non-activating movable Nub, no Recording claim until both sources open, expansion to five live transcript
  lines and direct last-minute catch-up, honest per-source Needs attention recovery, and synchronized
  native/WebView/menu-bar Pause and Finalizing states;
- the silent-remote regression: local speech is recorded from the beginning even when the other participant
  first speaks ten minutes later, and both appear in one complete meeting;
- post-flow system-audio dropout detection, visible source recovery, and bounded helper-process recycle/
  relaunch after repeated failures without splitting the meeting;
- scheduled versus impromptu title truth;
- both audio channels, overlap, speaker uncertainty, and live transcript;
- a system-audio source that opens without delivering buffers never reaches Recording, visibly retries, and
  cannot cause microphone/room playback to be labelled **You**;
- alternating remote turns use the correct Zoom display name only when timestamped evidence supports it;
  the local `(Me)` name never leaks onto the remote channel, rapid/overlapping remote turns stay unknown
  when evidence is closely split, and gallery/speaker/share layouts do not inherit a stale name;
- mute, pause/resume, scratchpad, optional video, thumbnail, and screen-sharing behavior;
- automatic end, notes, permission recovery, platform crash, and assistant relaunch;
- a 45-minute resource soak; and
- complete history after relaunch.

Record consent references outside the repository. Use pseudonymous participants and redacted exports. Do not
advertise automatic support for a platform merely because manual recording works; until its exact matrix
passes, Meetings remains Beta and manual recording is the fallback.

## Evidence rules

- A screenshot proves only visible state, not physical-key origin or audio correctness.
- A helper self-test proves only the helper's internal contract.
- Machine transcription of a benchmark does not replace human review where the gate requires it.
- Never convert a missing observation into a pass from logs alone.
- Link every failure to the smallest reproducible issue and retain evidence privately according to the
  operator's consent and data-retention policy.
