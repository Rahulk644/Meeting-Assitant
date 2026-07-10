# Architecture & decisions

This document is the durable "why" behind the Meetings module — the design decisions and the
non-obvious traps that cost real time to learn. The [README](README.md) covers *how* to install and
run; this covers *how it's built and why*, and the landmines to avoid. Read the **Traps** section before
changing capture, transcription, or packaging.

## The pipeline

```
 you (mic, AVAudioEngine) ─┐
 others (ScreenCaptureKit) ─┤─▶ stt/meeting-tap (Swift)
                            │     • self-detects the call from the SCK window list (Screen Recording only)
                            │     • energy gate → splits speech into WHOLE utterances (natural pauses)
                            ▼
              transcribe-server (transcribe.cpp + Metal · large-v3-turbo)   ← on demand, unloaded when idle
                            │  POST /inference → {"text": …}
                            ▼
              app/server.js ──Server-Sent Events──▶ http://localhost:8790  (the UI, app/index.html)
                            ▲
              meeting-state (Swift, Accessibility) ─┘  speaker names, mute/share state
```

Every component is on-device. No bot joins the call; no audio leaves the machine. A zero-dependency Node
server relays captions over SSE; the same UI runs in a browser (dev) or the Tauri window (packaged).

## Key decisions

- **transcribe.cpp, not whisper.cpp, for transcription.** [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp)
  ("llama.cpp for speech-to-text") runs Whisper — and 15+ other ASR families — as GGUF on ggml/Metal. On
  the *same* model and audio it measured **~2× faster, ~half the RAM, and cleaner output** (punctuation,
  no dropped word-onsets) than whisper.cpp's `whisper-server`. It ships CLI-only, so we wrap it in
  `transcribe-server` (below). whisper.cpp remains an automatic fallback. Choosing this engine also makes
  a future model picker (Parakeet / Qwen3-ASR / Voxtral) essentially free for the Dictation module.
- **Tauri, not Electron.** Rust shell + OS WebView rendering vanilla HTML/CSS/JS. Cross-platform, tiny
  footprint. The heavy compute (transcription) is native C++/Metal, so the UI language never bottlenecks.
  The packaged app and the browser build load the *same* UI from `localhost:8790` — one codebase.
- **On-device LLM via LiteRT-LM, not GGUF.** Post-call summary/ask uses **Gemma 4 E2B through LiteRT-LM**,
  which keeps it at *effective-2B* (~1 GB RAM) via Per-Layer-Embedding offload. A plain GGUF loads the
  full ~5 B params (~3 GB) and **hangs an 8 GB M1 Air**. OpenAI-compatible API, so one client also serves
  the BYO-key tier (Groq / OpenAI / Cerebras / custom).
- **ScreenCaptureKit, not Core Audio process taps.** Taps need the macOS 14.2+ SDK; this toolchain ships
  13.3. SCK is available there and also does app-scoped audio (isolates the meeting app from a background
  YouTube tab) and window enumeration (call self-detection) — three needs, one API.
- **Everything on demand.** The transcription server and the LLM load when needed and unload after an idle
  timeout (transcription ~2 min after a call; LLM ~5 min), so nothing heavy sits resident between calls.
  This mirrors Handy's `model_unload_timeout`.

## transcribe-server

