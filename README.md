# My Assistant

A private, **on-device** assistant for calls and text. The complete UI preserves the product vision;
**Dictation is the primary ready feature and Meetings ships as a beta.** Meetings provides real-time transcription
of *you and the other participants* across dedicated meeting apps and supported browser/chat calls, with best-effort speaker names. The selected speech model runs locally on the Apple GPU — **no bot joins
the call, no audio leaves your machine, no meeting-app captions toggle.**

**Status:** the Meetings module works end-to-end and ships as a **double-click desktop app**
([Tauri](https://tauri.app), `desktop/`) — no terminal, no dev commands. It:

- **detects Zoom, Teams, Webex, FaceTime, Google Meet, Slack huddles, Discord, and other supported calls**
  and transcribes *you and the other participants*, with **live speaker names**, capturing **only while
  you're in a call** — or **start / pause / stop by hand**. Manual recording works with any platform;
- **lets each user choose a local speech model** for their Mac, including Whisper speed/quality tiers,
  Parakeet and both Nemotron English/multilingual options, plus Oriserve Apex/Prime for Roman Hinglish;
- **adapts language and output controls to the active model**; English translation appears only for the
  translation-capable Whisper Medium multilingual and Large v3 options;
- **saves every call on-device** into a browsable history with **post-call summaries, action items, and
  "ask this meeting"**;
- can **screen-record** the call (whole call, or only while a screen is shared) and shows it in the
  meeting, with a natively generated frame as the card thumbnail; without video, the card uses the
  installed meeting app's icon or a platform identity tile;
- shows a **floating in-call panel** (live transcript + a scratchpad that saves to the meeting and feeds
  the summary, drag or minimize it) and a **menu-bar recording indicator**, and can make its window
  **invisible to screen sharing**;
- sends a native notification only after Core Audio confirms a real supported call; simply leaving a
  meeting app open does not produce a meeting notification.

The same complete UI remains available in an internal design-preview channel. Production visibility is
governed by [`PRODUCT_RELEASE_MATRIX.md`](PRODUCT_RELEASE_MATRIX.md); designed surfaces are not treated as shipped.
The long-term product, provider, model, monetization, and quality decisions live in
[`PRODUCT_STRATEGY.md`](PRODUCT_STRATEGY.md). Start or resume engineering work from
[`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md).

## How it works

```
 you (mic, AVAudioEngine)  ┐                         detect the call + capture, on the Apple GPU
 the other participants'   ├─▶  stt/meeting-tap (Swift)
 audio (Core Audio tap)    ┘      │ dedicated apps: Core Audio; browser/chat: audio + meeting-window evidence
                                  │ energy-VAD splits speech into per-utterance WAVs
                                  ▼
                    transcribe-server (transcribe.cpp + Metal GPU · selected local speech model)
                    on demand — started when a call begins, unloads after a user-set idle timeout (10 min default)
                                  │ transcript  (POST /ingest)
                                  ▼
                    Rust local_server ──Server-Sent Events──▶ authenticated 127.0.0.1:8790 UI
                                  ▲
                    meeting-state (Swift, Accessibility) ─┘  speaker names, mute/share state
```

- **Detect + capture** — `meeting-tap` finds the call from the **Core Audio process list** (which app is
  doing audio I/O — **no permission**), then captures the meeting app's audio with a **Core Audio process
  tap** (the *System Audio Recording Only* permission) **and** your mic, splitting each into utterances.
  ScreenCaptureKit is used **only** when you record video.
- **Transcribe** — `transcribe-server` (started on demand) transcribes each whole utterance on the GPU.
  Language, translation availability, output style, and audio preprocessing resolve from the active model;
  a user preference is preserved when switching through an incompatible model.
- **Names** — only after Core Audio confirms a real call, `meeting-state` wakes and reads Zoom's
  Accessibility tree for the active speaker (and mute/share); the Rust server stamps each remote line with
  who's speaking. Merely leaving Zoom open does no Accessibility scanning and cannot start a meeting.
- **Display** — the Tauri process owns a small Rust SSE/API server and streams captions to the minified
  `app/index.html` UI. A per-launch session secret protects every local route; no Node runtime is shipped.

## Transcription

**Settings → Dictation → Speech model** opens the local model library. Every download is user-initiated,
revision-pinned, size- and SHA-256-verified, resumable, cancellable, removable, and stored outside the app.
The active model is shared by dictation and meetings.

Dictation and Meetings expose automatic or explicitly pinned transcription language:

- **Transcription language** *(detection)* — **`Auto-detect` by default**, so mixed Hindi + English is
  recognized per utterance. Pin a language only if detection ever slips.

Translation is a distinct decoder task, not a language preference. It appears only when Whisper Medium
multilingual or Large v3 is active. Turbo, English-only Whisper, Parakeet, Nemotron, and the Roman-Hinglish
fine-tunes fail closed even if an old saved translation preference exists.

**Settings → Advanced** controls model residency: leave **Unload model when idle** on and choose the
timeout (immediately, 2/5/10/15/30/60 minutes). The default is **10 minutes** for fast back-to-back use.

So out of the box: **auto-detect → transcribe as spoken.**

Quality comes first from **transcribing whole utterances** — the capturer waits for a natural pause and
sends the engine the *entire* sentence, never a fixed-length slice. (Chopping mid-sentence was what made it
hallucinate — the model needs the surrounding words for context.)

The engine is **[transcribe.cpp](https://github.com/handy-computer/transcribe.cpp)** on Metal — the same
engine family [Handy](https://handy.computer) uses. On identical audio and model it measured **~2× faster
and ~half the RAM** of whisper.cpp in this project, with cleaner output (proper
punctuation, no dropped word-onsets). It runs behind `transcribe-server`, a tiny HTTP wrapper that holds
the model hot; the build is `stt/transcribe-server/build.sh` (pinned commit, Metal). whisper.cpp is kept
as an automatic fallback if that isn't built. Capture (mic, system audio, video) runs **only while you're
in a call**.

## Dictation

Press **Right Command** anywhere on your Mac, speak, and press it again — the words are transcribed on-device and
pasted straight into whatever text field is focused, **including full-screen apps**. It uses the same
on-device transcription as Meetings, so nothing leaves your machine.

- **Tap-to-toggle** by default — press to start, press to stop; or switch to **hold-to-talk** in
  **Settings → Dictation**. The shortcut is **rebindable**, and **Esc** cancels a recording.
- **Silence-aware** — a neural voice-activity detector (**Silero VAD**, bundled, loaded once and reset
  between clips like Handy) strips the quiet around your speech before transcription, so pauses and stray
  recordings don't turn into gibberish.
- A native **pill** states whether it is listening or polishing, shows elapsed time, names the configured
  finish shortcut, and keeps Esc cancellation visible. It floats above every app—even full-screen ones—
  without stealing focus. Turn it off, or reposition it, in **Settings → General**.
- **Every Settings → Dictation control is live** — the start/stop sound + feedback volume, clipboard
  behaviour, Automatic/Off punctuation, and the rebindable shortcuts all do what they say. **Custom
  Vocabulary** and language are shared with Meetings.

> **One-time setup:** on your first dictation macOS prompts once for **Microphone** — allow it.
> **Accessibility** (already granted from Meetings) is what lets it paste. That's all — **no Input
> Monitoring, no function-key changes** (the Carbon global shortcut needs no permission).

## Post-call AI

Every meeting is saved to `data/meetings/*.json` (on-device, git-ignored). Open one from the **Library**
for an AI **summary + action items**, and to **ask questions** about it. Rename a meeting inline. The
model is pluggable across three tiers (Settings → *AI Model*):

- **Local** *(private, optional in this beta)* — **Gemma 4 E2B** (text-only) via **LiteRT-LM** on the Apple
  GPU. It runs at *effective-2B* (**~1 GB RAM**, 2.4 GB disk), starts on demand, and unloads when idle. The
  source setup installs it, but the current DMG does **not** yet provide an in-app Intelligence-model
  installer; a clean beta Mac must use a configured BYOK provider for notes until that flow ships.
- **API key** — bring your own, any OpenAI-compatible provider (Groq / OpenAI / Cerebras …).
- **Cloud** — hosted (Pro), later.

## Screen recording

Optionally capture the call's **video** — **Settings → Meetings → Screen recording**: **Off** *(default)*,
**Always**, or **Only when shared**. Recordings are H.264 `.mp4` in `data/meetings/rec/` (on-device,
git-ignored); they play back in the meeting's detail view. The recorder writes a downscaled first-frame
JPEG itself, so thumbnails do not depend on `ffmpeg` being installed. Meetings without video show the
installed native app icon or a lightweight platform identity tile.

Tuned to stay light on long calls: capture is **downscaled to ≤1080p** and encoded by the **hardware
H.264 encoder** (VideoToolbox) at **10 fps, ≤4 Mbps** — screen content is mostly static, so this looks
fine and keeps files small. The writer **flushes a fragment every 10 s**, so RAM stays flat and a file
survives a hard crash (playable up to the last fragment). Whisper and the LLM are **on-demand** (loaded
only when needed, unloaded when idle) and run on the **Apple GPU (Metal + flash-attention)** — so nothing
heavy sits resident between or during calls beyond what's actively transcribing.

## Install the beta

The beta tester experience does not require Terminal, Xcode, Homebrew, Node, or Rust:

1. Open the provided DMG and drag **My Assistant** to **Applications**.
2. Try to open it once. If macOS blocks it, click **Done**, then open **System Settings → Privacy &
   Security → Open Anyway**, authenticate, and confirm **Open**.
3. Choose one speech model in the first-run library. Downloads range from about 44 MB to 1.1 GB and are
   verified before use.
4. Grant only the permissions needed by the feature you use: Microphone for dictation, System Audio for
   remote meeting participants, Accessibility for paste and speaker evidence, and Screen Recording only
   for optional video.

This extra Gatekeeper step is unavoidable for a downloadable app without an Apple Developer ID and
notarization. It occurs once per installed build identity; the DMG itself remains a normal drag-to-Applications
installer.

## Build from source

### Requirements

- **macOS 14.4+** on Apple Silicon — meeting audio is captured with Core Audio process taps, which need the
  macOS **14.2+ SDK** (`meeting-tap` builds with `-target …14.4`). Update Command Line Tools if
  `xcrun --show-sdk-version` reports < 14.
- Xcode Command Line Tools (`xcode-select --install`), [Homebrew](https://brew.sh), CMake, Node, Rust ([rustup](https://rustup.rs)) — `setup.sh` builds the transcription engine from source with CMake

```bash
./setup.sh                              # one-time native engines + optional local Intelligence model
cd desktop && npm install               # one-time deterministic desktop tooling
cd .. && ./script/build_and_run.sh --verify
./script/build_release_dmg.sh            # tested, signed beta DMG
```

Then **double-click "My Assistant.app"**. It asks for macOS permissions **only as each feature needs them**,
never upfront: **Microphone** + **System Audio Recording Only** (the Core Audio tap for the other
participants) the first time you record a meeting, **Accessibility** for speaker names and dictation paste,
and **Screen Recording** only if you turn on video recording. Grant each once. **Settings → General →
Permissions** shows live status where macOS exposes a reliable check and otherwise says **Requested on
first use**, with a recovery link. The app starts everything itself (no terminal).

> **Signing & portability.** The build signs with a **stable self-signed certificate** ("My Assistant Dev",
> set as `bundle.macOS.signingIdentity`) so macOS TCC grants **survive rebuilds** — no re-prompt every
> launch, and no manual `codesign` step. The `.app` bundles the web app + native helpers and reads writable
> state (saved meetings, the model) from `~/Library/Application Support/My Assistant/`, so it runs off the
> repo. **Speech models are not bundled** — on a machine without one the app opens the model library and asks
> the user to choose a download (currently about 44 MB–1.1 GB). The `.dmg` is built by
> `desktop/make-dmg.sh` (plain `hdiutil`), since
> Tauri's own DMG step runs a Finder AppleScript that fails non-interactively. This project deliberately
> does not require Apple's paid Developer ID. On current macOS, recipients try the app once, then choose
> **System Settings → Privacy & Security → Open Anyway** and confirm. Older releases may offer the same
> approval through Control-click → **Open**. That one-time Gatekeeper approval cannot be removed without
> Developer ID notarization.

## Development

Iterate on the UI in a browser, with capture from the shell:

```bash
./run.sh                            # transcribe-server + server on :8790, then the capturer · Ctrl-C to stop
cd desktop && npm run dev           # (optional) the native window instead of a browser tab
```

Ordinary runs default to the production surface. For internal design review only:

```bash
ASSISTANT_RELEASE_CHANNEL=preview ./run.sh
```

The release channel is server-owned; query parameters and saved settings cannot promote capabilities.

Evaluate pinned local speech candidates through the production native runtime:

```bash
./script/benchmark_speech.sh test
./script/benchmark_speech.sh prepare-models whisper-turbo-q8 apex-hinglish-q8
./script/benchmark_speech.sh prepare-data
./script/benchmark_speech.sh prepare-english-data
./script/benchmark_speech.sh prepare-translation-data
./script/benchmark_speech.sh prepare-noise-data
./script/benchmark_speech.sh run whisper-turbo-q8,apex-hinglish-q8

# Current shipped-model evidence: complete 500-clip set, three resumable trials, exact audio policy.
./script/benchmark_speech.sh prepare-full-data
./script/benchmark_speech.sh run-matrix whisper-turbo-q8

# Human Roman and real-noise work stays in the private benchmark cache.
./script/benchmark_speech.sh create-roman-pack
./script/benchmark_speech.sh capture-real list-devices
```

The data, models, and transcript-bearing reports remain in a private cache. See
[`docs/SPEECH_MODEL_BENCHMARK.md`](docs/SPEECH_MODEL_BENCHMARK.md) for the current Hinglish, English,
translation, preprocessing, and noise decisions and
[`benchmarks/speech/README.md`](benchmarks/speech/README.md) for the complete reproducibility contract.

Run `./run.sh` from a Terminal that has the **Screen-Recording + Accessibility** grants — dev builds are
ad-hoc-signed (grants don't persist across rebuilds), so dev rides on Terminal's stable grant; the
packaged `.app` gets its own. Flags pass through (`--app all`, `--mic off`, `--me "Name"`, `--gate off`,
`--max-seg 4`, …). **Use headphones** — on speakers the mic re-hears the remote voices (duplicate lines).

## Project layout

| Path | Role |
|---|---|
| `desktop/` | **Tauri app** — self-starts the stack; native window, menu-bar tray, screen-share stealth |
| `run.sh` · `setup.sh` | dev launcher · one-time install (engine + models + native helpers) |
| `stt/meeting-tap.swift` | Core Audio tap (audio, no Screen Recording) + ScreenCaptureKit (video) + process-list call detection + utterance segmentation |
| `stt/transcribe-server/` | our HTTP wrapper around [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) (source + `build.sh`, pinned commit) |
| `stt/whisper/` | GGUF transcription model (git-ignored; fetched by `setup.sh`) |
| `benchmarks/speech/` | pinned speech-model catalog, private-cache harness, and aggregate non-transcript evidence |
| `acceptance/` | physical Right-Command and real two-person meeting case definitions plus private evidence runner |
| `desktop/src-tauri/src/local_server.rs` | shipped Rust SSE/API server, meeting store, model lifecycle, summaries, Keychain and authenticated local routes |
| `app/server.js` | developer-only browser harness; not included in the packaged app |
| `app/index.html` | the UI — nav, live transcript, Library + detail (player), settings; light/dark |
| `meeting-state/meeting-state.swift` | AX layer: speaker names + in-meeting / mute / share |
| `agent-context/agent-context.swift` | On-demand, bounded, read-only AX snapshot for the Agents foundation |
| `agent-action/agent-action.swift` | On-demand, preview-only confirmed text Apply/Undo with exact AX target revalidation |
| `data/meetings/` | saved history + `rec/` screen recordings (git-ignored, on-device) |

The optional on-device LLM (LiteRT-LM + Gemma 4 E2B) is installed by the source-only `setup.sh` into
`~/.litert-lm`—not in the repo or DMG. An in-app local Intelligence installer remains release work.

## Design notes

For the full design rationale, the transcription/permissions deep-dives, and the **traps to avoid**, see
[ARCHITECTURE.md](ARCHITECTURE.md). Highlights:

The completed release, performance, privacy, security, install, and reverse-engineering review is in
[AUDIT_REPORT.md](AUDIT_REPORT.md).

Human release claims follow [`docs/ACCEPTANCE_TESTS.md`](docs/ACCEPTANCE_TESTS.md); Roman/noisy speech data
follows [`benchmarks/speech/CORPUS_PROTOCOL.md`](benchmarks/speech/CORPUS_PROTOCOL.md).

Contributor setup and standards are in [CONTRIBUTING.md](CONTRIBUTING.md); support, conduct, changes, and
third-party obligations are in [SUPPORT.md](SUPPORT.md), [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md),
[CHANGELOG.md](CHANGELOG.md), and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

- **On-device transcription, not the meeting app's captions** — Zoom captions are single-language, ~80%, and
  must be toggled on. Whisper `large-v3-turbo` is multilingual (handles Hindi-English), higher quality,
  always available. **On demand, not permanently resident** — spawned when a call starts and unloaded
  after the user-set idle timeout (10 minutes by default), so the speed/RAM tradeoff is explicit.
- **Audio-authoritative detection** — dedicated conferencing apps use live per-app Core Audio activity.
  Browsers and multipurpose chat apps require active audio **and** meeting-window evidence. Accessibility
  stays asleep while idle, so an open app, ordinary browser audio, or a notification cannot start recording.
- **Core Audio process taps for audio, ScreenCaptureKit only for video** — the taps capture the other
  participants under *System Audio Recording Only* (no Screen Recording); SCK is used only when recording
  video. Needs the macOS 14.2+ SDK (update Command Line Tools if `xcrun --show-sdk-version` < 14).

## Known limitations

- **Speaker names** read Zoom's active-speaker from the Accessibility tree. Solid in 1-on-1; best-effort
  in large gallery calls. Your own lines are labeled `You`.
- **Platform coverage is beta.** Dedicated apps auto-detect from active per-app audio. Browser and
  communication-app detection additionally requires Accessibility and recognized meeting evidence; if an
  app changes its UI or is not recognized, use **Record**. Manual recording works with any platform.
- **Self-signed, not Developer-ID** — the packaged app is signed with a stable self-signed certificate, so
  permission grants survive rebuilds. Gatekeeper shows an "unidentified developer" warning on another Mac;
  use **Privacy & Security → Open Anyway** after the first launch attempt on current macOS.
- Screen recordings are **silent** (audio mux is a follow-up); **Only-when-shared** currently keys off
  *your* share.
- System-audio capture can occasionally go silent after long uptime — reset with `sudo killall coreaudiod`.
- **Recording calls may require consent** depending on jurisdiction. That's on you.

## Roadmap

**Shipped**

- [x] **Self-detecting capture** — finds the call from the Core Audio process list (no permission) and captures the others via a Core Audio tap, transcribes you + the others, only while in a call
- [x] **Speaker names** from Zoom's AX tree (detector spawned as a direct child so its grant works)
- [x] **transcribe.cpp engine** (≈2× faster, ≈half the RAM of whisper.cpp) — **whole-utterance**, automatic language preserved, transcription as spoken; **custom vocabulary** biases recognition of names/jargon
- [x] **Curated local speech-model library** — Whisper Tiny/Base/Small/Medium English, Medium multilingual, Large v3 Turbo/Large v3, Parakeet Unified EN, both Nemotron models, and Oriserve Apex/Prime; verified resumable lifecycle and capability-driven settings
- [x] **Manual + auto recording** — **start / pause / stop by hand** (composed with auto-detect, so pause/stop really releases the mic), plus a **floating in-call panel** (live transcript + scratchpad)
- [x] **Real-call notifications + incremental recovery** — native alerts fire only on a Core Audio call edge; transcript JSON is atomically checkpointed throughout the call
- [x] **On-demand Whisper** (unloaded when idle) + on-demand LLM — light on an 8 GB Mac
- [x] **Meeting history** + **post-call AI** (summary, action items, "ask"), **rename**, recording-frame or app-icon **thumbnails**
- [x] **Cross-platform capture** — conservative native/browser/chat call detection plus universal manual recording
- [x] **Desktop app (Tauri)** — double-click, self-starts the stack, **menu-bar recording indicator**, ask-once permissions
- [x] **Screen-share stealth** · **video screen recording** (whole call / only-when-shared)
- [x] **Dictation** — press **Right Command** by default to speak into any app (even full-screen); on-device transcription with **cached Silero VAD** silence-stripping, pasted in place. Every Settings control is live (sound · volume · clipboard · overlay · **rebindable** shortcut + **Esc**-to-cancel)
- [x] **Portable `.app` + `.dmg`** — bundles the web app + native helpers, keeps data + models in Application Support, and **asks the user to choose a model** on first run; drag-to-install DMG via `make-dmg.sh`
- [x] **Rust-native packaged server** — no bundled Node runtime/process; authenticated local routes, Keychain-backed provider secrets, private transient audio, and checksum-verified model download
- [x] **Native low-overhead status pill** — the compact AppKit panel is created only on first use, appears across full-screen spaces without stealing focus, and leaves no hidden pill WebView resident while idle
- [x] **Agents foundation and compute setup** — explicit on-demand, read-only Accessibility context plus Local,
  Keychain BYOK, and ChatGPT account/Codex configuration. The official Codex runtime is detected externally,
  verified, isolated, and started only on demand; it is not bundled in the DMG. Draft planning and the Agent
  shortcut remain preview-only.
- [x] **Guarded Agent text-action foundation (preview)** — the model prepares text but cannot execute it;
  Apply is a separate click and is currently restricted to the exact captured process of a named disposable
  TextEdit document. The native helper revalidates and verifies one draft/selection edit or rolls it back,
  and one exact-value Undo is kept only in memory. Other apps remain read-only. No Send, Return, click, or
  multi-step action exists.

**Next**

- [ ] Close the physical Right-Command and real two-person platform acceptance matrices.
- [ ] Add in-flight audio recovery, redacted diagnostics export, and a clean-Mac local Intelligence installer.
- [ ] Measure every shipped speech model on representative Apple-silicon memory/latency/thermal profiles.
- [ ] Add cloud Speech as a separate provider contract, beginning with Groq and OpenAI.
- [ ] Add calendar sync and hardened platform adapters before Live Coach or cross-platform expansion.
- [ ] Pass read-only inspection/drafting across Gmail, Slack, LinkedIn, Docs, Notes, Slides, and Terminal;
  separately pass TextEdit-only Apply/Undo isolation, rollback, stale-target, and packaged-helper tests.
  Add each future app adapter only after its own synthetic mutation matrix passes. Keep Send and multi-step
  actions out until this one mutation class is reliable.
- [ ] Benchmark external Porcupine, microWakeWord, and sherpa-onnx KWS with explicit opt-in, real
  accents/noise, false-accept, latency, CPU/RAM, energy, suspension, privacy, integration, and licensing
  evidence before adopting any engine.

The ordered, compaction-safe backlog is maintained in
[`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md#known-release-blockers-and-follow-ups), not duplicated here.

## License

MIT — see [LICENSE](LICENSE).
