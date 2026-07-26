# Engineering health

This document is the durable technical-health register. It records current strengths, structural debt, and
the rule for resolving each item. It is not a work log or release artifact. Exact test counts, commit hashes,
dates, and physical evidence belong in CI or release notes.

## Current assessment

The product foundation is credible and deliberately narrow. Dictation has a real native capture,
transcription, target-safety, cleanup, and insertion path. Meetings Beta has automatic and manual capture,
checkpointed transcripts, corrections, scratchpad notes, post-call notes, and a local library. The production
release contract fails closed, models are verified before loading, sensitive paths are private, and packaged
artifacts exclude Node, model weights, telemetry, Agent helpers, and Agent research Web assets. The ordinary
Rust build also excludes the dormant Agent backend.

The automated source gates are green. Native helper self-tests are green. This establishes implementation
health, not physical product quality: real shortcuts, microphones, target apps, displays, permissions,
clean-machine installation, and two-person calls still require the named physical matrices.

## What is on point

- **A user-centred product boundary:** Dictation is primary, Meetings is visibly Beta, and Agents cannot be
  promoted by saved state or an ambient environment variable.
- **A lean runtime:** Rust/Tauri owns the app and authenticated loopback API; heavyweight speech and polish
  runtimes load on demand and unload after bounded idle periods.
- **Safe dictation semantics:** one authoritative final result, exact live-span ownership, conservative
  cleanup, secure-field exclusion, and clipboard fallback when target evidence drifts.
- **Defensible privacy and integrity:** local audio/transcription, Keychain-backed cloud credentials,
  revision-pinned models, checksum verification, private storage, and signed nested executables.
- **Recoverable meeting records:** incremental private checkpoints, explicit corrections, transcript
  revisions, and stale-derived-note handling.
- **A real portability seam:** automatic call-edge policy is dependency-free Rust; the Swift helper reports
  raw macOS evidence and keeps only Apple capture handles.
- **Strong release mechanics:** one native build path, production asset generation, exact mounted-DMG
  inspection, and rollback-safe installation of the inspected candidate.

## Engineering debt and risks

| Priority | Condition | Why it matters | Resolution / exit condition |
|---|---|---|---|
| P0 | Public distribution still uses a self-signed identity | Every recipient faces Gatekeeper friction and the artifact is not Apple-notarized | Developer ID signing, notarization, stapling, and mounted-artifact verification become mandatory |
| P0 | Physical product matrices are incomplete | Unit and synthetic tests cannot prove cross-app insertion, permissions, live audio, display, sleep/wake, or real meeting behavior | Complete `docs/ACCEPTANCE_TESTS.md` against the exact mounted candidate and retain redacted evidence outside Git |
| P0 | Local meeting Intelligence is a separately installed LiteRT-LM process without a complete in-app lifecycle | Clean beta users otherwise need BYOK; executable integrity, port ownership, install, status, removal, and recovery are not first-class | Ship a verified install/status/remove/unload contract or remove the local choice from production until it exists |
| P0 | RustSec reports allowed maintenance warnings in the cross-target Tauri graph | They are not active macOS vulnerabilities, but unexplained warnings become release-policy debt and matter before Windows/Linux | Target-scope the policy, track upstream upgrades, keep vulnerability findings release-blocking, and publish an SBOM/license bundle |
| P0 | Version truth is split across Cargo/Tauri, changelog headings, and artifact labels | A filename can imply a release that has no corresponding version/tag/evidence | Define one release-version procedure; local builds remain `unreleased`, and a named artifact requires an explicit label and release record |
| P1 | `local_server.rs`, `lib.rs`, and `index.html` are large merge hotspots | Parallel sessions will collide and unrelated responsibilities are difficult to review or test independently | Extract stable domain modules and provider traits before parallel feature work; preserve routes and behavior with contract tests |
| P1 | The browser developer harness duplicates parts of the Rust backend | It is useful for fast UI work but parity can drift | Keep it explicitly developer-only, share pure domain modules/fixtures, and add route-parity tests for every UI call it claims to support |
| P1 | Meeting speaker attribution is evidence-based, not Fathom-grade diarization and identity resolution | Active-speaker Accessibility evidence cannot consistently name every audio segment, especially in gallery views and overlapping speech | Qualify one local diarizer, combine it with platform/calendar evidence, expose confidence internally, and pass a representative DER/name-attribution corpus plus real calls |
| P1 | Meetings do not capture meeting chat or recover in-flight audio after a crash | Transcript + chat + scratchpad is the required source record; transcript checkpoints cannot recreate unheard audio | Add typed chat events where supported and bounded encrypted/private audio checkpoints with cleanup and recovery tests |
| P1 | The shared detector is portable, but the crate and remaining capture/target integrations are still macOS-shaped | Unconditional macOS dependencies and native helpers still prevent a Windows build despite the policy extraction | Keep `meeting_detection.rs` OS-free; introduce `AudioSource`, `MeetingEvidenceSource`, target, overlay, and permission traits plus target-specific dependency sections before new capture features; maintain the executable [`WINDOWS_HANDOFF.md`](WINDOWS_HANDOFF.md) packet and leave Windows unverified until X-00 runs on Windows |
| P2 | The Node and Rust services repeat routing, provider, model, and persistence decisions | Duplication increases cognitive load and regression risk | Keep only the minimal browser simulator; move shared behavior into fixtures/schema-driven contracts rather than creating a second production backend |
| P2 | Source assets and historical research live beside runtime assets | It is harmless today, but increases ambiguity about what is packaged or authoritative | Keep source brand files under an explicit source-assets directory or remove them after brand ownership is confirmed; keep point-in-time research labelled as research |

