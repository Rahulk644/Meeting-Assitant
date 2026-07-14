# My Assistant beta release audit — 14 July 2026

## Release decision

The Apple-silicon beta is suitable for a small, explicitly informed tester group. Dictation is the primary
ready feature; Meetings remains labelled **Beta**. The artifact is self-signed with the stable local
`My Assistant Dev` identity and hardened runtime, but it is not Apple-notarized. On current macOS, recipients
must attempt launch once and then use **Privacy & Security → Open Anyway** after dragging it to Applications.

This was an engineering release audit, not a formal third-party penetration test or a promise of zero
defects. No real private meeting was recorded during verification.

## Sendable artifact

| Item | Final value |
|---|---|
| DMG | `desktop/src-tauri/target/release/bundle/dmg/My_Assistant_0.1.0-beta.1_arm64.dmg` |
| SHA-256 | `b1bcb51a3a126f9d7463f575db82f7fca5045d71af116197d2190dcc7f27709a` |
| DMG size | 14,560,334 bytes (13.9 MiB) |
| Installed app | 28,200 KiB (27.5 MiB) |
| Target | Apple Silicon, macOS 14.4+ |
| Signing | self-signed, hardened runtime, no Apple Team ID or notarization |

The DMG was mounted read-only and verified to contain `My Assistant.app`, an Applications shortcut, and the
beta install guide. Deep app signing and all nested helper signatures pass `codesign --verify --strict`.

## Runtime footprint

| Final settled background process | Physical footprint |
|---|---:|
| Rust/Tauri app | 33.0 MiB |
| `meeting-tap` | 4.9 MiB |
| `meeting-state` | 2.2 MiB |
| **Aggregate** | **40.1 MiB** |

The final hidden-window pass also reported 109,072 KiB (106.5 MiB) aggregate RSS. RSS includes shared and
mapped WebKit pages, so macOS `vmmap` physical footprint is the more useful memory-pressure measure. A cold
visible-window pass can briefly be higher while WebKit initializes. `transcribe-server`, Whisper, and
LiteRT-LM were not resident at idle. Whisper unload remains user-configurable and defaults to 10 minutes.

The installed app is roughly 80% smaller than the earlier 139 MB bundle because the release no longer ships
or runs Node. The only WebView is the main application UI. The dictation status surface is a lazy native
AppKit panel, not a hidden WebView.

## Reliability and UX closure

- Right Command is the default system-wide start/stop trigger and remains rebindable.
- The native pill now states **Listening**, shows elapsed time, names the configured finish shortcut, exposes
  **Esc cancels**, and changes immediately to **Polishing your words…** after stop.
- Generation ownership prevents a slow older dictation from hiding or overwriting a newer one. A rapid
  second press moves out of Listening immediately.
- Each launch receives a unique authenticated UI URL. The previously failing WKWebView cache was deliberately
  restored, and the final build still loaded Dictation, Meetings, and Settings correctly.
- The packaged release contract is parsed and validated before the server, helpers, or WebView start. Missing
  or corrupt core capabilities fail startup instead of producing an empty sidebar.
- Port 8790 conflicts fail closed before a window or helper is created.
- App-started helpers watch the parent process and terminate after abnormal parent loss; normal quit also
  stops and reaps them.
- Meeting transcripts are written atomically with mode 0600. The first line is saved immediately, later lines
  are checkpointed at most two seconds apart, and interrupted meetings are finalized on relaunch.
- Native meeting notifications fire only on an authoritative Core Audio false-to-true call edge. An idle
  open Zoom/Teams process does not qualify.
- Existing valid speech models are adopted into private Application Support so replacing a build does not
  force another 845 MB download.
- Automatic speech language now reaches the decoder unchanged rather than being silently forced to English.
  Translation fails closed in production because the shipped Turbo model is transcription-only.
- The Qwen research runtime uses a bounded 512-token budget with repeated-cycle termination; this is
  benchmark infrastructure, not a silent change to the shipped Whisper default.

## Security and privacy posture

- Every loopback UI/API route requires a random per-launch secret exchanged for an HttpOnly, SameSite cookie;
  unauthenticated requests returned 401 and a forged Host returned 403.
- Provider keys use macOS Keychain and are excluded from JSON, WebView state, logs, temporary files, process
  arguments, and child-process configuration files.
