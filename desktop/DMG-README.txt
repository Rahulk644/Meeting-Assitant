MY ASSISTANT 0.1.0 — APPLE SILICON BETA

Requirements
• Apple Silicon Mac (M1 or newer)
• macOS 14.4 or newer
• Up to 2 GB free for the on-device speech model you choose on first use

Install
1. Drag My Assistant into Applications.
2. Open My Assistant from Applications once.
3. If macOS says Apple cannot verify it is free of malware, click Done — do not move it to Trash.
4. Open System Settings → Privacy & Security. Scroll to Security and click Open Anyway beside
   My Assistant. This option appears for about one hour after step 2.
5. Authenticate, then click Open in the final confirmation. Future launches work normally.

On older macOS versions, Control-clicking My Assistant in Applications and choosing Open may offer
the same one-time approval directly. If it does not, use Privacy & Security → Open Anyway above.

First use
• Choose and download a speech model when prompted (about 44 MB–1.1 GB).
• Grant Microphone permission for your voice.
• Grant System Audio Recording for other meeting participants.
• Grant Accessibility for system-wide dictation paste, meeting-window detection, and speaker names.

What is ready
• Dictation: production feature. Press Right Command to start and stop.
• Meetings: Beta. Dedicated meeting apps are detected from active per-app audio. Browser and
  communication-app calls use active audio plus meeting-window evidence to avoid false starts.
  Manual recording works with any meeting platform.

Privacy
Audio is captured and transcribed locally on this Mac. If you configure a cloud AI provider for
meeting notes, transcript text is sent to that selected provider; meeting audio is not.

Why the one-time approval?
This independent beta uses a stable local code-signing certificate rather than Apple's paid Developer ID.
macOS therefore cannot show Apple's lower-friction identified-developer dialog. The app remains code-signed
so accidental changes to the bundle are detectable, but macOS cannot identify the publisher through Apple.
There is no legitimate way for an independent build to remove this warning without Developer ID notarization.

Managed work Macs may block manual approval. In that case, the Mac's administrator must allow the app.
