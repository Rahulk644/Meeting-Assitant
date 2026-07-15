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

## Agent context, proposal, Apply, and Undo matrix

Use only disposable documents and dedicated test accounts; this is not permission to inspect private inboxes,
chats, profiles, documents, or terminal output. The suite covers Gmail, Slack, LinkedIn, Google Docs, Apple
Notes, Google Slides, Terminal, secure fields, permission recovery, full-screen/displays, prompt injection,
the physical global shortcut, both compute lanes, and stale/missing targets:

```sh
python3 acceptance/run.py new \
  --suite agent-context --operator-id qa-01 \
  --notes "Synthetic content only; packaged preview build"
```

Evidence must record aggregate roles/counts, timing, capability state, and the exact synthetic expected
outcome. Do not
attach screenshots or logs containing page text, document contents, account identifiers, tokens, API keys,
or personal data. A useful tree is not sufficient to pass the action milestone. The preview channel now has
one deliberately narrow mutation class: reviewed draft insertion or selection replacement followed by a
guarded one-step Undo in an explicitly disposable TextEdit document. Gmail, Slack, LinkedIn, Docs, Notes,
Slides, Terminal, and every other app are read-only in this phase and must never render Apply. Every TextEdit
Apply must visibly name the app/window/control, bind the exact captured process, prove fresh-state
revalidation, and verify the final output. Stale targets, non-TextEdit targets, secure fields, unsupported
controls, switching to another app before Apply, changed text or selection, edited-after-Apply Undo, expired
state, and relaunch must fail closed. Unexpected writes must either
restore and verify the original text and selection or produce an urgent unverified-state warning. Send,
Return, click, and multi-step actions remain nonexistent.

Keychain authorization has its own packaged case. Save one disposable provider key in an unchanged signed
build, exercise repeated model loads and proposals, close/reopen the window, and relaunch once. Count prompts
without recording the password or key. Repeated requests in one app session must use the process-only cache;
an unchanged build must respect macOS **Always Allow** across relaunches. Re-signing a developer build changes
the test subject and is recorded separately rather than presented as normal update behavior.

## Wake-word benchmark

The wake-word suite is a product and licensing gate, not evidence that wake word ships. It requires explicit
opt-in, an externally installed candidate, the tester's own Porcupine AccessKey when applicable, and a
dedicated consented corpus:

```sh
python3 acceptance/run.py new \
  --suite wake-word --operator-id qa-01 \
  --notes "Porcupine candidate; external adapter; private synthetic corpus"
```

Run the identical corpus through Porcupine, the tiny embedded-first microWakeWord control, and the maintained
desktop sherpa-onnx keyword-spotting control. Record false accepts,
false rejects, p50/p95 latency, CPU, physical footprint/RSS, energy, disk cost, suspension behavior, privacy,
and exact licensing terms. The detailed protocol is [`WAKE_WORD_BENCHMARK.md`](WAKE_WORD_BENCHMARK.md).

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
