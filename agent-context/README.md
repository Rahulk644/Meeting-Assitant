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

The complete product, safety, provider, wake-phrase, and acceptance contract lives in
[`../docs/AGENTS_ARCHITECTURE.md`](../docs/AGENTS_ARCHITECTURE.md).
