# Assistant

A private, **on-device** super-app for calls and text — **Dictation, Writing Assistant, Meetings, and
Agents** in one minimal shell. **Meetings ships first:** real-time transcription of *you and the other
participants* in any Mac video call (Zoom / Meet / Teams), with speaker names. Whisper runs on the
Apple GPU — **no bot joins the call, no audio leaves your machine, no meeting-app captions toggle.**

**Status:** the Meetings module works end-to-end — mic + the other participants, auto-scoped, with live
speaker names. Every call is **saved on-device** into a browsable history, with **post-call summaries,
action items, and "ask this meeting"** from an on-device LLM. It runs today in a browser GUI (a minimal
light/dark UI) and now also in a native, cross-platform [Tauri](https://tauri.app) shell — a real window
with a menu-bar tray and **screen-share stealth** (`desktop/`). Real-time in-call **coaching** is the
next milestone — see the [roadmap](#roadmap).

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

## Requirements

- macOS 13+ on Apple Silicon
- Xcode Command Line Tools (`xcode-select --install`), [Homebrew](https://brew.sh), Node

## Setup

```bash
./setup.sh     # installs whisper-cpp + LiteRT-LM, downloads the models (~3.2 GB), builds the helpers
```

Then grant **your Terminal** these permissions (System Settings → Privacy & Security):

- **Screen & System Audio Recording** — to hear the other participants
- **Accessibility** — for the speaker-names layer (when enabled)

## Usage

```bash
./run.sh       # then open http://localhost:8790 · Ctrl-C to stop
```

Get on a call and captions stream in live. By default it **auto-scopes to a running
Zoom/Teams/Webex/FaceTime** window (so a background YouTube tab is ignored) and also transcribes
**your own mic** as `You`. Flags pass straight through to the capturer:

```bash
./run.sh --app all                 # capture ALL system audio (use for Google Meet in a browser)
./run.sh --app com.google.Chrome   # scope to a specific app  (list them: stt/meeting-tap --list-apps)
./run.sh --mic off                 # don't transcribe your own mic
./run.sh --me "Niket"              # your label (default: your macOS name) → shown as "Niket (Me)"
./run.sh --expressions             # keep *sigh* / (laughs) as [sigh] cues (sentiment read)
```

> **Use headphones.** On speakers the mic re-hears the remote voices, so their words can show up twice.

> **Run it from a Terminal that has the Screen-Recording grant.** `run.sh` execs the capturer directly
> for this reason — macOS won't reliably persist screen-recording grants for ad-hoc-signed standalone
> apps, but Terminal's grant is stable.

## Project layout

| Path | Role |
|---|---|
| `run.sh` | one-command launcher |
| `setup.sh` | one-time install (engine + model + build) |
| `stt/meeting-tap.swift` | ScreenCaptureKit capture + VAD + POST to whisper-server |
| `stt/whisper/` | Whisper model (git-ignored; fetched by `setup.sh`) |
| `app/server.js` | SSE relay + meeting recording + summary / ask API |
| `app/index.html` | the app shell — nav, live transcript, Library + detail, settings; light/dark UI |
| `meeting-state/` | AX layer: speaker names + in-meeting / mute / share |
| `stt/llm/` | on-device LLM (git-ignored; fetched by `setup.sh`) for post-call AI |
| `data/meetings/` | saved meeting history (git-ignored, on-device) |
| `desktop/` | Tauri desktop shell — native window + menu-bar tray + screen-share stealth |

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

- [x] Transcribe **your own mic** + **scope capture to the meeting app** (`--app auto`)
- [x] Speaker **names** from Zoom's AX tree (1-on-1 solid)
- [x] **App shell** — module nav, settings, minimal **light/dark** UI
- [x] **Meeting history** — every call saved on-device; browsable Library + per-meeting detail
- [x] **Post-call AI** — summary, action items, "ask this meeting" (on-device LLM; API/cloud tiers pluggable)
- [x] **AI Model picker** in Settings — choose on-device (LiteRT-LM, started on demand) or a BYO-key API provider
- [x] **Desktop shell (Tauri)** — native window hosting the UI, menu-bar tray, screen-share **stealth** (`desktop/`)
- [ ] Desktop, cont. — app-managed sidecars, migrate the capture grant onto the signed app, global shortcut + launch-at-login, packaged installers
- [ ] **Real-time Copilot** — in-call coaching in an always-on **stealth** overlay (built on the shell above)
- [ ] Sharper names in **large gallery calls**; in-GUI device pickers
- [ ] The other modules — **Dictation, Writing Assistant, Agents**

## License

MIT — see [LICENSE](LICENSE).
