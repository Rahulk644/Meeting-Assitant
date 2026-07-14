# Project state

**Current as of:** 15 July 2026

**Canonical repository:** `/Users/rahulkhatri/Meeting-Assitant`

**Target:** Apple-silicon macOS 14.4+
**Distribution:** self-signed independent beta; no paid Apple Developer ID

This is the shortest durable entry point for resuming work. It describes current product truth, not a
chronological work log.

## Product boundary

- **Dictation:** primary beta. Right Command starts/stops system-wide, on-device dictation. The native
  AppKit status pill is non-activating, works across full-screen spaces, names the configured finish
  shortcut, and exposes Esc cancellation.
- **Meetings:** beta. Dedicated call apps detect from Core Audio activity; browser and multipurpose chat calls
  additionally require meeting-window evidence. Calls are captured without a bot, transcribed locally,
  stored on-device, and summarized with a configured local or cloud model. Manual capture works universally.
- **Writing Assistant and Agents:** design-preview only. They are not production promises.
- **Local speech models:** shipped as an explicit model library shared by dictation and meetings. Downloads
  are separate from the app and the active model controls languages, translation, output, and preprocessing.
- **Cloud speech, calendar sync, updates, accounts, teams, sharing, and billing:**
  not shipped.
- **Meeting Intelligence:** Groq, OpenAI, Cerebras, and custom OpenAI-compatible BYOK paths are shipped.
  The optional local Gemma/LiteRT-LM runtime works when installed by source setup, but the DMG has no in-app
  Intelligence-model installer yet; do not describe local notes as clean-install ready.

The exact visibility contract is [`PRODUCT_RELEASE_MATRIX.md`](../PRODUCT_RELEASE_MATRIX.md). Product and
provider sequencing lives in [`PRODUCT_STRATEGY.md`](../PRODUCT_STRATEGY.md). Competitive decisions live in
[`COMPETITIVE_INTELLIGENCE.md`](COMPETITIVE_INTELLIGENCE.md). Speech-model evidence and the current Hinglish
decision live in [`SPEECH_MODEL_BENCHMARK.md`](SPEECH_MODEL_BENCHMARK.md).

## Current architecture

```text
AppKit/Tauri shell
  ├─ native Right Command event tap, mic capture, VAD, paste, tray, and status pill
  ├─ authenticated Rust loopback service at 127.0.0.1:8790
  ├─ meeting-tap (Swift): real-call detection, system audio, mic, optional video
  ├─ meeting-state (Swift): Accessibility-derived speaker/mute/share evidence
  └─ transcribe-server (C++): on-demand transcribe.cpp/selected local speech model
```

The packaged app contains no Node runtime. `app/server.js` is a developer-only behavioural harness. Speech
models and user data live outside the app bundle under Application Support. Provider credentials live in
macOS Keychain.

## Reliability invariants

- The pill and tray are native; there is no secondary or hidden pill WebView.
- Every authenticated UI launch redirects to a non-secret per-launch URL so WKWebView cannot restore a stale
  blank or partially hidden DOM from an older build.
- The packaged release contract is validated before any window or helper starts; invalid core navigation
  fails startup instead of degrading to a partial UI.
- The audio detector is the only automatic meeting start/end authority. Dedicated apps require active
  per-app audio; browser/chat apps require that plus meeting-window evidence. An open app, YouTube tab, or
  notification sound is insufficient.
- A meeting-start notification is emitted only on the false-to-true real-call edge.
- Meeting JSON is owner-only and atomically replaced. The first transcript line is persisted immediately,
  subsequent lines are checkpointed at most two seconds apart, and a 10-second maintenance checkpoint is a
  second safety net.
- Meeting-title precedence is user rename → calendar/scheduled metadata → valid meeting-window title →
  `Impromptu meeting`. Transcript text is never guessed into a title.
- Video meetings write their first-frame thumbnail natively; non-video cards use the installed native app
  icon, with a platform identity tile fallback.
- Incomplete meeting JSON is finalized as interrupted on the next launch; transcript survival does not
  depend on summary generation.
- App-started helpers receive the parent PID and must exit when the parent disappears.
- Whisper unload is user-configurable and defaults to 10 idle minutes.
- Automatic speech language is preserved end-to-end; the app must never rewrite `auto` to English. Roman,
  natural mixed script, ordinary English, and English translation are separate model/evaluation tracks.
- Audio preprocessing is a model policy, not a global toggle. Whisper uses compact speech/noise protection;
  Parakeet, Nemotron, and the Oriserve fine-tunes use a speech gate that preserves specialist-model input.
- Model downloads are explicit, revision-pinned, exact-size and SHA-256 verified, resumable, cancellable,
  private (owner-only), atomically installed, removable, and never bundled into the DMG.
