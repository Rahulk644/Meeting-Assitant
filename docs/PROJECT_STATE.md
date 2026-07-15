# Project state

**Current as of:** 16 July 2026

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
- **Writing Assistant:** design-preview only. It is not a production promise.
- **Agents:** production exposes the on-demand read-only Accessibility inspector plus honest Local, API-key,
  and ChatGPT account/Codex compute configuration. API keys go directly to Keychain. Codex is a verified
  external runtime with an isolated profile and live account model controls; it is not bundled. The
  structured draft planner, global Agent shortcut, and one guarded text Apply/Undo boundary are preview-only.
  The model cannot execute; Apply is a separate click and is restricted to the exact captured process of a
  named disposable TextEdit document. Other apps remain read-only. The helper revalidates the complete field
  state and verifies the write or exact rollback; Send/Return/click/multi-step actions do not exist. Wake word
  remains blocked.
- **Local speech models:** shipped as an explicit model library shared by dictation and meetings. Downloads
  are separate from the app and the active model controls languages, translation, output, and preprocessing.
- **Cloud speech, calendar sync, updates, My Assistant accounts, teams, sharing, and billing:**
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
  ├─ agent-context (Swift, on demand): bounded read-only Accessibility snapshot
  ├─ agent-action (Swift, on demand, preview): confirmed draft/selection edit and guarded Undo
  ├─ codex_runtime (Rust, on demand): verified external Codex App Server, isolated auth/model/preview lane
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
- API keys never belong in JSON, SQLite, logs, process arguments, temporary files, or WebView state. Startup
  settings never decrypt Keychain items. An explicit provider request reads the credential at most once per
  app process, holds it only in process memory, and reuses that copy until quit.
- Agent context is captured only after an explicit user request, bounded by node/depth/child/time limits,
  uses opaque request-local element IDs, and redacts secure fields. Inspection alone is neither persisted nor
  sent to a provider. A preview request explicitly sends the instruction and fresh bounded snapshot only to
  the selected API/Codex lane; screen text is untrusted and output is reduced to draft/replace/no-op proposal
  data. The preview-only executor is a separate on-demand helper: it accepts only explicit Apply to the exact
  captured TextEdit process, revalidates bundle/window/role/label/full text/cursor/selection, and verifies the
  final value. Non-TextEdit drafts never receive an executable proposal. A transformed write is restored and
  verified; a rollback that cannot be proven is surfaced as urgent. A single prior value stays in Rust memory
  for at most five minutes and is restored only if the applied value is unchanged. Draft/original text never
  enters process arguments or logs.
- External Codex is accepted only from standard installed locations after native Security.framework
  validation of its `codex` identifier, Apple trust anchor, Developer ID certificate, and OpenAI Team ID
  `2DC432GLL2`. Cached paths are revalidated with a no-follow file handle held across every spawn, and the
  actual child process is validated before RPC; symlinks and replaced files fail closed. It is not touched
  during ordinary app launch: opening Agents with the
  ChatGPT · Codex lane selected triggers on-demand status, isolated device-code sign-in/out, model discovery,
  or a preview. Login polling cannot create competing processes, and no Codex process remains resident afterward.

## Release status

The last fully automated-verified **production** artifact is
`desktop/src-tauri/target/release/bundle/dmg/My_Assistant_0.1.0-beta.2_arm64.dmg` (14,759,965 bytes), SHA-256
`ace262ed08930aca9206378ee1617f72befe72c0af2248100e0fdc35224acc5a`. The app bundle is 28,704 KiB.
That artifact predates the 16 July anchored Codex signature correction and is held from new distribution.
The current branch rejects the former ad-hoc Team-ID substring spoof, accepts the official installed runtime,
and passes 44/44 Rust tests plus fmt, check, warnings-denied Clippy, and 33/33 Node tests. Build a new immutable
DMG and rerun the release pipeline before restoring sendable status.
After the packaged launch settled, the app plus the two meeting helpers measured about 38 MiB aggregate
physical footprint and 44,736 KiB aggregate RSS; the on-demand Agent helpers were not resident and no
app-owned Speech, LLM, or Codex process was resident. That artifact's release gate was 33/33 Node tests,
41/41 Rust tests, 17/17 speech/corpus/matrix tests, 3/3 acceptance-runner tests, zero desktop npm
vulnerabilities, and passing transcribe-server, meeting-tap, meeting-state, agent-context, and agent-action self-tests. All
12 revision-pinned model endpoints returned the exact catalog content length.
The DMG passed checksum and deep signature verification, carries the production model picker and
capability-gated Translation/Agent-compute payload, contains no Codex, Node, model, benchmark, transcript, or source artifact, launched
successfully, kept unauthenticated loopback access at 401, and rejected a forged Host at 403.

The artifact matched `codex/agent-actions` at build time but no longer matches current source after the
security correction. Its main executable is arm64 UUID
`CD4A10B1-EC33-3702-B2FC-D86A40D82328`, SHA-256
`1e42a14b0b0e6e2baa27b311a4554b2b6221d6a7bdd20b96a5ede5463e081dfa`. It launched from the exact
read-only DMG mount, rendered the production capability contract, defaulted a clean Agent compute profile to
the zero-network Local lane, opened an empty API-key lane without a Keychain dialog, and left no synthetic
provider, shortcut, debug, or Keychain item behind.
AGT-023 passed: three proposals plus model discovery reused one process-only credential copy, and the unchanged
installed candidate accessed the same disposable Keychain item after relaunch with zero authorization dialogs.
The controlled packaged TextEdit gate also passes cursor insertion, selection replacement, exact Undo,
app-switch rejection, stale text/selection rejection, and Undo-after-user-edit refusal (AGT-015/016/017/019/020).
The production bundle contains the signed preview helper but `agentActions`, draft preview, and the global Agent
shortcut are disabled by the production release contract. The helper remains process-bound and TextEdit-only.
Physical shortcut acceptance, expiry/relaunch
execution-state checks, non-TextEdit/secure-field breadth, and instrumented rollback outcomes remain open.
Do not describe the DMG as shipping Agent actions, and do not enable `agentActions` in production.

