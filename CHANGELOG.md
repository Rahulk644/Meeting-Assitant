# Changelog

All notable user-visible changes are recorded here. The project follows the structure of Keep a Changelog;
versions will use semantic versioning once the first public beta tag is cut.

## Unreleased

### Added

- Evidence-backed Zoom speaker names: each caption retains its real audio interval and the Rust meeting core
  matches it against a bounded, self-filtered active-speaker timeline. Trusted matches persist their
  provenance/confidence; stale, conflicting, and unsupported cases stay **Unknown speaker**.
- A native in-meeting Nub that expands into an adaptive live conversation surface with speaker-grouped
  transcript bubbles, honest capture-source status, and a direct **What did I miss?** action.
- Capability-gated live Dictation for Parakeet Unified and the two Nemotron streaming models. Provisional
  words appear directly in compatible focused text fields, and the exact tracked span is reconciled to the
  authoritative final transcript after cleanup and Smart Polish.
- Experimental **Smart Polish** for Dictation, off by default: deterministic English/Hindi/Hinglish cleanup,
  spoken punctuation, conservative number formatting, approved filler removal, custom vocabulary,
  protected spans, target-aware behavior, and a strict lossless model-output validator.
- A default **After I finish** cleanup-timing control for streaming speech models, plus an experimental
  **While I speak** mode that mechanically cleans only committed phrases. Tentative words remain untouched,
  and Gemma/Smart Polish continues to run only after Stop.
- A separate local polish-model library for Gemma 3 270M, 1B, and 4B with explicit download, resume,
  progress, cancel, exact-size/SHA-256 verification, atomic install, select, and remove flows. Model weights
  remain outside the app and DMG.
- A pinned static llama.cpp runtime with authenticated loopback-only access, one inference slot, recording-time
  warm-up, a twelve-second cold-path cap, mechanical fallback, and 60-second idle unload.
- Native Dictation states for Post-processing, visual changes, Inserted, safe fallback, no speech, and error; rich
  recent-dictation diffs stay in memory for the current session only.
- A minimal Accessibility helper that classifies the focused target without walking its tree. Secure fields
  skip model inference and never retain transcript, original text, or diffs.
- Deterministic cleanup, idempotence, protected-span, validator, model-catalog, and native pill regression
  coverage.

### Changed

- The Meetings Beta overlay now keeps recording controls secondary to the live transcript and catch-up
  action, grows vertically for recent speech, and preserves its screen edge while resizing.
- Local release candidates default to the honest `unreleased` artifact label; a named beta label must be
  supplied by the release operator or CI.
- Meeting Intelligence now states clearly that its local LiteRT-LM runtime is a separate installation, while
  meeting audio and transcription remain on-device for every current provider choice.
- Agents is now only a greyed-out, non-clickable **Coming soon** navigation label. Production exposes no
  Agent route, details, settings, shortcut, compute lane, or packaged Agent helper.
- Meetings remains explicitly labelled **Beta**.
- Final-only speech models retain the proven single-paste path. Streaming-capable models revise only their
  tracked provisional span in the focused editor and safely fall back to copying if reconciliation fails.
- The separate 50 MB live-preview model and its settings/download surface were removed. Whisper remains
  final-only; live words are available only from selected models with genuine streaming support.
- The native pill stays compact and status-only during recording and transcription. It never contains the
  dictated transcript; it may still show one concise before/after Smart Polish change after processing.
- When live text is already visible, Stop now shows **Post-processing…** for the complete reconciliation and
  cleanup wait instead of showing a long second **Transcribing…** state followed by a near-instant polish.
- The native pill uses semantic macOS colors, supports dark/high-contrast appearance, follows the active
  display, remains non-activating, and can show one concise before/after change.
- Production builds now use one native-resource pipeline, force the production release channel, require the
  microphone entitlement, and verify every signed executable and the mounted DMG before handoff.
- CI now stages the same production resources as the release, pins third-party actions by commit, runs with
  least-privilege permissions, and receives weekly dependency update proposals.

### Fixed

- The developer launcher now owns and reaps only the child processes it starts, refuses occupied ports, uses
  private repository logs, and no longer kills or leaks another worktree's UI/STT session.
- The browser developer harness now implements provider-key and model-catalog routes used by the shared UI;
  keys remain memory-only and the UI no longer falsely claims they were written to Keychain outside Tauri.
- Live Parakeet revisions now use verified real text-input events and reacquire the same focused composer
  after web/Electron re-renders. Editors such as Codex no longer discard apparent Accessibility edits, and
  final reconciliation no longer degrades to a clipboard-only result in that case.
