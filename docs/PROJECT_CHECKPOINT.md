# Open-source project checkpoint

## Short version

My Assistant began as an attempt to build a small, local-first alternative to heavyweight dictation and
meeting applications. It reached a meaningful engineering prototype: system-wide dictation, a native macOS
status surface, local model management, bot-free two-channel meeting capture, a local meeting library,
bounded Accessibility evidence, and optional private transcription compute.

It did not reach a finished cross-platform product. Scope expanded from dictation into meeting capture,
speaker identity, live assistance, knowledge, connectors, Agents, distribution, and Windows. Meanwhile,
[Rowboat](https://github.com/rowboatlabs/rowboat) provided the stronger knowledge/agent interface, and Wispr
Flow introduced a polished meeting-notetaker workflow. Maintaining a second complete application while also
integrating those systems would duplicate effort. Standalone feature development is therefore paused and
this repository is published as a source checkpoint for reuse and contribution.

This is not an abandonment notice. It is an explicit boundary: preserve the useful native work, document
what is proven, upstream focused pieces where they fit, and invite contributors without pretending the
remaining product and distribution work is complete.

## How it evolved

1. **Meeting Assistant:** explore light, bot-free local meeting capture and transcription.
2. **My Assistant:** make dictation the primary product and keep Meetings visibly Beta in a Rust/Tauri shell.
3. **Native meeting foundation:** separate microphone and meeting audio, add call detection, recovery state,
   Zoom Accessibility evidence, corrections, local notes, and private/self-hosted speech compute.
4. **Rowboat direction:** use Rowboat's Markdown knowledge graph, search, providers, and agent interface rather
   than recreating that layer in a second app.
5. **Wispr Flow integration:** let Wispr own its polished live/final meeting experience when selected, then
   import finalized meetings through its supported OAuth/MCP boundary into Rowboat knowledge.

The repository slug retains the historical `Meeting-Assitant` spelling so existing clones and links do not
silently break. The product name in source and documentation is **My Assistant**.

## What exists today

| Area | Source state | Honest limitation |
|---|---|---|
| Dictation | Native shortcut, microphone capture, local STT, target-safe insertion, optional guarded polish, model lifecycle | Apple Silicon/macOS only; full exact-DMG physical app/permission/editor matrix is incomplete |
| Meetings Beta | Manual/automatic capture, separate mic/system channels, transcript, corrections, notes/Ask, local library, native Nub | The broad Zoom/Meet/Teams/FaceTime/Slack and device/recovery matrix is incomplete |
| Zoom names | Timestamped, bounded Accessibility evidence with fail-closed attribution | Native Zoom only; layout/version/overlap/identical-name cases remain Beta evidence work |
| Self-hosted speech | Authenticated private worker on local loopback, HTTPS, or an SSH-tunnelled loopback; Keychain token; strict no-implicit-fallback routing | The reference CPU profile is qualified narrowly; it is not a hosted service or universal engine adapter |
| Packaging | Reproducible native-resource, signing, DMG, mounted-artifact, and rollback-safe install scripts | Independent self-signed beta; no Apple Developer ID notarization or public binary release |
| Windows | Platform-neutral policy contracts and a written handoff | No Windows build or physical validation has been performed on this Mac |
| Agents | Dormant research source behind an explicit build feature | Production intentionally exposes only a disabled **Coming soon** label |

The exact source-build surface is governed by [`PRODUCT_RELEASE_MATRIX.md`](../PRODUCT_RELEASE_MATRIX.md).
Its `shipped` label means implemented in the production-channel build, not generally available or fully
qualified. [`RELEASE_READINESS.md`](../RELEASE_READINESS.md) defines the higher distribution gate.

## Why progress stalled

The blocking issues are integration and evidence, not a missing demo:

- `local_server.rs`, `lib.rs`, and the WebView remain large ownership hotspots that make parallel work risky;
- reliable speaker identity requires evidence merging, diarization, echo handling, corrections, and physical
  platform/version testing—not merely an ASR model;
- macOS permissions, signing, system audio, display Spaces, device changes, and meeting end behavior require
  repeated exact-artifact physical tests;
- a credible public Mac distribution needs Developer ID notarization, an SBOM/license bundle, diagnostics,
  clean-install evidence, and rollback;
- Windows needs native WASAPI/UI Automation/installer owners and cannot be proven from a Mac; and
- Rowboat already solves much of the downstream knowledge and agent experience that this project had planned.

Adding more surface area before resolving those gates would make the repository look more complete while
making the product less trustworthy.

## What is proven and what is not

Automated tests cover the release registry, meeting lifecycle, speaker-evidence rules, corrections, model
catalog, private-provider routing, security boundaries, and many native/domain invariants. Local dogfood has
also exercised dictation and two-account Zoom paths during development.

Those facts do **not** prove:

- all physical shortcut, microphone, target-app, permission, Space, and multi-display cases;
- reliable names across Zoom layouts or other meeting platforms;
- clean near/far separation during arbitrary speaker playback and simultaneous speech;
- every device switch, sleep/wake, capture crash, network loss, and meeting-end case;
- clean installation on an unrelated Mac; or
- Windows/Linux support.

Use the evidence levels in [`RESEARCH_AND_ADOPTION_METHOD.md`](RESEARCH_AND_ADOPTION_METHOD.md) and the
physical protocol in [`ACCEPTANCE_TESTS.md`](ACCEPTANCE_TESTS.md). Do not promote a claim because a unit test,
self-test, prerecorded WAV, or screenshot passed.

## Work continuing outside the standalone app

- [Rowboat PR #833](https://github.com/rowboatlabs/rowboat/pull/833) proposes optional, bounded macOS Zoom
  Accessibility evidence for speaker names and meeting-end hints. It is deliberately Zoom-only and remains
  draft until its packaged physical matrix is repeatable.
- [Rowboat PR #834](https://github.com/rowboatlabs/rowboat/pull/834) imports finalized Wispr Flow meetings
  through the supported OAuth/MCP boundary into Rowboat's Markdown knowledge store. It does not use desktop
  credentials, private endpoints, or proprietary code.
- [Rowboat PR #835](https://github.com/rowboatlabs/rowboat/pull/835) proposes a provider-neutral self-hosted
  meeting-transcription seam. It preserves current managed/Deepgram behavior when the override is absent,
  accepts local loopback or authenticated TLS services, keeps credentials in Electron main, and specifies a
  bounded versioned protocol instead of hard-coding a hosting company or one model. It remains draft until
  the documented real-worker meeting matrix is complete.

These upstream contributions are intentionally independent. A maintainer can accept, reject, or revise one
without inheriting this application's complete architecture.

## Contribution map

Good bounded contributions include:

- extract domain/router modules from the large Rust and WebView files without behavior changes;
- add privacy-safe diagnostics and canonical record migrations;
- expand deterministic Zoom evidence fixtures without asserting physical coverage;
- run and document one physical acceptance slice against an exact commit/artifact;
- qualify FlexAudio, LocalVQE/AEC3, or diarization behind an existing internal seam;
- improve model/license/SBOM tooling without committing model weights;
- execute the Windows handoff on a real Windows host; and
- improve accessibility, error recovery, installation, and contributor documentation.

Please open an issue before new product surfaces, permissions, network services, packaged executables, or
large dependencies. Each contribution should name its source/provenance, privacy effect, evidence level,
unsupported cases, and rollback.

## Repository hygiene

The public repository intentionally excludes models, compiled native helpers, app bundles, DMGs, local
settings, recordings, transcripts, acceptance sessions, benchmark corpora, credentials, and extracted
third-party packages. Generated resources are rebuilt from pinned source through the documented scripts.

Security issues and private user data do not belong in public issues. Follow [`SECURITY.md`](../SECURITY.md)
and [`SUPPORT.md`](../SUPPORT.md).
