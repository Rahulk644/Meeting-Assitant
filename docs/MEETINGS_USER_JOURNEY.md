# Meetings Beta user journey

This is the observable contract for a bot-free local meeting. It tells an operator what My Assistant is
doing, what should be visible, what each state means, and when to stop a test. Automated checks do not replace
this physical journey.

## The promise

The user should never have to guess whether My Assistant is idle, recording, paused, finalizing, generating
notes, ready, or blocked. Opening a meeting app is not enough to start recording. A stable live-call signal
starts capture; a visible signal confirms it; Pause and Stop remain available; and the completed transcript
survives an Intelligence-provider failure.

While Zoom is in front, a movable 52 × 52 native AppKit **Nub** stays above the active meeting Space without
activating My Assistant. Its app icon and small state dot provide a quiet persistent signal. Clicking it
expands in place into an adaptive vertical conversation card: meeting identity and local-capture state
first, up to five speaker-grouped transcript bubbles second, **What did I miss?** third, and
Pause/Resume/Stop last. It grows only as transcript content requires, up to five lines. Clicking its
top-right control collapses it again. Dragging the background moves it for the current app session. The full
Transcript/Scratchpad/Assist history remains in My Assistant.

The Nub, in-app panel, and menu bar project the same Rust-owned lifecycle: Starting, Recording locally,
Needs attention, Pausing, Paused, Finalizing, or hidden when idle. They never infer state independently.

## Before the first call

1. Launch My Assistant and leave it running. Closing the red window button hides the window; the menu-bar app
   continues running.
2. Open **Settings → Meetings** and confirm:
   - **Display live recording indicator** is on;
   - **Detect meeting start & end automatically** is on;
   - **Desktop meeting apps** is on for Zoom; and
   - **Screen recording** is **Off** for the first audio-only test.
3. Select and download a speech model. Meetings and Dictation use the selected speech model.
4. Open **Settings → General → Permissions**. Microphone captures the user; System Audio Recording captures
   the other callers; Accessibility enriches names and meeting state. Screen Recording is unnecessary while
   meeting video is off.
5. Allow My Assistant notifications. The notification is an additional start signal; the persistent native
   panel remains the authoritative outside-app status.
6. Use headphones for the first acceptance call. Speaker playback can leak the remote voice into the mic and
   create duplicates until AEC qualification is complete.

If macOS presents a permission prompt during the first call, treat that call as setup. Grant the permission.
The helper retries a failed source while the call remains active, so the panel may recover from **Needs
attention** to **Recording locally** without an app restart. Leave and rejoin before judging channel quality
if macOS does not apply the grant to the active capture session.

## Exact Zoom journey

### 1. Idle

**User action:** Open Zoom but do not join or start a live call.

**What My Assistant does:** It checks bounded Core Audio process activity every two seconds. The microphone,
Zoom audio tap, and speech model remain off.

**What the user sees:** No recording notification, no animated menu-bar waveform, no red live dot, and no
call panel.

**Failure:** Merely opening Zoom starts recording.

### 2. Live call detected

**User action:** Join a real Zoom meeting with a consenting second person and connect computer audio.

**What My Assistant does:** Two matching observations are required, so the expected detection window is
about two to four seconds. Rust accepts one stable Zoom call edge, creates the live meeting record, starts
the selected speech runtime on demand, and asks the macOS adapter to capture the microphone and Zoom process
audio as separate sources.

**What the user sees outside My Assistant while sources start:**

- a macOS notification titled **Meeting detected · Zoom**, saying local capture is starting; and
- a tiny non-activating Nub above Zoom with an amber status dot.

Click the Nub. It expands without moving focus from Zoom. The meeting title remains the primary heading; the
secondary state says **Zoom · Starting local capture**. **Stop** remains reachable.

The product does not yet claim to be recording. When macOS successfully opens the microphone and Zoom Core
Audio tap, the secondary state changes to **Zoom · Recording locally · Mic + meeting audio ready**,
**Pause** and **Stop** are available, and the menu-bar waveform starts. Source-open
acknowledgement—not speech—is the readiness signal. A silent remote participant must never delay recording
or discard the user's microphone audio.

If either source cannot start, the expanded Nub says **Audio needs attention** and names the failed source.
**Open** brings My Assistant forward; the in-app panel exposes **Fix audio permissions** as
**Allow Microphone** or **Allow System Audio Recording** when one failed channel is known. It must not show
the recording state.

After system audio has produced buffers at least once, an eight-second buffer-heartbeat loss is a dropout:
the expanded Nub briefly reports **Needs attention** with **restarting automatically**, the native source is
torn down, and the existing retry loop reopens it. Three failed recoveries within one minute trigger a
clean helper-process recycle; the signed parent relaunches it with bounded backoff and the Rust meeting
record remains the same. Initial remote silence is never a dropout. Ordinary quiet after speech is also not
a dropout when the Core Audio tap continues delivering silent buffers.

**What the user sees when My Assistant is open:**

- Meetings automatically enters **Live**;
- a red dot appears beside **Meetings · Beta**;
- the in-app panel first says **Starting capture…**, then says **Recording locally** only after both channels
  are confirmed; it shows Zoom, the meeting title or **Impromptu meeting**, and an elapsed timer; and
