# Open-source component strategy

This document decides how My Assistant adopts and updates third-party building blocks. The objective is
maximum user value with minimum custom engineering and maintenance, without outsourcing product quality,
privacy, security, or reliability to an upstream project.

## The assembly rule

Use the highest-level component that meets the product contract:

1. a maintained library behind a small internal trait;
2. a pinned sidecar with an authenticated, bounded protocol;
3. a separately deployed service behind a versioned provider API;
4. a product or UX reference when its code boundary, footprint, quality, or license is unsuitable.

Never fork a whole application merely to obtain one subsystem. Never track an upstream default branch in a
release. “Upstream improves, therefore My Assistant improves” means an update bot proposes a pinned upgrade,
CI rebuilds and qualifies it, and a human accepts the evidence. It does not mean unreviewed runtime updates.

Licenses are part of the engineering contract. User permission cannot waive an upstream author's copyright
or license conditions. A component with incompatible or unclear terms is used only as a clean-room design
reference unless appropriate rights are obtained.

## Stable internal seams

Product logic depends on these interfaces, not on vendor types:

```text
AudioSource             microphone / system / process audio + device changes
EchoCanceller           aligned near-end microphone + far-end render audio; bypass and health
MeetingEvidenceSource   bounded OS audio/process/window observations; no lifecycle policy
MeetingDetector         OS-free call edges, source policy, and confidence evidence
TranscriptEngine        final and optional incremental speech hypotheses
Diarizer                timestamped speaker clusters and embeddings
IdentityResolver        cluster ↔ participant ↔ calendar identity/email evidence
MeetingEventSource      participant, chat, reaction, mute/share, and lifecycle events
CalendarProvider        events, attendees, meeting URLs, sync, revoke
BotMeetingProvider      schedule/join/leave/status/media/transcript/chat artifacts
IntelligenceProvider    notes, Ask, structured extraction, embeddings
KnowledgeStore          inspectable Markdown records, links, indexes, retrieval
WorkflowProvider        versioned meeting outputs → approved actions → idempotent audit receipts
```

Adapters may change; the event schema, privacy decisions, local store, corrections, and UI do not. Every
adapter reports capability metadata so unsupported behavior fails closed rather than becoming a dead toggle.
The shipped macOS adapter already reports observations to the dependency-free Rust detector; new policy must
land there rather than in Swift, WASAPI, Accessibility, or browser-extension adapters.

## Component decisions

