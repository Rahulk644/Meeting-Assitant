# Durable project state

Read this file first after a context compaction, task handoff, or long pause. It records the stable product
contract, architecture invariants, source-of-truth map, and remaining work. It deliberately does not record
dates, commit-by-commit history, transient test counts, artifact hashes, or branch narratives.

## Authority order

When documents disagree, resolve them in this order:

1. [`PRODUCT_RELEASE_MATRIX.md`](../PRODUCT_RELEASE_MATRIX.md) — what production may expose.
2. Executable contracts in `app/release.js`, its tests, and `desktop/src-tauri/tauri.conf.json` — what the
   current build actually enables and packages.
3. This file — the durable implementation state and priorities.
4. [`ARCHITECTURE.md`](../ARCHITECTURE.md) and [`SECURITY.md`](../SECURITY.md) — design and trust boundaries.
5. [`ENGINEERING_HEALTH.md`](ENGINEERING_HEALTH.md) — current structural debt and performance rules.
6. [`IMPLEMENTATION_ROADMAP.md`](IMPLEMENTATION_ROADMAP.md) and
   [`OSS_COMPONENT_STRATEGY.md`](OSS_COMPONENT_STRATEGY.md) — planned work and adoption decisions, not proof
   that a capability shipped.
7. [`PRODUCT_STRATEGY.md`](../PRODUCT_STRATEGY.md) — direction, not proof that a capability shipped.
8. [`CHANGELOG.md`](../CHANGELOG.md) and Git history — historical record only.

## Product contract

- **Dictation is the primary feature.** It must start from the configured global shortcut while another app
  is frontmost, display a compact native status pill on the active display, capture the microphone locally,
  transcribe locally, and place the result in the original target safely.
- **Meetings is Beta.** It captures calls without a bot, stores transcripts locally, and must remain visibly
  labelled Beta wherever represented as a product surface.
- **Agents is not an initial product feature.** Production shows exactly one greyed-out, disabled **Coming
  soon** navigation label. It has no route, details, settings, shortcut, boot initialization, compute lane,
  or packaged native helper. The default Rust build excludes the dormant routes, state, shortcut, compute
  runtime, and helper paths; an explicit `agent-research` Cargo feature is required to compile that backend.
  Production Web assets strip the research view/settings/code before minification. Dormant research source
  is not a release claim.
- Writing, commercial, updater, calendar, support, and other unfinished surfaces stay outside production
  until their release-matrix rows are explicitly promoted with evidence.

The release build forces `ASSISTANT_RELEASE_CHANNEL=production`; ambient shell state cannot accidentally
produce a preview DMG. Saved WebView state is sanitized against the server-owned capability map.

## Runtime shape

```text
My Assistant.app
├─ Rust/Tauri main process
│  ├─ authenticated loopback UI/API on 127.0.0.1:8790
│  ├─ native AppKit pill and tray
│  ├─ global shortcut and bare-modifier event tap
│  ├─ in-process microphone capture and Silero VAD
│  └─ dictation/meeting/model/settings orchestration
├─ dictation-target
│  └─ bounded Accessibility session for target classification and live-span reconciliation
├─ transcribe-server
│  └─ pinned transcribe.cpp with separate final and capability-gated streaming sessions
├─ polish-runtime
│  └─ pinned static llama.cpp server; optional Gemma weights are downloaded separately
├─ meeting-tap
│  └─ Core Audio process tap and optional ScreenCaptureKit recording
└─ meeting-state
   └─ bounded meeting-window, speaker, mute, and sharing evidence
```

The app bundle contains the minified WebView, licenses, and those five native executables. It contains no
Node runtime, speech/polish model weights, telemetry client, or Agent helper.

## Dictation state and safety invariants

1. **Shortcut.** The default bare Right Command shortcut is implemented by a native event tap and therefore
   requires Input Monitoring. Configured ordinary key combinations use Carbon and do not require Input
   Monitoring. Accessibility is independently required for insertion.
2. **Capture.** Microphone capture begins in the Rust process. The status pill is a non-activating AppKit
   panel on all Spaces; it must not steal the text target. The waveform is derived from the measured audio
   level, not an ornamental animation.
3. **Target.** `dictation-target` classifies only the focused role and a tiny prefix near the cursor. It never
   walks the full Accessibility tree, captures screenshots, logs field text, or processes secure fields.
4. **Live text.** Only selected model artifacts whose runtime implements native incremental streaming are
   eligible. Whisper remains final-only. A compatible field receives a tracked provisional span; tentative
   words may change, while committed words form the only stable cleanup boundary.
5. **Stop.** If live text is visible, the complete final-recognition and reconciliation wait is labelled
   **Post-processing…**. Final-only models show **Transcribing…** until recognition completes. Smart Polish,
   when enabled, always runs after Stop.
6. **Cleanup.** Mechanical cleanup is conservative, Unicode-aware, idempotent, and target-aware. The local
   Gemma pass is optional and guarded by a lossless validator. Any runtime, validation, or semantic failure
   preserves the deterministic result.
