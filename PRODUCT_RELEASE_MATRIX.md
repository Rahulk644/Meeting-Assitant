# Product Release Matrix

This file is the release authority for My Assistant. It separates implemented product behavior from the
surface that is safe to expose in a production build. The server-owned registry in `app/release.js` decides
what the browser may render. The Agents label is a deliberate exception: it is always present only as a
greyed-out, disabled **Coming soon** navigation placeholder. It is not a route and exposes no Agent details.

## Release channels

- `ASSISTANT_RELEASE_CHANNEL=production` — default for every packaged and ordinary local run. Unknown
  capabilities fail closed. The DMG pipeline overrides any ambient value and always generates production.
- `ASSISTANT_RELEASE_CHANNEL=preview` — internal design review only. It preserves the complete designed
  surface, including unfinished and blocked work. It cannot be enabled by URL or a saved UI preference and
  is never accepted by the release pipeline.

## Capability states

- `shipped` — visible in production and preview, with named verification evidence.
- `preview` — visible only in the internal design-preview channel.
- `hidden` — preserved in code but invisible in production; it is not currently part of the release.
- `blocked` — implementation or design exists, but a known missing dependency or failure prohibits exposure.

Changing a row to `shipped` requires updating `app/release.js`, adding or naming verification evidence, and
passing the production-surface contract tests. “Designed” and “shipped” are not synonyms.

## Primary navigation and commercial surfaces

| Capability key | Surface | State | Production | Preview | Backend / desktop support | Known limitation | Required verification / reopening condition |
|---|---|---:|---:|---:|---|---|---|
| `dictation` | Dictation navigation and view | shipped | Yes | Yes | Native global shortcut, mic capture, STT, capability-gated in-field live text, deterministic cleanup, optional guarded local polish, one final commit | Live text requires a selected streaming-capable model and a compatible editable field; recent dictations and diffs are session-only | Native hotkey, final-only paste, live-span reconciliation, cleanup, validator, and settings contracts |
| `meetings` | Meetings navigation and view, labelled Beta | shipped | Yes | Yes | Automatic/manual capture, SSE, local meeting store | Automatic browser/chat detection requires both active audio and meeting-window evidence | Recording lifecycle and meeting-library tests |
| `writingAssistant` | Writing Assistant navigation and view | preview | No | Yes | Design only | No system-wide writing runtime | Implement native selection/action runtime and test every action |
| `agents` | Disabled Coming soon navigation label | blocked | Label only | Internal only | No production route, initialization, settings, shortcut, helper, or details; saved route is sanitized | Agents is not an initial product feature | Make a new explicit product decision and release plan before exposing anything beyond the label |
| `feedbackSupport` | Feedback & Support | hidden | No | Yes | No destinations | Cards do not open real support targets | Wire and verify every destination |
| `guide` | Guide | hidden | No | Yes | No published guide | Placeholder content | Publish a real guide and verify link handling |
| `shareFriend` | Share with a friend | hidden | No | Yes | No share destination | Placeholder content | Add a real share action and verify it |
| `settings` | Settings navigation and view | shipped | Yes | Yes | Server-mirrored preferences plus native handlers | Only verified rows render in production | Production setting-handler contract |
| `pro` | Pro card and commercial claims | blocked | No | Yes | None | No billing, entitlement, cloud plan, or team sharing | Complete billing and entitlement verification |

Internal registry keys retained for dormant Agent research inherit the single blocked Agents contract above.
They are intentionally not itemized as product capabilities and cannot expand production packaging or UI.

## Meetings

