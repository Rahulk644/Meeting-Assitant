# Changelog

All notable user-visible changes are recorded here. The project follows the structure of Keep a Changelog;
versions will use semantic versioning once the first public beta tag is cut.

## Unreleased

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

### Changed

- Right Command is the first-run dictation shortcut.
- The native dictation pill now states the current phase, elapsed time, configured finish shortcut, and Esc
  cancellation instead of showing an unexplained waveform.
- The packaged application uses an in-process Rust service instead of bundling Node.
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

### Security

- Provider secrets are stored in macOS Keychain and excluded from settings, logs, temporary files, and
  process arguments.
- Model downloads are size- and SHA-256-verified before atomic installation.
- Local UI, helper, STT, host, origin, filename, and request-size boundaries are fail-closed.
