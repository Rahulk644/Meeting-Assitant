# Wake-word benchmark and adoption gate

This is the release authority for optional hands-free Agent activation. A wake word is not shipped and no
wake engine, model, AccessKey, or always-on audio process is in the DMG. The benchmark is explicitly opt-in
and uses only a dedicated, consented, non-private corpus.

## Candidate order

1. **Porcupine** is the primary product candidate. It offers native macOS runtimes, on-device inference, and
   custom wake words. It also requires a Picovoice AccessKey and applicable vendor/commercial terms. Those
   terms and redistribution constraints must be approved before integration; a tester supplies their own
   key outside Git and app settings.
2. **Rustpotter** is the lightweight control. It is pure Rust under Apache-2.0 with published model sizes
   from hundreds of kilobytes to a few megabytes. Its maintainer does not call it production-grade, so it is
   a benchmark control, not a default recommendation.
3. **openWakeWord** may be a research control only. Its primary runtime is Python, included pretrained model
   licenses are separate from the Apache-2.0 code, and the small C++ port is archived and brings ONNX
   Runtime. It cannot become the default merely because its code is open.

Official references:

- [Porcupine documentation](https://picovoice.ai/docs/porcupine/)
- [Picovoice terms](https://picovoice.ai/docs/terms-of-use/)
- [Rustpotter](https://github.com/GiviMAD/rustpotter)
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

Porcupine remains the leading candidate, but adoption requires all of the following:

- it beats or materially de-risks the lightweight control on the real product corpus;
- product-approved false-accept, false-reject, latency, CPU, physical-memory, and energy budgets are met on
  the lowest supported Mac;
- suspend/resume and privacy cases pass without a duplicate microphone stream or retained pre-wake audio;
- the AccessKey UX and commercial/redistribution terms are acceptable for a free/open-source core;
- an uninstall/disable path removes the external adapter and credentials cleanly; and
- the result is recorded in `PRODUCT_RELEASE_MATRIX.md` before any production capability is enabled.

Until those conditions pass, describe wake word as a blocked opt-in experiment—not implemented, bundled,
or promised.