- Parakeet Unified now uses its trained 560 ms chunk/lookahead streaming profile, cutting the first visible
  partial-text delay roughly in half while retaining the same 5.6-second left context and exact final pass.
- The install/run workflow now byte-verifies `/Applications/My Assistant.app`, unregisters and removes the
  transient build-tree `.app`, and re-registers only the canonical installation so macOS cannot launch an
  older indexed build by mistake.
- A rollback-safe candidate installer now verifies and mounts the DMG, installs that exact app rather than a
  second rebuild, smoke-tests its authenticated local service, and removes the transient build-tree bundle.
- Exact `<transcript>…</transcript>` envelopes mirrored by Gemma 3 270M are safely unwrapped before strict
  validation; commentary, code fences, malformed envelopes, and prompt-injection output still fail closed.
- A completed dictation no longer holds the serialized finish lock during its success/error animation, so a
  following dictation can finish promptly.
- The validator rejects semantic one-letter substitutions while still allowing adjacent transpositions and
  a small explicit list of common misspellings.
- Smart Polish model libraries now stay visibly layered above their dimmed settings backdrop in the
  packaged WebView.

### Security

- Polish-model output is treated as untrusted data and cannot add, remove, reorder, translate, change
  protected values/numbers, or bypass the deterministic fallback.
- The polish runtime binds to loopback, requires the per-launch secret, disables UI/slot/cache endpoints,
  runs offline, and rejects an occupied port rather than trusting another process.
- Replaced the external Codex runtime's spoofable signature-text check with a native Security.framework
  requirement anchored to Apple, the `codex` identifier, OpenAI's Developer ID certificate, and Team ID
  `2DC432GLL2`. Cached runtime paths are revalidated immediately before every process launch with a no-follow
  file handle held across spawn, and the actual child process is validated before RPC; symlinks and replaced
  files fail closed.

## [0.1.0-beta.2] - 2026-07-15

### Added

- Native AppKit dictation status pill with listening and transcribing states.
- Real-call meeting-start notifications driven by Core Audio call edges.
- Two-second atomic transcript checkpoints and interrupted-meeting recovery.
- Startup validation for packaged release capabilities, preventing missing or corrupt configuration from collapsing core navigation.
- Per-launch WebView URLs prevent WKWebView from restoring a stale blank or partially hidden interface from an older build.
- Keychain-backed provider credentials and authenticated loopback routes.
- Curated competitive-intelligence and product-release authority documents.
- Conservative browser and communication-app call detection using active per-app audio plus meeting-window
  evidence; universal manual recording remains available for any platform.
- Native first-frame meeting thumbnails and installed-app icon/platform-tile fallbacks.
- Reproducible, revision- and hash-pinned speech qualification across mixed-script Hinglish, Roman Hinglish,
  ordinary English, Hindi-to-English translation, exact product preprocessing, and deterministic synthetic
  noise, with private-cache data boundaries and aggregate OSS evidence.
- Resumable three-trial 500-clip speech matrices with model-order randomization, variance, hypothesis
  stability, and an explicit per-model `vad-compact` / `vad-gate` audio policy.
- Blind human Roman annotation packs, consent-attested field-noise capture/import tooling, and fail-closed
  double-review validation; private audio and transcript data remain outside Git.
- Machine-readable physical Right-Command and two-person meeting acceptance matrices with evidence-backed,
  private session recording.
- Sanitized full-set evidence from three complete 500-clip Whisper trials; transcript-bearing reports stay
  private and the committed aggregate records hashes, variance, preprocessing, and workload caveats.
- A curated on-device speech-model library shared by Dictation and Meetings: Whisper speed/quality tiers,
  Parakeet Unified English, both NVIDIA Nemotron streaming models, and Oriserve Apex/Prime for Roman
  Hinglish. Every entry shows size, best-for guidance, abilities, license, and installed/download state.
- Per-model language, translation, output-style, and audio-policy controls. Incompatible saved preferences
  are preserved as user intent but fail closed at inference time.
- An honest Agents foundation view plus a signed, on-demand native Accessibility inspector. It returns a
  bounded read-only snapshot, redacts secure fields, and exits without screenshot, network, persistence, or
  computer actions.
- Agent compute configuration for Local, API key, and ChatGPT account lanes. BYOK credentials go directly
  to Keychain; provider model IDs load on demand. The ChatGPT lane detects a verified official external Codex
  runtime, uses an isolated profile/device-code sign-in, and renders account-visible model/effort controls
  without adding Codex or Node to the DMG.
