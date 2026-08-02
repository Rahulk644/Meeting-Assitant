# Competitive intelligence research snapshot

This is point-in-time research evidence, not durable project memory, a current market survey, or proof that a
capability shipped. Current product direction lives in [`PRODUCT_STRATEGY.md`](../PRODUCT_STRATEGY.md);
implementation and OSS adoption decisions live in [`IMPLEMENTATION_ROADMAP.md`](IMPLEMENTATION_ROADMAP.md)
and [`OSS_COMPONENT_STRATEGY.md`](OSS_COMPONENT_STRATEGY.md).

**Observed:** 14 July 2026; local package refresh 26 July 2026; meeting/API refresh 2 August 2026

**Scope:** Wispr Flow, Handy, FluidVoice, Fathom, Granola, Fireflies, Meetily, Snaply, and Apple SpeechAnalyzer

**Authority:** This document informs product decisions. `PRODUCT_RELEASE_MATRIX.md` remains the authority for what is actually shipped.

## Executive decision

My Assistant should not become a clone of any one competitor. The winning combination is:

- **Wispr Flow's dictation interaction quality:** a transient status surface, contextual formatting, recoverable history, and voice-driven text transforms;
- **Handy's local speech engine discipline:** one native process, on-demand model loading, explicit model lifecycle controls, and a simple system-wide shortcut;
- **FluidVoice's language-first engine onboarding:** recommend one engine for the user's language, keep system-owned models separate from downloads, and expose alternatives without making every user understand model internals;
- **Fathom's meeting journey:** calendar awareness, per-meeting capture choices, clear detection notifications, live notes plus scratchpad, automatic post-call processing, and consent-aware recording;
- **Granola's meeting-output platform:** bot-free capture, human-authored notes, calendar context, approved writes to business systems, webhooks, and controlled MCP connections;
- **Fireflies' coverage and live-assist model:** bot and no-bot modes, broad call-app detection, a useful live pane, and search/Ask across meeting history;
- **Meetily's local resilience:** incremental transcript persistence, audio checkpoints, recovery after a crash, background model downloads, and local summary options.

The architecture should remain Rust/Tauri with narrowly scoped native macOS helpers. Copying the competitors' Electron process model would give up one of the clearest ways for My Assistant to win.

## How this research was performed

This was a clean-room product study using:

1. official product documentation and public release notes;
2. public open-source repositories for Handy, FluidVoice, and Meetily;
3. installed app metadata, code-signing metadata, process inventory, memory readings, and accessibility-exposed UI;
4. reversible black-box use of the installed apps.

The original 14 July study did not decompile Wispr Flow, Fathom, or Snaply. On 26 July, the installed
Fathom, Granola, and Fireflies packages and
separately extracted copies of their compiled first-party JavaScript were inspected to validate process
boundaries, permission use, event contracts, and connector boundaries. No proprietary code, assets, native
libraries, or extracted files enter this repository, and no implementation may be copied from them. They
remain clean-room behavioural and architecture references. No account credentials, local databases, user
documents, tokens, or network traffic were inspected. On 2 August, the installed Wispr Flow 1.6.326 package
was inspected through a temporary code-only extraction to identify its supported public API boundary and
meeting event schemas. The extraction remains outside this repository; private application credentials and
session tokens were neither recorded nor used.

The local observations below are point-in-time measurements on this Mac, not a universal benchmark. Use
`script/benchmark_app_footprint.sh` for repeatable bundle and live-process snapshots; it is read-only and
does not launch an app.

## Local package refresh — 26 July 2026

The current installed bundles reinforce the existing Tauri/native direction:

| App and observed version | Installed size | Package shape | Implication |
|---|---:|---|---|
| My Assistant 0.1.0 | 40 MB | Tauri app plus five bounded native executables; models stored separately | Preserve this footprint advantage and keep disabled features at zero idle cost |
| Fathom 3.5.0 | 329 MB | 255 MB Electron frameworks, 62 MB ASAR, four small arm64 native helpers | Study the helper/event boundaries and user journey, not the shell |
| Granola 7.441.6 | 606 MB | 483 MB Electron frameworks, 56 MB ASAR, native audio/calendar/TCC helpers, and multiple utility processes | Study the audio and connector contracts; reject the runtime footprint and cloud-cache assumptions |
| Fireflies 0.1.38 | 352 MB | 263 MB Electron frameworks, 67 MB ASAR, 20 MB Expo Web app, native audio monitor and keyboard/glass modules | Study no-driver capture, per-app policy, permission UX, reconnecting live transport, and Live Assist; reject the shell and cloud coupling |
| Wispr Flow 1.6.326 | about 570 MB | Electron frameworks, a large ASAR, unpacked native modules, and a Swift helper app | Study dictation/meeting interaction and evidence contracts; reject the multi-renderer baseline |

