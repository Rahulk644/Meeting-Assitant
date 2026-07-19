# My Assistant

My Assistant is a local-first macOS app for dependable system-wide dictation and private, no-bot meeting
transcription.

## Product status

| Surface | Release status | Contract |
|---|---|---|
| Dictation | Primary feature | Works system-wide, processes speech on-device, and inserts into the field that was focused when recording began. |
| Meetings | Beta | Captures local meeting audio without joining as a bot, shows a live transcript, and keeps the meeting library on the Mac. |
| Agents | Coming soon | A greyed-out, disabled navigation label only. The initial product exposes no Agent route, settings, shortcut, helper, or product details. |

The exact production boundary is enforced by [`PRODUCT_RELEASE_MATRIX.md`](PRODUCT_RELEASE_MATRIX.md) and a
server-owned release contract. Saved browser state cannot unlock a hidden surface, and the release pipeline
always packages the production channel.

## Dictation

Press **Right Command** to start, speak, then press it again to stop. The compact native pill stays centered
on the selected screen, shows elapsed time, and drives its waveform from the microphone level.

- Speech capture and transcription run locally.
- Whisper models insert one authoritative result after Stop.
- Parakeet and Nemotron models that implement genuine incremental decoding can show live text in compatible
  editable fields. Tentative words remain revisable until the model commits them.
- If live reconciliation becomes unsafe because the target or cursor changed, the app fails closed and
  preserves the final result on the clipboard rather than risking duplicate or misplaced text.
- **Smart Polish** is experimental and off by default. Deterministic cleanup and the optional local Gemma
  pass run after Stop. The optional **While I speak** setting applies only conservative mechanical cleanup to
  stable committed phrases; model polishing never runs while the user is speaking.
- Secure fields are excluded from live preview, history, visual diffs, and model processing.

The first launch asks the user to choose a revision-pinned speech model. Model files are downloaded
separately, verified by exact size and SHA-256, and never bundled in the DMG.

## Meetings Beta

Meetings captures the user's microphone and the other participants' app audio locally. Dedicated meeting
apps are detected from active per-app audio; browser and communication-app calls additionally require
meeting-window evidence to reduce false starts. Manual recording remains available for unsupported apps.

The beta includes a live transcript, speaker evidence where the meeting app exposes it, transcript and
participant corrections, meeting titles, notes, Ask, Markdown export, and an optional screen recording.
Meeting summaries can stay local when the optional local Intelligence runtime is installed, or use a
provider selected by the user with a key stored in macOS Keychain.

## Requirements

- Apple Silicon Mac
- macOS 14.4 or newer
- Free disk space for the selected speech model (roughly 44 MB–1.2 GB)

## Install the beta DMG

1. Open the DMG and drag **My Assistant** to **Applications**.
2. Open the app once.
3. This independent beta uses a stable self-signed certificate, not an Apple Developer ID. If macOS blocks
   the first launch, choose **Done**, then open **System Settings → Privacy & Security → Open Anyway** and
   confirm the launch.
4. Choose a speech model and grant only the permissions required by the features you use.

The app and every bundled executable are signed with hardened runtime, but the current beta is not
Apple-notarized. A Developer ID and notarization are required before lower-friction public distribution.

## macOS permissions

| Permission | Why it is needed |
|---|---|
| Microphone | Dictation and the user's side of meetings |
| Input Monitoring | The default bare **Right Command** shortcut while another app is frontmost; ordinary key combinations use Carbon and do not need it |
| Accessibility | Insert dictated text, recover the exact live-text target, and read meeting speaker/window evidence |
| System Audio Recording | Other meeting participants without a bot |
| Screen Recording | Optional meeting video only |

The app requests access on first use and links directly to the matching Privacy & Security pane. See
[`SECURITY.md`](SECURITY.md) for the complete privacy and data-retention contract.

## Architecture at a glance