| Capability key | Surface | State | Production | Preview | Support | Known limitation | Required verification / reopening condition |
|---|---|---:|---:|---:|---|---|---|
| `meetingRecord` | Record | shipped | Yes | Yes | Manual recording gate | Requires macOS capture permissions | Manual start and finalize tests |
| `automaticRecording` | Automatic recording, enabled by default | shipped | Yes | Yes | macOS Core Audio/Accessibility adapter feeding the shared Rust detector; two-observation start/three-miss end stability; browser/chat apps additionally require meeting-window evidence; desktop and browser source families can be disabled separately | Real two-person validation remains incomplete across the full advertised platform/version matrix; no bot joins | Rust call-edge/source-policy tests, adapter self-test, source-setting, and per-platform acceptance tests |
| `meetingDetectedNotification` | Native meeting-start notification | shipped | Yes | Yes | Emitted only on a Core Audio false-to-true call edge | First use may request macOS notification permission | Transition test plus native notification smoke test |
| `liveTranscript` | Live transcript with evidence-backed speaker labels | shipped | Yes | Yes | Selected qualified on-device speech model plus SSE; caption audio intervals are matched in Rust against a bounded Zoom Accessibility active-speaker timeline, with self filtering and persisted source/confidence | Supported Zoom layouts can show participant names; gallery/overlap is best-effort, unsupported or ambiguous turns remain **Unknown speaker**, and Chrome Meet names await the extension/identity path | Attribution unit tests plus physical two-person Zoom alternating/overlap/gallery acceptance on the exact installed candidate |
| `liveMeetingAssist` | Grounded live assist | shipped | Yes | Yes | Catch up from the last transcript minute; summarize-so-far, explicit actions, and follow-up questions use the current non-excluded transcript and selected local/BYOK Intelligence provider | Requires enough live transcript and a configured/installed model; results are suggestions and never write or send anything | Time-window/exclusion tests, fail-closed action route, and call-panel handler contract |
| `captureReadiness` | Starting / Recording / Needs attention | shipped | Yes | Yes | Rust lifecycle requires microphone open plus the first real meeting-audio buffer before Recording; a tap that opens without data times out and retries, participant silence does not block a healthy tap's buffers, post-flow heartbeat loss retries that source, and three failed recoveries in one minute recycle the supervised helper with bounded relaunch backoff | Physical two-person channel quality and real device-drop recovery remain Beta acceptance requirements; microphone captions stay **Unknown speaker** while separation is unverified | Rust phase/supervisor/attribution tests, helper first-buffer/dropout/recycle self-test, native and WebView state contracts, silent-remote and two-person calls |
| `callPanel` | Tiny native Nub, adaptive live overlay, and full in-app panel | shipped | Yes | Yes | Movable 52 × 52 non-activating AppKit Nub projects the Rust lifecycle above the active meeting Space; the branded collapsed state expands into an adaptive vertical conversation card with up to five speaker-grouped transcript bubbles, direct last-minute catch-up, and proportionate Pause/Resume/Stop controls; full history/Scratchpad/Assist remains in My Assistant | Position persists for the app session, not yet across relaunch; Windows native overlay remains X-02; physical full-screen/multi-display behavior requires Mac acceptance | Native ownership/unit tests, bounded transcript and grounded-assist contracts, five-second comprehension, silent first-click, full-screen/multi-display, and focus smoke tests |
| `scratchpad` | Scratchpad | shipped | Yes | Yes | Persisted into current meeting JSON | 8,000-character cap | Notes route and summary-prompt test |
| `pauseResumeStop` | Pause, Resume, Stop | shipped | Yes | Yes | Server gate releases/re-engages capture | Call-end debounce is 15 seconds | Gate transition tests |
| `meetingLibrary` | Meeting library | shipped | Yes | Yes | Local JSON store | No folders or calendar view | List/detail domain tests |
| `meetingThumbnails` | Recording frame or meeting-app identity thumbnail | shipped | Yes | Yes | Native first-frame JPEG; installed app icon; platform tile fallback | Browser platforms use a branded identity tile rather than a native app icon | Recording/no-recording endpoint tests |
| `meetingRename` | Rename meeting | shipped | Yes | Yes | Local title endpoint | Manual rename supersedes generated title | Persistence test |
| `meetingExport` | Export | shipped | Yes | Yes | Client-side Markdown download | Markdown only | Export handler contract |
| `meetingSummary` | Summary and regenerate | shipped | Yes | Yes | Local or configured OpenAI-compatible LLM | Requires a configured/installed model | Success, stale, failure, and retry states |
| `meetingActions` | Action items | shipped | Yes | Yes | Parsed from generated notes | Quality follows the selected model/template | Summary fixture verification |
| `meetingTranscript` | Transcript | shipped | Yes | Yes | Atomic local lines; first line immediate, then ≤2-second checkpoints plus 10-second safety pass | In-flight audio recovery is not yet implemented | Persistence, interruption, correction, and exclusion tests |
| `meetingAsk` | Ask this meeting | shipped | Yes | Yes | Uses corrected, non-excluded transcript | Requires a configured/installed model | Corrected-context endpoint test |
| `participantCorrections` | Rename, remove, and merge participant identities | shipped | Yes | Yes | Persisted correction API | User corrections are authoritative | Correction domain and API tests |
| `transcriptCorrections` | Reassign, edit, and exclude transcript lines | shipped | Yes | Yes | Persisted correction API | Original text is not separately versioned | Correction and stale-summary tests |
| `automaticTitles` | Scheduled or impromptu meeting titles | shipped | Yes | Yes | User title, calendar/scheduled metadata, detected meeting-window title, then `Impromptu meeting` | Google Calendar sync itself remains blocked; the precedence contract is ready for any calendar adapter | Title precedence fixtures |
| `automaticNotes` | Post-call note generation | shipped | Yes | Yes | Queued after a substantive call | Model unavailability becomes a retryable failure | Threshold, queue, and recovery tests |
| `meetingUpload` | Upload audio | preview | No | Yes | None | Placeholder action | Implement import, transcription, and failure recovery |
| `meetingShare` | Share | blocked | No | Yes | None | Placeholder action | Implement a real destination and privacy model |
| `calendarView` | Calendar view | preview | No | Yes | None | No calendar data source | Implement view plus an actually shipped source |
| `folders` | Folders | preview | No | Yes | None | No persistence | Implement CRUD, assignment, and migration behavior |
| `googleCalendarSource` | Google Calendar source | blocked | No | Yes | None | No OAuth/sync | Complete OAuth, sync, revoke, and privacy verification |
| `fathomSource` | Fathom source | blocked | No | Yes | None | No import adapter | Implement and verify import fidelity |
| `meetingShortcut` | Global meeting shortcut | blocked | No | Yes | None | Not registered natively | Add native registration and conflict handling |
| `storageLimits` | Storage limit | blocked | No | Yes | None | No enforcement or deletion policy | Implement enforcement, warnings, and non-destructive recovery |
| `recordingNotice` | Optional recording notice | hidden | No | Yes | Research only | Not a launch priority; must never block recording | Prove active-chat targeting, one-send behavior, and fallback |