7. **Commit.** The app replaces only the exact provisional span it owns. Target or cursor drift fails closed;
   the final text is copied rather than pasted into an uncertain location. Final-only models use one
   Accessibility-authorized paste and restore the previous clipboard when configured.
8. **Timeout.** One atomic terminal owner prevents a slow worker from inserting text after the watchdog has
   reported a timeout or after a newer dictation has taken ownership of the UI.
9. **Privacy.** Secure-field text never enters recent history, visual diffs, logs, or model inference.
   Dictation audio uses a private temporary file and is removed immediately after transcription.
10. **Idle footprint.** Applying enabled settings at launch does not warm the speech or polish model. The
    selected runtime begins warming only after real dictation or meeting capture starts, then follows its
    bounded idle-unload policy.

## Meetings Beta invariants

- Platform adapters publish bounded raw meeting observations; the dependency-free Rust `MeetingDetector`
  alone owns automatic call edges. A new call needs two matching observations and call end needs three
  missing observations, so brief process/device renegotiation does not split a meeting or repeat the
  notification. Desktop meeting apps and browser/huddle sources can be disabled independently. Browser and
  communication-app calls also require meeting-window evidence; calendar presence alone never starts
  recording.
- The user can always start and stop manually. Pause/resume and call-end transitions have one recording
  lifecycle authority.
- The Rust lifecycle projects Starting, Recording, Needs attention, Pausing, Paused, Finalizing, and Idle to every
  meeting surface. Recording requires the microphone to open and meeting audio to produce a real first
  buffer; an API-level tap start alone cannot claim readiness. Healthy Core Audio taps deliver buffers
  during participant silence, so this does not wait for either person to speak. A source that opens without
  buffers or loses its post-flow heartbeat is torn down and retried automatically. Three failed recoveries inside one
  minute recycle the supervised capture helper with bounded relaunch backoff without splitting the Rust
  meeting record. macOS renders a movable 52 × 52 non-activating branded Nub above the active meeting
  Space; it expands into an adaptive vertical conversation card with bounded speaker-grouped transcript,
  direct last-minute catch-up, and recording controls. Platform overlays remain projections and never
  become a second lifecycle authority.
- Meeting captions carry their actual audio interval. The Rust meeting core keeps a bounded, timestamped
  active-speaker evidence timeline, filters the local participant, and applies a remote display name only
  when trusted platform evidence dominates that interval. Source and confidence are persisted; stale,
  conflicting, unsupported, or absent evidence remains **Unknown speaker**. Zoom supplies the current
  production evidence; Chrome Meet names and general multi-remote separation remain later Beta work.
- The microphone is labelled **You** only after the independent meeting-audio channel is verified. While
  that channel is starting, failed, or recovering, microphone captions are retained as **Unknown speaker**
  because speaker playback may be present. Even after readiness, explicit timestamped Zoom `Talking:`
  evidence can reclassify a microphone segment to the named remote participant; generic/floating-tile
  evidence cannot override microphone ownership.
- Recoverable transcript checkpoints are private. Corrections increment transcript revision and mark stale
  derived notes rather than silently presenting them as current.
- Meeting notes can use an optional local runtime or an explicitly selected BYOK provider. Provider keys live
  in macOS Keychain and are never stored in settings or browser state.
- Live Assist is a Meetings Beta surface, not Agent mode. Catch-up uses only the last 60 seconds of the
  current non-excluded transcript; summary, explicit actions, and follow-up questions use only the meeting
  so far plus the operator's scratchpad. It performs no external write or send action.
- Meeting audio remains local. Optional screen recording is separately permission-gated.

## Repository and release invariants

- `main` is the integration and release branch. A release starts from a clean worktree whose `main` matches
  `origin/main`.
- Models, native binaries, app bundles, DMGs, local meetings/settings, acceptance sessions, demo renders,
  and benchmark corpora are generated or private artifacts and remain outside Git.
- `script/build_native_resources.sh` is the single production-native build entry point. Local install and
  release scripts both call it so a stale helper cannot silently enter one build path.
- `script/build_and_run.sh --verify` atomically replaces `/Applications/My Assistant.app`, unregisters and
  removes the transient build-tree bundle, and verifies the installed app. There should be exactly one
  launchable app bundle.
- `script/install_verified_dmg.sh` performs the same atomic/rollback-safe installation from the mounted
  candidate, proving the installed app is byte-identical to the artifact rather than another local rebuild.
- `script/build_release_dmg.sh` runs the quality gates, builds pinned runtimes, forces production assets,
  signs with hardened runtime, and calls `script/verify_release_artifact.sh`.
- The artifact verifier mounts the DMG read-only and rejects missing/adhoc/wrong-authority nested code,
  non-arm64 executables, missing microphone entitlement, packaged model weights, Agent helpers, Agent
  research in the WebView, or a non-production release payload.