- Transcript, Scratchpad, Assist, Pause, and Stop are visible.

The main window must not steal focus from Zoom. The movable Nub floats above Zoom; an explicit **Open** or
the menu-bar item **Show My Assistant** deliberately brings forward the full panel.

**Failure:** No notification/native panel within five seconds, repeated notifications for the same call,
the main window steals focus, the product says recording before both channels are confirmed, or a source
failure has no specific recovery state.

### 3. Live transcript

**User action:** Each person speaks a clear sentence and pauses naturally. Then speak over one another once.

**What My Assistant does:** Each channel is split at a natural pause, transcribed locally as a complete
utterance, and appended to the meeting. Temporary utterance audio is removed after transcription.

**What the user sees:** Transcript lines appear after the pause plus inference time, not word by word. The mic
channel is **You**. A remote line uses available Zoom/Accessibility evidence or an honest uncertain label.
The expanded Nub keeps the five most recent lines; the Live view keeps the longer feed.

**Silence regression:** Start the call, speak from the local microphone while the remote person remains
silent, and confirm the Nub already says **Recording** and self transcript lines appear. The remote person
may first speak ten minutes later; their first line must append to the same meeting. The first ten minutes
must remain present.

**Failure:** After 20–30 seconds of clear alternating speech there are no lines, only one channel appears,
the same remote sentence appears as both You and remote while headphones are used, or uncertain evidence is
shown as a confident person name.

### 4. Control and assistance

**Pause:** Every surface first says **Pausing** while the helper releases both sources, then says **Paused**
only after that release is acknowledged. The meeting record stays open. Resume returns to **Starting** until
both sources reopen, then continues the same timer and meeting.

**Scratchpad:** Text is saved into the live meeting after a short debounce. Generated notes do not overwrite
it.

**Assist:** After transcript exists, click the Nub and choose **What did I miss?**. Its answer appears in the
overlay and uses only the current non-excluded transcript from the last 60 seconds. The full panel also
offers **Summarize so far**, **Explicit action items**, and **Useful follow-ups** over the meeting so far plus
the scratchpad. They require a configured local/BYOK Intelligence provider and cannot write or send anything.

**Stop:** Stop flushes the scratchpad and ends this recording immediately. Automatic recording stays
suppressed for the remainder of the same Zoom call and returns for the next call.

**Failure:** Pause continues adding lines, Resume creates a second meeting, Scratchpad disappears, an Assist
failure interrupts capture, or Stop restarts recording during the same call.

### 5. Call end and saved meeting

**User action:** Leave Zoom without pressing Stop.

**What My Assistant does:** Three missing observations are required, so capture should stop about four to six
seconds after Zoom audio ends. A 15-second debounce prevents a brief Zoom device/process renegotiation from
splitting the call. The transcript is then finalized and notes are queued if the meeting contains at least
30 words and lasts at least 30 seconds.

**What the user sees:**

- the menu-bar waveform stops and both panels say **Finalizing** while the end debounce and save complete;
- the native panel then disappears;
- My Assistant returns to the meeting library;
- the meeting card appears with its title, Zoom, duration, and transcript snippet; and
- the card shows **Notes…**, **Notes**, **Retry notes**, or an honest too-short/provider error.

Opening the card exposes Summary, Action items, Transcript, and Ask. The transcript and scratchpad remain
available even if note generation fails.

**Failure:** Capture continues after the debounce, one call becomes multiple records, the meeting disappears
after relaunch, a failed summary loses the transcript, or the meeting is silently absent despite substantive
captions.

## State language

| State | Persistent meaning | User control |
|---|---|---|
| Ready | Monitoring only; no audio capture or model residency | Start manually or join a supported call |
| Detecting | Candidate evidence exists but is not stable yet | None; no recording claim |
| Starting | Stable call detected; mic and meeting-audio sources are being opened | Expand Nub or Stop |
| Recording locally | macOS has opened mic plus meeting-app audio; silence is valid and transcript capture is active | Expand Nub, Pause, Stop, or catch up |
| Pausing | Pause is requested; at least one capture source has not yet acknowledged release | Stop or wait |
| Paused | Same meeting remains open; capture is released | Resume or Stop |
| Finalizing | Capture is being released or is stopped; debounce/checkpoints are being committed | Wait; transcript remains local |
| Generating notes | Transcript is safe; optional Intelligence is working | Open transcript; retry if needed |
| Ready to review | Meeting record and available derived notes are saved | Correct, Ask, rename, export |
| Needs attention | A permission or audio channel failed | Open My Assistant and use the specific recovery action, or Stop |

These names come from one Rust-owned lifecycle projection. The capture helper reports bounded source-open
readiness and separate post-flow dropout health; the WebView, menu bar, notification, and AppKit Nub do not
independently infer recording.

## First acceptance call

Use a meeting longer than two minutes and speak more than 30 words per person. Record:

- detection time;
- notification and menu-bar signal;
- permission prompts;
- first self and remote transcript time;
- Pause/Resume/Stop behavior;
- overlap and speaker-label behavior;
- end-detection and save time;
- notes/provider result; and
- whether the meeting is still complete after relaunch.

Any missing visible state, ambiguous capture status, or failed channel is a product failure—not operator
confusion.