- Custom LLM endpoints require HTTPS or loopback HTTP. Meeting audio is never sent to a selected cloud LLM;
  only transcript text and user notes are sent when the user selects that provider.
- Speech-model installation requires the pinned byte size and SHA-256 before atomic rename and use.
- Saved user data and temporary audio use private directories/files. Recording paths and HTTP byte ranges are
  bounded and validated.
- The shipped bundle contains minified UI assets, native binaries, licences, and notices—no source server,
  Node runtime, model, debug symbols, or obsolete overlay page.

Self-signing is integrity metadata, not DRM. A determined person can inspect or patch a desktop binary and
re-sign their own copy. The project deters casual copying with optimized, symbol-stripped binaries and a
small release surface; it does not claim to make client code impossible to reverse engineer.

## Install and first-use experience

1. Open the DMG and drag **My Assistant** to **Applications**.
2. Try to open it once. If the unverified-app warning appears, click **Done**, not Move to Trash.
3. Open **System Settings → Privacy & Security**, click **Open Anyway**, authenticate, and confirm **Open**.
4. Download the on-device speech model when prompted (about 845 MB).
5. Grant Microphone for dictation; grant System Audio for remote meeting participants; grant Accessibility
   for paste and best-effort speaker names; grant Screen Recording only if video is enabled.

Cloud speech transcription is not available in this beta. Dedicated meeting apps auto-detect from active
per-app audio. Browser and communication-app calls require active audio plus meeting-window evidence;
manual recording works with any platform.

## Summarization truth

Meeting summaries, actions, and Ask use the provider selected in Settings. The local option expects Gemma 4
E2B through LiteRT-LM when installed. Groq, OpenAI, Cerebras, and a custom OpenAI-compatible HTTPS/loopback
provider are supported for meeting intelligence. These are LLM providers, not cloud speech providers.

## Verification completed

- Rust tests: **19 passed**.
- Node product/release tests: **25 passed**.
- Speech benchmark unit tests: **8 passed**; catalog/result JSON, Python syntax, shell syntax, and diff hygiene passed.
- Both Swift helpers compiled and passed self-tests; transcribe-server rebuilt from its pinned checkout and
  passed its protocol self-test.
- `npm audit --omit=dev` and full `npm audit`: **0 vulnerabilities**.
- Full optimized release build and stable self-signing: passed.
- DMG read-only mount/layout, app signature, nested signatures, production capability payload, and bundle
  inventory: passed. Production contains Dictation and Meetings, blocks Translation, and contains no model,
  benchmark, transcript manifest, or source-weight artifacts.
- Deliberately restored stale WebKit cache: final per-launch URL loaded all core navigation.
- Unauthenticated API, forged Host, and occupied-port behavior: 401, 403, and clean fail-closed exit.
- Final packaged UI: Dictation, Meetings Beta, Settings, and Right Command visible.
- Native pill listening state: visually inspected on the desktop.
- Settled idle process/RSS inventory: captured above; no speech or local-LLM server resident.
- Mixed-script Hinglish, Roman proxy, ordinary English, Hindi-to-English translation, exact product-VAD,
  and deterministic synthetic wind/babble tracks completed. See `docs/SPEECH_MODEL_BENCHMARK.md`; these are
  candidate-engineering results, not marketing accuracy claims.

## Remaining acceptance work

1. Run a manual final-build matrix for physical Right Command: rapid double tap, Esc cancel, focus retention,
   full screen, each overlay position, and multiple displays. macOS UI automation cannot emit a modifier-only
   key, so this remains a human acceptance check.
2. Record a consented test meeting with two real participants in every advertised native, browser, and chat
   call app/version; verify remote audio, speaker attribution, live transcript, notification, stop detection,
   recovery, and summary.
3. Add bounded recovery for audio that has not yet reached a transcript checkpoint at the instant of a hard
   crash.
4. Add a user-triggered redacted diagnostic archive and support ID.
5. Generate an SBOM and automate release evidence in CI.
6. Implement the curated speech/LLM provider catalog and hardware-aware local-model recommendations described
   in `PRODUCT_STRATEGY.md`.
7. Design an offline update-signing key, rollback protection, and update channel before enabling auto-update.

Competitive findings and the clean-room product backlog are in
[`docs/COMPETITIVE_INTELLIGENCE.md`](docs/COMPETITIVE_INTELLIGENCE.md). The durable current-state handoff is
[`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md).
