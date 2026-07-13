# Contributing

Thanks for helping improve My Assistant. It's a local-first macOS app: a Tauri (Rust) shell around a
dependency-free Node server that streams live transcripts and proxies on-device STT/LLM. Everything runs on
the user's machine — keep it that way.

## Repository layout

| Path | What it is |
|---|---|
| `app/` | The Node relay + UI server (`server.js`), the single-file UI (`index.html`), the dictation pill (`overlay.html`), and the extracted, unit-tested modules (`gate.js`, `multipart.js`). |
| `desktop/src-tauri/` | The Tauri shell (`src/lib.rs`): tray, overlay window, global hotkey / event tap, in-process dictation. |
| `meeting-state/` | Swift helper — speaker names + in-call detection via Accessibility. |
| `stt/` | On-device speech-to-text (`transcribe.cpp` / `whisper.cpp` servers). |

Architecture and the macOS permission model live in [ARCHITECTURE.md](ARCHITECTURE.md); the privacy model in
[SECURITY.md](SECURITY.md).

## Prerequisites

- macOS (the app links Core Audio / CoreGraphics / AppKit — it does not build on Linux).
- Node ≥ 18 (uses the built-in `node:test` runner; no npm dependencies).
- Rust stable + the Tauri CLI (`cargo install tauri-cli`), for the desktop shell.

## Build, run, test

```sh
# Server + UI only (fast loop — open the printed http://localhost URL)
cd app && npm start

# Run the server test suite (pure modules: the recording gate, multipart splicing)
cd app && npm test          # → node --test

# Full desktop app
cd desktop && npm run dev    # tauri dev
cd desktop && npm run build  # tauri build

# Type/borrow-check the Rust without a full build
cd desktop/src-tauri && cargo check
```

## Code standards

- **Comments explain the *present*, not the *past*.** State the invariant or the non-obvious *why* in one or
  two lines. Do **not** narrate previous implementations, what used to be slow, or how a bug was found — that
  is what `git log` and [ARCHITECTURE.md](ARCHITECTURE.md) are for. A comment that describes code that no
  longer exists is a bug.
- **Prefer a named constant to a magic number** (`NS_SCREEN_SAVER_LEVEL`, not `1000`). Let the code
  document itself so the comment can shrink or disappear.
- **Extract pure logic so it can be tested.** New non-trivial decision logic (state machines, parsers,
  byte-wrangling) should be a pure function/module with a test table — see `app/gate.js` +
  `app/test/gate.test.js` as the pattern.
- **Don't swallow errors silently.** An empty `catch {}` / `let _ =` erases the debugging path; at minimum
  log it (`console.error` / `log::warn!`).
- **No dead UI.** A setting that doesn't change behaviour shouldn't ship. Wire it or remove it.
- Keep the server dependency-free.

## Pull requests

1. Add or update tests for any logic change (`cd app && npm test` must pass).
2. `cargo check` must pass for desktop changes.
3. Keep the diff focused; describe the *why* in the PR body, not in a new block comment.
