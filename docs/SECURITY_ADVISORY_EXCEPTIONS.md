# Security advisory exceptions

Security vulnerabilities are release-blocking. An exception is permitted only for a scanner limitation
when an executable check proves the affected code is absent from the release dependency graph.

## RUSTSEC-2026-0235 — inactive optional `rkyv` dependency

- **Lockfile package:** `rkyv 0.7.46`
- **Advisory:** insufficient validation of archives containing `Rc`/`Arc`
- **Why it appears:** `tauri-plugin-log` depends on `byte-unit`; `byte-unit` enables `rust_decimal`; the
  `rust_decimal` package metadata lists `rkyv` as an optional feature, so Cargo records it in `Cargo.lock`.
- **Why release code is unaffected:** the `rkyv` feature is not enabled. `cargo tree --target
  aarch64-apple-darwin --edges normal -i rkyv@0.7.46` has no dependency path.
- **Enforcement:** [`script/audit_rust_dependencies.sh`](../script/audit_rust_dependencies.sh) first fails if
  `rkyv 0.7.46` enters the normal Apple Silicon graph, then passes only this advisory ID to `cargo audit`.
- **Removal:** delete the exception as soon as an upstream `byte-unit`/`rust_decimal` update removes the
  vulnerable optional package from the lockfile.

The script does not suppress unmaintained/unsound warnings; `cargo audit` reports them for visibility. They
remain tracked in [`ENGINEERING_HEALTH.md`](ENGINEERING_HEALTH.md), including Linux-only Tauri dependencies
that are not part of the current macOS artifact.
