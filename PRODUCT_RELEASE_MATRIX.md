# Product Release Matrix

This file is the release authority for My Assistant. It separates the complete product design from the
surface that is safe to expose in a production build. The full design remains in `app/index.html`; the
server-owned registry in `app/release.js` decides what the browser may render.

## Release channels

- `ASSISTANT_RELEASE_CHANNEL=production` — default for every packaged and ordinary local run. Unknown
  capabilities fail closed.
- `ASSISTANT_RELEASE_CHANNEL=preview` — internal design review only. It preserves the complete designed
  surface, including unfinished and blocked work. It cannot be enabled by URL or a saved UI preference.

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
| `dictation` | Dictation navigation and view | shipped | Yes | Yes | Native global shortcut, mic capture, STT, paste | Session history only | Native hotkey, paste, and settings contract |
| `meetings` | Meetings navigation and view | shipped | Yes | Yes | Automatic/manual capture, SSE, local meeting store | Automatic browser/chat detection requires both active audio and meeting-window evidence | Recording lifecycle and meeting-library tests |
| `writingAssistant` | Writing Assistant navigation and view | preview | No | Yes | Design only | No system-wide writing runtime | Implement native selection/action runtime and test every action |
| `agentFoundation` | Agents navigation and honest foundation view | shipped | Yes | Yes | Native read-only context status and roadmap | No model planning or computer actions | Production-surface contract and packaged release-payload verification |
| `agentContextInspector` | On-demand screen-context inspection | shipped | Yes | Yes | Bounded native Accessibility snapshot; secure-field redaction; no screenshot/network/persistence | Reads the active or topmost app behind My Assistant; full multi-app matrix and OCR/vision fallback remain | Swift self-test, Rust fail-closed test, direct Chrome snapshot smoke, and signed-resource verification |
| `agentComputeConfig` | Agent compute source and model controls | shipped | Yes | Yes | Optional local lane disclosure; Keychain BYOK provider/model discovery; externally installed official Codex detection, isolated profile, device-code sign-in, and account-visible model controls | Local Agent runtime has no install lifecycle; configuration alone does not run or mutate an Agent | Runtime-signature, secret-sanitization, model-discovery, and production-surface contract tests |
| `agentDraftPreview` | Structured draft planner | preview | No | Yes | API-key and ChatGPT account/Codex lanes; fresh bounded Accessibility context; typed draft/replace/no-op result; planner has no execution tool | Preview requires a configured compute lane; real-app matrix remains incomplete | Pass `agent-context` acceptance suite, prompt-injection controls, provider error/retry, and stale-snapshot tests before production |
| `agentActions` | Guarded Apply and one-step Undo | preview | No | Yes | Separate Swift AX executor; explicit Apply click; exact process/bundle/window/role/label/content/selection revalidation; verified mutation or verified rollback; five-minute in-memory undo | Executable Apply is limited to a named disposable TextEdit document; other apps remain read-only; proposal expires after two minutes; no Send/Return/click/multi-step action | Pass the TextEdit isolation/rollback matrix, then require a separate synthetic acceptance gate before adding each app adapter |
| `agentGlobalShortcut` | Configurable global Agent shortcut | preview | No | Yes | Native global-shortcut registration opens Agents and focuses the request field | Physical conflicts, relaunch restoration, focus, Spaces, and multi-display behavior remain unverified | Pass AGT-012 on a packaged build and add conflict/relaunch evidence |
| `wakeWord` | Optional hands-free Agent activation | blocked | No | No | Research and acceptance contract only | Porcupine needs a user/vendor AccessKey and commercial-term review; microWakeWord is embedded-first; sherpa-onnx is heavier and tuning-sensitive; no engine, model, or secret is bundled | Pass the identical Porcupine, microWakeWord, and sherpa-onnx benchmark plus privacy, suspension, resource, integration, and licensing gates |
| `agents` | Autonomous Agent action runtime | preview | No | Yes | Single explicit text draft/replace/undo chokepoint exists behind `agentActions` | No Send, click, wake phrase, multi-step action, audit history, OCR/vision fallback, or broad real-app acceptance | Complete the packaged action matrix before adding any new action class |
| `feedbackSupport` | Feedback & Support | hidden | No | Yes | No destinations | Cards do not open real support targets | Wire and verify every destination |
| `guide` | Guide | hidden | No | Yes | No published guide | Placeholder content | Publish a real guide and verify link handling |
| `shareFriend` | Share with a friend | hidden | No | Yes | No share destination | Placeholder content | Add a real share action and verify it |
| `settings` | Settings navigation and view | shipped | Yes | Yes | Server-mirrored preferences plus native handlers | Only verified rows render in production | Production setting-handler contract |
| `pro` | Pro card and commercial claims | blocked | No | Yes | None | No billing, entitlement, cloud plan, or team sharing | Complete billing and entitlement verification |

