# Desktop application

`desktop/` contains the Tauri build definition for the native macOS product. The release app is
self-contained for code: the authenticated UI/API server runs inside Rust, the app owns its tray and AppKit
dictation pill, and native speech/capture helpers are bundled as signed resources. No Node runtime or model
weight is shipped.

The WebView loads from `http://127.0.0.1:8790` only after the Rust listener is ready. A random per-launch
secret exchanges the one-time bootstrap URL for an HttpOnly cookie. If the port is already owned, startup
fails closed before a WebView or helper begins.

## Canonical workflows

Run these from the repository root:

```sh
# Build all native resources, build/sign the app, atomically install one canonical copy, and verify launch
./script/build_and_run.sh --verify

# Run every release gate and produce the inspected DMG
./script/build_release_dmg.sh

# Install the exact mounted candidate atomically and remove the transient build-tree app
./script/install_verified_dmg.sh desktop/src-tauri/target/release/bundle/dmg/My_Assistant_<version>-<label>_arm64.dmg

# Browser-only developer harness
./run.sh
```

Do not copy a build-tree `.app` manually. `build_and_run.sh` replaces `/Applications/My Assistant.app`
atomically, unregisters and removes the transient build bundle, and verifies that no second launchable copy
remains.

## Native resources

`script/build_native_resources.sh` is the single production build path for:

- `stt/meeting-tap`
- `meeting-state/meeting-state`
- `dictation-target/dictation-target`
- `stt/transcribe-server/transcribe-server`
- `polish-runtime/llama-server`

The first three share the app's stable identifier where their TCC behavior requires it. The model runtimes
use dedicated identifiers. All are signed with hardened runtime before bundling; microphone capture carries
the audio-input entitlement. Agent helpers are intentionally excluded.

## Signing and permissions

The current independent beta uses the stable self-signed `My Assistant Dev` certificate. Stable signing
keeps macOS code requirements consistent across local builds; ad-hoc binaries use a changing cdhash and can
invalidate previous TCC grants.

The main app uses `src-tauri/Entitlements.plist` for hardened microphone access. The default bare Right
Command shortcut also needs Input Monitoring; combination shortcuts use Carbon without it. Accessibility is
required for insertion and bounded target reconciliation. Meetings separately uses System Audio Recording,
and optional video uses Screen Recording.

The self-signed beta is not Apple-notarized. A received DMG therefore requires the documented one-time
**System Settings → Privacy & Security → Open Anyway** flow. Developer ID notarization is a separate public
distribution gate.

## Build outputs

- App: `src-tauri/target/release/bundle/macos/My Assistant.app`
- DMG: `src-tauri/target/release/bundle/dmg/My_Assistant_<version>-<label>_arm64.dmg`

`script/verify_app_bundle.sh` rejects wrong/adhoc nested signatures, missing hardened runtime, wrong
architecture, missing microphone entitlement, model weights, Agent helpers, or a non-production release
payload. `script/verify_release_artifact.sh` repeats the inspection against the read-only mounted DMG.

## Development notes

- Red-X hides the main window; the process remains available in the tray. Cmd-Q or the tray Quit item ends
  helpers and local servers.
- The main WebView is created after the local server answers, avoiding a dead recovered WebKit frame.
- `app/server.js` remains a browser development harness only. Native debug and release builds use the Rust
  server and the same production capability contract.
- `npm ci` installs the repository-pinned Tauri CLI and minifier. `npm audit` is part of the release gate.
- Release builds require pinned `cargo-audit` 0.22.2 and fail on RustSec vulnerability findings. Maintenance
  warnings remain visible so transitive dependency debt is not silently normalized.
