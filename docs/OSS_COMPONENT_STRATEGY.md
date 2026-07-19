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
MeetingDetector         call edges and confidence evidence
TranscriptEngine        final and optional incremental speech hypotheses
Diarizer                timestamped speaker clusters and embeddings
IdentityResolver        cluster ↔ participant ↔ calendar identity/email evidence
MeetingEventSource      participant, chat, reaction, mute/share, and lifecycle events
CalendarProvider        events, attendees, meeting URLs, sync, revoke
BotMeetingProvider      schedule/join/leave/status/media/transcript/chat artifacts
IntelligenceProvider    notes, Ask, structured extraction, embeddings
KnowledgeStore          inspectable Markdown records, links, indexes, retrieval
WorkflowProvider        approved trigger/action execution and audit receipts
```

Adapters may change; the event schema, privacy decisions, local store, corrections, and UI do not. Every
adapter reports capability metadata so unsupported behavior fails closed rather than becoming a dead toggle.

## Component decisions

| Component | Use | Integration boundary | Do not take | Promotion gate |
|---|---|---|---|---|
| [Tauri](https://github.com/tauri-apps/tauri) | Adopted shell | Direct locked dependency | Electron process model | Existing release, security, and physical UI gates |
| [CPAL](https://github.com/RustAudio/cpal) | Current microphone I/O | `AudioSource` implementation | Meeting orchestration or playback policy | Keep until a higher-level adapter proves equal latency and reliability |
| [Rodio](https://github.com/RustAudio/rodio) | Evaluate only if cue playback grows beyond `afplay` | A tiny `AudioCuePlayer` | Capture, meeting routing, or a permanent mixer | Smaller/reliable cross-platform cue path with no idle worker or material binary growth |
| [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) | Adopted primary speech runtime | Pinned authenticated `transcribe-server`; later consider direct library crate | Upstream CLI UX or unpinned model catalog | Existing corpus, protocol, memory, latency, streaming, and physical insertion gates |
| Handy | Research and model-qualification reference | Clean-room behavior and pinned compatible model metadata | Whole app, settings/UI clone, or upstream state model | Each borrowed artifact independently licensed, hashed, benchmarked, and attributed |
| [FlexAudio](https://github.com/Studio-Sadola/flexaudio) | Highest-priority audio portability qualification | `AudioSource`; first as an A/B adapter, then replace CPAL/Swift capture only where it wins | Blind migration, full-stack ownership, or assuming a young `0.x` crate is production-proven | Rust toolchain uplift; macOS mic/system/process parity; Windows WASAPI proof; device hot-swap; hours-long soak; TCC; CPU/RSS; no quality regression |
| [Screenpipe](https://github.com/screenpipe/screenpipe) | Selective technology and UX reference | Accessibility-first context, OCR fallback, event-driven capture, timestamped local index, permission ideas | 24/7 recording, full accessibility-tree retention, its database/app/agent stack, or its 0.5–3 GB / multi-GB-per-month operating model | Only a bounded, user-invoked subsystem with explicit filters, retention, privacy, and a lower measured footprint |
| [Anarlog](https://github.com/fastrepl/anarlog) | Source-level reference and selective extraction | Markdown meeting records, provider adapter patterns, local meeting UX/accessibility parsers after audit | Whole app replacement or abandoned/legacy paths merely because they compile | Per-module license/security review, tests, and a smaller boundary than implementing locally |
| [Muesli](https://github.com/pHequals7/muesli) | Research/control implementation, not a quality authority | Calendar URL extraction, local diarization wiring, onboarding and meeting-record immutability ideas after source audit | Whole app, unbenchmarked heuristics, or the assumption that a student-maintained project meets production edge cases | Borrowed module beats our implementation under code review, privacy/security checks, real calls, long-run resource tests, and Windows strategy |
| [FluidAudio](https://github.com/FluidInference/FluidAudio) | Apple-only speech/diarization benchmark candidate | Optional macOS `TranscriptEngine`/`Diarizer` adapter or model-evaluation reference | Shared cross-platform core, a second permanent speech stack, or macOS-version assumptions in product logic | Material Apple Silicon quality/latency/energy win, supported OS coverage, stable API/model terms, and final authoritative output compatibility |
| Meetily and similar local notetakers | Workflow and recovery reference | Import/export, checkpoint, local provider, and meeting-library patterns after targeted audit | Electron/Node packaging, entire backends, broad dependency graphs, or duplicate product state | A small independently testable module removes more code/maintenance than its dependency and migration cost |
| [speakrs](https://crates.io/crates/speakrs) | Primary local diarization candidate | `Diarizer` library adapter using ONNX/CoreML | Speaker names inferred from clusters alone | Accuracy/DER and overlap corpus, Apple Silicon latency/RSS, Windows backend, model license/checksum, live/backfill behavior |
| [pyannote-rs](https://crates.io/crates/pyannote-rs) | Diarization control candidate | Alternative `Diarizer` adapter | Python runtime in the desktop release | Same benchmark as `speakrs`; select one winner, do not ship two pipelines |
| Platform Accessibility APIs | Primary local participant/name/chat evidence where exposed | Small per-platform `MeetingEventSource` adapters; bounded tree queries | Full-tree polling, OCR-first parsing, silent edits, or claims of universal support | Fixture tests plus real Zoom/Meet/Teams matrices across view modes and versions |
| [Recall.ai](https://www.recall.ai/) | Preferred managed off-PC meeting provider to qualify | `BotMeetingProvider` over webhooks/API; server-side only | Vendor data model in the client, provider credentials in the desktop app, or treating Recall tenant labels as our authorization boundary | Meet/Zoom/Teams reliability, calendar scheduling/deduplication, participant/chat artifacts, retention/region/privacy, cost, webhook replay/idempotency, deletion |
| [Vexa](https://github.com/Vexa-ai/vexa) | Preferred self-host candidate to qualify | Same `BotMeetingProvider`; pinned containers on managed infrastructure | Embedding its multi-service stack in the desktop app or assuming a low-cost Hostinger plan supports real browser/media workloads | Platform matrix, per-meeting resource measurement, autoscaling, secrets, backups, upgrades/rollback, observability, transcript/speaker quality, failure recovery |
| [Attendee](https://github.com/attendee-labs/attendee) | Competitive self-host control | Same provider contract in a benchmark deployment | Copying code before exact license/dependency terms and provider SDK obligations are cleared | Must beat Vexa on supported platforms, reliability, operations, and total maintenance cost under the same acceptance suite |
| [Rowboat](https://github.com/rowboatlabs/rowboat) | Future knowledge/Agent-mode reference; source modules may be adopted after audit | Obsidian-compatible Markdown vault, backlinks/graph, memory extraction, inspectable live notes, MCP/tool provider adapters | Its Electron shell, an unrestricted autonomous runtime, hidden prompt-based permissions, or any production Agent surface under the current release contract | Separate Agent product brief; module/API audit; deterministic data permissions; approval/undo/audit; model/provider neutrality; footprint and packaging gates |
| [Activepieces](https://github.com/activepieces/activepieces) | Future long-tail workflow engine | Separately deployed `WorkflowProvider`; My Assistant owns auth, templates, approvals, receipts, and UX | Bundling its server in the desktop app, putting core meeting behavior in user workflows, or assuming enterprise embedding code is MIT | Validate exact core/enterprise boundary, embedding rights, tenant isolation, secret storage, upgrade/rollback, idempotency, and curated workflow completion rate |
| Google Calendar API + EventKit | Core calendar truth | First-party `CalendarProvider` adapters | Using Activepieces as the authoritative calendar sync or scraping participant email from meeting UI | OAuth/revoke, incremental sync, dedupe, recurrence, offline cache, privacy copy, real event fixtures |
| Fathom, Granola, and Fireflies | Product-quality references only | Detection timing, live transcript/names, scratchpad, chat, post-call usefulness, calendar/bot policy, recovery UX | Proprietary code/assets, misleading parity claims, Electron footprint, or workflows unrelated to the core jobs | Behavioral acceptance criteria are implemented independently and beat the current user journey on supported cases |

## Product-specific conclusions

### Audio

CPAL is a low-level cross-platform microphone library and is already useful. Rodio is mainly a higher-level
audio playback layer; it does not solve per-process/system meeting capture. FlexAudio is the most relevant
off-the-shelf path because it defines microphone, system, and per-process capture across macOS, Windows, and
Linux. It is also young, requires a newer Rust toolchain than the current crate, and therefore enters through
an A/B adapter—not a rewrite. If it passes, it can replace a large part of platform capture plumbing while
upstream improvements arrive through ordinary Cargo upgrades.

### Bot-free meetings

Keep the existing Core Audio + bounded Accessibility foundation. Add diarization as a separate post/live
worker and merge its speaker clusters with platform active-speaker evidence, the user's profile, and calendar
attendees. Accessibility is the first source for live participant/chat semantics; OCR is a bounded fallback
only when an explicitly supported app/version exposes no semantic tree. No OSS currently removes the need
for a product-specific evidence merger and acceptance corpus.

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
