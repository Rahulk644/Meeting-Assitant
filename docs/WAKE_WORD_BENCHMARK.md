# Wake-word benchmark and adoption gate

This is the release authority for optional hands-free Agent activation. A wake word is not shipped and no
wake engine, model, AccessKey, or always-on audio process is in the DMG. The benchmark is explicitly opt-in
and uses only a dedicated, consented, non-private corpus.

## Candidate order

1. **sherpa-onnx keyword spotting** is the primary open-runtime candidate. It is actively maintained under
   Apache-2.0, exposes C/C++ and Swift-friendly native APIs, supports Apple-silicon macOS, and has a
   documented streaming keyword spotter. It needs no vendor account or AccessKey. The current v1.13.4
   arm64 shared no-TTS library archive is 15.2 MB compressed; the unique extracted runtime libraries are
   about 31.3 MB. A representative English INT8 KWS model set is about 5.3 MB when only the selected
   encoder/decoder/joiner and tokenizer assets are installed. The official 19 MB model directory is larger
   because it carries both INT8 and FP32 alternatives. Those are disk figures, not measured RSS.
2. **Porcupine** is the proprietary footprint/accuracy control, not the default OSS integration. Its public
   repository carries Apache-2.0, and its current macOS arm64 engine plus one language model and keyword is
   roughly 1.3 MB. However, using the current SDK requires a Picovoice AccessKey that verifies account
   limits, and Picovoice's current service terms include restrictions relevant to competitive products.
   Do not ship it in the free/open core without an explicit legal/commercial decision; an opt-in benchmark
   tester must supply their own key outside Git and app settings.
3. **microWakeWord** is the footprint control. It is an active Apache-2.0 TensorFlow training project used
   by ESPHome/Home Assistant to produce small INT8 TFLite Micro wake-word models for constrained devices.
   That makes it the strongest modern open reference for minimum resource cost, but it is embedded-first:
   My Assistant would still need a maintained macOS audio/inference adapter and a properly licensed custom
   model. It is a benchmark control, not a ready Mac integration.
4. **openWakeWord** may be a research reference only. Its primary runtime is Python, included pretrained model
   licenses are separate from the Apache-2.0 code, and the small C++ port is archived and brings ONNX
   Runtime. It cannot become the default merely because its code is open.

Official references:

- [Porcupine documentation](https://picovoice.ai/docs/porcupine/)
- [Porcupine repository license](https://github.com/Picovoice/porcupine/blob/master/LICENSE)
- [Picovoice terms](https://picovoice.ai/docs/terms-of-use/)
- [microWakeWord](https://github.com/OHF-Voice/micro-wake-word)
- [ESPHome microWakeWord runtime](https://esphome.io/components/micro_wake_word/)
- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)
- [sherpa-onnx keyword-spotter example](https://github.com/k2-fsa/sherpa-onnx/blob/master/python-api-examples/keyword-spotter.py)
- [sherpa-onnx v1.13.4 macOS release assets](https://github.com/k2-fsa/sherpa-onnx/releases/tag/v1.13.4)
- [sherpa-onnx KWS model inventory](https://k2-fsa.github.io/sherpa/onnx/kws/pretrained_models/index.html)
- [openWakeWord](https://github.com/dscripka/openWakeWord)
- [archived openWakeWord C++ port](https://github.com/rhasspy/openWakeWord-cpp)

Vendor resource/accuracy claims are planning inputs, not My Assistant measurements. Every published claim
must come from the same product harness on a named Mac, exact engine/model version, exact phrase set, and
the committed measurement protocol below.

## Product behavior under test

- Off by default during the benchmark phase; enabling is an explicit user action.
- Local inference only. Pre-wake audio is never retained, logged, transcribed, or transmitted.
- Menu-bar state must distinguish Off, Listening, Paused, and Temporarily suspended.
- A visible and audible activation cue begins before post-wake instruction capture.
- One-click pause; automatic suspension during a meeting, dictation, screen lock, sleep, microphone loss,
  and any other active audio capture unless the user explicitly selects a proven compatible policy.
- Wake recognition and instruction transcription use separate buffers and lifecycles. A false wake must
  never inherit prior audio.
- The engine/model/AccessKey is installed externally during research. No credential may enter localStorage,
  JSON settings, logs, process arguments, crash reports, the repository, or the DMG.

## Corpus

Use a dedicated phrase and negative-audio corpus under the private acceptance cache, never the repository.
Each clip records only pseudonymous speaker ID, consent reference, accent/language group, microphone,
distance, environment, and measured/controlled noise condition.

Required positive matrix per candidate:

- at least 100 quiet-room attempts across representative accents, speaking rates, distances, and supported
  Mac microphones;
- the identical attempt matrix in real cafe, wind, and traffic conditions;
- whispered, conversational, and excited delivery plus near-phrase negatives;
- multiple candidate phrases before selecting the product phrase.

Required negative matrix per candidate:

- at least ten hours containing ordinary conversation, meetings, media, typing, music, silence, and phrases
  that are phonetically similar;
- no private calls or documents; no test audio reused for training without separate consent;
- every unintended transition into full instruction capture counts as a false accept.

## Measurements

Record per engine, model, phrase, Mac model, and macOS build:

| Dimension | Required output |
|---|---|
| Recall | attempts, misses, false-reject rate by accent/environment/distance |
| False accepts | triggers and false accepts per listening hour, with the triggering class |
| Latency | p50/p95 from phrase end to activation cue |
| CPU | settled idle/listening mean and p95, separated from the app baseline |
| Memory | incremental physical footprint and RSS |
| Energy | one-hour battery/energy-impact comparison against wake disabled |
| Disk/DMG | external adapter/model size and proof that core DMG size is unchanged |
| Reliability | sleep/wake, lock, audio-device change, crash/restart, meeting/dictation suspension |
| Privacy | files/log/network observation before and after activation |
| Licensing | exact code/model/runtime terms, redistribution rights, AccessKey and commercial constraints |

Use `python3 acceptance/run.py new --suite wake-word --operator-id <pseudonym>` to create the private,
evidence-pinned session. A pass requires the evidence file; no committed document should contain recorded
speech, transcript content, AccessKeys, personal identifiers, or private screenshots.

## Adoption decision

sherpa-onnx is the leading architectural candidate; Porcupine remains the small proprietary comparison and
microWakeWord remains the embedded footprint control. All three replace Rustpotter in the current decision
set. Adoption requires all of the following:

- it beats or materially de-risks both the tiny-footprint and desktop-open controls on the real product
  corpus, or an open control wins outright;
- product-approved false-accept, false-reject, latency, CPU, physical-memory, and energy budgets are met on
  the lowest supported Mac;
- suspend/resume and privacy cases pass without a duplicate microphone stream or retained pre-wake audio;
- any vendor AccessKey UX and commercial/redistribution terms are acceptable for a free/open-source core;
- an uninstall/disable path removes the external adapter and credentials cleanly; and
- the result is recorded in `PRODUCT_RELEASE_MATRIX.md` before any production capability is enabled.

Until those conditions pass, describe wake word as a blocked opt-in experiment—not implemented, bundled,
or promised.
