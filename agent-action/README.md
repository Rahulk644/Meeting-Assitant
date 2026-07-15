# Agent action helper

`agent-action` is the narrow native mutation boundary for the preview-only Agents action milestone. It
supports exactly three outcomes: insert a reviewed draft at the current cursor, replace the current
selection, or restore the single previous value through guarded Undo. It cannot click, submit, press Return,
send, browse, run a command, read a file, or perform a second action.

Build and test both Agent helpers from the repository root:

```bash
./script/build_agent_helpers.sh
```

The Rust server passes requests through stdin so draft and prior field content never appear in process
arguments. Before Apply, the helper resolves the exact captured process ID and revalidates its TextEdit
bundle ID, exact window title, focused control role and label, complete value, cursor, and selection against
the snapshot fingerprint. The validation build refuses every non-TextEdit target. Within that disposable
document it allows only settable, non-secure `AXTextField`, `AXTextArea`, `AXComboBox`, and `AXSearchField`
targets, verifies the resulting value, returns a private undo record through stdout, and immediately exits.
Because the current preview is an activating dashboard, the helper first checks the first ordinary on-screen
window behind My Assistant and refuses the action unless it still belongs to the exact captured TextEdit
process. It only then brings that process forward for final revalidation and mutation. Switching apps after
preparing a preview therefore invalidates the proposal.

If a target transforms a write, the helper attempts to restore and verify the complete original text and
selection. A verified restoration is returned as `apply_rolled_back`; an unverified final state is returned
as `verification_failed` and must be shown as urgent. Callers must never relabel that condition as a safe
cancellation. No live mutation test may use an operator's real inbox, chatbox, document, or terminal.

Undo is held only in Rust memory for five minutes. It restores the prior complete field value only if the
same target still contains the exact value produced by Apply. Any target, focus, selection, or content drift
cancels safely. Browser/UI responses never contain the prior field value.

The durable product and acceptance contract is
[`../docs/AGENTS_ARCHITECTURE.md`](../docs/AGENTS_ARCHITECTURE.md).
