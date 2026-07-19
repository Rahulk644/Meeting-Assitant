# Third-party notices

My Assistant is MIT licensed. It also builds on third-party software and separately downloaded models whose
terms continue to apply.

## Runtime components

- **Tauri, Wry, Rust crates, and JavaScript development packages:** see `desktop/src-tauri/Cargo.lock` and
  `desktop/package-lock.json` for the exact dependency graph and versions. Their upstream licenses apply.
- **transcribe.cpp:** pinned by `stt/transcribe-server/PINNED_COMMIT`; MIT license, copyright the
  transcribe.cpp authors. Its license is retained in the verified source checkout used by the build.
- **cpp-httplib:** single-header MIT-licensed HTTP library compiled into `transcribe-server`; copyright
  yhirose and contributors. Its license is retained with the pinned transcription source/build inputs.
- **llama.cpp:** Smart Polish builds `llama-server` from the exact revision recorded in
  `script/build_polish_runtime.sh`; MIT license, copyright the llama.cpp contributors. The release runtime
  is built static for this app and does not include model weights.
- **Silero VAD and ONNX Runtime components:** used by the in-process dictation VAD under their upstream
  licenses. Exact Rust wrapper versions are pinned in `Cargo.lock`.

## Separately downloaded models and optional tools

- **Whisper speech models:** Apache-2.0; downloaded separately from revision-pinned Handy conversions of
  OpenAI Whisper checkpoints, not stored in Git or bundled in the DMG.
- **NVIDIA Parakeet Unified EN:** CC-BY-4.0; downloaded separately through the revision-pinned local-runtime
  conversion named by the model catalog.
- **NVIDIA Nemotron Speech Streaming EN and Nemotron 3.5 ASR Streaming:** Open Model Development and
  Weights License 1.1 (OpenMDW-1.1); downloaded separately through the revision-pinned local-runtime
  conversions named by the catalog.
- **Oriserve Whisper Hindi2Hinglish Apex and Prime:** Apache-2.0 model-card terms; downloaded separately
  through the revision-pinned GGML artifacts named by the catalog.
- **Gemma 3 Smart Polish models (270M, 1B, and 4B):** optional, separately downloaded revision-pinned GGUF
  artifacts; never committed or bundled. Google's Gemma terms and the model repository notices apply before
  download and use.
- **Gemma / LiteRT-LM meeting Intelligence:** optional external installation, not bundled. Google's
  applicable model and tool terms apply to the user's installation.
- **FFmpeg:** invoked from the host system for thumbnails when present; it is not bundled.

## Evaluation-only assets

- **Trelis/CoSHE-500:** CC BY-NC 4.0, pinned and downloaded only into the private benchmark cache for
  evaluation. It is not training data, committed content, or a release asset.
- **Google FLEURS:** CC BY 4.0, pinned and downloaded only for English ASR and aligned translation controls.
- **indic-transliteration:** the pinned Python package is used only to construct a clearly labelled machine
  Roman proxy. It never substitutes for the human Roman promotion corpus.

Wispr Flow, Fathom, Fireflies, Snaply, Handy, FluidVoice, and Meetily product assets or proprietary
implementations are not included. Competitive study is behavioral and clean-room. Open-source code may be
adopted only after its exact license and attribution obligations are recorded here.

A generated SBOM and consolidated dependency-license bundle are required before the first public release;
this human-readable inventory does not replace that release gate.