```text
Native Tauri/Rust app
├─ AppKit tray and dictation pill
├─ in-process microphone capture, VAD, shortcut handling, and local API
├─ dictation-target (bounded Accessibility session for live field reconciliation)
├─ transcribe-server (pinned transcribe.cpp; selected speech model)
├─ polish-runtime (pinned llama.cpp; optional downloaded Gemma model)
├─ meeting-tap (Core Audio / ScreenCaptureKit capture)
└─ meeting-state (bounded Accessibility meeting evidence)
```

The packaged app has no Node runtime, telemetry service, model weights, or Agent helper. The WebView talks
only to an authenticated loopback service inside the Rust process. [`ARCHITECTURE.md`](ARCHITECTURE.md)
documents the trust boundaries and lifecycle in detail.

## Development

Prerequisites are Xcode Command Line Tools, Rust stable, Node, Git, Homebrew, and CMake. Creating a release
also requires `cargo-audit` 0.22.2 (`cargo install cargo-audit --version 0.22.2 --locked`).

```sh
# One-time local model and native-resource setup
./setup.sh

# Build, atomically install exactly one /Applications/My Assistant.app, launch, and verify it
./script/build_and_run.sh --verify

# Fast UI/developer-server loop
cd app && npm test

# Desktop quality gate
cd desktop/src-tauri
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked
cargo test --locked

# Complete signed local candidate (defaults to an honest `unreleased` artifact label)
./script/build_release_dmg.sh

# A named beta is created only alongside matching release evidence
RELEASE_LABEL=beta.N ./script/build_release_dmg.sh
```

The release command runs the source checks, builds every native resource from its pinned source, forces the
production UI contract, signs and inspects all nested executables, creates the DMG, mounts it read-only, and
verifies the exact installable contents.

## Repository map

| Path | Purpose |
|---|---|
| `app/` | Source WebView, dependency-free development server, release registry, and Node tests |
| `desktop/src-tauri/` | Rust/Tauri application, local service, dictation runtime, settings, and tests |
| `dictation-target/` | Bounded native live-text target and reconciliation helper |
| `polish-runtime/` | Pinned local Smart Polish runtime boundary; model weights stay external |
| `stt/` | Meeting capture and pinned transcription server integration |
| `meeting-state/` | Native meeting-window and speaker evidence helper |
| `acceptance/` | Machine-readable physical acceptance contracts and runner |
| `benchmarks/speech/` | Reproducible model qualification tooling and aggregate evidence |
| `script/` | Canonical build, verification, benchmark, install, and release entry points |

## Documentation

- [`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md) — durable project memory and the first file to read after
  a context compaction or handoff
- [`AGENTS.md`](AGENTS.md) — short operating contract that routes maintainers and coding agents to durable truth
- [`PRODUCT_RELEASE_MATRIX.md`](PRODUCT_RELEASE_MATRIX.md) — production capability authority
- [`docs/ENGINEERING_HEALTH.md`](docs/ENGINEERING_HEALTH.md) — current strengths, structural debt, and
  performance/documentation contracts
- [`docs/IMPLEMENTATION_ROADMAP.md`](docs/IMPLEMENTATION_ROADMAP.md) — end-to-end customer experience,
  actionable work, dependencies, estimates, and parallel-session rules
- [`docs/OSS_COMPONENT_STRATEGY.md`](docs/OSS_COMPONENT_STRATEGY.md) — component adoption decisions and safe
  upstream-update policy
- [`RELEASE_READINESS.md`](RELEASE_READINESS.md) — evergreen release gates and known distribution limits
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — runtime architecture and invariants
- [`SECURITY.md`](SECURITY.md) — privacy, storage, permissions, secrets, and reporting
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — development and pull-request standards
- [`docs/ACCEPTANCE_TESTS.md`](docs/ACCEPTANCE_TESTS.md) — physical test protocol
- [`docs/SPEECH_MODEL_BENCHMARK.md`](docs/SPEECH_MODEL_BENCHMARK.md) — speech-model evidence and promotion rules

Current artifact hashes, test counts, dates, and branch narratives belong in release notes and CI, not in
the durable project memory. The prioritized remaining work is maintained once in
[`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md#prioritized-next-work).

## License

MIT. See [`LICENSE`](LICENSE) and [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