- Language intent is preserved across switches and resolved against the active model at inference time.
  English-only and Roman-Hinglish models cannot receive an incompatible decoder language.
- Translation is visible only for Whisper Medium multilingual and Large v3. Turbo, English-only models,
  Parakeet, Nemotron, and Oriserve fail closed in both the UI and Rust service.
- Native transcription keeps valid partial text when a model reaches its output limit, reports truncation,
  and guarantees valid UTF-8 JSON.
- API keys never belong in JSON, SQLite, logs, process arguments, temporary files, or WebView state.

## Release status

The current automated-verified artifact is
`desktop/src-tauri/target/release/bundle/dmg/My_Assistant_0.1.0-beta.1_arm64.dmg` (14,586,155 bytes), SHA-256
`9836452224f514765c7f8e7b4bd7d744d89de4dfc942b09322d8a3198a4b20c7`. The app bundle is 28,260 KiB.
After a short post-launch idle, the app and both meeting helpers measured about 38.2 MiB aggregate physical
footprint and 49,600 KiB aggregate RSS; no speech or LLM process was resident. Verification is 29/29 Node
product/release/parity tests, 23/23 Rust tests, warnings-denied Clippy, 17/17 speech/corpus/matrix tests, 3/3
acceptance-runner tests, zero desktop npm vulnerabilities, and passing transcribe-server, meeting-tap, and
meeting-state self-tests. All 12 revision-pinned model endpoints returned the exact catalog content length.
The DMG passed checksum and deep signature verification, carries the production model picker and
capability-gated Translation payload, contains no model/benchmark/transcript/source artifact, launched
successfully, kept unauthenticated loopback access at 401, and rejected a forged Host at 403.

This is suitable for a small explicitly informed beta group, subject to the documented **Privacy & Security
→ Open Anyway** installation flow. The physical Right-Command matrix and real two-person meeting matrix below
remain human acceptance work; do not describe the beta as zero-bug or notarized.

Rebuild and remeasure after any native/helper change; do not send an older artifact merely because it exists.
The beta has no Apple Developer ID, so current macOS recipients must attempt launch and use **Privacy &
Security → Open Anyway** once. Automatic
background updates cannot be made trustworthy until a separate updater signing-key design is completed.

## Known release blockers and follow-ups

1. Add bounded audio recovery checkpoints; transcript recovery exists, but an utterance in flight during a
   hard crash can still be lost.
2. Add a user-triggered, redacted diagnostic archive and support ID.
3. Complete the physical Right-Command rapid-toggle/cancel/failure/focus/full-screen/multi-display acceptance
   session defined in `acceptance/cases.json`; automation and synthetic key events are not evidence.
4. Add explicit per-meeting Off / Transcript / Audio / Video choice and complete the real two-person
   platform/version acceptance matrix for every advertised native/browser/chat adapter.
5. Generate a dependency SBOM and complete third-party notice packaging.
6. Add signed update manifests only after an offline update-key and rollback design is documented and tested.
7. Complete quality evidence without blocking user choice: real café/wind/traffic recordings, long-form
   fixtures, a physical model switch matrix, and representative RAM/latency/thermal measurements. Keep
   Whisper Large v3 Turbo as the recommended default; never market proxy WER as human Hinglish accuracy.
8. Add and benchmark macOS 26 SpeechAnalyzer as an optional system Speech adapter, retain the local catalog
   fallback, and separate future cloud Speech providers from meeting-note Intelligence providers.
9. Add an in-app local Intelligence install/status/remove flow; until then, clean beta testers use BYOK for
   meeting notes even though dictation and transcription remain fully local.
10. The verified branch is ahead of `main` and the private repository has no PR, tag, or GitHub release.
    Review/merge it before treating GitHub's default branch or community profile as current OSS truth.

## Resume protocol

1. Read this file, `PRODUCT_RELEASE_MATRIX.md`, and `git status` before editing.
2. Preserve unrelated working-tree changes and inspect the current branch before making release changes.
3. Run `./script/prepare_web_assets.sh` before desktop checks/builds.
4. Use `./script/build_release_dmg.sh` as the release gate; it owns Node, benchmark, Rust format/Clippy/check/test,
   native helper, dependency audit, signing, build, and DMG verification.
5. Launch only the absolute repo-built app path during testing. `/Applications/My Assistant.app` may be an
   older comparison build.
6. Update this state page and the release matrix when product truth changes; keep historical detail in Git
   and user-facing changes in `CHANGELOG.md`.

Human acceptance truth is defined in [`ACCEPTANCE_TESTS.md`](ACCEPTANCE_TESTS.md). Speech corpus collection
and double-review truth is defined in
[`benchmarks/speech/CORPUS_PROTOCOL.md`](../benchmarks/speech/CORPUS_PROTOCOL.md).
