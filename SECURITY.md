# Security & Privacy

My Assistant is built to keep your calls and dictation **on your Mac**. This document states exactly what
runs locally, what (if anything) leaves the machine, where data is stored, and how to report a
vulnerability. For the engineering detail behind these claims, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Privacy model

- **On-device by default.** Audio capture, voice-activity detection, transcription (Whisper via
  [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) on the Apple GPU), and post-call
  summarization (when the optional Gemma/LiteRT-LM runtime has been installed) all run as local processes.
  **No bot joins your calls, and no audio or transcript leaves the machine** while the local provider is
  available and selected.
- **No account, no telemetry.** There is no sign-in, no analytics, and no crash/usage reporting. The app
  makes no network request on its own beyond the two explicit, opt-in cases below.

## What can leave the machine (only when you opt in)

1. **First-run model download.** On first launch, if no speech model is already present, the app asks the
   user to choose a 44 MB–1.1 GB revision-pinned model before downloading it from Hugging Face. Decline and
   nothing is fetched. Cloud speech
   transcription is not implemented in this beta, so the local model is required for dictation and meeting
   transcripts. Developers can run `./setup.sh` to fetch it separately.
2. **Bring-your-own cloud model (optional).** If you select a cloud LLM in *Settings → AI Model*
   (Groq / OpenAI / Cerebras / custom), the **meeting text** you summarize or ask about is sent to that
   provider using **your** API key. This is the only path that transmits meeting content, and only to the
   provider you choose. Keep **On-device** selected to stay fully local; the current DMG does not yet install
   that optional Intelligence runtime on a clean Mac, so summary/Ask will fail closed until it is installed or
   a BYOK provider is configured.

## Where your data lives

All runtime data is written under `~/Library/Application Support/My Assistant/` and is **never** part of
the repository (the whole `data/` tree is git-ignored):

| Path | Contents |
|---|---|
| `data/meetings/*.json` | saved transcripts, summaries, and notes |
| `data/meetings/rec/*.mp4` | screen recordings — only if you enable video recording |
| `models/` | the downloaded on-device speech model |
| `settings.json` | non-secret preferences (a durable mirror of the UI's settings) |
| `data/tmp/` | private (`0700`) short-lived audio/request files, deleted immediately or on next launch |

Dictation history is held **in memory only**. Audio uses a uniquely named `0600` file inside the private
application directory only while it is being transcribed and is deleted immediately afterwards.

## API keys

Bring-your-own API keys are stored in **macOS Keychain**, not `settings.json`, and are sent per request only
to the provider you selected. Summary and Ask routes ignore caller-supplied destinations and credentials.

## Local service boundary

The packaged UI/API server runs inside the Tauri process and fails closed if its loopback port is already
owned. A random per-launch secret is exchanged for an HttpOnly, SameSite cookie; native helpers use the
same secret through their environment. Requests with an unexpected Origin, missing token, or wrong Host
are rejected. The transcription helper independently requires authentication and caps uploads at 64 MB.

The first-run speech model is accepted only when its exact size and pinned SHA-256 match the audited Hugging
Face artifact. Incomplete or mismatched downloads are deleted and never loaded.

## macOS permissions

The app requests the **minimum** macOS permission for each feature, and only when that feature is first
used — Microphone for your voice, *System Audio Recording Only* for other participants, Accessibility for
speaker names and dictation paste, and Screen Recording only if you turn on video recording. **Settings →
General → Permissions** shows live status where macOS exposes a reliable check; Microphone and System Audio
use accurate **Requested on first use** guidance plus recovery links rather than invented allowed/denied state. Full model:
[ARCHITECTURE.md → macOS permissions & code-signing](ARCHITECTURE.md#macos-permissions--code-signing--the-biggest-trap).

## Recording consent

Recording a call may require the consent of the other participants depending on your jurisdiction — some
regions require **all-party** consent. Complying with applicable law is **your responsibility** as the
operator; it is a legal question, not a technical one.

## Reporting a vulnerability

This is a private, single-maintainer project. If you find a security issue, please contact the maintainer
directly (or open a private issue) rather than disclosing it publicly.
