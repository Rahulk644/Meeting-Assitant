# Assistant

A private, **on-device** super-app for calls and text — **Dictation, Writing Assistant, Meetings, and
Agents** in one minimal shell. **Meetings ships first:** real-time transcription of *you and the other
participants* in any Mac video call, with speaker names. Whisper runs on the Apple GPU — **no bot joins
the call, no audio leaves your machine, no meeting-app captions toggle.**

**Status:** the Meetings module works end-to-end and ships as a **double-click desktop app**
([Tauri](https://tauri.app), `desktop/`) — no terminal, no dev commands. It:

- **detects the call itself** and transcribes *you and the other participants*, with **live speaker
  names**, capturing **only while you're in a call** — or **start / pause / stop by hand**;
- **auto-detects the spoken language and outputs English** (Whisper translate; detection + translate
  both toggleable in Settings);
- **saves every call on-device** into a browsable history with **post-call summaries, action items, and
  "ask this meeting"**;
- can **screen-record** the call (whole call, or only while a screen is shared) and shows it in the
  meeting, with a frame as the card thumbnail;
- shows a **floating in-call panel** (live transcript + a per-call scratchpad, drag or minimize it) and a
  **menu-bar recording indicator**, and can make its window **invisible to screen sharing**.

The same UI also runs in a plain browser for development. Real-time in-call **coaching** — the Copilot —
is the next milestone; see the [roadmap](#roadmap).

## How it works

```
 you (mic, AVAudioEngine)  ┐                         detect the call + capture, on the Apple GPU
 the other participants'   ├─▶  stt/meeting-tap (Swift)
 audio (ScreenCaptureKit)  ┘      │ self-detects the call from the window list (no Accessibility needed)
                                  │ energy-VAD splits speech into per-utterance WAVs
                                  ▼
                    transcribe-server (transcribe.cpp + Metal GPU · large-v3-turbo)
                    on demand — started when a call begins, unloaded ~2 min after
                                  │ transcript  (POST /ingest)
                                  ▼
                    app/server.js ──Server-Sent Events──▶ http://localhost:8790  (the UI)
                                  ▲
                    meeting-state (Swift, Accessibility) ─┘  speaker names, mute/share state
```

- **Detect + capture** — `meeting-tap` finds the call in the ScreenCaptureKit window list (a Zoom
  meeting / floating-video / share window) using only **Screen Recording**, then taps the meeting app's
  system audio **and** your mic, splitting each into utterances.
- **Transcribe** — `transcribe-server` (started on demand) transcribes each whole utterance in ~3 s on
  the GPU, auto-detecting the language and translating to English.
- **Names** — `meeting-state` reads Zoom's Accessibility tree for the active speaker (and mute/share);
  `server.js` stamps each remote line with who's speaking.
- **Display** — a zero-dependency Node server streams captions over SSE to `app/index.html` (light/dark
  shell: module nav, live transcript, Library + detail, settings).

## Transcription

**Settings → Meetings** exposes two independent Whisper controls:

- **Transcription language** *(detection)* — **`Auto-detect` by default**, so mixed Hindi + English is
  recognized per utterance. Pin a language only if detection ever slips.
- **Translate to English** *(output)* — on by default: whatever the spoken language, the caption comes
  out in English. Off = transcribe as spoken.

So out of the box: **auto-detect → English.**

Quality comes first from **transcribing whole utterances** — the capturer waits for a natural pause and
sends the engine the *entire* sentence, never a fixed-length slice. (Chopping mid-sentence was what made it
hallucinate — the model needs the surrounding words for context.)

The engine is **[transcribe.cpp](https://github.com/handy-computer/transcribe.cpp)** running
`large-v3-turbo` (q8) on Metal — the same engine [Handy](https://handy.computer) ships. On identical
audio and model it's **~2× faster and ~half the RAM** of whisper.cpp, with cleaner output (proper
punctuation, no dropped word-onsets). It runs behind `transcribe-server`, a tiny HTTP wrapper that holds
the model hot; the build is `stt/transcribe-server/build.sh` (pinned commit, Metal). whisper.cpp is kept
as an automatic fallback if that isn't built. Capture (mic, system audio, video) runs **only while you're
in a call**.

## Post-call AI

Every meeting is saved to `data/meetings/*.json` (on-device, git-ignored). Open one from the **Library**
for an AI **summary + action items**, and to **ask questions** about it. Rename a meeting inline. The
model is pluggable across three tiers (Settings → *AI Model*):

- **Local** *(default, private)* — **Gemma 4 E2B** (text-only) via **LiteRT-LM** on the Apple GPU. It runs
  at *effective-2B* (**~1 GB RAM**, 2.4 GB disk), so it stays light even on an 8 GB Mac — a plain GGUF
  loads the full ~5 B params and can hang. `setup.sh` installs it; the app **starts it on demand** (first
  summary/ask) and **unloads it when idle**, so it never competes with Whisper for RAM during a call.
- **API key** — bring your own, any OpenAI-compatible provider (Groq / OpenAI / Cerebras …).
- **Cloud** — hosted (Pro), later.

## Screen recording

Optionally capture the call's **video** — **Settings → Meetings → Screen recording**: **Off** *(default)*,
**Always**, or **Only when shared**. Recordings are H.264 `.mp4` in `data/meetings/rec/` (on-device,
git-ignored); they play back in the meeting's detail view and a frame becomes the card thumbnail.

Tuned to stay light on long calls: capture is **downscaled to ≤1080p** and encoded by the **hardware
H.264 encoder** (VideoToolbox) at **10 fps, ≤4 Mbps** — screen content is mostly static, so this looks
fine and keeps files small. The writer **flushes a fragment every 10 s**, so RAM stays flat and a file
survives a hard crash (playable up to the last fragment). Whisper and the LLM are **on-demand** (loaded
only when needed, unloaded when idle) and run on the **Apple GPU (Metal + flash-attention)** — so nothing
heavy sits resident between or during calls beyond what's actively transcribing.

## Requirements

- macOS 13+ on Apple Silicon
- Xcode Command Line Tools (`xcode-select --install`), [Homebrew](https://brew.sh), CMake, Node, Rust ([rustup](https://rustup.rs)) — `setup.sh` builds the transcription engine from source with CMake

## Install & run the app

```bash
./setup.sh                                          # one-time: build transcribe.cpp + LiteRT-LM + models
cd desktop && npm install                           # one-time: the Tauri CLI
npx tauri build --bundles app                       # build Assistant.app
codesign --force --deep --sign - \
  src-tauri/target/release/bundle/macos/Assistant.app   # ← re-sign valid, or macOS re-prompts every launch
cp -R src-tauri/target/release/bundle/macos/Assistant.app /Applications/
```

Then **double-click Assistant.app**. On the first call macOS prompts for **Microphone**, **Screen
Recording**, and **Accessibility** (three separate toggles) — grant each once. The app starts everything
itself (no terminal); get on a call and it transcribes.

> **Why the `codesign` step?** Tauri's ad-hoc "linker-signed" output is an *invalid* signature, so macOS
> can't verify the app and re-asks for permissions on every launch. Re-signing makes it valid → grants
> stick. This build still reuses the models/binaries `setup.sh` installed on *this* machine; a portable,
> **Developer-ID-signed + notarized** installer (grants that survive rebuilds, runs on any Mac) is on the
> roadmap.

## Development

Iterate on the UI in a browser, with capture from the shell:

```bash
./run.sh                            # transcribe-server + server on :8790, then the capturer · Ctrl-C to stop
cd desktop && npm run dev           # (optional) the native window instead of a browser tab
```

Run `./run.sh` from a Terminal that has the **Screen-Recording + Accessibility** grants — dev builds are
ad-hoc-signed (grants don't persist across rebuilds), so dev rides on Terminal's stable grant; the
packaged `.app` gets its own. Flags pass through (`--app all`, `--mic off`, `--me "Name"`, `--gate off`,
`--max-seg 4`, …). **Use headphones** — on speakers the mic re-hears the remote voices (duplicate lines).

## Project layout

| Path | Role |
|---|---|
| `desktop/` | **Tauri app** — self-starts the stack; native window, menu-bar tray, screen-share stealth |
| `run.sh` · `setup.sh` | dev launcher · one-time install (engine + models + native helpers) |
| `stt/meeting-tap.swift` | ScreenCaptureKit capture (audio **and** video) + call self-detection + utterance segmentation |
| `stt/transcribe-server/` | our HTTP wrapper around [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) (source + `build.sh`, pinned commit) |
| `stt/whisper/` | GGUF transcription model (git-ignored; fetched by `setup.sh`) |
| `app/server.js` | SSE relay + meeting store + on-demand Whisper/LLM + summary/ask + video serving (no deps) |
| `app/index.html` | the UI — nav, live transcript, Library + detail (player), settings; light/dark |
| `meeting-state/meeting-state.swift` | AX layer: speaker names + in-meeting / mute / share |
| `data/meetings/` | saved history + `rec/` screen recordings (git-ignored, on-device) |

The on-device LLM (LiteRT-LM + Gemma 4 E2B) is installed by `setup.sh` into `~/.litert-lm` — not in the repo.

## Design notes

For the full design rationale, the transcription/permissions deep-dives, and the **traps to avoid**, see
[ARCHITECTURE.md](ARCHITECTURE.md). Highlights:

- **On-device transcription, not the meeting app's captions** — Zoom captions are single-language, ~80%, and
  must be toggled on. Whisper `large-v3-turbo` is multilingual (handles Hindi-English), higher quality,
  always available. **On demand, not resident** — spawned when a call starts, unloaded ~2 min after, so
  it isn't holding ~1.5 GB between calls (first call after idle waits ~10 s for the model to load).
- **Self-detection over the Accessibility detector** — the capturer detects the call from the SCK window
  list (Screen Recording only). Accessibility is then needed *only* for speaker names, so the core loop
  survives if that grant is missing.
- **ScreenCaptureKit, not Core Audio process taps** — taps need the macOS 14.2+ SDK; this toolchain
  ships 13.3, where SCK is available.

## Known limitations

- **Speaker names** read Zoom's active-speaker from the Accessibility tree. Solid in 1-on-1; best-effort
  in large gallery calls. Your own lines are labeled `You`.
- **Native meeting apps only** for *auto*-detection — it recognizes Zoom/Teams/Webex/FaceTime windows.
  **Browser calls** (Google Meet, Zoom web, Slack huddles) aren't auto-detected yet — start them by hand
  with **Record**.
- **Ad-hoc signing** — every rebuild changes the app's fingerprint, so you re-grant the three permissions
  once per build. Developer-ID + notarization (roadmap) fixes this permanently.
- Screen recordings are **silent** (audio mux is a follow-up); **Only-when-shared** currently keys off
  *your* share.
- ScreenCaptureKit audio can occasionally go silent after long uptime — reset with `sudo killall coreaudiod`.
- **Recording calls may require consent** depending on jurisdiction. That's on you.

## Roadmap

**Shipped**

- [x] **Self-detecting capture** — finds the call from the window list (Screen Recording only), transcribes you + the others, only while in a call
- [x] **Speaker names** from Zoom's AX tree (detector spawned as a direct child so its grant works)
- [x] **transcribe.cpp engine** (≈2× faster, ≈half the RAM of whisper.cpp) — **whole-utterance**, **auto-detect → English**; language + translate toggles; **custom vocabulary** biases recognition of names/jargon
- [x] **Manual + auto recording** — **start / pause / stop by hand** (composed with auto-detect, so pause/stop really releases the mic), plus a **floating in-call panel** (live transcript + scratchpad)
- [x] **On-demand Whisper** (unloaded when idle) + on-demand LLM — light on an 8 GB Mac
- [x] **Meeting history** + **post-call AI** (summary, action items, "ask"), **rename**, recording **thumbnails**
- [x] **Desktop app (Tauri)** — double-click, self-starts the stack, **menu-bar recording indicator**, ask-once permissions
- [x] **Screen-share stealth** · **video screen recording** (whole call / only-when-shared)

**Next**

- [ ] **Real-time Copilot** — in-call coaching in an always-on **stealth** overlay (the keystone)
- [ ] **Portable installer** — Developer-ID sign + notarize + bundle the models, so any Mac runs it (no `setup.sh`, no re-grant)
- [ ] **Browser + Slack** meetings — detect Google Meet / Zoom web / Slack huddles
- [ ] **audio in screen recordings**; **global shortcut** + **launch-at-login** (need Tauri plugins)
- [ ] The other modules — **Dictation, Writing Assistant, Agents**

## License

MIT — see [LICENSE](LICENSE).
