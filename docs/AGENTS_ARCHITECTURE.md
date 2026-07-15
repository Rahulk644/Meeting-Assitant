# Agents architecture and product contract

This is the durable authority for the Agents pillar. It separates the product vision from what the current
macOS build actually does. `PRODUCT_RELEASE_MATRIX.md` remains the final authority for production visibility.

## Product intent

The user should be able to speak naturally while another app is in front:

> “Reply that Friday works, keep it warm, and do not send yet.”

My Assistant should understand the relevant screen, prepare the requested result in the right place, and
keep the user in control. Push-to-talk and a global shortcut come first. An optional local wake phrase can
remove the keyboard step after its battery, privacy, accent, and false-trigger gates pass.

This is not ordinary dictation. Dictation converts speech to text in the focused field. An Agent interprets
an instruction against screen context, may use tools, and must verify that the intended outcome happened.

## Current production truth

The first foundation is deliberately read-only:

- the Agents view is visible and labels itself **Foundation preview**;
- **Inspect app behind My Assistant** invokes `agent-context/agent-context` once and the helper exits;
- the helper reads a bounded Accessibility snapshot of the frontmost app, or the topmost regular app behind
  My Assistant when the dashboard itself is frontmost;
- secure fields are redacted and every element receives an opaque request-local `el_N` identifier;
- the response is capped by element, depth, per-node child, time, process-output, and HTTP limits;
- no screenshot, audio, model, network, computer action, or persistence is involved.

The current helper is an on-demand native Swift resource. It adds no idle process or idle RAM. The Rust
loopback service enforces a 1.5-second process deadline and 256 KiB output ceiling. The helper itself targets
180 returned elements, 1,200 visited elements, depth 12, 80 children per node, and a 750 ms capture budget.
These are safety limits, not a claim that every app exposes a complete Accessibility tree.

## Correct screen-understanding architecture

The raw idea “send the whole Accessibility tree and let a model return an AX path” is directionally right,
but the robust implementation is a hybrid compiler with typed references:

```text
voice instruction
      │
      ▼
ContextSnapshot (active app + active window + focused/selected context)
      │
      ├─ 1. Accessibility tree — structured, cheap, default
      ├─ 2. OS OCR — only when visible text is missing
      ├─ 3. app adapter / browser DOM — when explicitly supported
      └─ 4. screenshot + vision — last resort, separately disclosed
      │
      ▼
bounded untrusted context + opaque snapshot_id / element_id
      │
      ▼
typed plan → policy gate → user preview/confirmation → native executor
      │
      ▼
fresh observation verifies the result
```

The model never receives live AX object pointers and never invents an Accessibility path. It returns a
typed intent such as `replace_selection`, `set_value`, `invoke`, or `open_url` referencing
`{snapshot_id, element_id}`. Before mutation, the executor verifies that the snapshot is still current, the
same app/window is active, the target is enabled and appropriate, and the field is not secure. Any screen
change invalidates the prior snapshot.

Screen text is untrusted data. Web pages, documents, emails, and chats can contain instructions intended to
hijack an agent. Context must be delimited as untrusted; it cannot override system policy, user intent,
confirmation rules, provider boundaries, or tool permissions.

## Safety and action tiers

All side effects must pass one native policy chokepoint. The model cannot bypass it by wording an action
differently.

| Tier | Examples | Default |
|---|---|---|
| Read | inspect screen, read selected text | allow after explicit invocation |
| Draft | compose/rewrite, fill a field without submitting | preview; one-step undo |
| External | send, publish, join, purchase, share, delete | explicit confirmation at execution time |
| Restricted | password/secret access, security-setting changes, arbitrary code | block unless a separately designed capability proves necessity and safety |

The first mutation milestone should support **draft, replace, undo** only. It must never press Send, Return,
Publish, Buy, Join, Delete, or Confirm automatically. A visible kill switch is required before multi-step
actions ship.

## Interaction states

Use one compact native surface, not another persistent WebView:

1. **Idle** — no agent capture.
2. **Listening** — microphone active, cancel always visible.
3. **Understanding** — capture/transcription finished; bounded screen context is being compiled.
4. **Preview** — proposed text or plan; Apply, Edit, Cancel.
5. **Confirm** — only for the exact external/destructive step.
6. **Running** — named current action plus Stop.
7. **Verifying** — fresh observation checks the outcome.
8. **Done / Needs help** — concise result and undo/recovery path.

The menu-bar surface can show agent availability and wake-phrase status later. It must never imply that the
microphone or screen is being observed when it is not.