## Dictation

| Capability key | Surface | State | Production | Preview | Support | Known limitation | Required verification / reopening condition |
|---|---|---:|---:|---:|---|---|---|
| `dictationSystem` | System-wide dictation and enable control | shipped | Yes | Yes | Native recorder, global shortcut, paste | Apple Silicon/macOS target | Native smoke test |
| `appleSpeechAnalyzer` | Apple Speech — System engine | blocked | No | No | Architecture decision and public implementation research only | Requires macOS 26+, a supported locale and OS-managed assets; not yet connected to the provider contract | Add SDK/runtime stub bridge, asset progress/cancel, Whisper fallback, formatting tests, and product-specific quality/performance benchmarks |
| `curatedSpeechPicker` | On-device speech model library | shipped | Yes | Yes | Twelve revision-pinned local models; size and SHA-256 verification; resumable download/cancel/remove/select lifecycle; active-model routing; model-specific language, translation, output, and preprocessing; production UI contract | Model quality labels are guidance, not universal rankings. Downloads are user-initiated and consume disk space; larger models need more RAM and decode time | Native download/cancel/switch/delete smoke test on a clean Mac and representative Apple Silicon memory/latency measurements |
| `customVocabulary` | Custom vocabulary | shipped | Yes | Yes | Shared transcription prompt | Prompt-size cap | Config application test |
| `languagePunctuation` | Language and punctuation | shipped | Yes | Yes | Server preserves Automatic or applies the explicit decoder language | Automatic output script follows the selected model; never silently rewritten to English | Dictation request and automatic-language regression tests |
| `clipboardBehavior` | Clipboard behavior | shipped | Yes | Yes | Native restore/retain paths | Paste target controls final formatting | Native behavior test |
| `dictationOverlay` | Overlay and position | shipped | Yes | Yes | Native all-spaces pill | Fixed set of positions | Native command and visibility test |
| `soundVolume` | Sound and volume | shipped | Yes | Yes | Native cues | System sound assets | Native config test |
| `dictationShortcut` | Shortcut configuration | shipped | Yes | Yes | Carbon combination or bare modifier | Limited supported key set | Registration and invalid-input tests |
| `recentDictations` | Recent dictations | shipped | Yes | Yes | In-memory session list | Not persisted by design | Session feed test |
| `smartPolish` | Experimental Smart Polish | shipped | Yes | Yes | Off by default; cleanup timing defaults to After I finish; live speech models may mechanically clean only committed prefixes; Gemma remains final-only; optional Gemma 3 270M/1B/4B via authenticated loopback llama.cpp; strict lossless validator; mechanical fallback; visual diff; target-aware handling | Model quality varies by language and hardware; tentative live words are never mechanically rewritten; secure fields never enter history or model inference | ≥250 exact golden cases, committed/tentative boundary tests, property tests, real 270M runtime smoke, download lifecycle, target/focus, and packaged physical acceptance |
| `dictationUpload` | Upload voice note | preview | No | Yes | None | Placeholder action | Implement file import, transcription, and recovery |
| `documentationLink` | Documentation link | hidden | No | Yes | None | No real destination | Publish and verify the destination |

