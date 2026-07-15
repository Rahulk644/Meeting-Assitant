# My Assistant product strategy and execution order

This document is the product authority for what My Assistant becomes. `PRODUCT_RELEASE_MATRIX.md` remains
the authority for what is actually visible in a production build. A feature described here is not shipped
until the release matrix names its evidence.

## Product promise

My Assistant combines three products without making the user operate three products:

1. dependable system-wide dictation;
2. private, no-bot meeting capture with Fathom-quality workflow; and
3. contextual writing and meeting intelligence.

Agents are the fourth product pillar: voice-driven, screen-aware assistance that can draft and later execute
governed actions without forcing the user to leave the app they are working in. The architecture, safety
tiers, wake-phrase boundary, and ordered delivery plan live in
[`docs/AGENTS_ARCHITECTURE.md`](docs/AGENTS_ARCHITECTURE.md). The shipped foundation is read-only; this
strategy must never be read as evidence that computer actions or a wake phrase are already available.

The core remains useful for free with local processing. Users pay for costs the project incurs or for
material team/service value, not to unlock basic privacy, reliability, updates, or security fixes.

The first public product is macOS. Windows reuses the Rust domain/provider core after macOS quality gates
are met. Android is a later mobile product with different capture and interaction constraints, not a direct
port of the desktop shell.

## The two-engine mental model

Transcription and reasoning are independent choices. Never combine them into one ambiguous “AI model” row.

### Speech

Speech turns audio into a timestamped transcript. The simple picker is:

- **Automatic (recommended)** — selects an installed local model using device memory, language, latency,
  and battery state;
- **On this Mac** — private/offline model choices; and
- **Cloud API** — the user's provider key and explicit privacy disclosure.

The shipped local catalog is deliberately curated. User-selectable means “available to try,” not “proven
best on every Mac, language, and acoustic condition”:

| Product label | Model family | Job | Release position |
|---|---|---|---|
| System — Fast and light | Apple SpeechAnalyzer / SpeechTranscriber | OS-managed on-device engine for supported locales on macOS 26+ | blocked pending adapter, asset lifecycle, fallback, and product benchmarks |
| Worldwide — Balanced | Whisper Large v3 Turbo Q8 | multilingual everyday default | shipped recommended default |
| Worldwide — Maximum Whisper | Whisper Large v3 Q5 | slower quality-first and English translation option | shipped explicit choice; powerful-Mac measurements pending |
| English — Speed tiers | Whisper Tiny/Base/Small/Medium English Q8 | user-controlled size/quality trade-offs | shipped explicit choices |
| English — Streaming specialists | Parakeet Unified EN and Nemotron Speech Streaming EN | fast English with timestamp-capable model families | shipped explicit choices; product benchmarks pending |
| Multilingual — Streaming specialist | Nemotron 3.5 ASR Streaming | 28-language local runtime | shipped explicit choice; not auto-selected pending benchmarks |
| Hinglish — Roman | Oriserve Apex Q8 and Prime | Roman Hinglish typing | shipped explicit choices; Apex leads the current machine-derived proxy |
| Translate to English | Whisper Medium multilingual Q5 and Large v3 Q5 | separate translation decoder task | shipped only when a capable model is active |

SpeechAnalyzer adds no model weight to the app, runs outside its memory space, and receives OS-managed model
updates. It should be tried first by Automatic on macOS 26+ only after it passes our own dictation, accent,
code-switching, long-form, and meeting-audio benchmark for the selected locale. Runtime/locale failure falls
back to Whisper without losing the captured audio. The current public English benchmark beats Whisper Small,
not Whisper Large v3 Turbo or Parakeet, and must not be generalized beyond its test set.

Parakeet Unified EN is an English specialist, not a worldwide default. Nemotron 3.5 is exposed through the
same local transcribe.cpp family used by Handy, but remains an explicit choice until our own representative
Mac, language, noise, thermal, and long-session evidence supports a stronger recommendation. Whisper remains
the safe worldwide default on Apple Silicon.

