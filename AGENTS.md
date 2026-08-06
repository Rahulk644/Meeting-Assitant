# Repository operating contract

This file routes maintainers and coding agents to durable project truth. It is intentionally short; do not
turn it into a chronological work log.

## Read first

1. [`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md) — durable product state, invariants, verification, and
   prioritized work.
2. [`docs/PROJECT_CHECKPOINT.md`](docs/PROJECT_CHECKPOINT.md) — candid maintainer status and contribution
   boundary.
3. [`PRODUCT_RELEASE_MATRIX.md`](PRODUCT_RELEASE_MATRIX.md) — authority for production visibility.
4. [`docs/IMPLEMENTATION_ROADMAP.md`](docs/IMPLEMENTATION_ROADMAP.md) and
   [`docs/ENGINEERING_HEALTH.md`](docs/ENGINEERING_HEALTH.md) — future work and structural debt.
5. [`docs/OSS_COMPONENT_STRATEGY.md`](docs/OSS_COMPONENT_STRATEGY.md) and
   [`docs/RESEARCH_AND_ADOPTION_METHOD.md`](docs/RESEARCH_AND_ADOPTION_METHOD.md) — build/adopt/update and
   evidence policy.
6. [`ARCHITECTURE.md`](ARCHITECTURE.md) and [`SECURITY.md`](SECURITY.md) — runtime and trust boundaries.
7. [`CONTRIBUTING.md`](CONTRIBUTING.md) — build, test, review, and documentation standards.

## Non-negotiable product boundary

- Dictation is primary.
- Meetings is Beta and must remain labelled Beta.
- Agents is only a greyed-out, disabled **Coming soon** navigation label. Production has no Agent route,
  details, settings, startup work, compute lane, or packaged helper.
- Release builds force the production capability contract and fail closed.

## Working rules

- Preserve unrelated and ignored user files. Never commit models, native outputs, app data, transcripts,
  audio, acceptance evidence, `.app` bundles, DMGs, or demo renders.
- Build production native resources only through `script/build_native_resources.sh`.
- Use `script/build_and_run.sh --verify` for the canonical installed development app.
- Use `script/build_release_dmg.sh` for a release and `script/install_verified_dmg.sh` to validate the exact
  mounted candidate.
- Do not claim physical shortcut, microphone, cross-app, multi-display, clean-install, or two-person meeting
  coverage from unit tests or synthetic input.
- Update `docs/PROJECT_STATE.md` only when a durable invariant or priority changes. Put hashes, dates, test
  counts, branch narratives, and transient status in CI or release notes instead.
