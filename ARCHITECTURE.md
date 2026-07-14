# Architecture & decisions

This document is the durable "why" behind the Meetings module — the design decisions and the
non-obvious traps that cost real time to learn. The [README](README.md) covers *how* to install and
run; this covers *how it's built and why*, and the landmines to avoid. Read the **Traps** section before
changing capture, transcription, or packaging.

## The pipeline

```
 you (mic, AVAudioEngine) ──────┐
 others (Core Audio tap; SCK ───┤─▶ stt/meeting-tap (Swift)
   only when recording video)   │     • dedicated apps: Core Audio; browser/chat: audio + AX meeting evidence
                                │     • audio-only ⇒ "System Audio Recording Only"; Screen Recording = video
                                │     • energy gate → splits speech into WHOLE utterances (natural pauses)
                            ▼
              transcribe-server (transcribe.cpp + Metal · large-v3-turbo)   ← on demand, unloaded when idle
                            │  POST /inference → {"text": …}
                            ▼
              Rust local_server ──Server-Sent Events──▶ http://127.0.0.1:8790  (the UI, app/index.html)
                            ▲
              meeting-state (Swift, Accessibility) ─┘  speaker names, mute/share state
```

Every component is on-device. No bot joins the call; no audio leaves the machine unless the user explicitly
selects a cloud transcription or summary provider. The authenticated Rust server inside the Tauri process
relays captions over SSE and serves the same UI used by the developer browser harness. No Node runtime or
second always-on server process is shipped.

## Key decisions

- **transcribe.cpp, not whisper.cpp, for transcription.** [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp)
  ("llama.cpp for speech-to-text") runs Whisper — and 15+ other ASR families — as GGUF on ggml/Metal. On
  the *same* model and audio it measured **~2× faster, ~half the RAM, and cleaner output** (punctuation,
  no dropped word-onsets) than whisper.cpp's `whisper-server`. It ships CLI-only, so we wrap it in
  `transcribe-server` (below). whisper.cpp remains an automatic fallback. Choosing this engine also keeps
  a future model picker (Parakeet / Qwen3-ASR / Voxtral) cheap — the wire format is already engine-neutral.
- **Tauri, not Electron.** Rust shell + OS WebView rendering vanilla HTML/CSS/JS. Cross-platform, tiny
  footprint. The heavy compute (transcription) is native C++/Metal, so the UI language never bottlenecks.
  The packaged app and the browser build load the *same* UI from loopback port 8790 — one codebase.
- **On-device LLM via LiteRT-LM, not GGUF.** Post-call summary/ask uses **Gemma 4 E2B through LiteRT-LM**,
  which keeps it at *effective-2B* (~1 GB RAM) via Per-Layer-Embedding offload. A plain GGUF loads the
  full ~5 B params (~3 GB) and **hangs an 8 GB M1 Air**. OpenAI-compatible API, so one client also serves
  the BYO-key tier (Groq / OpenAI / Cerebras / custom).
