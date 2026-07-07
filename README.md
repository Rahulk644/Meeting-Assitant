# Assistant

A private, **on-device** super-app for calls and text — **Dictation, Writing Assistant, Meetings, and
Agents** in one minimal shell. **Meetings ships first:** real-time transcription of *you and the other
participants* in any Mac video call (Zoom / Meet / Teams), with speaker names. Whisper runs on the
Apple GPU — **no bot joins the call, no audio leaves your machine, no meeting-app captions toggle.**

**Status:** the Meetings module works end-to-end and ships as a **double-click desktop app**
([Tauri](https://tauri.app), `desktop/`) — no terminal, no dev commands. It transcribes *you and the
other participants* (auto-scoped, with live speaker names, and **capturing only while you're in a
call**), **saves every call on-device** into a browsable history with **post-call summaries, action
items, and "ask this meeting"**, can **screen-record** the call (whole call, or only while a screen is
shared), and can make its window **invisible to screen sharing**. The same UI also runs in a plain
browser for development. Real-time in-call **coaching** — the Copilot — is the next milestone; see the
[roadmap](#roadmap).

## How it works

```
 the other participants'                       you open a browser at:
 audio (system output) ──ScreenCaptureKit──▶  stt/meeting-tap  (Swift)
                                                 │ energy-VAD → per-utterance WAV
                                                 ▼
                                    whisper-server  (whisper.cpp + Metal GPU)
                                    large-v3-turbo · model stays hot in memory
                                                 │ transcript  (POST /ingest)
                                                 ▼
                                    app/server.js ──Server-Sent Events──▶ http://localhost:8790
                                                                          live transcript UI
```

- **Capture** — `meeting-tap` taps the meeting app's system-output audio (the remote voices) via
  ScreenCaptureKit **and your mic** via AVAudioEngine, splitting each into utterances on silence.
- **Transcribe** — a hot `whisper-server` transcribes each utterance in ~1s (≈4.6× real-time on an M1).
- **Display** — a zero-dependency Node server streams captions over SSE to the app UI
  (`app/index.html`): a minimal light/dark shell with the module nav, live transcript, and settings.
- **Names** — `meeting-state/` reads Zoom's Accessibility tree for the active-speaker name (and
  mute / share state); `server.js` stamps each remote line with who's speaking.

## Post-call AI

Every meeting is recorded to `data/meetings/*.json` (on-device, git-ignored). Open one from the
**Library** for an AI **summary + action items**, and to **ask questions** about it. The model is
pluggable across three tiers (Settings → *AI Model*):

- **Local** *(default, private)* — **Gemma 4 E2B** (text-only) via **LiteRT-LM** on the Apple GPU. It runs
  at *effective-2B* (**~1 GB RAM**, 2.4 GB disk), so it stays light even on an 8 GB Mac — a plain GGUF
  loads the full ~5 B params and can hang. `setup.sh` installs it (`uv tool install litert-lm`) + imports
  the model; the app **starts it on demand** (first summary/ask) and **unloads it when idle**, so it never
  competes with Whisper for RAM during a call. Nothing leaves your machine.
- **API key** — bring your own, any OpenAI-compatible provider (Groq / OpenAI / Cerebras …) for higher
  quality. One client covers local + these.
- **Cloud** — hosted (Pro), later.

Without a model configured, the AI panel simply shows a "set one up" hint — transcription is unaffected.

## Screen recording

Optionally capture the call's **video** (the meeting window / shared screen) alongside the transcript —
**Settings → Meetings → Screen recording**:

- **Off** *(default)* — audio + transcript only.
- **Always** — record the whole call.
- **Only when shared** — record just the parts where a screen is being shared (saves space; keeps the
  demos and slides).

Recordings are H.264 `.mp4` in `data/meetings/rec/` (on-device, git-ignored) and play back in the
meeting's detail view. Capture — mic, system audio, and video — runs **only while you're in a call**.

## Requirements

- macOS 13+ on Apple Silicon
- Xcode Command Line Tools (`xcode-select --install`), [Homebrew](https://brew.sh), Node

## Install & run the app

```bash
./setup.sh                          # one-time: whisper-cpp + LiteRT-LM + models (~3.2 GB) + native helpers
cd desktop && npm install           # one-time: the Tauri CLI (needs Rust — https://rustup.rs)
npx tauri build --bundles app       # build Assistant.app
cp -R src-tauri/target/release/bundle/macos/Assistant.app /Applications/
```

Then **double-click Assistant.app**. On the first call macOS prompts for **Microphone**, **Screen
Recording**, and **Accessibility** — grant each once and it sticks. The app starts everything itself (no
terminal); get on a Zoom/Teams call and it transcribes, and — if you enable screen recording in Settings
— records.

> This build reuses the models/binaries `setup.sh` installed on *this* machine; it isn't a portable
> installer yet (that needs code-signing + notarization — on the roadmap).

## Development

Iterate on the UI in a browser, with capture from the shell:

```bash
./run.sh                            # whisper + server on :8790, then the capturer · Ctrl-C to stop
                                    # open http://localhost:8790
cd desktop && npm run dev           # (optional) the native window instead of a browser tab
```

Capture **auto-scopes to a running Zoom/Teams/Webex/FaceTime** window (a background YouTube tab is
ignored), transcribes **your mic**, and runs **only while you're in a call**. Flags pass through:

```bash
./run.sh --app all                 # capture ALL system audio (e.g. Google Meet in a browser)
./run.sh --gate off                # capture even when no call is detected (Google Meet, which AX can't see)
./run.sh --mic off                 # don't transcribe your own mic
./run.sh --me "Niket"              # your label (default: your macOS name) → "Niket (Me)"
```

> **Use headphones.** On speakers the mic re-hears the remote voices, so their words can show up twice.

> Run `./run.sh` from a Terminal that has the **Screen-Recording grant** — dev builds are ad-hoc-signed
> (grants don't persist across rebuilds), so dev capture rides on Terminal's stable grant. The packaged
> `.app` gets its own grant that sticks.

## Project layout

| Path | Role |
|---|---|
| `desktop/` | **Tauri app** — self-starts the stack; native window, tray, screen-share stealth |
| `run.sh` | dev launcher (whisper + server + capturer) |
| `setup.sh` | one-time install (engine + model + native helpers) |
| `stt/meeting-tap.swift` | ScreenCaptureKit capture (audio **and video**) + VAD + POST to whisper-server |
| `stt/whisper/` | Whisper model (git-ignored; fetched by `setup.sh`) |
| `app/server.js` | SSE relay + meeting recording + summary/ask + video serving (no deps) |
| `app/index.html` | the UI — nav, live transcript, Library + detail (with player), settings; light/dark |
| `meeting-state/` | AX layer: speaker names + in-meeting / mute / share |
| `data/meetings/` | saved history + `rec/` screen recordings (git-ignored, on-device) |

The on-device LLM (LiteRT-LM + Gemma 4 E2B) is installed by `setup.sh` into `~/.litert-lm` — not in the repo.

## Design notes

- **On-device Whisper, not the meeting app's captions** — Zoom captions are single-language, ~80%
  quality, and must be toggled on. Whisper `large-v3-turbo` is multilingual (handles Hindi-English
  code-switching), higher quality, and always available.
- **ScreenCaptureKit, not Core Audio process taps** — taps are cleaner but need the macOS 14.2+ SDK;
  this toolchain ships 13.3, where SCK is available.
- **`whisper-server` (hot model), not `whisper-cli`** — the CLI reloads the model on every call; the
  server keeps it resident on the GPU.

## Known limitations

- **Speaker names** work by reading Zoom's active-speaker from the Accessibility tree (see
  `meeting-state`). Reliable in 1-on-1 calls; best-effort in large gallery-view calls. Your own lines
  are labeled `You`.
- `--app auto` scopes to a running **native** meeting app (Zoom/Teams/Webex/FaceTime). **Google Meet
  runs in a browser** — pass `--app <browser bundle id>`; otherwise auto falls back to all system audio.
- Energy-threshold VAD (not Silero yet) — may mis-segment in very noisy calls. Defences against
  Whisper's noise hallucinations: the mic gate (`--mic-rms`) blocks room noise, low mean word-
  confidence utterances are dropped (`--min-conf`), and `*sigh*`/`[music]` annotations are stripped
  (unless `--expressions` keeps them as sentiment cues).
- **Use headphones.** On speakers, your mic re-captures the remote voices, so their words can appear
  twice (once named, once as you). Headphones keep the two streams clean.
- ScreenCaptureKit audio can occasionally go silent after long uptime — reset with
  `sudo killall coreaudiod`.
- **Recording calls may require consent** depending on jurisdiction. That's on you.

## Roadmap

**Shipped**

- [x] Transcribe **your mic** + the other participants, scoped to the meeting app, **only while in a call**
- [x] Speaker **names** from Zoom's AX tree (1-on-1 solid)
- [x] **Meeting history** + **post-call AI** — summary, action items, "ask this meeting"
- [x] **AI Model picker** — on-device (LiteRT-LM, started on demand) or an OpenAI-compatible provider
- [x] Minimal **light/dark UI** — Snaply-style settings modal, Fathom-style meetings
- [x] **Desktop app (Tauri)** — double-click `Assistant.app`, self-starts the whole stack, menu-bar tray
- [x] **Screen-share stealth** — window invisible to screenshots / shared screens
- [x] **Video screen recording** — whole call, or only while a screen is shared

**Next**

- [ ] **Real-time Copilot** — in-call coaching in an always-on **stealth** overlay (the keystone)
- [ ] **Slack huddles** — capture + detection (an Electron app; closer to the Meet path than Zoom)
- [ ] **Portable installer** — bundle the models, code-sign + notarize, so any Mac runs it (no `setup.sh`)
- [ ] Record-only-when-shared for the **remote's** share; **audio in the video**; global shortcut + launch-at-login
- [ ] Sharper names in **large gallery calls**; in-GUI device pickers
- [ ] The other modules — **Dictation, Writing Assistant, Agents**

## License

MIT — see [LICENSE](LICENSE).