## Settings

| Capability key | Rows / panels | State | Production | Preview | Support | Known limitation | Required verification / reopening condition |
|---|---|---:|---:|---:|---|---|---|
| `appearance` | Theme | shipped | Yes | Yes | UI handler plus durable mirror | None | Handler contract |
| `accent` | Accent color | shipped | Yes | Yes | UI handler plus durable mirror | None | Handler and contrast check |
| `trayVisibility` | Tray visibility | shipped | Yes | Yes | Native command | Desktop only | Tauri command smoke test |
| `dictationOverlay` | Dictation overlay and position | shipped | Yes | Yes | Native command | Desktop only | Tauri command smoke test |
| `screenShareInvisibility` | Invisible to screen-share | shipped | Yes | Yes | Native content protection | Desktop only | Tauri command smoke test |
| `permissions` | Corrected permission rows | shipped | Yes | Yes | Live Accessibility/Screen Recording status; first-use guidance for Mic/System Audio | macOS has no reliable app-level live System Audio status here | Native status plus recovery-link test |
| `liveMeetingIndicator` | Live meeting indicator | shipped | Yes | Yes | UI handler | None | Handler contract |
| `automaticMeetingDetection` | Automatic meeting detection | shipped | Yes | Yes | OS adapter emits bounded raw observations; shared Rust owns hysteresis and source policy; browser/chat apps require audio plus meeting-window evidence | Real platform/version acceptance matrix remains incomplete | Rust detector/contract tests, helper evidence self-test, and platform acceptance tests |
| `deeperMeetingReasoning` | Deeper note reasoning | shipped | Yes | Yes | Summary request option | Slower generation | Request fixture |
| `transcriptionLanguage` | Transcription language | shipped | Yes | Yes | Capture config | Supported language list | Config test |
| `translation` | Translate meeting transcripts to English | shipped | Yes | Yes | Shown only for multilingual Whisper models; native translation task; UI and Rust both fail closed after an incompatible model switch | Translation is model-generated and can lose nuance; unavailable for English-only, Hinglish, Parakeet, and Nemotron models | Native multilingual-to-English smoke test plus representative FLEURS/product quality measurements |
| `videoRecording` | Video recording | shipped | Yes | Yes | ScreenCaptureKit path | Optional Screen Recording permission | Native capture verification |
| `meetingTemplates` | Meeting templates | shipped | Yes | Yes | Durable templates in summary prompt | Custom templates are local only | Prompt fixture |
| `aiProvider` | AI provider and immediate privacy statement | shipped | Yes | Yes | Optional separately installed LiteRT-LM local runtime or Groq/OpenAI/Cerebras/custom OpenAI-compatible providers | The DMG does not install the local Intelligence runtime/model; clean beta Macs need BYOK or separate setup for notes | Provider switch/privacy-copy tests plus a verified in-app local install/status/remove lifecycle |
| `unloadModel` | Unload model when idle and timeout (0/2/5/10/15/30/60 min) | shipped | Yes | Yes | Server handler; 10-minute default | Applies to app-started model | Handler and production-surface contract |
| `profileAccount` | Profile, email, plan, upgrade, sign out | hidden | No | Yes | Local placeholder only | No account system | Reduce to honest local identity or implement account system |
| `startupBehavior` | Start at login and start hidden | blocked | No | Yes | None | No native launch behavior | Implement native behavior and relaunch verification |
| `updater` | Check for updates | blocked | No | Yes | None | No updater | Add signed updater and failure recovery |
| `writingSettings` | Writing Assistant panel | preview | No | Yes | Stored design state only | No writing runtime | Ship the Writing Assistant first |
| `meetingSource` | Meeting source selector | blocked | No | Yes | Local only | Google/Fathom unavailable | Ship and verify at least one external source |
| `calendarView` | Default Calendar view | preview | No | Yes | No calendar view | None | Ship calendar view and source |
| `folders` | Folder setting | preview | No | Yes | None | No persistence | Ship folder behavior |
| `storageLimits` | Audio storage | blocked | No | Yes | None | No enforcement | Ship storage enforcement and recovery |
| `meetingShortcut` | Global meeting shortcut | blocked | No | Yes | None | No native registration | Ship registration and conflicts |
| `whatsNew` | What’s new | blocked | No | Yes | None | No release-note behavior | Add versioned release notes and test once-only opening |
| `experimental` | Experimental panel | preview | No | Yes | Design state only | Individual experiments incomplete | Gate every experiment independently |
| `supportCreator` | Support creator | hidden | No | Yes | No destination | Placeholder toast | Wire a real destination |

## Accessibility contract

`accessibilitySemantics` is `shipped`. Production requires native buttons for actions, tab semantics for
meeting detail and call-panel tabs, keyboard activation, visible existing-style focus, modal Escape and
focus restoration, and polite recording-state announcements. The visual CSS system remains unchanged.

## Release gate

A production build is releasable only when:

- every visible capability is `shipped` and names verification evidence;
- every visible CTA performs its advertised action and every visible setting has an application handler;
- no enabled production action resolves to “coming soon” (the disabled Agents navigation label is not an action);
- hidden routes are sanitized to Meetings and local state cannot promote a capability;
- transcript and participant mistakes are repairable and corrections stale derived notes visibly;
- privacy copy updates with the selected processing mode;
- permission labels and recovery instructions match macOS behavior;
- new meetings have recognizable titles and substantive calls queue notes;
- summary failure preserves the transcript and offers retry;
- Meetings → detail → Transcript → Summary → Export → Settings works by keyboard and with VoiceOver;
- production and preview contract tests pass; and
- the existing visual language is preserved.