The extracted Fathom first-party package is small compared with its shell: 76 compiled JavaScript files use
about 616 KB, with about 1.3 MB of assets. This supports the earlier conclusion that most installed size is
runtime/framework overhead and that the product's durable value is its service, models, data, and workflow,
not a large desktop codebase.

The inspected Fathom package confirms these independently implementable contracts:

- a native meeting monitor publishes bounded window lifecycle and Chrome tab/window mapping events;
- a separate audio monitor publishes microphone-input process and device activity;
- screenshots and metadata can be captured during configured meeting modes and are queued to signed upload
  URLs; the package also exposes an accessibility-tree-dump option to the screenshot command;
- the Chrome extension bridge uses a Unix-domain socket and tightens its mode to owner read/write; and
- Fathom protects its own windows from capture.

Those facts prove capability, not the exact user-facing trigger, capture frequency, retention policy, or
server-side use for every account. Those must be tested through the product and verified against published
policy before making broader claims.

My Assistant should retain the narrower privacy boundary: Dictation never captures screens or walks the
Accessibility tree; meeting video remains separately permission-gated; meeting audio stays local; and a
future cloud transfer must be explicit and scoped. Fathom's event split is still a strong validation of the
planned `MeetingDetector` and `MeetingEventSource` seams.

Granola separates its desktop work into bounded audio, microphone-monitor, calendar, permission, window,
encrypted-cache, and third-party-meeting utility processes. Its local encrypted database behaves as a
rebuildable synchronized cache; that is not a safe pattern for My Assistant's authoritative local meeting
records, which must remain recoverable rather than being deleted on an open/decryption failure.

Its floating **Nub** is a separate always-on-top panel: tiny and movable while collapsed, expandable to live
transcript and a direct last-minute catch-up action, and visible across full-screen Spaces. Its audio health
contract does not treat initial remote silence as failure. Dropout recovery begins only after system audio
has first produced a signal/buffer, then restarts a lost audio process within a bounded policy. Those
behaviours validate separating source-open readiness, transcript content, and post-flow liveness.
My Assistant implements that as a lighter source recreation first, escalating repeated failures to a
supervised helper-process recycle with bounded parent-owned relaunch.

The extracted live-surface bundles make the interaction contract more specific. Granola treats the Nub,
live transcript, transcript bubbles, brief, and active-listening answer as separate components. The Nub
opens a focusable side transcript panel, groups self speech and other speech into visually distinct bubbles,
keeps **What did I miss?** beside the current transcript, and adapts the transcript surface between a compact
caption and a taller searchable history. Fireflies persists the user's compact/full live-view choice and
keeps one realtime content view while switching it between a floating click-through panel and a normal
resizable window; saved full-window bounds are clamped to the active display. Fathom's packaged shell
instead receives a server-owned live-window configuration and creates a bounded transparent `SimpleWindow`,
so its exact live renderer is not authoritative local source. My Assistant keeps one lighter native AppKit
panel because there is no WebView state to reparent, but adopts the independently implemented product
properties: quiet branded presence, adaptive vertical growth, speaker-grouped conversation, contextual
catch-up, no focus theft, and one explicit route to the full app.

The inspected native audio package contains WebRTC AEC3 identifiers and exposes separate render/system-audio
and microphone paths, device-change handling, and echo-canceller metrics. That is strong evidence for the
missing acoustic-echo-cancellation qualification task: feed the exact system-audio render stream and the
microphone capture stream through an independently integrated, open-source WebRTC AudioProcessing boundary.
The upstream implementation is BSD-licensed, but the current Rust wrapper has build and versioning caveats;
it must pass speaker/headset, delay, hot-plug, CPU, memory, and licensing tests before adoption. Do not copy
or redistribute Granola's native module.

Granola also validates a useful connector boundary. The desktop presents connections and approved actions,
while OAuth secrets and third-party execution remain server-side. Connector calls carry a stable connector
identity, connection identity, typed arguments, result, and action identity. MCP calls additionally carry
the server and tool identity. Workflows trigger from durable events such as `meeting_ended`, apply explicit
filters, and produce auditable action results. My Assistant should implement the same product properties
through its own contract: stable meeting outputs, least-privilege connections, preview and confirmation,
idempotency, receipts, and recovery. Connectors are meeting features, not a reason to expose Agents early.