An older comparison build formerly present at `/Applications/My Assistant.app` had a different arm64 UUID and
was the source of the observed macOS reopen/crash loop. Its evidence was captured and the duplicate bundles
were removed after the final candidate passed; `/Applications/My Assistant.app` is now the only installed copy.
Never infer test identity from the display name alone.

The corrected source remains a small-beta candidate, but beta.2 is not the current sendable artifact. A newly
verified build will remain subject to the documented **Privacy & Security → Open Anyway** installation flow.
The physical Right-Command matrix and real two-person meeting matrix below remain human acceptance work; do
not describe the beta as zero-bug or notarized.

Rebuild and remeasure after any native/helper change; do not send an older artifact merely because it exists.
The beta has no Apple Developer ID, so current macOS recipients must attempt launch and use **Privacy &
Security → Open Anyway** once. Automatic
background updates cannot be made trustworthy until a separate updater signing-key design is completed.

## Known release blockers and follow-ups

1. Build and verify a new immutable DMG containing the anchored Codex runtime validation; do not newly
   distribute beta.2.
2. Add bounded audio recovery checkpoints; transcript recovery exists, but an utterance in flight during a
   hard crash can still be lost.
3. Add a user-triggered, redacted diagnostic archive and support ID.
4. Complete the physical Right-Command rapid-toggle/cancel/failure/focus/full-screen/multi-display acceptance
   session defined in `acceptance/cases.json`; automation and synthetic key events are not evidence.
5. Add explicit per-meeting Off / Transcript / Audio / Video choice and complete the real two-person
   platform/version acceptance matrix for every advertised native/browser/chat adapter.
6. Generate a dependency SBOM and complete third-party notice packaging.
7. Add signed update manifests only after an offline update-key and rollback design is documented and tested.
8. Complete quality evidence without blocking user choice: real café/wind/traffic recordings, long-form
   fixtures, a physical model switch matrix, and representative RAM/latency/thermal measurements. Keep
   Whisper Large v3 Turbo as the recommended default; never market proxy WER as human Hinglish accuracy.
9. Add and benchmark macOS 26 SpeechAnalyzer as an optional system Speech adapter, retain the local catalog
   fallback, and separate future cloud Speech providers from meeting-note Intelligence providers.
10. Add an in-app local Intelligence install/status/remove flow; until then, clean beta testers use BYOK for
   meeting notes even though dictation and transcription remain fully local.
11. Complete read-only Agent inspection/drafting across Gmail, Slack, LinkedIn, Docs, Notes, Slides, Terminal,
    secure/unsupported fields, permission recovery, prompt injection, providers, full-screen, and displays.
    Cursor Apply, selection replacement, app-switch/stale-state rejection, exact Undo, Undo-after-drift refusal,
    and unchanged-build Keychain authorization now pass on the packaged preview candidate. Still close physical
    global-shortcut, expiry/relaunch execution-state, secure/unsupported-target, packaged-helper residency,
    verified rollback, and urgent rollback-failure cases. Do not add another app adapter, Send, or multi-step
    mutation until those gates pass.
12. Run the identical Porcupine, microWakeWord, and sherpa-onnx KWS wake-word suite. Adopt none until real
    accents/noise, false accepts, latency, CPU/RAM, energy, suspension, privacy, integration, and licensing
    gates pass.
13. The verified work is a stacked draft-PR chain ahead of `main`: PR #1 `craft/cleanup-tests-docs`, PR #2
    `codex/agent-context-foundation`, PR #3 `codex/agent-compute-providers`, then PR #4
    `codex/agent-actions`. Review and merge them in that order before treating the private repository's
    default branch or community profile as current OSS truth, then create the first tag and GitHub beta
    release.

## Resume protocol

1. Read this file, `PRODUCT_RELEASE_MATRIX.md`, and `git status` before editing.
2. Preserve unrelated working-tree changes and inspect the current branch before making release changes.
3. Run `./script/prepare_web_assets.sh` before desktop checks/builds.
4. Use `./script/build_release_dmg.sh` as the release gate; it owns Node, benchmark, Rust format/Clippy/check/test,
   native helper, dependency audit, signing, build, and DMG verification.
5. Pin the exact candidate UUID/hash before launch. The repo bundle and `/Applications/My Assistant.app` may
   otherwise refer to different builds with the same bundle identifier and display name.
6. Update this state page and the release matrix when product truth changes; keep historical detail in Git
   and user-facing changes in `CHANGELOG.md`.

Human acceptance truth is defined in [`ACCEPTANCE_TESTS.md`](ACCEPTANCE_TESTS.md). Speech corpus collection
and double-review truth is defined in
[`benchmarks/speech/CORPUS_PROTOCOL.md`](../benchmarks/speech/CORPUS_PROTOCOL.md).