- The independent beta uses the stable self-signed `My Assistant Dev` identity. It is integrity-signed but
  not Apple-notarized; recipients must use Privacy & Security → Open Anyway.

## Verification commands

```sh
# Complete source gate
cd app && npm test
cd ../desktop && npm audit
cd src-tauri
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked
cargo test --locked
cargo audit

# Corpus/acceptance-runner unit tests
cd ../../..
./script/benchmark_speech.sh test

# Exact installed development build
./script/build_and_run.sh --verify

# Complete signed and mounted DMG validation
./script/build_release_dmg.sh
```

Machine tests establish logic and packaging contracts. They do not substitute for a physical Right Command,
real microphone, real target app, clean-machine permission, or real two-person meeting test.

## Prioritized next work

### P0 — distribution and physical reliability

1. Obtain an Apple Developer ID, add notarization credentials in CI or a protected release environment, and
   make stapling mandatory for a public channel. Keep the self-signed channel explicitly beta-only.
2. Run the physical dictation matrix from [`ACCEPTANCE_TESTS.md`](ACCEPTANCE_TESTS.md) on the mounted DMG:
   background apps, full-screen Spaces, multiple displays, sleep/wake, permission revoke/regrant, rapid
   start/stop, clipboard modes, and supported/unsupported live editors.
3. Run real microphone streaming tests for every catalog model marked live-capable. Record first-text latency,
   stable-prefix behavior, final reconciliation, target drift, and final-only fallback.
4. Run a clean-macOS-user install: Gatekeeper path, model download/verification, Input Monitoring,
   Microphone, Accessibility, System Audio, and optional Screen Recording.
5. Add a privacy-safe diagnostic export containing versions, permission state, model IDs, timings, and error
   codes but never audio, transcript text, API keys, focused-field text, or identities.
6. Resolve or explicitly target-scope RustSec maintenance warnings inherited through Tauri. Linux-only GTK3
   lockfile entries do not execute in the macOS artifact, while active cross-platform transitive warnings
   should be tracked to an upstream upgrade. Vulnerability findings remain release-blocking.
7. Generate an SPDX or CycloneDX SBOM and a consolidated dependency-license bundle, then include or publish
   both with every public binary release.
8. Establish one release-version source and procedure. Ordinary local artifacts remain labelled
   `unreleased`; a named beta requires matching version, changelog, tag/release evidence, and artifact label.

### P1 — Meetings Beta graduation

1. Extract the packaged service, app lifecycle, and WebView merge hotspots into the stable provider/domain
   seams defined by [`IMPLEMENTATION_ROADMAP.md`](IMPLEMENTATION_ROADMAP.md). Dormant Agent research is
   already excluded from the default compile and production WebView; preserve that boundary during the split.
2. Complete the real two-person meeting matrix across Zoom, Meet, Teams, FaceTime, Slack, input/output device
   changes, browser tabs, sleep/wake, pause/resume, and call-end recovery.
3. Package or implement a first-class verified install/status/remove lifecycle for local meeting Intelligence
   so clean Macs do not require BYOK for notes.
4. Add meeting chat, qualified local diarization, automatic identity resolution, calendar truth and attendee
   provenance, bounded audio crash recovery, and local Markdown-backed cross-meeting knowledge.

### P2 — product expansion

1. Add an optional server-side meeting provider behind one canonical meeting schema: qualify Recall first for
   managed PC-off capture, then Vexa/Attendee under the same tests before selecting a self-hosted path.
2. Build Rowboat-inspired inspectable Markdown memory and useful meeting prep/follow-up as meeting features;
   keep Agents disabled until a separate brief defines scope, safety, packaging, privacy, and acceptance.
3. Ship updater, signed release feed, versioned release notes, and rollback only after notarized artifacts are
   stable.
4. Define contextual voice replies as a bounded, reversible Dictation extension. Future connectors use the
   versioned meeting-output/action/receipt boundary in
   [`CONNECTOR_ARCHITECTURE.md`](CONNECTOR_ARCHITECTURE.md); add Activepieces only later for long-tail
   approved workflows, never for core capture or recovery.
5. Keep Windows work ready through target-gated contracts, platform-neutral fixtures, and the executable
   [`WINDOWS_HANDOFF.md`](WINDOWS_HANDOFF.md) packet. Start and verify X-00/adapters only on a Windows host
   after shared audio, meeting-event, storage, and performance contracts are stable and macOS quality metrics
   are frozen; a Mac cross-compile is not a Windows support claim.

## Resume protocol

At the start of resumed work:

1. Read this file and `PRODUCT_RELEASE_MATRIX.md`.
2. Run `git status --short --branch` and compare `main` with `origin/main`; never assume a summarized branch
   or artifact is current.
3. Inspect the working diff before editing. Preserve unrelated user files and never commit ignored runtime or
   demo artifacts.
4. Re-run the narrow tests for the touched contract, then the complete release gate before claiming a DMG.
5. Put artifact-specific SHA-256, size, version, date, and evidence in release notes or CI output—not here.
