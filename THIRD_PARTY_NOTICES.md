# Third-party notices

My Assistant is MIT licensed. It also builds on third-party software and separately downloaded models whose
terms continue to apply.

## Runtime components

- **Tauri, Wry, Rust crates, and JavaScript development packages:** see `desktop/src-tauri/Cargo.lock` and
  `desktop/package-lock.json` for the exact dependency graph and versions. Their upstream licenses apply.
- **transcribe.cpp:** pinned by `stt/transcribe-server/PINNED_COMMIT`; MIT license, copyright the
  transcribe.cpp authors. Its license is retained in the verified source checkout used by the build.
- **Silero VAD and ONNX Runtime components:** used by the in-process dictation VAD under their upstream
  licenses. Exact Rust wrapper versions are pinned in `Cargo.lock`.

## Models and optional tools

- **Whisper speech models:** downloaded separately, not stored in Git or bundled in the DMG. The selected
  model's upstream license and model-card terms apply.
- **Gemma / LiteRT-LM:** optional external installations, not bundled. Google's applicable model and tool
  terms apply to the user's installation.
- **FFmpeg:** invoked from the host system for thumbnails when present; it is not bundled.

Wispr Flow, Fathom, Fireflies, Snaply, Handy, and Meetily product assets or proprietary implementations are
not included. Competitive study is behavioural and clean-room. Open-source code may be adopted only after
its exact license and attribution obligations are recorded here.

A generated SBOM and consolidated dependency-license bundle are required before the first public release;
this human-readable inventory does not replace that release gate.
