# Contributing

Thank you for improving My Assistant. The initial product is a local-first macOS app with two supported
surfaces: Dictation and Meetings (Beta). Agents is a disabled **Coming soon** label; contributions must not
expose Agent details, routes, settings, runtimes, or helpers in a production build.

The standalone application is currently maintained as a source checkpoint rather than a finished public
release. Read [`docs/PROJECT_CHECKPOINT.md`](docs/PROJECT_CHECKPOINT.md) before choosing work. Small fixes,
tests, portability seams, privacy improvements, documentation, and independently verifiable native adapters
are welcome; broad feature additions need an issue first so contributors do not build against a paused
product direction.

Read [README.md](README.md), [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md),
[docs/ENGINEERING_HEALTH.md](docs/ENGINEERING_HEALTH.md), and [ARCHITECTURE.md](ARCHITECTURE.md) before
changing runtime behavior. Consult [docs/IMPLEMENTATION_ROADMAP.md](docs/IMPLEMENTATION_ROADMAP.md) for task
boundaries and [docs/OSS_COMPONENT_STRATEGY.md](docs/OSS_COMPONENT_STRATEGY.md) before adopting a component.
Use [docs/RESEARCH_AND_ADOPTION_METHOD.md](docs/RESEARCH_AND_ADOPTION_METHOD.md) for evidence and provenance.
Update project state only when a lasting invariant or priority changes.

Before starting a large change, open an issue containing the user problem, intended boundary, privacy
impact, acceptance evidence, and explicit non-goals. A maintainer acknowledgement is especially important
for new dependencies, packaged helpers, network providers, permissions, or product surfaces.

## Repository map

| Path | Responsibility |
|---|---|
| `app/` | HTML/CSS/JS interface, server-owned release contract, development harness, and Node tests. |
| `desktop/src-tauri/` | Tauri shell and packaged Rust service: shortcuts, audio, transcription, target-safe insertion, settings, and local APIs. |
| `stt/` | Meeting capture and the pinned local transcription runtime. |
| `meeting-state/` | Bounded native meeting-state helper. |
| `dictation-target/` | Bounded native helper for focused-field classification and live-text reconciliation. |
| `polish-runtime/` | Build destination for the pinned Smart Polish runtime; model weights are never committed or bundled. |
| `benchmarks/speech/` | Reproducible model qualification and aggregate-only evidence. |
| `acceptance/` | Product-level physical acceptance cases and runner tests. |
| `script/` | Canonical build, verification, benchmark, and packaging entry points. |

`agent-context/`, `agent-action/`, and their Rust/UI integration are dormant research. They are not part of
the initial product or release payload. Do not expand or expose them without a separate product decision and
an update to the release contract.

## Prerequisites

- Apple Silicon Mac running macOS 14.4 or later.
- Xcode Command Line Tools.
- Rust stable and Node.js 18 or later.
- The repository-pinned desktop dependencies installed with `cd desktop && npm ci`.
- `cargo-audit` 0.22.2 for a release (`cargo install cargo-audit --version 0.22.2 --locked`).
- For signed local bundles, the `My Assistant Dev` certificate described in [desktop/README.md](desktop/README.md).

## Development and verification

Use the narrowest useful loop while developing:

```sh
# UI and development server
cd app && npm test
cd app && npm start

# Rust quality gate
cd desktop/src-tauri
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked
cargo test --locked
../../script/audit_rust_dependencies.sh

# Speech benchmark and acceptance-runner tests (no model or private-corpus download)
./script/benchmark_speech.sh test

# Build every production native resource, then build/install the one canonical app
SIGN_IDENTITY="My Assistant Dev" ./script/build_native_resources.sh
./script/build_and_run.sh --verify

# Full signed local candidate and mounted-image verification (`unreleased` unless explicitly labelled)
./script/build_release_dmg.sh
```

`script/build_native_resources.sh` is the only production native-resource build entry point. Adding a
packaged executable requires updating that script, `tauri.conf.json`, CI staging, code-signing rules, the
bundle verifier, architecture documentation, and third-party notices together.

## Engineering standards

- Preserve the product boundary. Production exposes Dictation, Meetings (Beta), and a disabled Agents label.
- Fail closed at privacy, permission, model-integrity, release-channel, code-signing, and focused-target
  boundaries. A failed target reconciliation copies the final result rather than writing to an uncertain app.
- Keep comments about present invariants and non-obvious reasons. Historical narration belongs in Git history
  or the changelog.
- Extract decision logic into small testable units. Add regression tests for every fixed defect.
- Do not silently swallow failures that affect recording, transcription, insertion, storage, or security.
- Keep the Node development harness behaviorally aligned with packaged Rust routes where both exist.
- Put every adopted OSS behind a stable internal seam. Pin production inputs; do not track an upstream branch
  or auto-deploy dependency updates without qualification, license review, and rollback.
- Do not commit model weights, native build outputs, audio, transcripts, participant identities, credentials,
  screenshots containing private data, or human-acceptance evidence with raw speech.
- Never claim a physical macOS or real-conversation case passed from synthetic input, a helper self-test, or a
  prerecorded loop. Record unresolved physical coverage as an explicit release risk.

## Pull requests

A pull request should explain the user-facing reason, the invariant being protected, and how the change was
verified. Update the following when applicable:

- `CHANGELOG.md` for user-visible changes;
- `PRODUCT_RELEASE_MATRIX.md` and `app/release.js` for capability-state changes;
- `ARCHITECTURE.md` or `SECURITY.md` for durable runtime or trust-boundary changes;
- `docs/PROJECT_STATE.md` only for durable product/release invariants;
- `THIRD_PARTY_NOTICES.md` for a new runtime, model, dataset, or redistributed dependency.

Before requesting review, run the automated gates relevant to the diff and use
`script/build_release_dmg.sh` for any release, signing, entitlement, resource, or packaging change.

Keep a pull request reviewable: one problem, one stable boundary, focused tests, and no generated/private
artifacts. If adopting upstream code, record its URL, exact revision, license, retained notices, local changes,
qualification suite, and rollback. Proprietary products may inform interoperability or behavior, but their
code, assets, credentials, private APIs, and copied implementation details do not belong in a contribution.
