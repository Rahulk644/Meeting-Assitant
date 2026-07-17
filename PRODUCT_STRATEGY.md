# Product strategy

This document describes direction and sequencing. It is not evidence that a feature shipped.
[`PRODUCT_RELEASE_MATRIX.md`](PRODUCT_RELEASE_MATRIX.md) remains the authority for production visibility,
and [`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md) owns the prioritized engineering backlog.

## Product promise

My Assistant should make spoken work disappear into the workflow:

1. dependable system-wide dictation that feels native, fast, and private;
2. no-bot meeting capture that produces a trustworthy, correctable record; and
3. useful local or explicitly selected intelligence built on that record.

The product should be valuable without an account, cloud processing, or a subscription. Users may choose a
cloud provider when it offers materially better intelligence, but the app must state exactly what leaves the
Mac and store the provider key in macOS Keychain.

## Current product boundary

- **Dictation is the primary product.** It receives the highest reliability, latency, and interaction-quality
  priority.
- **Meetings is Beta.** It is available for real use while its cross-app, cross-device, and recovery matrix is
  completed.
- **Agents is Coming soon only.** The initial product shows a disabled label and no details. Any future Agent
  work begins with a new product decision; dormant research does not create an implied roadmap commitment.
- Writing, commercial, calendar, updater, and unsupported integration surfaces remain outside production
  until they are implemented, verified, and deliberately promoted.

## Product principles

### Local first, explicit otherwise

Audio and transcription stay on-device. Optional cloud intelligence is opt-in per configured provider, and
the UI names the data being sent before use. No telemetry or My Assistant account is required.

### Truth over theatre

The app must label what it is doing: **Listening**, **Transcribing**, **Post-processing**, **Inserted**,
**Copied**, or an actionable failure. Decorative “live” output is not acceptable; live text is enabled only
when the selected model implements real incremental decoding and the target can be reconciled safely.

### One authoritative result

Live text is provisional. The final full-utterance recognition remains authoritative, Smart Polish runs only
after Stop, and insertion owns one tracked target span or one final paste. The product never trades text
integrity for animation.

### Correctable records

Meeting transcripts, speakers, and titles can be wrong. Corrections must be durable, and derived notes must
be marked stale when their source transcript changes.

### Honest capability gates

Designed, implemented, tested, packaged, and shipped are different states. A visible production control must
perform its advertised behavior. A saved preference, query parameter, or ambient build environment cannot
promote an unfinished surface.

## Engine strategy

Speech and intelligence are separate choices.

### Speech

The local catalog is curated rather than exhaustive:

- Whisper Large v3 Turbo is the balanced multilingual default.
- Other Whisper sizes expose explicit speed/quality or translation trade-offs.
- Parakeet and Nemotron are specialist choices and may expose live dictation only when the pinned runtime
  confirms its native streaming contract.
- Hinglish specialists remain explicit options until representative human evaluation supports broader
  recommendation.

Every artifact URL is revision-pinned and verified by size and SHA-256. Automatic selection must eventually
consider language, memory, latency, battery/thermal state, and benchmark evidence; it must remain explainable
and overridable. Apple SpeechAnalyzer is a future adapter, not a current product claim.

### Intelligence

Intelligence performs dictation cleanup, meeting summaries, actions, and questions. Local processing is
preferred. BYOK providers are allowed only through the Rust service with Keychain-backed credentials, HTTPS
or loopback endpoint validation, bounded requests, and immediate privacy copy.

Smart Polish begins with deterministic cleanup. Optional local Gemma output is accepted only when a strict
validator proves that protected numbers, URLs, commands, vocabulary, and semantic content survived. Model
unavailability or rejection returns the deterministic result without data loss.

## Delivery order

### Stage 1 — Dictation reliability

- physical Right Command and permission-recovery coverage;
- one exact installed app and one canonical native-resource build path;
- real streaming-model latency and reconciliation evidence;
- compact, centered, level-driven native overlay across displays and Spaces;
- safe final insertion, clipboard behavior, cancellation, timeout, and target-drift handling;
- downloadable model lifecycle and privacy-safe diagnostics.

### Stage 2 — Meetings Beta graduation

- real two-person matrices across major meeting apps and audio devices;
- diarization and editable identity quality;
- local Intelligence install/status/remove lifecycle;
- crash recovery, audio/video muxing, calendar truth, and local search;
- summary and Ask quality evaluated against corrected transcripts.

### Stage 3 — Distribution maturity

- Apple Developer ID notarization and stapling;
- signed updater and release feed with rollback;
- clean-machine onboarding and permission diagnostics;
- SBOM, consolidated dependency licenses, support/security response path, and repeatable release provenance.

### Stage 4 — Deliberate expansion

- reversible, selection-based Writing Assistant after its native target/runtime is proven;
- explicit integrations only when their authentication, privacy, and failure recovery are complete;
- Windows only after the shared Rust contracts and macOS quality metrics are stable;
- Agents only after a separate product brief and release program are approved.

## Success criteria

Dictation quality is measured by successful cross-app activation, time to listening, time to first live text
where supported, final word accuracy, safe insertion rate, reconciliation failures, and permission recovery.

Meetings quality is measured by capture start/stop correctness, speaker attribution, transcript correction
rate, recovery after interruption, note usefulness, privacy clarity, and end-to-end completion on supported
platforms.

No capability is promoted solely from unit tests, synthetic keyboard events, prerecorded audio, a demo video,
or a successful build. Physical evidence complements—not replaces—the automated release contract.