The pinned speech qualification is in
[`docs/SPEECH_MODEL_BENCHMARK.md`](docs/SPEECH_MODEL_BENCHMARK.md). Whisper completed three deterministic
500-clip mixed-script runs with zero truncations and remains the shipped default. Qwen HiACC measured better
mixed-script WER in two complete runs, but produced 13 partial outputs per run and is retained only as
historical research evidence: do not add it to the user catalog, recommend, auto-select, bundle, or continue
its promotion benchmark. Apex leads the
machine-derived Roman proxy, but that score is not publishable without the prepared human Roman corpus being
transcribed and independently reviewed. Whisper fine-tunes were not categorically rejected: Zero STT and
Shuja improve their Whisper Medium control on mixed speech, but both regress English. These are engineering
findings, not enough evidence to change the shipped default before real-noise and provider-lifecycle gates.

The first cloud speech adapters should be **Groq** (not xAI Grok) for hosted Whisper Large v3 Turbo/Large
v3 and **OpenAI** for its transcription and diarization models. Add a realtime-specialist provider only
after the provider contract and privacy UX are proven. Do not put Cerebras in Speech unless it launches an
actual ASR API; today it belongs in Intelligence.

### Intelligence

Intelligence cleans dictation, generates meeting notes/actions, answers questions, searches history, and
later powers coaching and writing actions. Its simple picker is:

- **Automatic (recommended)**;
- **On this Mac**; and
- **Cloud API**.

Local choices begin with Gemma because the efficient variants are designed for device use:

| Device recommendation | Initial choice | Operating rule |
|---|---|---|
| memory-constrained Mac | Gemma 4 E2B text-only/mobile | unload Speech before loading Intelligence |
| mid/high-memory Mac | Gemma 4 E4B text-only/mobile | offer only after an on-device benchmark passes |
| workstation | larger Gemma option | Advanced only; require disk/RAM warning and measured quality gain |

Google's published approximate mobile text-only base-memory figures are 0.84 GB for E2B and 2.2 GB for
E4B, excluding runtime and KV cache. Device recommendations must therefore come from a real first-run
benchmark and available-memory check, not model parameter counts alone.

Cloud Intelligence launches with adapters for OpenAI, Anthropic, Gemini, Groq, Cerebras, and a custom
OpenAI-compatible endpoint. The app presents stable jobs such as **Fast notes**, **Best notes**, and
**Private**, while model IDs live in a remotely updateable, signed catalog. Exact model selection remains
under Advanced.

## Provider and model architecture

The Rust core owns all provider calls. The WebView receives capability metadata and connection status, but
never a secret.

- `SpeechProvider`: batch/streaming, timestamps, word confidence, language detection, translation,
  diarization, vocabulary prompt, and data-retention disclosure.
- `IntelligenceProvider`: chat/structured output, context limit, streaming, tool/JSON support, and privacy
  disclosure.
- `ModelManifest`: stable ID, engine, artifact URLs, exact size, SHA-256/signature, license, languages,
  minimum/recommended memory, architecture, quantization, and benchmark version.
- `ResourceCoordinator`: permits only the heavy processes that fit. On an 8 GB Mac, transcription unloads
  before local summarization loads. Downloads are resumable, verified, cancellable, and individually
  removable.
- `HardwareProfile`: chip, architecture, total/available memory, free disk, Metal support, thermal/battery
  state, and a short opt-in benchmark. Recommendations are explainable and always overridable.

API keys are stored per provider in macOS Keychain. Native HTTPS reads a key only for the selected
provider; it never enters settings JSON, WebView state, logs, temporary files, shell commands, or process
arguments. Custom remote endpoints require HTTPS; loopback HTTP is allowed for local servers. Every
provider supports Test, Disconnect/Delete key, and a clear “what leaves this Mac” statement.

## Fathom-grade meeting experience

### Before a meeting

- The menu bar shows the next calendar meeting only inside the chosen horizon (six hours by default).
- A scheduled meeting and an active call are distinct states. Calendar presence never starts recording.
- When Core Audio confirms a real call, show one native notification/panel: **Meeting detected — taking
  notes**, with Pause/Stop. If automatic notes are off, offer **Start notes** instead.
