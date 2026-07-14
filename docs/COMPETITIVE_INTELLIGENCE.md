# Competitive intelligence and product decisions

**Observed:** 14 July 2026

**Scope:** Wispr Flow, Handy, FluidVoice, Fathom, Fireflies, Meetily, Snaply, and Apple SpeechAnalyzer

**Authority:** This document informs product decisions. `PRODUCT_RELEASE_MATRIX.md` remains the authority for what is actually shipped.

## Executive decision

My Assistant should not become a clone of any one competitor. The winning combination is:

- **Wispr Flow's dictation interaction quality:** a transient status surface, contextual formatting, recoverable history, and voice-driven text transforms;
- **Handy's local speech engine discipline:** one native process, on-demand model loading, explicit model lifecycle controls, and a simple system-wide shortcut;
- **FluidVoice's language-first engine onboarding:** recommend one engine for the user's language, keep system-owned models separate from downloads, and expose alternatives without making every user understand model internals;
- **Fathom's meeting journey:** calendar awareness, per-meeting capture choices, clear detection notifications, live notes plus scratchpad, automatic post-call processing, and consent-aware recording;
- **Fireflies' coverage and live-assist model:** bot and no-bot modes, broad call-app detection, a useful live pane, and search/Ask across meeting history;
- **Meetily's local resilience:** incremental transcript persistence, audio checkpoints, recovery after a crash, background model downloads, and local summary options.

The architecture should remain Rust/Tauri with narrowly scoped native macOS helpers. Copying the competitors' Electron process model would give up one of the clearest ways for My Assistant to win.

## How this research was performed

This was a clean-room product study using:

1. official product documentation and public release notes;
2. public open-source repositories for Handy, FluidVoice, and Meetily;
3. installed app metadata, code-signing metadata, process inventory, memory readings, and accessibility-exposed UI;
4. reversible black-box use of the installed apps.

No account credentials, private databases, network traffic, or proprietary source code were extracted. Wispr Flow, Fathom, and Snaply were not decompiled. The local observations below are a point-in-time measurement on this Mac, not a universal benchmark.

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

Official references: [Wispr Flow navigation](https://docs.wisprflow.ai/articles/5096240724-navigating-the-wispr-flow-app-desktop-ios-and-android), [Command Mode](https://docs.wisprflow.ai/articles/4816967992-how-to-use-command-mode), [Context Awareness](https://docs.wisprflow.ai/articles/4678293671-feature-context-awareness), and [shortcut support](https://docs.wisprflow.ai/articles/2612050838-supported-unsupported-keyboard-hotkey-shortcuts).

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

- Use a **curated catalog** with Automatic as the default and no more than four recommended user-facing jobs.
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

Decision for My Assistant:

- Model the pre-call choice as **Off / Transcript only / Audio / Video**. Bot capture is a later hosted mode, not a hidden fallback.
- Only show a meeting notification when an actual call edge is detected. A calendar event or an open Zoom process is insufficient.
- Keep a six-hour upcoming-meeting menu-bar horizon by default, configurable by the user.
- During capture, show live transcript and scratchpad as separate tabs. Scratchpad content is user-authored and must never be rewritten silently.
- End detection must finalize the transcript once, persist it, and queue notes automatically with visible progress and retry.
- Make consent a product feature. Recording should never become covert merely because capture is local or bot-free.

Official references: [Fathom Desktop App](https://help.fathom.video/en/articles/449088), [new bot-free experience](https://help.fathom.video/en/articles/11577345), [Slack Huddles](https://help.fathom.video/en/articles/11870465), and [recording consent](https://help.fathom.video/en/articles/6150977).

### Fireflies: coverage, mode clarity, and meeting intelligence

Observed useful patterns:

- Detection offers two explicit actions: invite the bot or capture system audio without a bot.
- No-bot recording supports many meeting, calling, browser, and phone-call surfaces.
- Live Assist opens automatically with real-time transcript, notes, AskFred, AI skills, and instant summaries.
- The desktop app includes a diagnostic export, permission management, update status, meeting-prep notification, and screen-share hiding.
- AskFred works both within one meeting and across meeting history.
- Bot mode and no-bot mode have honest capability differences: named speakers, sales assist, and saved media are not promised where the system-audio mode cannot provide them.

Decision for My Assistant:

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

The simple picker should expose jobs, not a wall of model names:

| User-facing job | Initial engine | Status before shipping |
|---|---|---|
| Automatic (recommended) | chooses using OS, locale, measured quality, memory, latency, and power | hardware profiler plus benchmark required |
| System — Fast and light | Apple SpeechAnalyzer on macOS 26+ | implement availability/asset/fallback bridge; pass our dictation and meeting benchmark |
| Worldwide balanced | Whisper Large v3 Turbo | current baseline; preserve |
| Live English | Parakeet Unified EN 0.6B | integrate and benchmark on supported Macs |
| Live multilingual | Nemotron 3.5 ASR Streaming 0.6B | integrate as preview; benchmark languages and thermal cost |
| Highest accuracy | Whisper Large v3 | powerful-Mac option; require a measured quality gain |

SpeechAnalyzer becomes the first candidate on supported macOS 26 machines because it adds no model weight to
the app and runs outside its memory space. Whisper remains the worldwide and older-OS safety net because it
covers 99 languages. Parakeet Unified is an English streaming specialist. Nemotron 3.5 is the promising
multilingual streaming path, but its macOS runtime, language quality, memory, and battery cost must pass our
own test set before it can replace Whisper automatically.

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
3. Implement signed model manifests, verified resumable downloads, removal, and disk/RAM warnings.
4. Add Parakeet Unified EN and Nemotron 3.5 behind benchmark gates.
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
