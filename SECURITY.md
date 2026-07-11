# Security & Privacy

My Assistant is built to keep your calls and dictation **on your Mac**. This document states exactly what
runs locally, what (if anything) leaves the machine, where data is stored, and how to report a
vulnerability. For the engineering detail behind these claims, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Privacy model

- **On-device by default.** Audio capture, voice-activity detection, transcription (Whisper via
  [transcribe.cpp](https://github.com/handy-computer/transcribe.cpp) on the Apple GPU), and post-call
  summarization (Gemma via LiteRT-LM) all run as local processes. **No bot joins your calls, and no audio
  or transcript leaves the machine** in the default configuration.
- **No account, no telemetry.** There is no sign-in, no analytics, and no crash/usage reporting. The app
  makes no network request on its own beyond the two explicit, opt-in cases below.

## What can leave the machine (only when you opt in)

1. **First-run model download.** On first launch, if the ~845 MB speech model isn't already present, the
   app **asks before downloading** it from Hugging Face. Decline and nothing is fetched — configure a cloud
   key instead, or run `./setup.sh` to fetch it yourself.
2. **Bring-your-own cloud model (optional).** If you select a cloud LLM in *Settings → AI Model*
   (Groq / OpenAI / Cerebras / custom), the **meeting text** you summarize or ask about is sent to that
   provider using **your** API key. This is the only path that transmits meeting content, and only to the
   provider you choose. Keep the default **On-device** model selected to stay fully local.

## Where your data lives

All runtime data is written under `~/Library/Application Support/My Assistant/` and is **never** part of
the repository (the whole `data/` tree is git-ignored):

| Path | Contents |
|---|---|
| `data/meetings/*.json` | saved transcripts, summaries, and notes |
| `data/meetings/rec/*.mp4` | screen recordings — only if you enable video recording |
| `models/` | the downloaded on-device speech model |
| `settings.json` | your preferences (a durable mirror of the UI's settings) |

Dictation results are held **in memory only** (session-lived) and are not written to disk.

## API keys

Bring-your-own API keys are stored **locally** on your machine (in the app's settings) and sent **per
request** to the provider you selected. They are never written to any tracked file in this repository and
never transmitted anywhere except that provider.

## macOS permissions

The app requests the **minimum** macOS permission for each feature, and only when that feature is first
used — Microphone and *System Audio Recording Only* for meeting capture, Accessibility for speaker names
and dictation paste, and Screen Recording only if you turn on video recording. **Settings → General →
Permissions** shows the live grant status of each. Full model:
[ARCHITECTURE.md → macOS permissions & code-signing](ARCHITECTURE.md#macos-permissions--code-signing--the-biggest-trap).

## Recording consent

Recording a call may require the consent of the other participants depending on your jurisdiction — some
regions require **all-party** consent. Complying with applicable law is **your responsibility** as the
operator; it is a legal question, not a technical one.

## Reporting a vulnerability

This is a private, single-maintainer project. If you find a security issue, please contact the maintainer
directly (or open a private issue) rather than disclosing it publicly.
