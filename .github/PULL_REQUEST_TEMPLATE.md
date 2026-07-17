## Why

Describe the user problem or invariant this change addresses.

## What changed

Keep this focused on current behaviour. Link the release-matrix capability when relevant.

## Verification

- [ ] `cd app && npm test`
- [ ] `cd desktop/src-tauri && cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cd desktop/src-tauri && cargo check --locked && cargo test --locked`
- [ ] `./script/benchmark_speech.sh test`
- [ ] `cd desktop/src-tauri && cargo audit`
- [ ] Native helper checks/builds completed when touched
- [ ] Production surface and fail-closed capability routing verified when UI capabilities changed
- [ ] Privacy/security boundaries reviewed when data, permissions, providers, or network paths changed
- [ ] Documentation and `CHANGELOG.md` updated when product truth changed

## Privacy and recovery

State what data is read, written, retained, transmitted, or recoverable. Write “No change” when applicable.