## Code disposition

| Area | Decision | Reason |
|---|---|---|
| Dictation native pipeline | Keep and harden | It is the product's strongest differentiated foundation |
| `transcribe-server` wrapper | Keep until a direct library boundary proves equal or better | It is small, authenticated, engine-neutral at the API, pinned, and already qualified |
| `meeting-tap`, `meeting-state`, `dictation-target` | Keep as platform adapters; shrink their contracts over time | Their bounded OS privileges are preferable to a monolithic helper or Electron process |
| Developer Node server | Keep as a simulator, never package | It materially speeds UI iteration; parity must be explicit and tested |
| Dormant Agent implementation | Keep behind the explicit `agent-research` Cargo feature; strip from production Web assets | Default builds and production artifacts stay lean while bounded research remains reproducible |
| Preview-only placeholder surfaces | Remove when they stop serving active design review | Hidden inert controls are not a backlog; roadmap documents are the backlog |
| Ignored models, app data, recordings, benchmarks, and native build outputs | Preserve locally, never clean automatically | They may be expensive or private user material and are already outside release/source control |

## Performance contract

Quality is the first gate and efficiency is the second. An optimization that reduces transcript correctness,
target safety, diarization quality, or recovery is rejected.

Every material runtime change must compare the same states on the same machine:

1. installed size and bundled executable count;
2. cold launch to usable UI;
3. idle process count, RSS, CPU, energy impact, and wakeups;
4. time from shortcut to listening and Stop to final insertion;
5. time to first live meeting text and final transcript availability;
6. memory with each model loaded and memory released after its idle timeout;
7. disk growth per meeting hour, including checkpoints and optional video; and
8. crash/restart recovery time and orphaned-process/file count.

Until representative baselines are frozen, a change that regresses an established metric by more than ten
percent needs an explicit architecture decision and a user-visible quality benefit. New background services
must have zero idle cost when their feature is disabled.

## Documentation contract

- `PRODUCT_RELEASE_MATRIX.md` alone authorizes production visibility.
- `docs/PROJECT_STATE.md` records stable product truth and priority, not weekly progress.
- `docs/IMPLEMENTATION_ROADMAP.md` owns future work, dependencies, effort, and completion criteria.
- `docs/OSS_COMPONENT_STRATEGY.md` owns build-versus-adopt decisions and upstream intake policy.
- This file owns current structural debt and health rules.
- `ARCHITECTURE.md` and `SECURITY.md` describe implemented runtime and trust boundaries; future designs do not
  belong there until adopted.
- Competitive and benchmark documents are evidence snapshots, not release truth or durable memory.

When one of these authorities changes, update every direct contradiction in the same pull request. Do not
append a dated status paragraph; rewrite the durable truth.
