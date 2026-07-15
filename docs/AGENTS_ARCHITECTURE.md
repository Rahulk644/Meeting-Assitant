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

## Current product truth

The first foundation is deliberately read-only:

- the Agents view is visible and labels itself **Foundation preview**;
- **Inspect app behind My Assistant** invokes `agent-context/agent-context` once and the helper exits;
- the helper reads a bounded Accessibility snapshot of the frontmost app, or the topmost regular app behind
  My Assistant when the dashboard itself is frontmost;
- secure fields are redacted and every element receives an opaque request-local `el_N` identifier;
- the response is capped by element, depth, per-node child, time, process-output, and HTTP limits;
- no screenshot, audio, computer action, or persistence is involved in inspection.

The production Agents view also exposes compute **configuration**, not autonomous execution:

- **Local** is an honest unavailable lane until a separate install/detect/checksum/RAM/unload lifecycle exists;
- **API key** supports Groq, OpenAI, Cerebras, and custom OpenAI-compatible endpoints, stores the credential
  directly in macOS Keychain, and discovers the provider's current model IDs only after an explicit click;
- **ChatGPT account** detects an official, externally installed native Codex runtime, verifies its OpenAI code
  signature, starts it only on demand with an isolated app-owned `CODEX_HOME`, and obtains account-visible
  model, reasoning-effort, and personality support from Codex App Server.

The structured draft planner and configurable global shortcut are preview-only. A preview can return
`draft_text`, `replace_selection`, or `none`, but the native server reduces every result to a bounded display
object and forces confirmation. There is no Apply, Send, click, shell, file, search, network-tool, or AX
mutation path. API-provider network access is limited to the explicitly selected model request; the Codex
experiment uses a restricted read-only sandbox with no readable roots or tools.

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

Porcupine is the primary candidate because it is native, on-device, supports custom wake words, and the
vendor publishes a small embedded footprint. It is **not** in the app: current use requires a Picovoice
AccessKey and the commercial/redistribution terms must be accepted for the actual product. Vendor accuracy
and resource claims are inputs to our benchmark, not My Assistant evidence.

Rustpotter is the first lightweight control because it is pure Rust, Apache-2.0, and has sub-4 MB model
options, but its own maintainer does not describe it as production-grade. `dscripka/openWakeWord` remains a
useful Apache-2.0 code/training reference; its included pretrained models have separate non-commercial
terms and its primary runtime is Python. `rhasspy/openWakeWord-cpp` is archived and expects ONNX Runtime.
The identical opt-in corpus must compare Porcupine and Rustpotter across real accents/noise, false accepts,
latency, CPU, physical footprint, energy, suspension behavior, integration maturity, and licensing before
an engine is selected. Apple does not expose a general third-party Siri-style wake-word API.

The complete evidence contract is [`WAKE_WORD_BENCHMARK.md`](WAKE_WORD_BENCHMARK.md) and the machine-readable
cases are in `acceptance/cases.json`.

## Providers and ChatGPT account compute

ChatGPT subscriptions and API billing remain separate. My Assistant does not turn a ChatGPT subscription
into a general OpenAI API key. The supported account lane embeds OpenAI's documented **Codex App Server**:
the official external runtime owns device-code authentication and tokens; My Assistant receives only
sanitized account state, plan type, rate state when requested, and the models/configuration that App Server
documents for that account. This consumes the user's available Codex allowance and remains subject to their
plan limits. It is labelled **ChatGPT account · Codex**, never “free OpenAI API.”

The runtime is deliberately absent from the DMG. Detection is limited to standard installed ChatGPT, Codex,
or native Codex CLI locations and execution is refused unless macOS verifies OpenAI Team ID `2DC432GLL2`.
Each request starts the native binary directly—Node is neither bundled nor kept resident—and uses a private
isolated profile rather than the user's general Codex configuration. Device-code OAuth secrets remain in
that profile under the official runtime's ownership and are never copied into settings, localStorage, logs,
arguments, or the UI.

The API-key lane remains independent: it is billed by the selected provider and its key lives in Keychain.
Model lists are loaded from the selected provider's authenticated `/models` endpoint. ChatGPT account model
and reasoning controls come from App Server `model/list`, so the UI does not hard-code entitlements.

Official references:

- [Codex App Server](https://developers.openai.com/codex/app-server)
- [Using Codex with a ChatGPT plan](https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan)
- [ChatGPT and API billing are separate](https://help.openai.com/en/articles/9039756-managing-billing-settings-on-chatgpt-web-and-platform)
- [OpenAI API authentication](https://developers.openai.com/api/reference/overview#authentication)

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

1. Complete the read-only matrix across Gmail, Slack, LinkedIn, Google Docs, Apple Notes, Google Slides,
   Terminal, secure fields, full-screen, multiple displays, and permission denial/recovery using only
   synthetic/test-account content.
2. Physically verify the preview global shortcut across conflicts, relaunch, hidden-window focus, Spaces,
   and multiple displays; then replace the dashboard request box with a native non-activating Listening →
   Understanding → Preview surface.
3. Exercise both API-key and ChatGPT account planners across provider failures, model capability changes,
   prompt injection, output truncation, and stale snapshots. Keep all results display-only.
4. Implement one native draft/replace/undo executor with snapshot revalidation, field-type policy, explicit
   preview, fresh-result verification, and no submit controls.
5. Add OCR and app-specific adapters only where the Accessibility acceptance matrix proves a gap, then add a
   separately permissioned screenshot/vision fallback only if those adapters remain insufficient.
6. Run the Porcupine-versus-Rustpotter opt-in wake benchmark; adopt neither until privacy, accuracy,
   CPU/RAM, energy, suspension, and licensing gates pass.
7. Expand to governed multi-step skills, audit history, and team/cloud capabilities only after the
   single-action contract is reliable.
