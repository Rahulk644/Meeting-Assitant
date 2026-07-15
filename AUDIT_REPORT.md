# My Assistant macOS beta audit

**Audit date:** 15 July 2026

**Scope:** Apple-silicon macOS beta, current `codex/agent-context-foundation` branch

**Decision:** suitable for a small, explicitly informed beta group; not a general public release

This is the release-evidence snapshot. It is intentionally not a chronological work log and is not a
formal third-party penetration test or a promise of zero defects. Product visibility remains governed by
[`PRODUCT_RELEASE_MATRIX.md`](PRODUCT_RELEASE_MATRIX.md); the compaction-safe handoff is
[`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md).

## Sendable artifact

| Item | Verified value |
|---|---|
| DMG | `desktop/src-tauri/target/release/bundle/dmg/My_Assistant_0.1.0-beta.1_arm64.dmg` |
| SHA-256 | `45bcb951072f1b72dee742fa45a9e0dec67a71898ad6134663e1422195c45787` |
| DMG size | 14,626,990 bytes (13.9 MiB) |
| Installed app | 28,380 KiB (27.7 MiB) |
| Target | Apple Silicon, macOS 14.4+ |
| Signing | stable self-signed `My Assistant Dev`, hardened runtime, no Team ID/notarization |

The DMG checksum and `hdiutil verify` pass. A read-only mount contains `My Assistant.app`, an Applications
shortcut, and the beta install guide. The app and every nested helper pass strict deep signature
verification. All five executables are arm64 and depend only on operating-system libraries/frameworks.

Gatekeeper rejects this self-signed app as expected because it is not Apple-notarized. A recipient must drag
it to Applications, try to open it once, then use **System Settings → Privacy & Security → Open Anyway**.
Without a paid Developer ID, that is the minimum honest distribution friction; self-signing cannot remove it.

## Product and UX closure

- Production exposes Dictation, Meetings Beta, verified Settings, and an explicitly read-only Agents
  foundation. Writing Assistant, autonomous actions, wake phrase, accounts, billing, sharing, calendar sync,
  updater, and other unfinished surfaces remain preview/hidden.
- Agent context runs only after an explicit click, redacts secure fields, uses bounded request-local element
  IDs, takes no screenshot, sends nothing to a provider, persists nothing, and exits after one snapshot.
- Right Command is the default rebindable dictation trigger. The native AppKit pill is created lazily, does
  not steal focus, states Listening/Polishing, shows elapsed time and the configured finish shortcut, and
  exposes Esc cancellation.
- Rapid second-press generation ownership prevents an older dictation from hiding or overwriting a newer one.
- Real-call detection is Core-Audio-authoritative. Browser/chat apps additionally require meeting-window
  evidence; an open call app, ordinary browser audio, or a notification cannot start capture.
- Meeting notifications occur only on an authoritative false-to-true call edge.
- Transcript JSON is private and atomic: first line immediately, later lines at most two seconds apart, with
  a ten-second maintenance checkpoint. Interrupted meeting JSON is recovered on relaunch.
- Meeting cards use a native recording frame, installed app icon, or platform identity tile. The developer
  browser harness now matches the shipped Rust fallback instead of showing broken non-video thumbnails.
- The local speech-model library exposes 12 explicit choices with pinned revision, exact size/SHA-256,
  resume/cancel/remove/select lifecycle, and model-specific language, translation, output, and preprocessing.
- Translation is conditionally shipped only for Whisper Medium multilingual and Whisper Large v3; every
  incompatible model fails closed in both UI and Rust.

## Meeting Intelligence boundary

Groq, OpenAI, Cerebras, and custom OpenAI-compatible BYOK providers are available for Summary, Actions, and
Ask. Keys live in macOS Keychain. Only transcript text and user notes are sent to the explicitly selected
cloud provider; meeting audio is not.

The optional local Gemma 4 E2B path works through LiteRT-LM when installed by the source setup, starts on
demand, and unloads when idle. The DMG does **not** yet provide an in-app Intelligence-model installer.
Therefore a clean beta Mac needs BYOK for meeting notes even though dictation and meeting transcription are
fully local. The UI/runtime fail closed when the local model is absent; documentation no longer calls this
clean-install local notes.

## Runtime footprint

Measured after a fresh absolute-path packaged launch and settle, with no Speech/LLM model resident:

| Process | Physical footprint | RSS |
|---|---:|---:|
| Rust/Tauri app | 36 MiB | 37,248 KiB |
| `meeting-tap` | 4,978 KiB | 14,496 KiB |
| `meeting-state` | 2,289 KiB | 8,048 KiB |
| **Aggregate** | **about 43.1 MiB** | **59,792 KiB (58.4 MiB)** |

RSS includes shared/mapped pages; macOS physical footprint is the better memory-pressure indicator. The
speech engine and optional local LLM remain on demand. `agent-context` was absent from the idle process list;
its signed binary adds 118,304 bytes to the app and runs only for one inspection. Speech residency is
user-configurable and defaults to ten idle minutes. The installed app contains no Node runtime and no
permanently hidden pill WebView.

## Verification evidence

The final artifact was built through `./script/build_release_dmg.sh`, which now enforces:

- **29/29** Node product/release/parity tests;
- **26/26** Rust tests, `cargo fmt --check`, `cargo check --locked`, and warnings-denied Clippy;
- **17/17** speech/corpus/matrix tests and **3/3** acceptance-runner unit tests;
- shell/JSON/Python validation plus `meeting-tap`, `meeting-state`, `agent-context`, and `transcribe-server` self-tests;
- full desktop npm audit: **0 vulnerabilities**;
- optimized Tauri build, stable helper/app signing, DMG signing/layout/checksum verification.

Additional release checks passed:

- absolute repo-built app launched and stayed running;
- unauthenticated loopback access returned **401** and a forged Host returned **403**;
- production payload includes the curated picker, conditional Translation, meeting notification, thumbnail
  capabilities, and the read-only Agent foundation; autonomous Agents and blocked Apple SpeechAnalyzer remain off;
- a direct Chrome Accessibility smoke returned a bounded structured snapshot, the signed helper is present
  in the app, and no Agent helper remains resident after inspection;
- bundle inventory contains minified UI, native binaries, licenses, and notices—no Node runtime, source,
  model weights, audio, transcript-bearing reports, benchmark data, or debug maps;
- all 12 revision-pinned model endpoints returned HTTP 200 with content length equal to the catalog;
- production/model-library UI and the corrected non-video thumbnail flow were visually exercised with a
  real browser; no client console errors remained.

## Security, privacy, and reverse engineering

- Every local UI/API route requires a random per-launch secret exchanged for an HttpOnly SameSite cookie.
- Provider keys use Keychain and are excluded from JSON, WebView state, logs, temporary files, arguments,
  and child configuration.
- Custom remote LLM endpoints require HTTPS; loopback HTTP is allowed for local runtimes.
- Model installation verifies exact byte size and SHA-256 before owner-only atomic rename/use.
- Saved meetings and transient audio use owner-only paths; recording names, byte ranges, request sizes,
  origins, hosts, and helper/STT authentication fail closed.

Self-signing is integrity metadata, not DRM. The optimized, stripped binary deters casual inspection, but a
determined person can inspect or patch any client-side desktop app and re-sign their copy. The project makes
no claim that executable code is impossible to reverse engineer.

## Open gaps and release blockers

1. Complete the physical Right-Command matrix: rapid toggle, Esc, empty/error paths, focus retention,
   full-screen apps, every overlay position, and multiple displays. Synthetic keys are not evidence.
2. Complete consented two-person tests across every advertised native/browser/chat app and supported version,
   including remote audio, notification, attribution, pause/stop, recovery, summary, and permission repair.
3. Add bounded recovery for audio/utterances not yet converted into a transcript checkpoint at hard failure.
4. Add the clean-Mac in-app local Intelligence install/status/remove lifecycle.
5. Add redacted rotating diagnostics, support ID, and a user-previewable export archive.
6. Complete real café/wind/traffic/overlap/long-form speech evidence plus per-model Mac memory, latency,
   thermal, and clean download/cancel/switch/delete measurements.
7. Generate and automate an SBOM, dependency policy, and complete third-party attribution review.
8. Design an offline updater signing key, rollback protection, and release channels before enabling updates.
9. Complete the Agent read-only acceptance matrix across Safari, Chrome, Mail, Notes, Slack, Terminal,
   Electron, secure fields, permission recovery, full-screen, and multiple displays. Then add a global
   shortcut and draft/replace/undo only; wake phrase and multi-step actions remain later gated milestones.
10. Review and merge draft PR [#1](https://github.com/Rahulk644/Meeting-Assitant/pull/1) into the default branch,
   then create the first signed/tagged GitHub beta release. The repository is still private and has no tag or
   GitHub release.

Until the physical matrices close, describe Dictation as the primary beta and Meetings as Beta—never as
zero-bug, notarized, or universally verified.