- **Core Audio process taps for audio, ScreenCaptureKit only for video** (Fathom's model). The other
  participants are captured with a **Core Audio process tap** scoped to the meeting app — the
  "System Audio Recording Only" TCC category, **no Screen Recording**. Call self-detection reads the same
  Core Audio **process list** (which app is doing audio I/O). Dedicated conferencing apps need no additional
  detection permission. Browsers and multipurpose chat apps also require narrowly scoped Accessibility
  evidence from the audio-active app, preventing YouTube or a notification sound from starting a meeting.
  ScreenCaptureKit (→ "Screen & System Audio Recording") is used **only** when you record video.
  Net: an audio-only meeting asks for **Microphone + System Audio Recording Only**; Screen Recording is
  video-only. Taps need the macOS **14.2+ SDK** (so `meeting-tap` builds with `-target …14.4`; update Command
  Line Tools if `xcrun --show-sdk-version` < 14).
- **Everything on demand.** The transcription server and the LLM load when needed and unload after an idle
  timeout (transcription is user-configurable, 10 min default; LLM ~5 min), so nothing heavy must sit
  resident indefinitely between calls.
  This mirrors Handy's `model_unload_timeout`.

## transcribe-server

A ~130-line HTTP wrapper (`stt/transcribe-server/transcribe-server.cpp`, using single-header cpp-httplib)
around transcribe.cpp's C API. It loads a GGUF once, holds it hot, and serves `POST /inference`
**byte-compatible with whisper.cpp's `whisper-server`** — multipart `file` + `language` + `translate` +
`prompt` (custom-vocabulary bias → the whisper run-ext's `initial_prompt`), returns `{"text": …}` and may
add `"truncated": true` when the engine returns valid partial text at its output limit. The response path
validates UTF-8 before JSON escaping, so a byte-limited Indic transcript cannot corrupt the protocol. Because
the interface matches, the capturer and Rust server are engine-agnostic; swapping engines is a spawn-path
change, not a client change.

- **Build:** `stt/transcribe-server/build.sh` fetches transcribe.cpp at a **pinned commit**
  (`PINNED_COMMIT`), drops our `.cpp` + `httplib.h` in as a CMake example target, builds with Metal, and
  re-signs the binary (see Traps → signing). The build runs a protocol self-test covering Hindi, JSON
  metacharacters, and malformed UTF-8. `setup.sh` runs it and downloads the GGUF. First build
  compiles ggml (~2–4 min); the clone is cached under `.transcribe.cpp/` (git-ignored).
- **Lifecycle:** the in-process Rust server spawns it only when audio reaches transcription and kills it
  after the user-configurable idle timeout (10 minutes by default). It binds only to loopback and requires
  a per-launch bearer token inherited through the environment. It needs **no TCC permissions** because it
  only reads transient WAV bytes from the trusted app over localhost.

## Transcription: the hard-won rules

These were expensive to learn. Do not re-litigate them.

1. **Transcribe WHOLE utterances, never fixed-length slices.** The capturer waits for a natural pause
   (`SILENCE_HANGOVER`, 0.7 s) and sends the entire sentence; `--max-seg` (25 s) is only a runaway-monologue
   safety cap. Chopping every N seconds was the sole cause of the "way off" garbling — fragments cut
   mid-word decode cold and hallucinate, and short fragments make language auto-detect misfire into the
   wrong script. **This was never a Whisper-config problem.** Verified by A/B against Handy's own WAVs:
   on whole utterances the output is clean and identical to Handy.
2. **Language and translate are two independent knobs.** `language` = *detection* (default `auto`, because
   the primary user speaks Hindi + English); `translate` = a separate decoder task. Automatic is preserved
   end-to-end and must never be rewritten to English. Translation defaults off and is blocked in production
   while Turbo is the installed model because Turbo was not trained for translation. Re-enable it only when
   a qualified translation-capable model has its own install/routing/removal lifecycle.
3. **Anti-hallucination is temperature-0, not a VAD prefilter.** A Silero VAD prefilter was removed — it
   *degraded* clean speech (dropped onsets, stripped punctuation) and only ever masked the chunking bug.
   The fluent garbage on noise/silence comes from Whisper's **temperature fallback** escalating on
   high-entropy input; `transcribe-server` pins `temperature=0` with fallback off (`temperature_inc=0`)
   plus a stricter `no_speech_thold`, so non-speech decodes to nothing. Defense-in-depth: the capturer's
   **energy gate** blocks sub-threshold silence/noise *upstream* (it never reaches the engine); whole
   utterances give context; temperature-0 handles what's left. Verified on a silence/noise/transient/speech
   matrix. (A per-token confidence gate is **not possible** with this whisper GGUF — it exposes zero
   tokens; don't retry that approach.)
   **Current Whisper dictation is the exception — and not a contradiction.** This rule governs the *meeting* path: whole,
   already-energy-gated utterances, where a VAD prefilter only clipped clean speech. **Dictation feeds
   *short push-to-talk clips*** with silence a user leaves around a phrase, so it *does* run **Silero VAD**
   (in the app, on the `cpal` buffer) to strip that silence — otherwise Whisper loops on a long/idle
   recording. This VAD is not a provider invariant: exact-pipeline benchmarks show that it damages Apex and
   Qwen HiACC. Every new speech provider must pass raw-versus-product preprocessing A/B tests and may select
   a different silence/denoise policy. Different inputs and models need different answers.

## macOS permissions & code-signing — the biggest trap

Get these wrong and the double-click app silently fails or nags on every launch.

- **Four TCC permissions, each asked only for the feature that needs it:** Microphone, **System Audio
  Recording Only** (`AudioCapture` — Core Audio taps), Screen Recording (`ScreenCapture` — ScreenCaptureKit),
  Accessibility. Granting one does not grant the others. An **audio-only meeting** needs Microphone (your
  voice) + System Audio Recording Only (the others' voices via the tap) — **not** Screen Recording. Screen
  Recording is asked for **only when recording video**. Dedicated-app detection reads the Core Audio process
  list without permission; browser/chat detection also uses meeting-only Accessibility evidence. Accessibility
  is also used for Zoom speaker names and dictation paste; the Carbon hotkey itself needs no permission.
  **Settings → General → Permissions** surfaces reliable live status for
  Accessibility and Screen Recording through `perm_status`. Microphone and System Audio Recording are
  labeled *Requested on first use* with recovery links because this layer does not expose a dependable live
  status for them.
- **Permission-needing helpers are direct children.** `meeting-state` and `meeting-tap` are spawned directly
  by Rust, so macOS can associate their checks with the app's stable identity. `meeting-state` NDJSON is
  forwarded over a tiny authenticated in-process localhost request (no polling or per-update `curl`).
- **Ad-hoc signing pins the grant to a bare cdhash — it dies on the next rebuild.** swiftc's linker output
  *and* Tauri's default bundle output are ad-hoc; ad-hoc code has no stable identity, so when you grant, TCC
  can only store a bare `cdhash H"…"` requirement (no identifier, no cert). The very next rebuild changes the
  cdhash → the grant silently stops matching (the Settings toggle still reads ON, but it's a **dead entry**)
  → re-prompt on every launch. `flags=0x2` ("proper" ad-hoc) vs `0x20002` (linker-signed) makes **no**
  difference — both are ad-hoc. The fix is a **real certificate**, not "better ad-hoc."
- **Sign the app AND every direct-child helper with the same self-signed cert *and the app's identifier*.**
  `tauri.conf.json` sets `bundle.macOS.signingIdentity: "My Assistant Dev"`; `setup.sh` signs `meeting-tap` /
  `meeting-state` with `codesign --force --identifier com.myassistant.desktop --sign "My Assistant Dev"`.
  Two effects: (1) TCC stores `identifier "com.myassistant.desktop" and certificate leaf = …` —
  **cdhash-independent**, so grants survive rebuilds; (2) because each helper carries the *app's* identifier
  it satisfies the app's Designated Requirement and **rides the app's single grant** instead of demanding its
  own (a helper with its own id would need — and re-prompt for — a separate grant, even cert-signed).
  `security find-identity -v` *hides* the cert as `CSSMERR_TP_NOT_TRUSTED`, but it still signs fine; create it
  once via Keychain Access → *Create a Certificate* (Code Signing, self-signed). Untrusted by Gatekeeper, so
  This is intentionally not an Apple Developer ID. Recipients can still install the DMG, but on current
  macOS must attempt launch, then use **Privacy & Security → Open Anyway** so macOS records their explicit
  approval (older versions may expose this via Control-click → Open). Ordinary launches work afterward.
  A completely warning-free first launch is the one benefit this project cannot provide without Apple's
  paid Developer ID and notarization.
- **Switching an already-granted binary from ad-hoc → cert invalidates the old grant** (its stored `csreq` is
  a stale cdhash, not the new identity). `tccutil reset ScreenCapture com.myassistant.desktop` once (or remove
  the entry in System Settings), relaunch, re-grant — then it sticks across all future rebuilds. Diagnose by
  decoding the stored requirement: `sqlite3 "/Library/Application Support/com.apple.TCC/TCC.db" "SELECT
  hex(csreq) FROM access WHERE service='kTCCServiceScreenCapture' AND client='com.myassistant.desktop'"` — a
  bare `cdhash H"…"` = dead ad-hoc pin; `identifier … and certificate leaf …` = durable. (Real case: Mic +
  Accessibility were identity-based and never nagged, while Screen Recording stayed pinned to a dead cdhash and
  re-prompted every launch — same app, three grants, only one broken.)
- **Accessibility is cached at launch** — after granting it, restart the process to pick it up.
- Editing the Swift/JS helpers does **not** churn the app's grant; only rebuilding the Tauri `.app` does.
  Batch anything needing `tauri build` into one rebuild + one re-grant.

## Call detection

`meeting-tap` self-detects "in a call" from per-app Core Audio activity. For a dedicated conferencing app,
audio activity is sufficient. For a browser or multipurpose chat app, the same process must also expose
platform-specific meeting evidence through Accessibility. It then POSTs a source-tagged `inMeeting` and gates
capture on it. The server accepts automatic start/end only from this combined detector's `source: "coreaudio"`;
the separate `meeting-state` stream is metadata, never lifecycle authority. This is the key false-positive
boundary: an idle app, YouTube tab, or ordinary notification cannot start a meeting.

The same Core Audio detector publishes an internal distributed notification to `meeting-state`.
Accessibility is completely idle until that notification says a call is active. During the call it uses
`AXObserver` for native UI changes plus a low-frequency reconciliation pass because Zoom's embedded
Chromium UI does not emit every participant/speaker mutation. A heartbeat timeout puts it back to sleep if
the capturer disappears. This internal event is **not** the user-facing “Meeting detected” notification;
it is the efficient signal that a future top-center panel or macOS notification should follow.

## Recording control: auto-detect composed with manual

Recording is driven by an **effective state** the server computes from two inputs, not by detection alone:

- **`detected`** — the raw "in a call" state reported authoritatively by `meeting-tap` from Core Audio,
  plus meeting-window evidence for browser/chat platforms.
  `meeting-state` can enrich the active call but cannot change this value.
- **`manual`** — an optional UI override (`on` / `paused` / `off`), set via `POST /api/meeting/control`.

```
effectiveRecording = manual==='on'            ? true
                   : manual∈{paused,off}       ? false
                   : autoDetect && detected           // 'auto' — the Detect-automatically setting
```

That one value drives the meeting lifecycle (start / finalize), the elapsed timer, and the tray + overlay.

- **Start** records even with no detected call — an in-person meeting, or a browser call self-detection
  misses. **Pause** releases the mic and drops captions but keeps the meeting open. **Stop** finalizes now.
- A **call-relative** override (pause / **resume** / stop) is scoped to the call it was made in
  (`manualDetected`): the moment `detected` changes — the call starts or ends — it clears and auto takes
  over, so the next call still records itself. **This must include `resume`** (which also sets
  `manual='on'`) — otherwise a resumed auto-detected call *never finalizes* when it ends (stuck "LIVE",
  the mic and Whisper never release). A **genuine manual recording** (`manualHard` — a Start with no call
  under it) is exempt and persists until you Stop, so a stray detection can't hijack it. **Trap:** clear on
  the `detected` edge, not the finalize path (a screen-share blip would re-arm early otherwise).
- `meeting-tap` reads `manual` + `autoDetect` from `/api/state` and gates *capture* (the mic) on them —
  so a pause/stop genuinely releases the mic, not just the UI — while still reporting raw `detected` so the
  server can re-arm. Configuration is fetched with in-process `URLSession` (no two-second `curl` subprocess);
  captions that slip through before the next update are dropped server-side when
  `!effectiveRecording()`.

The **live overlay** (`app/index.html`) mirrors this state: a floating, draggable in-call panel — recording
status, timer, a tail of the live transcript, a scratchpad, and a minimize-to-chip — gated on the *Display
live recording indicator* setting. Its Pause/Stop call the same control endpoint; scratchpad notes POST to
`/api/meeting/notes` (debounced, flushed before Stop) → saved on the meeting JSON, shown in the detail, and
fed into the summary prompt.

## Capture & recording

- **Meeting-gated** (`--gate on`): mic + system taps run only during a detected call; the mic is released
  otherwise. `--gate off` remains a developer/manual fallback for any unrecognized platform.
- **Video** is optional (off / always / only-when-shared). Tuned to stay light on long calls: SCK
  downscales to ≤1080p on the GPU, hardware H.264 at 10 fps ≤4 Mbps, and `movieFragmentInterval = 10 s` so
  RAM stays flat and a hard-crashed file is still playable. Finalizes on call-end and on SIGINT/SIGTERM via
  a **retained** `DispatchSourceSignal` (a local signal source deallocates and never fires — a real bug).
  The first complete frame is written natively as a small private JPEG; no `ffmpeg` dependency is shipped.
  Without video, the library resolves the installed native app icon through `NSWorkspace`, then falls back
  to a lightweight platform identity tile.
- **Use headphones.** On speakers the mic re-hears the remote voices → duplicate lines (acoustic echo, no
  code fix). If SCK goes silent after long uptime: `sudo killall coreaudiod`.

## Speaker names

`meeting-state` reads names from Zoom's video-tile Accessibility descriptions (unlocked by
`AXManualAccessibility=true`); the always-on-top floating-video window follows the active speaker.
**Trap:** Zoom labels non-people ("Set up professional audio…", "Waiting for host…") with the same
AX-string shape as tiles, so `validName()` must reject UI/chrome words (whole-word list) and cap length,
and `canonName()` strips trailing "(Me)"/"(Host, me)" so a person doesn't appear twice. `--selftest` covers
it. The helper only installs its observer and reconciliation timer after Core Audio confirms a call. Solid
1-on-1; best-effort in large gallery calls.

## Dictation

Push-to-talk (or tap-to-toggle) transcription into any focused text field, running **entirely in the Tauri
app process** — a deliberate reversal of an earlier design that lived in `meeting-tap`. Running in-process
is what lets the app's *own* Accessibility grant authorize the paste, and it removes a ~300 ms
per-keypress helper spawn. The loop:

1. **Hotkey** — a global shortcut (default **Right Command**) with a bare-modifier event tap; ordinary
   key combinations use `tauri-plugin-global-shortcut` backed by Carbon's `RegisterEventHotKey`. The
   shortcut is rebindable and one binding drives both tap-to-toggle and hold-to-talk modes.
2. **Record** — the mic is captured **in-process with `cpal`** on a dedicated thread (a cpal `Stream` is
   `!Send`: its owning thread builds it, parks on an atomic stop flag, and drops it on that same thread).
   The app itself prompts for **Microphone** on first use (`NSMicrophoneUsageDescription` in Info.plist).
3. **Strip silence (Silero VAD)** — the buffer is resampled to 16 kHz (box-average low-pass, not plain
   decimation, which aliases) and run through **Silero VAD** — the `voice_activity_detector` crate over a
   **statically-linked ONNX Runtime** (`ort`), the same model family Handy bundles as `silero_vad_v4.onnx`.
   The detector is **built once and cached**, calling `reset()` between utterances to clear only the LSTM
   state — exactly Handy's pattern (its `SileroVad` holds the session for the app's life and resets between
   clips), so the model load never recurs per dictation. Only speech regions reach Whisper; a clip with no
   speech produces nothing. This is what stops Whisper from hallucinating a fluent loop on a long or
   accidental recording. (Why this is *not* a contradiction of the "no VAD prefilter" rule: see Transcription
   rule 3 — dictation feeds short clips, the meeting path feeds gated whole utterances.)
4. **Transcribe** — the speech-only WAV POSTs to the authenticated **`POST /api/transcribe`** route (Rust
   starts `transcribe-server` on demand, then streams the request to it). Dictation pins
   **`language=en` + the transcribe task** (`translate=false`), matching Handy's *runtime* (its debug log
   shows `task=Transcribe, language=en` even though its settings say `translate_to_english` — the translate
   task only engages for a non-English source). This gives clean English on short mixed Hindi-English clips.
5. **Paste** — synthesize **⌘V** as a `CGEvent` with an explicit Command flag (core-graphics), authorized
   by the **app's own Accessibility grant**. Use the raw keycode 9 (`kVK_ANSI_V`), never a character —
   character→keycode goes through HIToolbox, whose layout query is main-thread-only and aborts the app when
   called from the transcription thread.

Progress and results ride the existing SSE channel: `emit_dstate()` POSTs `dictation-state` to `/ingest`
(drives the main Dictation view), while Rust updates the native status pill directly. The final text POSTs
`{type:'dictation'}` to populate Dictation history.

**Settings are real, and split by who applies them.** The server owns only what it applies at transcription
time — **language** and **punctuation** (Off strips it) — via `/api/config`. Everything the app process owns
is pushed straight to it through the **`set_dictation`** command: the start/stop **sound** + **feedback
volume** (`afplay -v`), **clipboard behaviour** (restore your clipboard vs. keep the dictated text), the
**overlay** on/off + position, push-to-talk, and the **rebindable** start-and-cancel shortcuts. The hotkey
is swapped live (unregister old → register new, validated so an unmappable combo keeps the working one);
**Esc-to-cancel** is a modifier-less global grab armed only while recording and released the instant it ends.
Custom vocabulary still feeds the transcribe prompt. Rows we can't yet honor on-device (system-audio muting,
auto-pause) were **removed** rather than shipped inert — every switch in the panel now does what it says.

**The dictation pill** — a compact, borderless, click-through native **`NSPanel`** built with objc2/AppKit.
It uses `NSScreenSaverWindowLevel`, `canJoinAllSpaces | fullScreenAuxiliary`, and
`orderFrontRegardless`, so it remains visible above full-screen apps without activating My Assistant or
stealing focus from the text field. It is allocated lazily on first use and reused after being ordered out.
No hidden pill WebView or WebKit renderer remains resident while the app is idle.

**Setup:** none of the old friction — no Input Monitoring grant, no 🌐/Fn-key remap. Microphone
auto-prompts on first use; Accessibility is the single shared grant (with Meetings) for the paste.

## Packaging & portability

A packaged `.app` is self-contained for **code**, and resolves **data + model** to writable, per-user
locations — so it runs when moved off the repo:

- **Bundled into `Contents/Resources/`** (via `bundle.resources`): minified web assets (`webui/`) and the
  Swift/STT binaries (`stt/`, `meeting-state/`). The server itself is compiled into the Rust app binary.
  `init_paths()` points `RES_BASE` at the Resources dir in a release build and at the repo in dev; helpers
  spawn from there. The web dir is staged as
  `webui/`, **not** `app/` — a top-level `app/` resource collides with the crate's own `app` binary during
  Tauri's dev staging (EEXIST).
- **Writable state → `~/Library/Application Support/My Assistant/`** (the bundle is read-only): saved
  meetings + recordings under `data/`, the model under `models/`. Secrets are stored in macOS Keychain,
  transient audio lives in a private `0700` temp directory, and saved files are written with owner-only
  permissions.
- **The model is NOT bundled** (845 MB). The server resolves it across `[models dir → repo → ./run.sh
  layout]`; if it's absent the UI **asks first** (Handy's way — never silent): `GET /api/model/status` → a
  first-run prompt → `POST /api/model/download` streams the GGUF from Hugging Face into `models/` with
  progress over SSE.