Primary implementation references: [WebRTC AEC3 source](https://webrtc.googlesource.com/src/+/refs/heads/main/modules/audio_processing/aec3),
[WebRTC BSD license](https://webrtc.googlesource.com/src/+/refs/heads/main/LICENSE), and
[Rust WebRTC AudioProcessing wrapper](https://docs.rs/crate/webrtc-audio-processing/latest).

### Comparator decision

Fathom, Granola, and Fireflies now cover the useful product axes. Fathom is the lightweight
capture/consent reference; Granola is the human-note/audio/connector reference; Fireflies is the no-driver
cross-app, permission, notification, and live-assist reference. No additional comparator should delay the
Meetings Beta implementation or physical test matrix.

Krisp remains outside the baseline. Its virtual audio driver changes the machine under test and is not
needed to implement the Core Audio process-tap path. Qualify it only later in an isolated, reversible
audio-quality experiment after current latency, echo, routing, energy, and device-change baselines are
frozen.

Official references: [Granola bot-free meeting workflow](https://www.granola.ai/blog/how-to-use-ai-to-take-meeting-notes-a-step-by-step-guide-for-2026),
[Fireflies no-bot desktop capture](https://guide.fireflies.ai/articles/6666374717-how-to-record-meetings-without-a-bot-on-the-fireflies-desktop-app),
and [Krisp AI Meeting Assistant](https://krisp.ai/ai-meeting-assistant/).

## Local footprint snapshot

RSS is shown because it is available consistently across the compared process groups. It can double-count shared frameworks, so it should be treated as an operational comparison rather than exact unique physical memory.

| App and observed version | Installed size | Idle/background observation | Process shape | Product implication |
|---|---:|---:|---:|---|
| Wispr Flow 1.6.7 | 567 MB | about 386 MiB aggregate RSS with the Hub closed | 11 processes | Excellent interaction design, expensive always-on Electron baseline |
| Handy 0.9.0 | 40 MB | about 66 MiB RSS before model use | 1 process | Best architecture reference for local dictation |
| Handy 0.9.0 after loading Whisper Large v3 Turbo | model stored separately | about 912 MiB RSS immediately after dictation | 1 process | Heavy model residency must be explicit, timed, and observable |
| Fathom 3.3.3 | 351 MB | about 470 MiB aggregate RSS with its tray UI open | 8 processes | Strong meeting workflow; do not copy the Electron footprint |
| Snaply 0.1.84 | 564 MB | not runtime-tested in this pass | Electron bundle | A negative size/complexity reference, not a UX target |

The final My Assistant comparison must be re-measured from the rebuilt release using the same visible-window and background states. The performance gate should cover process count, aggregate RSS, physical footprint, CPU wakeups, cold start, first-dictation latency, model-loaded memory, and memory after the configured unload timeout.

## What each competitor teaches us

### Wispr Flow: dictation benchmark

Observed useful patterns:

- The product is split into a **Hub** for history/personalization and a small **Flow Bar** for daily operation.
- Dictations remain recoverable, including dismissed results. Search, copy, feedback, and per-entry actions are close to the transcript.
- Dictionary, snippets, per-app style, transforms, and scratchpad are separate concepts instead of one overloaded settings page.
- Shortcut configuration is centralized and checks for conflicts. It covers dictation, hands-free mode, transforms, command mode, paste/copy-last, scratchpad, and cancel.
- Context Awareness uses the active app and limited nearby text to adapt names, tone, punctuation, and formatting.
- Command Mode supports the exact writing workflow intended for My Assistant: select text, speak an instruction, and replace or answer inline.
- Privacy controls separate model-improvement consent, cloud sync, context awareness, and local transcript storage.

What not to copy:

- A 567 MB Electron bundle and a double-digit always-on process set.
- A permanently resident multi-renderer architecture for a shortcut-driven utility.
- Cloud-first assumptions in the core dictation path.

Decision for My Assistant:

- Keep the native compact dictation pill and make it strictly status-oriented: listening, elapsed time, transcribing, success, or actionable error.
- Put history, vocabulary, style, and writing actions in the main app, not in the pill.
- Add recoverable last-result behavior before adding richer animation.
- Treat nearby-text context as a separately permissioned capability with a clear on/off control and a small data boundary.

The supported Wispr Flow developer API is distinct from the signed-in desktop application's private
transport. The documented public endpoints are `wss://platform-api.wisprflow.ai/api/v1/dash/ws` for
server-held API keys, `wss://platform-api.wisprflow.ai/api/v1/dash/client_ws` for short-lived client keys,
and matching `/api` and `/client_api` REST endpoints. The documented result is formatted text plus language,
timing, and token metadata; it does not expose diarization, word timestamps, participant identities, or the
desktop meeting-refinement pipeline. Access currently requires Wispr approval and an organization API key,
so a consumer Flow subscription is not a supported credential. My Assistant may add it as an optional
`TranscriptEngine` only after supported access is granted; private desktop tokens or embedded service
credentials must never be reused.

[`script/probe_wispr_flow_api.mjs`](../script/probe_wispr_flow_api.mjs) exercises only that documented
boundary. Its credential-free mode verifies service reachability and the authorization gate; with an
explicit partner-issued organization key or client JWT, it can submit a user-selected 16-kHz WAV without
reading or printing desktop credentials. On 2 August the warm-up returned HTTP 200 while a correctly shaped
credential-free transcription request returned HTTP 401, confirming that a Flow subscription session is
not silently accepted as API authorization.

The current desktop meeting implementation also confirms that named speakers are an evidence-merging
problem, not an ASR endpoint feature. It stores separate mic/system/refined transcript sources, participant
records, timestamped speaking/mute observations, and user speaker-map corrections, then uploads those
artifacts for refinement when cloud synchronization is enabled. Calendar attendee names are candidates,
not proof of who spoke. My Assistant should preserve the same independent product properties through its
own canonical event log and `IdentityResolver`, keeping unsupported or conflicting identity as Unknown.

Official references: [Wispr Flow navigation](https://docs.wisprflow.ai/articles/5096240724-navigating-the-wispr-flow-app-desktop-ios-and-android), [Command Mode](https://docs.wisprflow.ai/articles/4816967992-how-to-use-command-mode), [Context Awareness](https://docs.wisprflow.ai/articles/4678293671-feature-context-awareness), [shortcut support](https://docs.wisprflow.ai/articles/2612050838-supported-unsupported-keyboard-hotkey-shortcuts), [API introduction](https://api-docs.wisprflow.ai/introduction), [WebSocket API](https://api-docs.wisprflow.ai/websocket_api), [REST API](https://api-docs.wisprflow.ai/rest_api), and [client-side authentication](https://api-docs.wisprflow.ai/client_side_auth_basics).

### Handy: local dictation and model operations benchmark

Observed useful patterns:

- The installed app idles as one Rust/Tauri process at roughly 66 MiB RSS before a speech model is used.
- Right Command is a first-class shortcut; push-to-talk and tap-to-toggle are explicit modes.
- Model lifecycle is user-controlled, including immediate, timed, and never-unload choices.
- Recording retention, clipboard behavior, paste methods, debug logging, history, and retry are operational settings rather than hidden implementation details.
- The current 0.9 UI identifies streaming capability, language coverage, model size, quantization, speed, and accuracy.
- Its recommended 2026 streaming choices are Parakeet Unified for English and Nemotron 3.5 for multilingual speech.
- It offers minimal and live overlay styles, but keeps the overlay separate from the main settings window.

What not to copy:

- The model page exposes dozens of choices. That is useful for an enthusiast tool but creates decision fatigue in a product for general users.
- API keys are redacted from debug output but remain part of the serialized settings object. My Assistant's Keychain boundary is stronger and should remain.
- The pink visual system and dense settings layout are Handy's identity, not ours.

Decision for My Assistant:

- Use a **curated catalog** with Whisper Large v3 Turbo as the recommended default and no more than four
  highlighted user-facing jobs.
- Keep exact engine, quantization, and custom model controls under Advanced.
- Preserve the user's 10-minute default unload setting, add an immediate **Unload now** action, and show whether a model is currently resident.
- Save the captured audio before transcription begins so a failed model run can be retried with another provider without asking the user to repeat the dictation.

Open-source reference: [Handy repository](https://github.com/cjpais/Handy).

### Apple SpeechAnalyzer and FluidVoice: highest-priority system-engine research

Apple's new `SpeechAnalyzer` / `SpeechTranscriber` path is a materially different option from a bundled
model. On macOS 26+, the OS owns the per-locale assets, downloads and updates them, and runs inference
outside the app's memory space. Apple explicitly positions the model for live and recorded long-form,
distant, meeting, and conversational audio, with volatile and finalized results plus sample-accurate time
ranges. That makes it unusually attractive for app size, memory pressure, live transcript UX, and
word/audio alignment.

A public third-party LibriSpeech run measured SpeechAnalyzer at 2.12% WER on `test-clean` and 4.56% on
`test-other`, versus 3.74% / 7.95% for Whisper Small. This is useful evidence, not a universal victory:
it used English read speech on one M2 Pro, did not test Whisper Large v3 Turbo, Parakeet, code-switching,
accents, meetings, or diarization, and normalized punctuation out of the score. My Assistant must reproduce
the comparison on its own dictation and two-channel meeting fixtures before changing Automatic routing.

A local capability probe on this development Mac (macOS/SDK 26.5) confirmed that `SpeechTranscriber` is
available using the installed Command Line Tools and currently reports 30 supported locales. Those include
`en-IN` but not Hindi. This proves that an adapter can be prototyped now; it also proves that the Apple path
cannot replace Whisper's worldwide and Hindi/code-switching role.

Handy discussion #1031 now contains an MIT-licensed draft Rust/Swift bridge, inspected on the contributor's
`feat/apple-speech` branch at commit `fe9db08530795e8d35f2f854589702d302828d80`. Its contributor reported very
fast startup and inference on one short recording, and the Handy maintainer invited a PR. The draft also
documents practical integration details: Xcode 26 SDK detection with an older-SDK stub, OS-managed locale
assets, mandatory analyzer finalization, and a tested sentence-capitalization pass. This is an implementation
lead, not reviewed upstream code yet.

FluidVoice already exposes Apple SpeechAnalyzer beside Parakeet, Nemotron, Cohere, and Whisper through one
provider contract. Its strongest product lesson is the language-first onboarding flow: show a recommended
engine, a built-in zero-download alternative where available, and tuck the full catalog behind an explicit
choice. Its current meeting surface is file transcription, not Fathom-style automatic meeting capture, so
it does not replace My Assistant's meeting architecture.

Licensing boundary: FluidVoice's current source is GPLv3 and must not be copied into this MIT project.
Behavioural lessons and Apple API documentation are safe research inputs. If implementation code is reused,
prefer the MIT Handy bridge after review, retain attribution, and record the exact commit and modifications.

Decision for My Assistant:

- Add **Apple Speech — System** as the first new local adapter, available only when runtime and locale checks
  pass on macOS 26+; preserve Whisper as the transparent fallback.
- Put it under a **System model** label rather than pretending it is a zero-byte app-managed download.
- Treat volatile results as preview text and persist only finalized segments; always finish or cancel the
  analyzer explicitly.
- Benchmark recognition, punctuation/casing, first-result latency, final latency, energy, mixed Hindi-English,
  accents, far-field speech, long dictation, and real meeting audio before making it Automatic.
- Keep Parakeet as the likely English streaming/accuracy specialist and Whisper as the worldwide fallback.

Primary references: [Apple WWDC SpeechAnalyzer session](https://developer.apple.com/videos/play/wwdc2025/277/),
[public LibriSpeech benchmark and raw outputs](https://get-inscribe.com/blog/apple-speech-api-benchmark.html),
[Handy discussion #1031](https://github.com/cjpais/Handy/discussions/1031), and
[FluidVoice repository](https://github.com/altic-dev/FluidVoice) at inspected commit
`dc30a0484ef60af292edf9081817c240360da5bb`.

### Fathom: meeting workflow benchmark

The installed 3.3.3 app exposes the product model the user described:

- automatic capture rules for all, internal, external, or no scheduled meetings;
- a default capture mode and a per-meeting override;
- video-first capture with an audio fallback;
- separate **Join Meeting** and **Capture Meeting** notifications;
- the next meeting in the toolbar;
- screen-share invisibility and automatic mute detection;
- a consent chat message;
- live summary, scratchpad, transcript, and automatic post-call processing;
- a reserved real-time coaching surface.

Its process inventory also shows a useful separation of concerns: the Electron UI is backed by small native audio and meeting-monitor helpers. The meeting monitor explicitly starts with Zoom, Google Meet, Microsoft Teams, and Slack Huddle adapters. That validates an adapter architecture, but not any specific internal implementation.

Fathom's native meeting helper does contain local participant and active-speaker observation plumbing. It
emits `participants-changed` and `active-speaker-changed`, maintains a participant monitor over bounded
Accessibility queries, and includes platform-specific speaking semantics for Zoom, Google Meet, and Slack
Huddles. Its native bridge also carries meeting context plus audio/video frames across the Chrome-extension
boundary, while Fathom's Name Picker handles the user's own Zoom display identity. No local acoustic
diarizer or final cluster-to-person transcript resolver was found in the desktop package. The useful
conclusion is architectural: local UI events provide timestamped identity evidence, but the lightweight
capture client alone is not the complete source of Fathom's named transcript intelligence.

Decision for My Assistant:

- Model the pre-call choice as **Off / Transcript only / Audio / Video**. Bot capture is a later hosted mode, not a hidden fallback.
- Only show a meeting notification when an actual call edge is detected. A calendar event or an open Zoom process is insufficient.
- Keep a six-hour upcoming-meeting menu-bar horizon by default, configurable by the user.
- During capture, show live transcript and scratchpad as separate tabs. Scratchpad content is user-authored and must never be rewritten silently.
- End detection must finalize the transcript once, persist it, and queue notes automatically with visible progress and retry.
- Make consent a product feature. Recording should never become covert merely because capture is local or bot-free.

Official references: [Fathom Desktop App](https://help.fathom.video/en/articles/449088), [new bot-free experience](https://help.fathom.video/en/articles/11577345), [Name Picker](https://help.fathom.video/en/articles/297152), [Slack Huddles](https://help.fathom.video/en/articles/11870465), and [recording consent](https://help.fathom.video/en/articles/6150977).

### Granola: human notes, audio quality, and connector benchmark

The installed 7.441.6 package is the strongest reference for what happens after capture:

- user notes remain a first-class input instead of being overwritten by a generated summary;
- capture, microphone monitoring, calendar, permission status, window coordination, encrypted cache, and
  third-party meeting automation are separated behind bounded process messages;
- the native audio boundary combines microphone and system/render audio, responds to device changes, and
  contains WebRTC AEC3 echo-canceller identifiers and metrics;
- connectors cover messaging, notes, CRM, email/calendar, webhooks, and custom MCP servers;
- connector execution is normalized around connector, connection, action/tool, arguments, result, and
  destination identities; and
- organization policy can restrict connectors and MCP server/tool access.
- transcript words retain provider speaker clusters, while a separate detected-speaker identity can replace
  the generic cluster label;
- active-speaker changes are retained as a bounded timestamped history, filtered against the local
  participant, adjusted for observation lag, and resolved by the dominant evidence over the transcript
  chunk's actual interval—not whichever tile is visible after transcription finishes.

Decision for My Assistant:

- Treat the corrected meeting record plus scratchpad as the source. Generated notes never silently replace
  user-authored notes.
- Qualify WebRTC AEC3 through our own `AudioSource` seam to solve speaker-loopback duplication. Headsets
  should bypass work that cannot improve their path.
- Keep OAuth secrets, provider SDKs, retries, and write execution out of the desktop process. The desktop
  previews, asks for approval, and shows a durable receipt.
- Keep the same evidence categories but implement the merger in the shared Rust core. Caption timing,
  trusted platform evidence, self filtering, uncertainty, and corrections are product contracts rather
  than code taken from a reference app.
- Start with follow-up email/Slack drafts and signed webhooks, then Notion and CRM writes. Broad connector
  count matters less than reliable completion, idempotency, and recovery.
- Put MCP behind the same allowlisted action contract. It is not an authorization bypass or an excuse to
  launch Agents.

What not to copy:

- a 606 MB Electron bundle, large universal framework payload, or a collection of resident utility
  processes;
- deleting an authoritative local database after an open/decryption failure;
- treating a cloud-synchronized cache as the source of truth for local-first meetings; or
- desktop-held provider refresh tokens and provider-specific data models leaking into the meeting core.

The resulting independent contract is documented in
[`CONNECTOR_ARCHITECTURE.md`](CONNECTOR_ARCHITECTURE.md).

### Fireflies: coverage, mode clarity, and meeting intelligence

The installed 0.1.38 package was inspected through bundle metadata, first-party compiled modules, shipped
source maps, accessibility-exposed UI, and a read-only product walkthrough. No meeting content, credentials,
private databases, tokens, or network traffic were inspected, and no Fireflies code or assets enter this
repository.

Observed useful patterns:

- Detection offers two explicit actions: invite the bot or capture system audio without a bot.
- No-bot recording supports many meeting, calling, browser, and phone-call surfaces.
- On modern macOS, no-bot capture uses Chromium's Core Audio tap system-loopback path plus ordinary
  microphone capture. It does not require a virtual audio driver.
- A small native monitor emits process audio/microphone start-stop evidence. Browser meeting URLs are looked
  up only for the audio-active browser; current macOS builds use bounded Apple Events tab queries rather than
  requiring an extension. URL shapes are validated and polling-mode URLs must remain stable before use.
- The real-time client has one active meeting/sender owner, sends audio chunks and microphone activity,
  emits transcripts, measures bytes/chunks/drops, and distinguishes terminal client failures from transient
  reconnects with bounded exponential backoff.
- Settings separate automatic capture, detection notifications, per-app exclusions, notification
  persistence, and consent-message automation. The permission page names only Microphone and System Audio
  for ordinary no-bot capture; Screen Recording is not presented as a default requirement.
- The compact live surface keeps transcript, notes, and Intelligence close to pause/stop. Its four useful
  actions are last-minute catch-up, summary-so-far, explicit action items, and follow-up questions.
- The desktop uses separate compact always-on-top and full normal windows around one reusable live content
  view. That validates a presentation split, though its observed compact default is substantially larger
  than a Nub and should not set My Assistant's footprint.
- The desktop app also includes a diagnostic export, update status, meeting-prep notification, and screen-share hiding.
- AskFred works both within one meeting and across meeting history.
- Bot mode and no-bot mode have honest capability differences: named speakers, sales assist, and saved media are not promised where the system-audio mode cannot provide them.
- Compiled transcript helpers keep a provider `speaker_id` separate from `speakerMeta`; when an identity is
  not available the UI falls back to `Speaker N`, and a later label correction updates that mapping. This
  reinforces that diarization clusters and participant identity are separate facts.

Decision for My Assistant:

- Keep the existing native Core Audio process tap. Add stable call-edge hysteresis and separate desktop-app
  versus browser/huddle enablement; do not install a virtual driver or adopt Electron.
- Keep browser capture extension-optional. The local app must work first with active-audio plus bounded
  meeting-window evidence. A later Chrome extension can improve exact tab/meeting metadata through a narrow,
  authenticated event adapter without owning capture or the canonical meeting record.
- Live Assist is a meeting feature, not Agent mode. Every action is grounded in the current non-excluded
  transcript; last-minute catch-up uses an actual 60-second window; it never sends or writes anything.
- Show capability differences before capture begins. Never imply that diarization provides participant names when the app only knows Speaker 1 and Speaker 2.
- Add a one-click diagnostic bundle with redaction before public beta support expands.
- Make local full-text search the first cross-meeting feature; add cited Ask across meetings only after the local index and source-link contract are reliable.
- Do not promise every platform at launch. Maintain an adapter support matrix with detected, captured, speaker-aware, video-capable, and verified columns.

Official references: [bot-free desktop recording](https://guide.fireflies.ai/articles/6666374717-how-to-record-meetings-without-a-bot-on-the-fireflies-desktop-app), [desktop app guide](https://guide.fireflies.ai/articles/1208704416-getting-started-with-the-fireflies-desktop-app), and [Global AskFred](https://guide.fireflies.ai/articles/1512776728-learn-about-global-askfred).

### Meetily: local recovery reference, not a codebase template

Useful open-source patterns found at commit `0281737d87d26352fb0adc78c8c0975f691b23d1`:

- transcript segments are written incrementally to IndexedDB during capture;
- audio is written in 30-second checkpoints and can be reconstructed after interruption;
- interrupted meetings remain recoverable for seven days;
- onboarding supports background model downloads, cancellation, and hardware-aware recommendations;
- updates use Tauri's signed updater flow;
- local Whisper/Parakeet transcription, SQLite meeting storage, local summaries, and OpenAI-compatible providers are already separated conceptually.

Patterns not to copy:

- The stop path polls every 500 ms for up to 60 seconds and adds fixed waits. My Assistant should use events plus a bounded finalization timeout.
- Provider keys are stored directly in SQLite columns. My Assistant must keep secrets in macOS Keychain.
- The repository contains old/duplicate implementation files and large tooling artifacts. It is a feature reference, not the housekeeping standard.

Open-source reference: [Meetily repository](https://github.com/Zackriya-Solutions/meetily).

## Curated speech catalog decision

The shipped picker exposes one understandable model library with job-oriented labels, exact size, abilities,
and model-specific settings. It does not auto-rank untested hardware or claim universal quality:

| User-facing job | Initial engine | Status before shipping |
|---|---|---|
| Worldwide balanced | Whisper Large v3 Turbo | shipped recommended default |
| Fast / accurate English | Parakeet Unified EN 0.6B | shipped user-selectable |
| Live English | Nemotron Speech Streaming EN 0.6B | shipped user-selectable |
| Live multilingual | Nemotron 3.5 ASR Streaming 0.6B | shipped user-selectable; 28-language local build |
| English speed tiers | Whisper Tiny / Base / Small / Medium English | shipped user-selectable |
| Translate to English | Whisper Medium multilingual Q5 | shipped; best current local translation benchmark |
| Maximum Whisper quality | Whisper Large v3 Q5 | shipped powerful-Mac option |
| Roman Hinglish | Oriserve Apex / Prime | shipped user-selectable; Apex is the faster recommended Hinglish option |

SpeechAnalyzer remains a future system option on supported macOS 26 machines. Whisper remains the worldwide
and older-OS safety net. No model is silently auto-selected: the user downloads and tests what works for their
voice and Mac, while the product preserves a conservative recommended default.

Primary model references: [Apple SpeechAnalyzer](https://developer.apple.com/documentation/speech/speechanalyzer), [Whisper Large v3 Turbo](https://huggingface.co/openai/whisper-large-v3-turbo), [Parakeet Unified EN](https://huggingface.co/nvidia/parakeet-unified-en-0.6b), [Nemotron 3.5 ASR](https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b), and [Parakeet TDT v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3).

## Ordered adoption backlog

Current implementation status is tracked here only where it changes the priority decision. The complete
shipped/preview truth remains in `PRODUCT_RELEASE_MATRIX.md`.

### P0 — reliability before feature expansion

1. **Implemented:** persist the first transcript line immediately and checkpoint later lines atomically at
   most two seconds apart, with a 10-second maintenance safety pass.
2. Checkpoint meeting audio on a bounded interval and recover after app or system failure.
3. **Implemented:** make every bundled helper exit when the parent app exits.
4. Finish the native dictation-pill regression suite: rapid toggle, cancel, empty speech, transcription failure, and target-app focus.
5. Add a redacted diagnostic export and stable support ID.
6. Re-measure idle, model-loaded, post-unload, and meeting-capture memory in release builds.

### P1 — product-quality meeting beta

1. Add per-meeting Off / Transcript / Audio / Video choice.
2. Replace title/process guesses with explicit platform adapters and event-driven call edges.
3. **Implemented for supported native apps:** show a notification only on the authoritative Core Audio
   real-call edge; do not notify merely because Zoom or another meeting app is open.
4. Finalize once on call end, then automatically create notes with retryable progress.
5. Add editable speaker mapping with honest confidence and generic labels when identity is unknown.
6. Add consent guidance and a recording indicator that is visible to the operator.

### P2 — curated models and provider platform

1. Add the macOS 26 SpeechAnalyzer adapter with runtime/locale detection, OS-asset progress, cancellation,
   finalized-result handling, and a transparent Whisper fallback.
2. Build a versioned benchmark corpus for short dictation, punctuation/casing, accents, code-switching,
   far-field speech, long files, and consented two-participant meeting audio.
3. **Implemented:** revision-pinned catalog, verified resumable downloads, cancellation, selection, removal,
   partial recovery, model-specific settings, and size/RAM guidance.
4. **Implemented:** Parakeet Unified EN, Nemotron Speech Streaming EN, and Nemotron 3.5 are user-selectable.
5. Implement Speech providers separately from Intelligence providers.
6. Launch Speech BYOK with Groq and OpenAI first; add a specialist such as Deepgram only after the contract is proven.
7. Launch Intelligence BYOK with OpenAI, Anthropic, Gemini, Groq, Cerebras, and custom OpenAI-compatible HTTPS/loopback endpoints.
8. Keep every key in Keychain and expose Test / Disconnect / What leaves this Mac for each provider.

### P3 — differentiation

1. Writing palette: select text, press a shortcut, type or dictate an instruction, preview a diff, apply, and undo.
2. Local search across meeting history, followed by cited cross-meeting Ask.
3. Versioned meeting templates and team-specific output contracts.
4. Live coach modes after capture reliability is isolated from coaching latency and failure.
5. Hosted sync, public links, bot capture, and team integrations only where the service cost or team value justifies monetization.

## Non-negotiable gates

- No dead navigation or placeholder action in a production build.
- No API key in JSON, SQLite, logs, shell arguments, temporary files, or WebView state.
- No model or local LLM remains resident beyond the user's configured policy.
- No transcript is lost because summary generation, diarization, export, or an update failed.
- No named-speaker claim without evidence that links a voice/channel to a participant identity.
- No silent recording design. Consent and jurisdictional responsibility must be visible.
- No cross-platform expansion until macOS dictation and meeting capture pass repeatable reliability and performance gates.
