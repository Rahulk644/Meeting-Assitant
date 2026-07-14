# macOS human acceptance program

Automated tests prove state logic, security boundaries, and build integrity. They cannot prove a physical
modifier-only key, focus behavior across third-party apps, a second human speaker, current meeting-platform
UI evidence, or multi-display/full-screen placement. Those claims require recorded human acceptance.

The canonical cases are machine-readable in [`acceptance/cases.json`](../acceptance/cases.json). Results,
screenshots, call details, and logs are private by default under `~/.cache/my-assistant-acceptance`; do not
commit them. A pass requires an existing evidence file, and the session pins its byte size and SHA-256 so
later edits are detectable. `blocked` is honest unfinished work, not a pass.

## Physical Right Command

Use the exact packaged app/DMG being considered for release, not `cargo run`, an older copy in Applications,
Wispr Flow, or a synthesized keystroke. Create a session:

```sh
python3 acceptance/run.py new \
  --suite right-command --operator-id qa-01 \
  --notes "Installed from candidate DMG"
```

The session covers rapid double taps, a new dictation while an older one is finishing, Esc cancellation,
push-to-talk, closed-window/background use, representative target apps, full-screen Spaces, multiple
displays, silence, missing model, permission recovery, sleep/wake, clipboard modes, input-device failure,
and a 25-cycle soak.

Run the guided, resumable walkthrough with `python3 acceptance/run.py walk --session /private/path/session.json`.
It writes one private observation file per completed case; screenshots, logs, and recordings can be attached
later with the explicit `record` command.

The packaged app writes privacy-safe `acceptance:right-command` breadcrumbs for physical modifier edges and
Listening/Transcribing/Cancelled/Idle phases. They contain no audio or transcript. Collect the app's normal
disk log plus screenshots or screen recordings as evidence; the rapid-toggle case should show two physical
down edges and an immediate move from Listening to Transcribing rather than a stuck pill.

Record a result and check completion:

```sh
python3 acceptance/run.py record --session /private/path/session.json \
  --case RC-001 --result pass --evidence /private/path/rapid-toggle.log

python3 acceptance/run.py summary --session /private/path/session.json
```

The summary exits non-zero while any case is pending, failed, or blocked.

## Two-person meeting matrix

Use two consenting people in a live call. A prerecorded loop, one-person simulation, or helper self-test is
not platform acceptance. Create one session per platform and exact platform version:

```sh
python3 acceptance/run.py new \
  --suite two-person-meeting \
  --platform google-meet-chrome --platform-version "Chrome x / Meet web" \
  --operator-id qa-01
```

The current matrix covers Zoom native, Google Meet in Chrome and Safari, Teams native and browser, Slack
Huddles, Discord, and FaceTime. Each session verifies no notification when the app is merely open; one start
notification after real call evidence; title precedence; both audio channels; speaker attribution and
uncertainty; overlap; live transcript; mute, pause, and scratchpad behavior; video and non-video thumbnails;
screen sharing; automatic call end and note generation; permission and crash recovery; a 45-minute soak; and
history after relaunch.

For every session record:

- the candidate app hash/version, macOS build, Mac model, platform/browser version, and capture settings;
- pseudonymous participant IDs and consent references outside the committed repo;
- screenshots/logs for detection, transcript, summary state, and thumbnail choice;
- a redacted meeting export for line ordering and persistence checks;
- starting/peak/settled RSS and disk growth for the soak case;
- a linked issue for every failure, with the smallest reproducible sequence.

Do not advertise a platform as automatically supported merely because manual recording works there. Until
its exact matrix passes, automatic coverage remains beta and manual recording is the fallback.
