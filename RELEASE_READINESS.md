# Release readiness

This is an evergreen release contract, not an audit diary. Artifact-specific version, date, size, SHA-256,
test counts, and operator evidence belong in CI output or the corresponding release notes.

## Supported artifact

- Apple Silicon (`arm64`)
- macOS 14.4 or newer
- Drag-to-Applications DMG
- Independent beta signing identity: `My Assistant Dev`
- Speech and Smart Polish model weights downloaded separately after explicit user choice

The independent channel is self-signed and is therefore not Apple-notarized. It is suitable for controlled
beta distribution with documented **Open Anyway** instructions, not a low-friction public launch.

## Automated release gate

`./script/build_release_dmg.sh` is the only canonical DMG entry point. A successful run must:

1. validate shell, Python, JSON, benchmark, and acceptance-runner sources;
2. pass Node tests and the desktop npm vulnerability audit;
3. pass Rust formatting, Clippy with warnings denied, locked compile checks, tests, and the RustSec audit;
4. build every production native executable from the pinned source path;
5. force the packaged WebView to the production release channel;
6. strip and sign the main app and all nested executables with hardened runtime;
7. verify the main microphone entitlement;
8. reject packaged model weights and Agent helpers;
9. create and sign the DMG;
10. mount the exact DMG read-only and repeat the bundle inspection; and
11. print the final artifact path, byte size, and SHA-256.

A compile, an `.app` directory, or an unmounted DMG is not a completed release.

For local candidate validation, `script/install_verified_dmg.sh <artifact>` installs the exact mounted app
with rollback, launches it, checks the authenticated service boundary, and removes the transient build-tree
bundle so Launch Services has one canonical application.

## Production-surface gate

- Dictation is visible and primary.
- Meetings is visible and labelled Beta.
- Agents is visible only as a disabled **Coming soon** label and exposes no details.
- Hidden routes cannot be restored from localStorage or direct navigation.
- Every visible production setting and CTA has an implementation handler.
- Smart Polish remains experimental, off by default, local, guarded, and final-only for model inference.
- Live dictation is exposed only for the selected model's genuine streaming capability and a compatible
  target; unsafe reconciliation fails closed.

## Security and privacy gate

- Audio and default transcription remain local.
- Remote meeting-text processing occurs only through an explicitly selected provider.
- API keys remain in macOS Keychain.
- Runtime directories and temporary files are private.
- Secure fields never enter history, diffs, logs, or model inference.
- Loopback services require per-launch authentication and bounded requests.
- Downloaded models are revision-pinned, size-checked, and SHA-256-verified.
- The bundle contains no telemetry client, Node runtime, model weights, or Agent helper.
- Permissions and installation copy match actual macOS behavior, including Input Monitoring for bare Right
  Command.

## Repository gate

- The intended release commit is on `main` and `main` matches `origin/main`.
- The worktree is clean apart from ignored local/generated material.
- Generated binaries, models, app data, acceptance evidence, demo renders, `.app` bundles, and DMGs are not
  tracked.
- CI uses least-privilege permissions, concurrency cancellation, timeouts, and commit-pinned third-party
  actions.
- Dependency update automation is configured.
- README, product state, release matrix, security contract, architecture, changelog, third-party notices, and
  acceptance documentation agree on the production boundary.

## Physical acceptance still required

Automation cannot certify these cases:

- physical Right Command while each supported target app is frontmost;
- full-screen Spaces, multiple displays, screen changes, and overlay position;
- real waveform response to silence, normal speech, and loud speech;
- supported live editors, editor re-rendering, cursor/target drift, and safe final fallback;
- permission denial, grant, revoke, and recovery for every macOS permission;
- sleep/wake, rapid start/stop, long dictation, microphone changes, and app relaunch;
- clean-user model download and checksum recovery;
- real two-person meeting capture across supported apps, browsers, and audio devices; and
- install and first launch from the mounted DMG on a clean macOS user or machine.

The machine-readable protocol is in [`docs/ACCEPTANCE_TESTS.md`](docs/ACCEPTANCE_TESTS.md). Physical cases
must be recorded as physical; synthesized keys, looped audio, helper self-tests, and screenshots do not count
as substitutes.

## Distribution decision

The self-signed beta may ship only with its Gatekeeper limitation stated next to the download. A public
identified-developer release remains blocked until a Developer ID build passes `notarytool`, is stapled, and
the stapled artifact passes this same inspection.

No release should be described as zero-bug, universally compatible, Apple-verified, or fully production-ready
without the corresponding evidence.
