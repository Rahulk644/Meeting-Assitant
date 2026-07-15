# Agent context helper

`agent-context` is the native, on-demand, read-only Accessibility collector for the Agents foundation.

```bash
xcrun swiftc -O agent-context.swift -o agent-context
./agent-context --selftest
./agent-context --once | python3 -m json.tool
```

The helper emits one bounded JSON snapshot and exits. It performs no screenshot capture, audio capture,
network access, persistence, or UI mutation. Secure fields omit values. The packaged app signs it with the
same stable identity as the main app and invokes it through authenticated `POST /api/agent/context` with a
1.5-second process timeout and a 256 KiB response ceiling.

For a safely editable focused field, the snapshot also emits its cursor/selection range and a SHA-256 state
fingerprint over role, label, complete value, and range. The raw complete value is not added to the public
snapshot. The separate preview-only `agent-action` helper uses the fingerprint for exact fresh-target
revalidation; request-local `el_N` identifiers are never treated as persistent AX object references.

The complete product, safety, provider, wake-phrase, and acceptance contract lives in
[`../docs/AGENTS_ARCHITECTURE.md`](../docs/AGENTS_ARCHITECTURE.md).
