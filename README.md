# Meeting Assistant

Private, **on-device**, real-time transcription of the *other participants* in any Mac video call
(Zoom / Meet / Teams). Whisper runs on the Apple GPU — **no bot joins the call, no audio leaves your
machine, no meeting-app captions toggle.** It's the foundation for a real-time sales copilot.

**Status:** transcription works end-to-end, live, in a browser GUI. Speaker names, your own mic, and
the coaching layer are on the [roadmap](#roadmap).

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

- **Capture** — `meeting-tap` taps system-output audio (the remote voices) via ScreenCaptureKit and
  splits it into utterances on silence.
- **Transcribe** — a hot `whisper-server` transcribes each utterance in ~1s (≈4.6× real-time on an M1).
- **Display** — a zero-dependency Node server streams captions to a clean browser UI.
- **Names** *(WIP)* — `meeting-state/` reads Zoom's Accessibility tree for the active-speaker name and
  mute/share state.

## Requirements

- macOS 13+ on Apple Silicon
- Xcode Command Line Tools (`xcode-select --install`), [Homebrew](https://brew.sh), Node

## Setup

```bash
./setup.sh     # installs whisper-cpp, downloads the model (~830 MB), builds the native helpers
```

Then grant **your Terminal** these permissions (System Settings → Privacy & Security):

- **Screen & System Audio Recording** — to hear the other participants
- **Accessibility** — for the speaker-names layer (when enabled)

## Usage

```bash
./run.sh       # then open http://localhost:8790 · Ctrl-C to stop
```

Get on a call (or play any audio) and captions stream in live.

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
| `app/server.js` · `app/index.html` | caption relay (SSE, `/ingest`) + live UI |
| `meeting-state/` | AX layer: in-meeting / mute / share (+ names, WIP) |

## Design notes

- **On-device Whisper, not the meeting app's captions** — Zoom captions are single-language, ~80%
  quality, and must be toggled on. Whisper `large-v3-turbo` is multilingual (handles Hindi-English
  code-switching), higher quality, and always available.
- **ScreenCaptureKit, not Core Audio process taps** — taps are cleaner but need the macOS 14.2+ SDK;
  this toolchain ships 13.3, where SCK is available.
- **`whisper-server` (hot model), not `whisper-cli`** — the CLI reloads the model on every call; the
  server keeps it resident on the GPU.

## Known limitations

- Transcribes the **other** participants only — your own mic isn't captured yet.
- Every line is labeled `Speaker` — real names are WIP (`meeting-state`).
- Energy-threshold VAD (not Silero yet) — may mis-segment in very noisy calls.
- ScreenCaptureKit audio can occasionally go silent after long uptime — reset with
  `sudo killall coreaudiod`.
- **Recording calls may require consent** depending on jurisdiction. That's on you.

## Roadmap

- [ ] Speaker **names** — wire `meeting-state` active-speaker → captions
- [ ] Transcribe **your own mic** (second stream)
- [ ] **Google Meet** coverage (Chrome-extension DOM)
- [ ] Real-time **sales coaching** — feed the transcript to Claude (the original goal)

## License

MIT — see [LICENSE](LICENSE).