- Browser and native Zoom, Google Meet, Teams, Webex, FaceTime, Slack huddles, and Discord share one
  capture contract. The beta combines per-app audio with narrowly scoped meeting-window evidence; mature
  adapters must replace fragile UI strings with DOM/native signals wherever the platform exposes them.

### During a meeting

- A native, non-activating top panel shows recording state, elapsed time, live transcript, and scratchpad.
- Transcript and scratchpad are separate; user notes are never rewritten silently.
- Pause, resume, and stop release capture correctly. Video is optional and visibly separate from audio
  notes.
- “You” comes from the microphone channel. Remote speaker separation uses diarization plus meeting-app
  participant evidence. Names remain editable because diarization identifies voices, not identities.
- The later Live Coach consumes bounded transcript windows and explicit modes (sales, speaking, interview),
  and is off by default. It must never delay or destabilize capture.

### After a meeting

- A call-end edge plus a short debounce finalizes the transcript and immediately queues note generation.
- The detail opens with progress, then Summary, Decisions, Action items, Topics, Transcript, Scratchpad,
  Recording, and Ask.
- Templates are user-owned and versioned. Start with General, Sales, Customer research, Recruiting,
  Stand-up, and One-on-one; custom instructions are supported.
- Corrections to speakers or transcript make derived notes visibly stale and regenerable.
- Search is local first. Cross-meeting Ask cites the source meeting and transcript segment.

No-bot capture is the free/private core. A meeting bot is a later paid service because it requires hosted
infrastructure, platform integrations, reliability operations, and continuing cost. It must remain an
explicit mode, never the hidden default.

## Writing Assistant interaction

The sidebar can contain settings/history, but it is not the primary workflow.

1. Select text in any app and press the Writing shortcut.
2. A small native palette appears near the selection with Polish, Shorten, Fix grammar, Change tone, and
   **Describe a change**.
3. The instruction can be typed or dictated. “Select text, then speak the edit” is a first-class path.
4. Show a compact diff/preview with Apply, Replace, Copy, and Undo. Never overwrite without a reversible
   user action.
5. Let users pin custom actions. With no selection, the palette becomes Compose mode.

Basic local and bring-your-own-key writing actions stay free. Hosted usage, shared team style guides,
governed workflows, and organization analytics are reasonable premium value.

## Free and paid boundary

### Free forever

- local dictation and curated local speech models;
- local no-bot meeting capture, transcript, scratchpad, templates, notes, actions, export, and search;
- local Intelligence choices and bring-your-own API keys;
- basic writing actions;
- security fixes, signed updates, changelog, migrations, and diagnostics export; and
- user control over local data, retention, deletion, and model removal.

### Paid because it creates real cost or team value

- included hosted speech/LLM usage and managed storage/sync;
- public share links on the product domain;
- bot-join mode;
- team libraries, permissions, admin, compliance, retention, and audit controls;
- live sales/speaking coach and team analytics;
- CRM, email, task, and workflow automations; and
- shared templates/style guides and priority support.

Do not paywall bug fixes, privacy controls, updates, export, or a user's own API keys.

## Professional operations

- **Updates:** Tauri's updater uses a separate cryptographic signing key and an HTTPS release manifest.
  Auto-check is on by default; installation behavior is user-controlled and changelog is shown before or
  after update. Losing the updater private key strands existing installations, so keep it encrypted in an
  offline backup plus the release secret store. A manual DMG and SHA-256 remain available.
- **macOS trust limitation:** updater signatures verify project ownership but do not replace Apple
  notarization. Without a paid Developer ID, every new recipient must attempt launch and explicitly choose
  **Privacy & Security → Open Anyway** on current macOS (some older versions offer Control-click → Open).
  Test self-signed updates on clean Macs before enabling automatic installation.
- **Diagnostics:** rotating structured operational logs only—no audio, transcripts, prompts, clipboard
  text, participant names, or keys. “Export diagnostics” builds a previewable local archive containing
  app/OS/architecture versions, model inventory/hashes, permission states, resource measurements,
  sanitized settings, recent operational errors, and relevant crash metadata. Nothing uploads itself.