| Component | Use | Integration boundary | Do not take | Promotion gate |
|---|---|---|---|---|
| [Tauri](https://github.com/tauri-apps/tauri) | Adopted shell | Direct locked dependency | Electron process model | Existing release, security, and physical UI gates |
| [CPAL](https://github.com/RustAudio/cpal) | Current microphone I/O | `AudioSource` implementation | Meeting orchestration or playback policy | Keep until a higher-level adapter proves equal latency and reliability |
| [Rodio](https://github.com/RustAudio/rodio) | Evaluate only if cue playback grows beyond `afplay` | A tiny `AudioCuePlayer` | Capture, meeting routing, or a permanent mixer | Smaller/reliable cross-platform cue path with no idle worker or material binary growth |
| [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) | Adopted primary speech runtime | Pinned authenticated `transcribe-server`; later consider direct library crate | Upstream CLI UX or unpinned model catalog | Existing corpus, protocol, memory, latency, streaming, and physical insertion gates |
| [Wispr Flow Voice Interface API](https://api-docs.wisprflow.ai/introduction) | Optional hosted dictation-quality candidate after Wispr grants organization access | `TranscriptEngine` provider using the documented WebSocket/client-auth contract | Desktop session tokens, private gRPC transport, undocumented endpoints, or treating formatted dictation text as meeting diarization | Partner-issued API/client key; pinned request schema; corpus quality/latency/privacy/cost; revocation and failover; no participant-name claims |
| Handy | Research and model-qualification reference | Clean-room behavior and pinned compatible model metadata | Whole app, settings/UI clone, or upstream state model | Each borrowed artifact independently licensed, hashed, benchmarked, and attributed |
| [FlexAudio](https://github.com/Studio-Sadola/flexaudio) | Highest-priority audio portability qualification; current `0.2` API advertises mic/system/process capture on macOS, Windows, and Linux | `AudioSource`; first as an A/B adapter, then replace CPAL/Swift capture only where it wins | Blind migration, full-stack ownership, or treating its capability table as our production proof | Rust 1.85 toolchain uplift; macOS mic/system/process parity; device hot-swap; hours-long soak; TCC; CPU/RSS; no quality regression. X-02 separately proves Windows/WASAPI |
| [Screenpipe](https://github.com/screenpipe/screenpipe) | Selective technology and UX reference | Accessibility-first context, OCR fallback, event-driven capture, timestamped local index, permission ideas | 24/7 recording, full accessibility-tree retention, its database/app/agent stack, or its 0.5–3 GB / multi-GB-per-month operating model | Only a bounded, user-invoked subsystem with explicit filters, retention, privacy, and a lower measured footprint |
| [Anarlog](https://github.com/fastrepl/anarlog) | Source-level reference and selective extraction | Markdown meeting records, provider adapter patterns, local meeting UX/accessibility parsers after audit | Whole app replacement or abandoned/legacy paths merely because they compile | Per-module license/security review, tests, and a smaller boundary than implementing locally |
| [Muesli](https://github.com/pHequals7/muesli) | Selective OSS reference for evidence-driven meeting detection, local diarization wiring, and aligned neural AEC | Adapt its multi-signal candidate-resolution and session-stabilization concepts behind `MeetingEvidenceSource`; use its LocalVQE integration as the control for our direct `EchoCanceller` adapter | Whole app, Apple-only product policy, generic Speaker 1/2 clusters treated as participant names, or unbenchmarked heuristics | Our adapter beats the current detector/AEC baseline under code review, privacy/security checks, real calls, long-run resource tests, and Windows strategy |
| [FluidAudio](https://github.com/FluidInference/FluidAudio) | Apple-only speech/diarization benchmark candidate, including speaker-enrollment experiments | Optional macOS `TranscriptEngine`/`Diarizer` adapter or model-evaluation reference | Shared cross-platform core, a second permanent speech stack, treating enrolled voice slots as platform participant identity, or macOS-version assumptions in product logic | Material Apple Silicon quality/latency/energy win, supported OS coverage, stable API/model terms, and final authoritative output compatibility |
| Meetily and similar local notetakers | Workflow and recovery reference | Import/export, checkpoint, local provider, and meeting-library patterns after targeted audit | Electron/Node packaging, entire backends, broad dependency graphs, or duplicate product state | A small independently testable module removes more code/maintenance than its dependency and migration cost |
| [speakrs](https://github.com/avencera/speakrs) | Primary local diarization candidate; its Rust community-1-style pipeline preserves overlap and offers CoreML/ONNX modes | `Diarizer` library adapter, initially post-call and then measured for bounded live updates | Speaker names inferred from clusters alone, upstream benchmark claims accepted without reproduction, or uncontrolled first-use model downloads | Reproduce DER/overlap and latency/RSS on our meeting corpus; pin model license/checksum; prove offline lifecycle and corrections; X-03 owns Windows qualification |
| [pyannote-rs](https://github.com/thewh1teagle/pyannote-rs) | Lightweight diarization/VAD control candidate | Alternative `Diarizer` adapter used in the same benchmark | Assuming it implements the complete pyannote community-1 pipeline or adding Python to the desktop release | Same corpus as `speakrs`; select one winner, do not ship two permanent pipelines |
| [LocalVQE](https://github.com/localai-org/LocalVQE) | Primary open-source neural AEC qualification candidate, validated by Muesli's shipping integration | Direct pinned `EchoCanceller` adapter fed timestamp-aligned render and mic streams; qualify the small echo-only v1.4 model first | Muesli's bundled binary/model, always-on processing for headsets, hidden downloads, or assuming upstream quality from benchmark tables | Echo return loss/double-talk/near-end quality, delay/reference-loss behavior, device changes, CPU/RSS, long-call stability, model/library checksum and license/SBOM, safe bypass, and X-03 Windows proof |
| [WebRTC AudioProcessing Rust wrapper](https://github.com/tonarino/webrtc-audio-processing) | AEC3 control and fallback candidate | Alternative `EchoCanceller` adapter under the same corpus and metrics as LocalVQE | Granola's native module, always-on processing for headsets, or adoption from upstream feature labels alone | Beat LocalVQE or materially improve a documented failure class under the same speaker/headset, delay, hot-plug, CPU/RSS, licensing, and rollback gates |
| Platform Accessibility APIs | Primary local participant/name/chat evidence where exposed | Small per-platform `MeetingEventSource` adapters; bounded tree queries | Full-tree polling, OCR-first parsing, silent edits, or claims of universal support | Fixture tests plus real Zoom/Meet/Teams matrices across view modes and versions |
| [Recall.ai](https://www.recall.ai/) | Preferred managed off-PC meeting provider to qualify | `BotMeetingProvider` over webhooks/API; server-side only | Vendor data model in the client, provider credentials in the desktop app, or treating Recall tenant labels as our authorization boundary | Meet/Zoom/Teams reliability, calendar scheduling/deduplication, participant/chat artifacts, retention/region/privacy, cost, webhook replay/idempotency, deletion |
| [Vexa](https://github.com/Vexa-ai/vexa) | Preferred self-host candidate to qualify | Same `BotMeetingProvider`; pinned containers on managed infrastructure | Embedding its multi-service stack in the desktop app or assuming a low-cost Hostinger plan supports real browser/media workloads | Platform matrix, per-meeting resource measurement, autoscaling, secrets, backups, upgrades/rollback, observability, transcript/speaker quality, failure recovery |
| [Attendee](https://github.com/attendee-labs/attendee) | Competitive self-host control | Same provider contract in a benchmark deployment | Copying code before exact license/dependency terms and provider SDK obligations are cleared | Must beat Vexa on supported platforms, reliability, operations, and total maintenance cost under the same acceptance suite |
| [Rowboat](https://github.com/rowboatlabs/rowboat) | Experimental main intelligence/knowledge interface and future Agent foundation | Separate local integration branch or upstream contribution; Rowboat consumes our native dictation, meeting capture, evidence, transcript, and overlay services through versioned contracts | Silently replacing the current production shell, unrestricted autonomy, hidden prompt-based permissions, or enabling a production Agent surface under the current release contract | Native-sidecar contract and upgrade strategy; deterministic data permissions; approval/undo/audit; model/provider neutrality; footprint, packaging, migration, and physical workflow gates |
| [Activepieces](https://github.com/activepieces/activepieces) | Future long-tail workflow engine | Separately deployed `WorkflowProvider`; My Assistant owns auth, templates, approvals, receipts, and UX | Bundling its server in the desktop app, putting core meeting behavior in user workflows, or assuming enterprise embedding code is MIT | Validate exact core/enterprise boundary, embedding rights, tenant isolation, secret storage, upgrade/rollback, idempotency, and curated workflow completion rate |
| Google Calendar API + EventKit | Core calendar truth | First-party `CalendarProvider` adapters | Using Activepieces as the authoritative calendar sync or scraping participant email from meeting UI | OAuth/revoke, incremental sync, dedupe, recurrence, offline cache, privacy copy, real event fixtures |
| Fathom, Granola, and Fireflies | Product-quality references only | Detection timing, live transcript/names, scratchpad, chat, post-call usefulness, calendar/bot policy, recovery UX | Proprietary code/assets, misleading parity claims, Electron footprint, or workflows unrelated to the core jobs | Behavioral acceptance criteria are implemented independently and beat the current user journey on supported cases |

## Product-specific conclusions

### Audio

CPAL is a low-level cross-platform microphone library and is already useful. Rodio is mainly a higher-level
audio playback layer; it does not solve per-process/system meeting capture. FlexAudio `0.2` is the most
relevant off-the-shelf path because it defines microphone, system, per-process, hot-swap, and device
notification behavior across macOS, Windows, and Linux. It is also new, has a self-reported rather than
product-proven capability matrix, and requires Rust 1.85 while this crate declares 1.77.2. It therefore
enters through an A/B `AudioSource` adapter after the current real-call baseline—not a rewrite. If it passes,
it can replace platform capture plumbing while upstream improvements arrive through qualified Cargo
upgrades.

### Immediate meeting adoption order

1. Preserve the working Core Audio path and establish two-person Zoom/Google Meet evidence first.
2. Implement bounded in-flight audio recovery using the canonical record/checkpoint contract; this is product
   logic, not a reason to adopt another full meeting app.
3. Qualify FlexAudio behind `AudioSource` and keep it only where it matches or improves channel quality,
   device recovery, soak stability, footprint, and permission behavior.
4. Benchmark `speakrs` against `pyannote-rs` after the canonical schema exists. Diarization improves identity
   but does not block an honest narrow Beta that labels uncertain speakers.
5. Qualify direct LocalVQE against raw bypass and AEC3 when speaker-mode evidence shows echo or loopback
   duplication. Headsets and missing/unreliable far-end references bypass processing that cannot help them.
6. Keep Recall/Vexa, calendars, connectors, and compounding knowledge on their existing later seams. They do
   not delay bot-free capture, transcript, scratchpad, or grounded Live Assist.

### Bot-free meetings

Keep the existing Core Audio + bounded Accessibility foundation. Add diarization as a separate post/live
worker and merge its speaker clusters with platform active-speaker evidence, the user's profile, and calendar
attendees. Accessibility is the first source for live participant/chat semantics; OCR is a bounded fallback
only when an explicitly supported app/version exposes no semantic tree. No OSS currently removes the need
for a product-specific evidence merger and acceptance corpus. `speakrs` remains the cross-platform Rust
candidate because it exposes the complete clustering pipeline and overlap; FluidAudio remains an Apple
benchmark/enrollment reference. Muesli demonstrates the expected limitation plainly: diarization can render
Speaker 1/Speaker 2 without proving either participant's real name.

### Participant names and email

Audio can identify repeated voices but cannot discover an email address. Email comes from authorized calendar
attendee data or a bot provider artifact. Recall documents participant emails for calendar-scheduled bots as
a gated capability; calendar raw events also contain organizer/attendee addresses. Store each mapping with a
source and confidence, never infer an address from a display name, and keep the user able to revoke calendar
access and delete identity data.

### Meetings when the computer is off

This requires a server-side bot; no local capture library can do it. The client sets a default per calendar
or meeting: **No bot**, **Ask me**, or **Join for me**. The backend schedules a provider bot, handles updates
idempotently, captures transcript/chat/participant artifacts, and writes them into the same canonical meeting
record as local capture. Recall is the fastest managed route. Vexa is the leading self-host route, but it is
an operations product with browsers, media, databases, queues, storage, secrets, and upgrades—not something
to run inside the desktop app.

### Knowledge and agents

Rowboat is the main interface for the local integration program, while My Assistant remains the owner of
the lightweight native capture and product-trust boundaries. This does not change the current production
capability contract: Agents remain disabled until the separate release gates are approved and passed.

Rowboat's strongest reusable idea is not “many agents”; it is transparent compounding memory: ordinary
Markdown notes, stable links between people/projects/decisions/commitments, editable generated knowledge,
and tools that operate on that store. Meeting transcript + meeting chat + scratchpad is the immutable source;
post-call notes and entity pages are derived and carry provenance. This can improve meeting prep and follow-up
before any autonomous Agent UI exists. Production Agents still remain disabled until their separate release
contract is approved.

Its chat mutation model is useful when made explicit: a chat can propose creating/updating Markdown knowledge,
generate an artifact, or invoke a connected tool. My Assistant should show the affected records/destination,
source evidence, exact proposed change, required approval, result receipt, and recovery path. The model may
suggest a mutation; product code enforces what it can read or change.

### Workflow automation

Core journeys—capture, transcription, notes, identity, recovery, calendar scheduling, and the first-party
follow-up experience—remain product code. Activepieces is for the long tail after those work: “when a meeting
ends, create a Linear issue,” “send approved notes to Slack,” or a user-authored cross-product workflow. Start
with curated templates and explicit approval. Expose a builder only after users repeatedly need custom flows.

## Upstream intake pipeline

Each adopted component gets a small manifest entry containing upstream URL, version or commit, license,
expected checksum/source, owner, adapter, qualification suite, and rollback version. Then:

1. an update bot opens a pull request; no direct-to-main upgrades;
2. CI verifies source identity, lockfile, licenses, RustSec/npm audit, and SBOM delta;
3. contract tests run through the internal adapter rather than vendor examples;
4. speech/audio/diarization changes run the pinned quality and performance corpus;
5. packaged components pass architecture, signing, entitlement, authentication, and clean-install checks;
6. high-risk upgrades soak behind an internal feature flag; and
7. the previous pinned version remains a one-change rollback.

An upstream patch that does not pass is not an improvement to My Assistant. The thin adapter makes waiting
or rolling back cheap while other upstream components continue to advance independently.