- **The `.dmg`** is built by `desktop/make-dmg.sh` (plain `hdiutil`, a drag-to-Applications installer) —
  **not** Tauri's DMG step, whose create-dmg fork always runs a Finder AppleScript that fails in any
  non-interactive shell. So `bundle.targets` is `["app"]`: `npm run build` makes the signed `.app`,
  `make-dmg.sh` makes the installer. Signing uses the stable self-signed **My Assistant Dev** identity
  (`bundle.macOS.signingIdentity`). The DMG includes a short install guide for the one-time current-macOS
  **Privacy & Security → Open Anyway** approval required by this no-paid-account distribution route.

## Traps checklist (the landmines)

- Don't reintroduce fixed-length transcription chunking — transcribe whole utterances.
- Don't add a VAD prefilter to the **meeting** path (whole utterances) — it clips clean speech; use
  temperature-0. **Dictation is the exception:** its short push-to-talk clips *do* use Silero VAD in-app to
  strip surrounding silence (see Dictation). Different input, different rule.
- Don't leave any helper ad-hoc / `linker-signed` — re-sign it as `--identifier com.myassistant.desktop
  --sign "My Assistant Dev"` after every build (ad-hoc pins a cdhash that dies on the next rebuild).
- Don't put permission-needing helpers behind an intermediary process — spawn them directly from Rust.
- Don't match off-screen meeting windows in detection — gate secondary windows on `isOnScreen`.
- Don't set `excludesCurrentProcessAudio = true` — it wedges SCK into silent capture.
- Don't run the on-device LLM as a raw GGUF — use LiteRT-LM, or it hangs an 8 GB Mac.
- Don't move dictation back into a helper binary or spawn one per keypress — it runs **in the app process**
  (cpal mic + Carbon global shortcut + CGEvent paste), so the app's *own* Accessibility grant covers the
  paste and there's no spawn lag. The Carbon global shortcut needs **no** permission (not Input Monitoring).
- Don't paste with `enigo`/character keys off the main thread, and don't give the pill a plain NSWindow —
  use raw keycode 9 for ⌘V (no HIToolbox layout lookup → no SIGTRAP), and a non-activating `NSPanel` for
  the pill (a plain NSWindow won't float over another app's full-screen space at any level).
- Don't position the pill (or any window) with `primary_monitor()` + a *global* `set_position` — with an
  external display attached, the right/left-edge positions land off-screen (the symmetric centre positions
  stay visible, so it reads as "centre works, edges don't"). Position relative to the monitor the window is
  actually on: `current_monitor()` + its `position()` origin offset, then clamp x/y into that monitor.
- Don't stage web resources into a top-level `app/` bundle dir — it collides with the crate's own `app`
  binary during Tauri's dev staging (EEXIST). Stage them under `webui/`.
- Don't rely on Tauri's `.dmg` step in a non-interactive shell — its create-dmg fork always runs a Finder
  AppleScript that fails there. Build the DMG with `desktop/make-dmg.sh` (plain `hdiutil`).
- Don't ship a Settings row that does nothing — every dictation control maps to real behavior (sound ·
  volume · clipboard · overlay · rebindable hotkey via `set_dictation`; language · punctuation via the
  server). If it can't be honored on-device, remove it rather than fake it.

## Repo layout

| Path | Role |
|---|---|
| `desktop/` | Tauri app — self-starts the stack; tray, screen-share stealth, close-to-tray; **in-process dictation** (global hotkey · cpal mic · Silero VAD · CGEvent paste · NSPanel pill) |
| `stt/meeting-tap.swift` | Core Audio tap (audio, no Screen Recording) + SCK (video) + audio/meeting-evidence call detection + utterance segmentation |
| `stt/transcribe-server/` | our transcribe.cpp HTTP wrapper (source + pinned-commit `build.sh`) |
| `stt/whisper/` | GGUF model (git-ignored; fetched by `setup.sh`) |
| `meeting-state/meeting-state.swift` | Accessibility layer: speaker names, mute/share |
| `desktop/src-tauri/src/local_server.rs` | packaged authenticated UI/API + SSE relay + meeting store + on-demand STT/LLM + model verification + streaming video |
| `app/server.js` | developer-only browser harness; never bundled in the release app |
| `app/index.html` | the UI — nav, live transcript, Library + detail, settings; light/dark |
| `desktop/src-tauri/src/native_pill.rs` | native AppKit dictation status pill (the menu-bar dropdown is also native) |
| `desktop/make-dmg.sh` | portable drag-to-install `.dmg` (hdiutil; bypasses Tauri's AppleScript DMG step) |
| `run.sh` · `setup.sh` | dev launcher · one-time build + model download |