- **Quality:** “zero bugs” is not a measurable promise. The release bar is zero known P0/P1 defects, no
  known crash/recovery failure, versioned migrations with rollback fixtures, crash-free session targets,
  and a staged beta rollout with the ability to stop or roll back an update.
- **Open source readiness:** choose a license, publish `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`,
  `SECURITY.md`, architecture decisions, model/license inventory, reproducible build instructions, SBOM,
  dependency policy, issue templates, and the release/benchmark evidence. Do not call it community-ready
  before those are true.

## Execution order

### P0 — reliability foundation (current gate)

- complete the physical Right-Command shortcut/focus/full-screen/multi-display matrix;
- add bounded in-flight audio recovery and consented two-person platform acceptance;
- add redacted rotating logs, a support ID, and previewable diagnostics export;
- add a clean-Mac in-app local Intelligence install/status/remove lifecycle;
- add clean-install/model-switch/resource measurements and an automated SBOM/dependency policy.

### P1 — curated engines

- separate Speech and Intelligence settings/providers;
- add the macOS 26 SpeechAnalyzer system adapter, benchmark it, and retain Whisper fallback on unsupported
  systems/locales or failed system-asset preparation;
- add a hardware scorer and resource coordinator on top of the shipped verified local model lifecycle;
- ship Groq/OpenAI Speech behind a contract separate from the existing meeting-note Intelligence providers;
- make E2B/E4B local Intelligence recommendations evidence-based and installable without developer tools.

### P2 — excellent no-bot meetings

- harden native and browser call adapters and add per-meeting Off/Transcript/Audio/Video choices;
- add menu-bar upcoming-meeting state, Calendar sync, and robust scheduled/impromptu metadata;
- add audio muxing to optional screen recordings and bounded crash recovery;
- add diarization, editable identity mapping, calendar sync, and local cross-meeting search;
- real-call beta matrix across apps, languages, audio devices, sleep/wake, and permission recovery.

### P3 — writing and meeting intelligence

- selection-based Writing palette with voice instructions and reversible diff;
- email/task extraction and explicit export/integration destinations;
- cited Ask across meeting history;
- first bounded Live Coach modes behind an experimental flag.

### P4 — paid services

- hosted credits, sync/team library, public sharing, bot mode, coach analytics, admin/compliance, and
  integrations. Billing and entitlement truth remain server-side and fail closed.

### P5 — additional platforms

- Windows after shared-core contracts and macOS product metrics stabilize;
- Android after defining its mobile capture, permissions, and meeting workflow independently.

## Current release truth

The distributable is a beta: Dictation is the primary supported feature; Meetings is explicitly beta.
Unfinished Writing, Agents, commercial, support, calendar, and update surfaces remain preserved in the
preview channel rather than being deleted or falsely advertised. See `PRODUCT_RELEASE_MATRIX.md`.

## Primary references

- Apple SpeechAnalyzer: <https://developer.apple.com/documentation/speech/speechanalyzer>
- Apple WWDC SpeechAnalyzer session: <https://developer.apple.com/videos/play/wwdc2025/277/>
- Public SpeechAnalyzer/Whisper benchmark: <https://get-inscribe.com/blog/apple-speech-api-benchmark.html>
- Handy native Apple Speech discussion: <https://github.com/cjpais/Handy/discussions/1031>
- FluidVoice: <https://github.com/altic-dev/FluidVoice>
- Groq Speech to Text: <https://console.groq.com/docs/speech-to-text>
- OpenAI diarized transcription: <https://developers.openai.com/api/docs/models/gpt-4o-transcribe-diarize>
- NVIDIA Parakeet TDT v3: <https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3>
- NVIDIA Nemotron 3.5 ASR Streaming: <https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b>
- Google Gemma model documentation: <https://ai.google.dev/gemma/docs/core>
- Tauri updater: <https://v2.tauri.app/plugin/updater/>
- Handy: <https://github.com/cjpais/Handy>
- Meetily: <https://github.com/Zackriya-Solutions/meetily>