## Meetings

| Capability key | Surface | State | Production | Preview | Support | Known limitation | Required verification / reopening condition |
|---|---|---:|---:|---:|---|---|---|
| `meetingRecord` | Record | shipped | Yes | Yes | Manual recording gate | Requires macOS capture permissions | Manual start and finalize tests |
| `automaticRecording` | Automatic recording, enabled by default | shipped | Yes | Yes | Active per-app Core Audio; browser/chat apps additionally require meeting-window evidence | Real two-person validation remains incomplete across the full advertised platform/version matrix; no bot joins | Call-edge, source-authority, browser-setting, and per-platform acceptance tests |
| `meetingDetectedNotification` | Native meeting-start notification | shipped | Yes | Yes | Emitted only on a Core Audio false-to-true call edge | First use may request macOS notification permission | Transition test plus native notification smoke test |
| `liveTranscript` | Live transcript | shipped | Yes | Yes | On-device Whisper plus SSE | Attribution depends on available participant evidence | Capture/SSE smoke test |
| `callPanel` | Floating call panel | shipped | Yes | Yes | Mirrors recording state | Main-window panel; screen-share safety is a native setting | Recording-state UI contract |
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
| `automaticMeetingDetection` | Automatic meeting detection | shipped | Yes | Yes | Dedicated apps use active audio; browser/chat apps also require meeting-window evidence | Real platform/version acceptance matrix remains incomplete | Gate, helper self-test, and platform acceptance tests |
| `deeperMeetingReasoning` | Deeper note reasoning | shipped | Yes | Yes | Summary request option | Slower generation | Request fixture |
| `transcriptionLanguage` | Transcription language | shipped | Yes | Yes | Capture config | Supported language list | Config test |
| `translation` | Translate meeting transcripts to English | shipped | Yes | Yes | Shown only for multilingual Whisper models; native translation task; UI and Rust both fail closed after an incompatible model switch | Translation is model-generated and can lose nuance; unavailable for English-only, Hinglish, Parakeet, and Nemotron models | Native multilingual-to-English smoke test plus representative FLEURS/product quality measurements |
| `videoRecording` | Video recording | shipped | Yes | Yes | ScreenCaptureKit path | Optional Screen Recording permission | Native capture verification |
| `meetingTemplates` | Meeting templates | shipped | Yes | Yes | Durable templates in summary prompt | Custom templates are local only | Prompt fixture |
| `aiProvider` | AI provider and immediate privacy statement | shipped | Yes | Yes | Preinstalled LiteRT-LM local runtime or Groq/OpenAI/Cerebras/custom OpenAI-compatible providers | The DMG does not yet install the optional local Intelligence model; clean beta Macs need BYOK for notes | Provider switch/privacy-copy tests plus an in-app local install/status/remove lifecycle |
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
- no production action resolves to “coming soon”;
- hidden routes are sanitized to Meetings and local state cannot promote a capability;
- transcript and participant mistakes are repairable and corrections stale derived notes visibly;
- privacy copy updates with the selected processing mode;
- permission labels and recovery instructions match macOS behavior;
- new meetings have recognizable titles and substantive calls queue notes;
- summary failure preserves the transcript and offers retry;
- Meetings → detail → Transcript → Summary → Export → Settings works by keyboard and with VoiceOver;
- production and preview contract tests pass; and
- the existing visual language is preserved.