- A preview-only structured Agent draft planner for API-key and ChatGPT account compute plus a preview-only
  configurable global Agent shortcut.
- A separate preview-only native Agent text executor: explicit Apply, exact process/app/window/field/content/
  selection revalidation, verified draft insertion or selection replacement, verified rollback, and one
  guarded memory-only Undo. Executable Apply is deliberately restricted to a named disposable TextEdit
  document while cross-app inspection/drafting remains read-only. The model has no execution tool, and Send,
  Return, click, shell, file, search, and multi-step actions remain absent.
- Evidence-backed Agent app and wake-word acceptance suites. Porcupine is the leading external wake-word
  candidate; microWakeWord and sherpa-onnx KWS are the modern open controls. No engine, model, or credential is bundled.

### Changed

- Right Command is the first-run dictation shortcut.
- The native dictation pill now states the current phase, elapsed time, configured finish shortcut, and Esc
  cancellation instead of showing an unexplained waveform.
- The packaged application uses an in-process Rust service instead of bundling Node.
- External Codex detection is lazy: ordinary launch and the Local/API Agent lanes do not create a Codex
  profile or process; the check begins only after the ChatGPT · Codex lane is opened.
- Speech-model residency defaults to a user-configurable 10-minute idle timeout.
- First-run model copy now states that cloud speech is unavailable in the current beta.
- Scheduled/calendar titles now outrank meeting-window titles; unscheduled calls are labelled
  `Impromptu meeting` instead of guessing a title from transcript text.
- The DMG start-here guide now documents the current macOS Privacy & Security → Open Anyway flow.
- Translation is shown only while Whisper Medium multilingual or Whisper Large v3 is active; Turbo,
  English-only, Parakeet, Nemotron, and Oriserve models remain transcription-only.
- Speech preprocessing is selected per model: Whisper compacts VAD speech frames, while Parakeet,
  Nemotron, and Oriserve retain utterance context and use VAD only as a speech gate.
- Qwen is excluded from the user model library and further promotion benchmarking because of its current
  memory/output trade-offs. Its completed measurements remain as historical engineering evidence.

### Fixed

- Meetings without screen recordings now show a platform identity thumbnail in both the shipped app and the
  developer browser harness instead of a broken image in the harness.
- The release registry now covers every capability named by the release matrix, with a regression test that
  prevents documentation and production gating from drifting apart.
- Rust now passes a warnings-denied Clippy gate, and CI enforces it.
- Rapid second-press dictation now transitions to Transcribing instead of remaining stuck on Listening.
- The obsolete WebKit pill lifecycle that could crash after dictation has been removed.
- Native helpers now terminate when the parent application exits unexpectedly.
- Existing speech models can be migrated into the current Application Support location.
- A loopback-port conflict now exits cleanly before creating a window or helper instead of leaving an
  unreachable background process.
- Native transcription now preserves valid partial output when a model reaches its token limit, reports the
  truncation, and guarantees valid UTF-8 JSON for byte-limited Hindi text.
- Automatic dictation language now reaches the decoder unchanged instead of being silently forced to English.
- Qwen decoding now uses a bounded 512-token budget with repeated-cycle termination, preventing runaway
  loops while retaining valid partial speech.
- ChatGPT device-code status polling no longer starts competing Codex processes against the active login
  flow; verified runtime detection and version metadata are cached for the app process.
- Provider settings no longer decrypt Keychain credentials during startup. Explicit provider use performs
  one serialized Keychain read per app process—even when requests overlap—and reuses only a volatile
  process-memory copy, preventing repeated authorization dialogs and hidden-window startup stalls.
- The preview Agent shortcut now moves the dashboard to the active Space, routes through a durable WebView
  entry point, and focuses the request field after hidden-window restoration.
- Preview TextEdit Apply now rejects a proposal if another ordinary app was opened behind the dashboard
  after capture; stale text and selection checks remain exact and fail closed.
- A recreated native window now reapplies both enabled and disabled screen-share privacy state instead of
  retaining stale protection from a previous window.

### Security

- Provider secrets are stored in macOS Keychain and excluded from settings, logs, temporary files, and
  process arguments.
- Model downloads are size- and SHA-256-verified before atomic installation.
- Local UI, helper, STT, host, origin, filename, and request-size boundaries are fail-closed.
- Screen-derived Agent context is explicit, local, bounded, treated as untrusted data, and unavailable to
  future mutation paths without snapshot revalidation and confirmation policy.
- Legacy or malformed Agent compute settings are reduced to an allowlisted non-secret schema. Official Codex
  OAuth state remains owned by its isolated external profile and can be cleared through App Server logout.