## Wake phrase

Wake phrase is a separate always-on audio subsystem, not an extension of the Accessibility scanner. It is
not enabled in the current build.

Required contract:

- opt-in and off by default during early testing;
- wake inference is local; pre-wake audio never leaves the device and is not retained;
- a persistent menu-bar state and immediate audible/visual activation cue;
- one-click pause plus automatic suspension during calls, screen lock, sleep, and active dictation unless
  the user chooses otherwise;
- custom phrase evaluation across accents and real rooms; the product cannot ship a developer-only phrase;
- measured incremental CPU, physical footprint, battery drain, false accepts, and false rejects on supported
  Macs before enabling it by default.

`dscripka/openWakeWord` is a useful Apache-2.0 research/training reference, but its primary runtime is
Python. `rhasspy/openWakeWord-cpp` is MIT and avoids Python, but currently expects ONNX Runtime and provides
only a small C++ inference program, not a mature macOS audio/product integration. Neither is adopted until a
native harness proves the custom phrase and resource budgets. Apple does not expose a general third-party
Siri-style wake-word API.

## Providers and “Sign in with ChatGPT”

ChatGPT subscriptions and API billing are separate. OpenAI documents “Sign in with ChatGPT” for its own
Codex clients, but there is no documented public third-party OAuth flow that lets My Assistant consume a
user's ChatGPT Plus/Pro allowance. Do not imitate private Codex endpoints or collect ChatGPT session tokens.

Supported product paths are:

- OpenAI API BYOK stored in macOS Keychain and billed to the user's API account;
- other explicit provider BYOK adapters;
- a My Assistant cloud plan with clear metering and privacy terms;
- a future delegated OpenAI authorization adapter only if OpenAI publishes and supports it for third-party
  apps.

Official references:

- [ChatGPT Plus does not include API usage](https://help.openai.com/en/articles/6950777-what-is-chatgpt-plus)
- [ChatGPT and API billing are separate](https://help.openai.com/en/articles/9039756-billing-settings-in-chatgpt-vs-platform)
- [OpenAI API authentication](https://developers.openai.com/api/reference/overview#authentication)
- [Codex-specific Sign in with ChatGPT](https://help.openai.com/en/articles/11381614-api-codex-cli-and-sign-in-with-chatgpt)

## Primary-source competitive evidence

These projects informed the contracts above; no source code was copied into My Assistant:

- [openclaw/Peekaboo](https://github.com/openclaw/Peekaboo) — native Swift, bounded Accessibility
  collection, opaque element IDs, direct AX actions, stale-snapshot handling, and screenshot/OCR fallback.
- [QwenLM/open-computer-use](https://github.com/QwenLM/open-computer-use) — native Swift active-window
  Accessibility snapshot plus optional screenshot and indexed elements.
- [ghostwright/ghost-os](https://github.com/ghostwright/ghost-os) — Accessibility-first search, app-specific
  fallbacks, local vision escalation, and post-action verification.
- [AmrDab/clawdcursor](https://github.com/AmrDab/clawdcursor) — fused a11y/OCR UI map, snapshot-scoped element
  references, secure-field redaction, untrusted-screen framing, and one safety gate.
- [sam-siavoshian/agent-notch](https://github.com/sam-siavoshian/agent-notch) — native status surface, live
  action feedback, voice entry, and kill-switch concepts; its screenshot-first agent loop is not our default.
- [farzaa/clicky](https://github.com/farzaa/clicky) and [farzaa/freewrite](https://github.com/farzaa/freewrite)
  — earlier public screen-aware writing experiments; useful product context, not current HeyClicky evidence.

## Ordered implementation

1. Physically verify the read-only context snapshot across Safari, Chrome, Mail, Notes, Slack, Terminal,
   Electron, full-screen, multiple displays, secure fields, and permission denial/recovery.
2. Add a global Agent shortcut and a native non-activating Listening → Understanding → Preview surface.
3. Add a provider-neutral structured planner; screen data is untrusted and API keys remain native/Keychain.
4. Ship a narrow draft/replace/undo executor with snapshot revalidation and fresh-result verification.
5. Add OCR and app-specific adapters only where the Accessibility acceptance matrix proves a gap.
6. Add separately permissioned screenshot/vision fallback with a clear disclosure.
7. Benchmark a local wake phrase harness; ship only after privacy, accuracy, CPU/RAM, battery, and suspension
   gates pass.
8. Expand to governed multi-step skills, audit history, and team/cloud capabilities after the single-action
   contract is reliable.