A ~130-line HTTP wrapper (`stt/transcribe-server/transcribe-server.cpp`, using single-header cpp-httplib)
around transcribe.cpp's C API. It loads a GGUF once, holds it hot, and serves `POST /inference`
**byte-compatible with whisper.cpp's `whisper-server`** — multipart `file` + `language` + `translate` +
`prompt` (custom-vocabulary bias → the whisper run-ext's `initial_prompt`), returns `{"text": …}`. Because
the interface matches, the capturer and Node server are engine-agnostic; swapping engines is a spawn-path
change, not a client change.

- **Build:** `stt/transcribe-server/build.sh` fetches transcribe.cpp at a **pinned commit**
  (`PINNED_COMMIT`), drops our `.cpp` + `httplib.h` in as a CMake example target, builds with Metal, and
  re-signs the binary (see Traps → signing). `setup.sh` runs it and downloads the GGUF. First build
  compiles ggml (~2–4 min); the clone is cached under `.transcribe.cpp/` (git-ignored).
- **Lifecycle:** `server.js` (`ensureWhisper`) spawns it when a call starts, kills it when idle. It needs
  **no TCC permissions** (it only reads WAV bytes over localhost), so switching to it required no rebuild
  and no re-grant — the packaged app picks it up via the repo path on next launch.

## Transcription: the hard-won rules

These were expensive to learn. Do not re-litigate them.

1. **Transcribe WHOLE utterances, never fixed-length slices.** The capturer waits for a natural pause
   (`SILENCE_HANGOVER`, 0.7 s) and sends the entire sentence; `--max-seg` (25 s) is only a runaway-monologue
   safety cap. Chopping every N seconds was the sole cause of the "way off" garbling — fragments cut
   mid-word decode cold and hallucinate, and short fragments make language auto-detect misfire into the
   wrong script. **This was never a Whisper-config problem.** Verified by A/B against Handy's own WAVs:
   on whole utterances the output is clean and identical to Handy.
2. **Language and translate are two independent knobs.** `language` = *detection* (default `auto`, because
   the primary user speaks Hindi + English); `translate` = *output* (default on → English). Default
   behavior: auto-detect → English. Both are live-configurable in Settings.
3. **Anti-hallucination is temperature-0, not a VAD prefilter.** A Silero VAD prefilter was removed — it
   *degraded* clean speech (dropped onsets, stripped punctuation) and only ever masked the chunking bug.
   The fluent garbage on noise/silence comes from Whisper's **temperature fallback** escalating on
   high-entropy input; `transcribe-server` pins `temperature=0` with fallback off (`temperature_inc=0`)
   plus a stricter `no_speech_thold`, so non-speech decodes to nothing. Defense-in-depth: the capturer's
   **energy gate** blocks sub-threshold silence/noise *upstream* (it never reaches the engine); whole
   utterances give context; temperature-0 handles what's left. Verified on a silence/noise/transient/speech
   matrix. (A per-token confidence gate is **not possible** with this whisper GGUF — it exposes zero
   tokens; don't retry that approach.)

## macOS permissions & code-signing — the biggest trap

Get these wrong and the double-click app silently fails or nags on every launch.

- **Three separate TCC permissions:** Microphone, Screen Recording (`ScreenCapture`), Accessibility.
  Granting one does not grant the others. Screen Recording covers SCK audio/video **and** window
  enumeration; Accessibility is only for Zoom speaker names.
- **Grants reach a direct child, not a grandchild.** A helper spawned *through node* (`app → node → helper`)
  is a grandchild; macOS attributes its permission check to node, not the app, so it's never trusted.
  Therefore `meeting-state` is spawned **directly by Rust** (`start_meeting_state`), and its NDJSON is
  forwarded to the server via `curl`. `meeting-tap` (also a direct child) inherits Mic + Screen Recording.
- **Ad-hoc "linker-signed" binaries do not persist TCC grants.** Both Tauri's bundle output *and* the raw
  `swiftc`/linker output carry a `linker-signed` ad-hoc signature (`flags=0x20002`) that macOS will not
  hold a grant against → it re-prompts every launch even though the toggle reads ON. **Fix: re-sign with
  `codesign --force --sign -`** (→ `flags=0x2`, proper ad-hoc). This must be done for the `.app` **and**
  for `meeting-tap` / `meeting-state` / `transcribe-server`. `setup.sh` and `build.sh` do it after every
  build; **always re-sign a helper after recompiling it.**
- **Ad-hoc means grants survive relaunches but not rebuilds.** Any rebuild changes the cdhash → one
  re-grant. The permanent fix (grants survive rebuilds, runs on any Mac) is **Developer-ID signing +
  notarization** — the portable-installer milestone.
- **Accessibility is cached at launch** — after granting it, restart the process to pick it up.
- Editing the Swift/JS helpers does **not** churn the app's grant; only rebuilding the Tauri `.app` does.
  Batch anything needing `tauri build` into one rebuild + one re-grant.

## Call detection

`meeting-tap` self-detects "in a call" from the SCK window list (Screen Recording only — no Accessibility),
POSTs `inMeeting` to the server, and gates capture on it. **Trap:** an idle native app can keep meeting-ish
windows alive off-screen (e.g. Zoom retains a `zoom floating video window` when no call is running). Match
the main `Zoom Meeting` window unconditionally (it exists only during a real call), but count the
floating-video / share-statusbar windows **only when `isOnScreen`** — otherwise the app false-detects a
call whenever the meeting app is merely open.

## Recording control: auto-detect composed with manual

Recording is driven by an **effective state** the server computes from two inputs, not by detection alone:

- **`detected`** — the raw "in a call" the capturers self-report (`meeting-tap` from the window list,
  `meeting-state` from Accessibility).
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
  server can re-arm. Captions that slip through before it polls are dropped server-side when
  `!effectiveRecording()`.

The **live overlay** (`app/index.html`) mirrors this state: a floating, draggable in-call panel — recording
status, timer, a tail of the live transcript, a scratchpad, and a minimize-to-chip — gated on the *Display
live recording indicator* setting. Its Pause/Stop call the same control endpoint; scratchpad notes POST to
`/api/meeting/notes` (debounced, flushed before Stop) → saved on the meeting JSON, shown in the detail, and
fed into the summary prompt.

## Capture & recording

- **Meeting-gated** (`--gate on`): mic + system taps run only during a detected call; the mic is released
  otherwise. `--gate off` = always capture (browser calls we can't yet recognize by window).
- **Video** is optional (off / always / only-when-shared). Tuned to stay light on long calls: SCK
  downscales to ≤1080p on the GPU, hardware H.264 at 10 fps ≤4 Mbps, and `movieFragmentInterval = 10 s` so
  RAM stays flat and a hard-crashed file is still playable. Finalizes on call-end and on SIGINT/SIGTERM via
  a **retained** `DispatchSourceSignal` (a local signal source deallocates and never fires — a real bug).
- **Use headphones.** On speakers the mic re-hears the remote voices → duplicate lines (acoustic echo, no
  code fix). If SCK goes silent after long uptime: `sudo killall coreaudiod`.

## Speaker names

`meeting-state` reads names from Zoom's video-tile Accessibility descriptions (unlocked by
`AXManualAccessibility=true`); the always-on-top floating-video window follows the active speaker.
**Trap:** Zoom labels non-people ("Set up professional audio…", "Waiting for host…") with the same
AX-string shape as tiles, so `validName()` must reject UI/chrome words (whole-word list) and cap length,
and `canonName()` strips trailing "(Me)"/"(Host, me)" so a person doesn't appear twice. `--selftest` covers
it. Solid 1-on-1; best-effort in large gallery calls.

## Traps checklist (the landmines)

- Don't reintroduce fixed-length transcription chunking — transcribe whole utterances.
- Don't add a Silero/VAD prefilter to fix hallucinations — it degrades clean speech; use temperature-0.
- Don't leave any helper `linker-signed` — re-sign with `codesign --force --sign -` after every build.
- Don't spawn permission-needing helpers through node — spawn them directly from Rust.
- Don't match off-screen meeting windows in detection — gate secondary windows on `isOnScreen`.
- Don't set `excludesCurrentProcessAudio = true` — it wedges SCK into silent capture.
- Don't run the on-device LLM as a raw GGUF — use LiteRT-LM, or it hangs an 8 GB Mac.

## Repo layout

| Path | Role |
|---|---|
| `desktop/` | Tauri app — self-starts the stack; tray, screen-share stealth, close-to-tray |
| `stt/meeting-tap.swift` | SCK capture (audio + video) + call self-detection + utterance segmentation |
| `stt/transcribe-server/` | our transcribe.cpp HTTP wrapper (source + pinned-commit `build.sh`) |
| `stt/whisper/` | GGUF model (git-ignored; fetched by `setup.sh`) |
| `meeting-state/meeting-state.swift` | Accessibility layer: speaker names, mute/share |
| `app/server.js` | SSE relay + meeting store + on-demand engine/LLM + summary/ask + video serving (no deps) |
| `app/index.html` | the UI — nav, live transcript, Library + detail, settings; light/dark |
| `run.sh` · `setup.sh` | dev launcher · one-time build + model download |
