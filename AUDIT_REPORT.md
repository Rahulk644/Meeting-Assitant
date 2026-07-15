# My Assistant macOS beta audit

**Audit date:** 15 July 2026

**Scope:** Apple-silicon macOS beta; verified production artifact built from current `codex/agent-actions` source

**Decision:** suitable for a small, explicitly informed beta group; not a general public release

This is the release-evidence snapshot. It is intentionally not a chronological work log and is not a
formal third-party penetration test or a promise of zero defects. Product visibility remains governed by
[`PRODUCT_RELEASE_MATRIX.md`](PRODUCT_RELEASE_MATRIX.md); the compaction-safe handoff is
[`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md).

## Sendable artifact

| Item | Verified value |
|---|---|
| DMG | `desktop/src-tauri/target/release/bundle/dmg/My_Assistant_0.1.0-beta.2_arm64.dmg` |
| SHA-256 | `ace262ed08930aca9206378ee1617f72befe72c0af2248100e0fdc35224acc5a` |
| DMG size | 14,759,965 bytes (14.1 MiB) |
| Installed app | 28,704 KiB (28.0 MiB) |
| Target | Apple Silicon, macOS 14.4+ |
| Signing | stable self-signed `My Assistant Dev`, hardened runtime, no Team ID/notarization |

This is the latest verified sendable production build. It contains the signed preview-only Agent action
helper, but the production contract disables draft preview, Apply/Undo, and the Agent shortcut; do not present
the helper's presence as evidence that those features ship. The DMG checksum and
`hdiutil verify` pass. A read-only mount contains `My Assistant.app`, an Applications
shortcut, and the beta install guide. The app and every nested helper pass strict deep signature
verification. All five executables are arm64 and depend only on operating-system libraries/frameworks.

Gatekeeper rejects this self-signed app as expected because it is not Apple-notarized. A recipient must drag
it to Applications, try to open it once, then use **System Settings → Privacy & Security → Open Anyway**.
Without a paid Developer ID, that is the minimum honest distribution friction; self-signing cannot remove it.

## Product and UX closure

- Production exposes Dictation, Meetings Beta, verified Settings, and an explicitly read-only Agents
  foundation. Writing Assistant, autonomous actions, wake phrase, My Assistant accounts, billing, sharing, calendar sync,
  updater, and other unfinished surfaces remain preview/hidden.
- Agent context runs only after an explicit click, redacts secure fields, uses bounded request-local element
  IDs, takes no screenshot, sends nothing to a provider, persists nothing, and exits after one snapshot.
- Production cleanly configures Local, API-key, and ChatGPT account/Codex compute without pretending that
  configuration is an action runtime. BYOK uses Keychain and provider model discovery. ChatGPT detects and
  verifies an official external Codex binary, isolates its profile, exposes account model/effort controls,
  and adds nothing to the DMG. Ordinary launch and the Local/API lanes do not touch Codex; status starts only
  after the user opens Agents with ChatGPT · Codex selected. Draft planning, the global shortcut, and the
  mutation endpoint remain preview-only. Preview Apply is restricted to the exact captured process of a
  named disposable TextEdit document; every other app remains read-only. Send does not exist.
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
| Rust/Tauri app | 31 MiB | 27,792 KiB |
| `meeting-tap` | 4,802 KiB | 11,088 KiB |
| `meeting-state` | 2,129 KiB | 5,856 KiB |
| **Aggregate** | **about 38 MiB** | **44,736 KiB (43.7 MiB)** |

RSS includes shared/mapped pages; macOS physical footprint is the better memory-pressure indicator. The
speech engine and optional local LLM remain on demand. Both Agent helpers were absent from the idle process
list and run only for explicit Agent operations. The cleaned packaged launch defaulted to Local compute and
created no My Assistant-owned Codex process; unrelated ChatGPT-owned App Server processes were excluded from
the measurement. Speech residency is user-configurable and defaults to ten idle minutes. The installed app contains no Node runtime and no
permanently hidden pill WebView.

## Verification evidence

The final artifact was built through `./script/build_release_dmg.sh`, which now enforces:

- **33/33** Node product/release/parity tests;
- **41/41** Rust tests, `cargo fmt --check`, `cargo check --locked`, and warnings-denied Clippy;
- **17/17** speech/corpus/matrix tests and **3/3** acceptance-runner unit tests;
- shell/JSON/Python validation plus `meeting-tap`, `meeting-state`, `agent-context`, `agent-action`, and `transcribe-server` self-tests;
- full desktop npm audit: **0 vulnerabilities**;
- optimized Tauri build, stable helper/app signing, DMG signing/layout/checksum verification.

The optimized production app passes strict deep signature verification, embeds both signed arm64 Agent
helpers behind disabled production capabilities, and carries the production release contract. The mounted
candidate (arm64 UUID `CD4A10B1-EC33-3702-B2FC-D86A40D82328`, main-binary SHA-256
`1e42a14b0b0e6e2baa27b311a4554b2b6221d6a7bdd20b96a5ede5463e081dfa`) launched from the exact DMG,
defaulted clean Agent compute to Local, and opened an empty API-key lane without a Keychain dialog. AGT-023 passed with private, hash-pinned
evidence: three proposals and one model-list request produced no repeated Keychain prompt in one process, and
the unchanged installed candidate reused the credential after relaunch with zero prompts. The physical
packaged gate now passes cursor insertion, selection replacement, exact Undo, app-switch rejection, stale
text/selection rejection, and Undo-after-user-edit refusal. Physical shortcut, expiry/relaunch execution
state, secure/unsupported targets, packaged-helper residency, and instrumented rollback outcomes remain open;
those preview capabilities therefore remain disabled in production.

The macOS reopen/crash dialogs observed during this pass were traced to an older comparison binary with a
different arm64 UUID that was still registered under the same bundle identifier in `/Applications`. Its
evidence was captured, it was replaced, and the duplicate bundles were removed after the gate; the candidate
itself did not create a new crash report during graceful quit/relaunch. Release QA must pin the binary
UUID/hash rather than trusting the app name.

Additional release checks passed:

- absolute repo-built app launched and stayed running;
- unauthenticated loopback access returned **401** and a forged Host returned **403**;
- production payload includes the curated picker, conditional Translation, meeting notification, thumbnail
  capabilities, and the read-only Agent foundation; autonomous Agents and blocked Apple SpeechAnalyzer remain off;
- a direct Chrome Accessibility smoke returned a bounded structured snapshot, the signed helper is present
  in the app, and no Agent helper remains resident after inspection;
- bundle inventory contains minified UI, native binaries, licenses, and notices—no Node runtime, source,
  Codex runtime/package, model weights, audio, transcript-bearing reports, benchmark data, or debug maps;
- packaged preview UI visibly exposes all three compute lanes, a secure API-key field, provider/model/reasoning
  controls, and the detected external ChatGPT/Codex version while continuing to state that actions are off;
- all 12 revision-pinned model endpoints returned HTTP 200 with content length equal to the catalog;
- production/model-library UI and the corrected non-video thumbnail flow were visually exercised with a
  real browser; no client console errors remained.

## Security, privacy, and reverse engineering

- Every local UI/API route requires a random per-launch secret exchanged for an HttpOnly SameSite cookie.
- Provider keys use Keychain and are excluded from JSON, WebView state, logs, temporary files, arguments,
  and child configuration. Startup settings do not decrypt credentials; explicit provider use reads a key at
  most once per process and keeps the reusable copy only in volatile process memory.
- Agent compute settings are allowlisted and secret-free. The external Codex path requires a strict OpenAI
  Team-ID signature check, uses a private isolated profile, supports documented logout, and is never bundled.
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
9. Complete read-only Agent inspection/drafting across Gmail, Slack, LinkedIn, Docs, Notes, Slides, Terminal,
   secure fields, prompt injection, provider errors, permission recovery, and full-screen/displays. Separately
   pass the physical packaged TextEdit exact-process, stale-target, Apply/Undo, rollback, and global-shortcut
   matrix. Add each future app adapter only after its own synthetic mutation gate; wake phrase, Send, and
   multi-step actions remain gated.
10. Run identical Porcupine, microWakeWord, and sherpa-onnx KWS benchmarks using the committed opt-in
    protocol; do not adopt any engine before privacy, accuracy, resource, energy, suspension, integration,
    and licensing evidence passes.
11. Review and merge the stacked draft PR chain into the default branch, then create the first signed/tagged
    GitHub beta release. The current order is PR #1 cleanup/release baseline, PR #2 read-only Agent context,
    PR #3 Agent compute providers, then PR #4 isolated TextEdit actions. The repository is still private and
    has no tag or GitHub release.

Until the physical matrices close, describe Dictation as the primary beta and Meetings as Beta—never as
zero-bug, notarized, or universally verified.
