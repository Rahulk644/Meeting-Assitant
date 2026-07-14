# Speech model qualification: Hinglish, English, translation, and noise

**Status:** engineering qualification; shipped default unchanged; not a marketing accuracy claim

**Measured:** 14–15 July 2026, Apple M1 / 8 GB / macOS 26.5

**Aggregate evidence:**
[`full 500-clip repeated result`](../benchmarks/speech/results/2026-07-15-m1-full-mixed.json) and
[`multi-track discovery result`](../benchmarks/speech/results/2026-07-14-m1-smoke.json)

This document is the durable speech-model decision. Transcript-bearing reports, weights, and evaluation
audio stay in `~/.cache/my-assistant-bench`; they are intentionally not committed.

## Product decision

Keep Whisper Large v3 Turbo Q8 as the shipped default. It completed three identical-accuracy 500-clip runs,
is already integrated, and had no truncated output. A future replacement still needs a safe provider catalog,
installed-model audio routing, downloader/removal/rollback, human Roman and field-noise evidence, and product
acceptance. The benchmark does **not** say that Whisper variants are bad. It says different variants solve
different jobs:

- **Mixed-script Hinglish:** Qwen3-ASR HiACC Q6 measured better recognition accuracy, but it is now an
  optional catalog/research entry only. Two complete 500-clip runs each produced 13 partial outputs (11
  repetition-loop stops and two 512-token caps) and used 1,053–1,188 MiB peak server RSS. By product decision
  it will not be recommended, auto-selected, bundled, or benchmarked further for promotion.
- **Roman Hinglish:** Apex Q8 is the leading tested candidate. It scores 37.79% against a deterministic
  transliteration proxy versus 85.09% for stock Turbo forced to English. This is a strong A/B signal, not a
  publishable accuracy number. A blind 500-clip annotation pack now exists, but transcription and independent
  human review are not complete.
- **Ordinary English transcription:** HiACC (4.37% WER), Whisper Turbo (4.75%), Whisper Medium (4.75%), and
  Qwen Base (4.75%) all pass this 24-clip smoke control. Apex regresses to 9.70%, synthetic-trained Shuja
  Medium to 11.79%, lightweight Hhs Whisper Small to 14.26%, and Zero STT materially to 21.67%.
- **Hindi speech translated to English:** the shipped Turbo model is not a translation model and returned
  Hindi. Whisper Medium Q5 was the best tested option. Translation stays hidden until a separate
  translation-capable model can be installed and routed.
- **Noisy audio:** Qwen/HiACC are materially stronger than Turbo in the synthetic wind case. Severe competing
  speech remains unsolved for every tested model. Synthetic tests locate failure boundaries; they do not
  establish real café or outdoor quality. The consented field-capture and double-review workflow is ready;
  actual café, wind, traffic, and clean recordings still require people and physical environments.

## Fair evaluation tracks

One WER cannot fairly rank these jobs, so the harness keeps four tracks separate:

1. **Natural mixed script:** a 24-clip discovery set and the complete 500-clip CoSHE evaluation set with human
   mixed Devanagari/Latin references.
2. **Roman Hinglish proxy:** the same clips with references deterministically transliterated by pinned
   `indic-transliteration==2.3.82`. This is explicitly `machine-derived-proxy` and only supports A/B decisions.
3. **English ASR:** 24 deterministic `en_us` FLEURS validation clips with human English transcripts.
4. **English translation:** 24 Hindi FLEURS recordings aligned by sentence ID to their human English FLoRes
   sentences. Translation WER is strict; character-F2 adds surface similarity, not semantic judgment.

CoSHE is CC-BY-NC-4.0 and evaluation-only. FLEURS is CC-BY-4.0. Neither dataset is training material for
this project or part of an app release.

## Full 500-clip repeated result

The complete CoSHE set contains 12,710.29 seconds (3.53 hours) of audio. Whisper used its pinned
`vad-compact` policy, retaining 11,553.31 seconds; Qwen used `vad-gate`, which preserves the clip after the
speech-presence check. These are exact Rust production-policy paths, not separately reimplemented filters.

| Candidate | Complete trials | WER | Character-F2 | Speed | Peak server RSS | Truncated / degenerate | Disposition |
|---|---:|---:|---:|---:|---:|---:|---|
| **Whisper Turbo Q8** | **3 × 500** | **39.31% every run** | **68.25 every run** | 5.64–6.67x | 959–987 MiB | 0 / 8 every run | shipped default |
| Qwen HiACC Q6 | 2 × 500 | 20.88% every run | 83.20 every run | 6.12–7.56x | 1,053–1,188 MiB | 13 / 3 every run | optional research only; third run stopped by product decision |

Both candidates produced exactly the same hypothesis for every sample across their completed trials. That
makes the accuracy and failure counts reproducible. Speed and RSS varied because this was an interactive
8 GB workstation under other workloads and memory pressure; those figures are indicative, not an isolated
lab comparison. Qwen's better WER does not override its product disposition or its 2.6% partial-output rate.

## 24-clip mixed-script discovery

Each model ran through the pinned native `transcribe-server`. These early rows retain their original
`product VAD` label for historical readability; the current explicit policies are `vad-compact` for shipped
Whisper and `vad-gate` for Qwen/Apex research controls.

| Candidate | Audio path | WER | Speed | Peak server RSS | Truncated / degenerate | Decision |
|---|---|---:|---:|---:|---:|---|
| Whisper Turbo Q8 | raw | 59.92% | 8.49x | 1,038 MiB | 0 / 1 | shipped baseline control |
| Whisper Turbo Q8 | product VAD | 47.99% | 7.14x | 987 MiB | 0 / 0 | VAD helps this model |
| Qwen Base Q6 | raw + guarded 512 budget | 23.39% | 6.89x | 1,197 MiB | 1 / 0 | strong candidate |
| Qwen Base Q6 | product VAD | 24.83% | 6.69x | 1,189 MiB | 0 / 0 | safer but slightly less accurate |
| **Qwen HiACC Q6** | **raw** | **22.31%** | **8.68x** | 1,192 MiB | **0 / 0** | best mixed candidate |
| Qwen HiACC Q6 | product VAD | 28.38% | 7.06x | 1,078 MiB | 2 / 1 | VAD destroys its advantage |
| Srota Q6 | raw + 512 budget | 41.46% | 8.19x | 1,230 MiB | 0 / 0 | do not promote |
| Whisper Medium Q5 | raw | 57.17% | 5.99x | 870 MiB | 0 / 4 | base control |
| Zero STT Q5 | raw | 48.57% | 5.93x | 872 MiB | 0 / 4 | real Hinglish gain, English regression |
| Zero STT Q5 | forced Hindi | 74.03% | 5.36x | 797 MiB | 0 / 11 | reject configuration |
| Hhs Whisper Small Q5 | raw | 60.44% | **12.1x** | **381 MiB** | 0 / 0 | reject; small but inaccurate |
| Hhs Whisper Small Q5 | product VAD | 62.44% | 10.4x | **379 MiB** | 0 / 1 | VAD also worsens it |
| Shuja Whisper Medium Q5 | raw | 49.71% | 6.44x | 890 MiB | 0 / 0 | synthetic fine-tune gain, still weak |
| Shuja Whisper Medium Q5 | product VAD | 53.96% | 5.01x | 813 MiB | 0 / 3 | VAD worsens it |

The clean product VAD retained 84.74% of audio on average, but only 39.93% for the most aggressively cut
clip. Its effect is model-dependent. No future provider may inherit this VAD merely because Whisper uses it.

## Roman Hinglish

| Candidate | Audio path | Proxy WER | Speed | Peak server RSS | Degenerate | Meaning |
|---|---|---:|---:|---:|---:|---|
| **Apex Q8** | **raw** | **37.79%** | 9.39x | 1,004 MiB | 1 | best Roman A/B candidate |
| Apex Q8 | product VAD | 49.20% | 7.42x | 939 MiB | 2 | do not use current VAD |
| Stock Turbo, forced English | raw | 85.09% | 8.89x | 964 MiB | 3 | poor Roman control |
| Stock Turbo, forced English | product VAD | 55.79% | 7.77x | 963 MiB | 0 | improved but still behind Apex |

Roman output is desirable for the primary Hinglish typing use case. The current proxy is enough to keep
Apex in the shortlist, but not enough to market an accuracy percentage. A consented, human-reviewed Roman
test set is required.

## Ordinary English transcription

| Candidate | WER | Character-F2 | Speed | Peak server RSS | Decision |
|---|---:|---:|---:|---:|---|
| **Qwen HiACC Q6** | **4.37%** | 96.08 | 14.4x | 1,243 MiB | passes smoke regression gate |
| Whisper Turbo Q8 | 4.75% | 96.30 | 4.5x | 1,048 MiB | passes; shipped control |
| Whisper Medium Q5 | 4.75% | 95.98 | 4.9x | **858 MiB** | passes |
| Qwen Base Q6 | 4.75% | **96.63** | 13.7x | 1,090 MiB | passes |
| Srota Q6 | 6.46% | 95.23 | **14.8x** | 1,240 MiB | mild regression |
| Apex Q8 | 9.70% | 92.22 | 4.5x | 1,010 MiB | meaningful English tradeoff |
| Shuja Whisper Medium Q5 | 11.79% | 90.75 | 3.81x | 816 MiB | English regression; one degenerate |
| Hhs Whisper Small Q5 | 14.26% | 88.17 | 12.6x | **378 MiB** | lightweight but unacceptable regression |
| Zero STT Q5 | 21.67% | 83.92 | 4.4x | 888 MiB | reject as automatic default |

This is a clean 24-clip smoke gate, not a general English benchmark. It does prove that HiACC's Hinglish
gain did not come with an obvious clean-English regression on the same deterministic selection.

## Hindi speech to English translation

| Candidate | Strict WER | Speed | Peak server RSS | Decision |
|---|---:|---:|---:|---|
| Turbo Q8 `translate=true` | 113.44% | 4.15x | 964 MiB | invalid product path; returned Hindi |
| **Whisper Medium Q5** | **65.42%** | 5.75x | **860 MiB** | best tested translation option |
| Whisper Medium Q8 | 66.01% | **6.15x** | 1,109 MiB | no quality gain over Q5 |
| Whisper Large v3 Q5 | 66.80% | 3.11x | 1,493 MiB | heavier and no gain on this set |

OpenAI's [Whisper documentation](https://github.com/openai/whisper) states that Turbo is not trained for
translation and should return the original language; Medium or Large must be used for translation. The old
UI exposed an English-translation toggle while only Turbo was installed. The capability now fails closed and
requires a separately qualified model before it can reappear.

## Synthetic 5 dB stress tests

The first 12 CoSHE clips were mixed deterministically with either modulated low-frequency noise (`wind`) or
three other speech clips (`babble`). The manifest stores the method, seed, SNR, source hashes, and mix gain.

| Condition | Audio path | Turbo | Qwen Base | HiACC |
|---|---|---:|---:|---:|
| wind 5 dB | raw | 75.70% | 35.23% | **33.49%** |
| wind 5 dB | product VAD | 66.86% | 36.28% | **34.77%** |
| babble 5 dB | raw | 83.26% | **72.56%** | 73.84% |
| babble 5 dB | product VAD | 100.70% | **63.37%** | 72.09% |

The VAD helps Turbo under wind, hurts both Qwen variants under wind, helps Qwen Base under babble, and
catastrophically hurts Turbo under babble. The engineering conclusion is model-specific preprocessing and a
separate denoiser A/B—not “always enable more cleanup.” Real consented café, traffic, wind, fan, echo,
distance, overlap, and silence fixtures remain mandatory.

## Candidate universe and qualification status

This is a high-signal Hugging Face/GitHub discovery snapshot, not a claim that every uploaded checkpoint on
the internet has been executed. Candidates were filtered by task relevance, license/provenance, available
weights, and product runtime feasibility.

| Family / candidate | Status | Evidence / blocker |
|---|---|---|
| Whisper Turbo, Medium, Large v3 | tested | controls for mixed, English, and translation |
| Apex Whisper Hinglish | tested | strong Roman proxy; needs human Roman set |
| Zero STT Whisper Medium fine-tune | tested | improves mixed Hinglish over Medium, but badly regresses English |
| Trelis Whisper Large v3 fine-tune | deferred safely | source weights are about 6.2 GB and the catalog requires a 12 GB free-disk safety floor for source, reference, and quantized copies |
| Hhsjsnns Whisper Small fine-tune | tested | only 194 MB on disk / about 381 MB peak server RSS, but 60.44% mixed and 14.26% English WER; reject |
| Shuja Whisper Medium fine-tune | tested | improves base Medium from 57.17% to 49.71% mixed WER, but is nowhere near its synthetic card claim, regresses English to 11.79%, and worsens with VAD |
| Qwen Base, Srota, HiACC | tested; optional research only | HiACC has the best measured mixed-script accuracy, but Qwen is excluded from the recommendation/automatic/promotion path by product decision |
| addyo07 noisy Qwen ONNX | integration required | custom ONNX path, not the shipped runtime; needs an isolated adapter and independent evaluation |
| addyo07 Nemotron RNNT ONNX | integration required | exported encoder/decoder lack a production streaming feature/cache/RNNT runner; author scores are not independent |
| Hinglish Conformer CTC GGUF | runtime incompatible | the pinned production server rejects the file at model load; upstream NVIDIA terms also need review |
| WhissleAI ONNX | excluded from OSS catalog | gated/custom licensing is incompatible with the intended open model catalog |
| incomplete Wav2Vec uploads | excluded pending provenance | missing tokenizer/runtime/provenance prevents reproducible qualification |

The catalog pins every downloaded or converted production-runtime artifact by source revision, size, and
SHA-256. A model card claim is never copied into the measured-results table as if it were our result.

## Product defects exposed and fixed

1. **Automatic dictation was secretly forced to English.** The Rust proxy and legacy developer harness now
   preserve `language=auto`; a regression test prevents this from returning.
2. **Turbo translation was exposed despite being unsupported.** Translation defaults off, stale saved state
   cannot re-enable it, the server release contract forces it off, and the release matrix marks it blocked.
3. **Qwen output handling was too short.** The pinned native patch raises the output budget from 256 to 512
   tokens and terminates repeated token cycles. Qwen Base improved from 30.85% with 8 truncations to 23.39%
   with one recovered/truncated sample; a 1,024-token experiment was worse because a loop ran longer.
4. **The benchmark mislabeled clean English as a script mismatch.** English references are now comparable
   for mixed/roman-capable multilingual models; mixed-script and Roman Hinglish still require separate refs.
5. **Preprocessing was previously inferred rather than exact.** The Rust audio functions now live in a small
   shared module and the benchmark example calls the same code as dictation.
6. Existing partial-transcript and UTF-8 protections remain: valid partial text survives native truncation,
   and malformed trailing bytes cannot create invalid JSON.

These are source fixes. They belong in a shareable artifact only after the release build and audit recorded
in `AUDIT_REPORT.md` are refreshed.

## Promotion gate

1. Complete the prepared blind 500-clip human Roman transcription and independent review; collect and review
   the consented real-noise product corpus in clean, café, wind, traffic, and secondary conditions.
2. Whisper's full 500-clip CoSHE set is complete three times. Larger English/translation sets and any future
   promotion candidate still require three complete runs; Qwen is explicitly not such a candidate.
3. Add long-form dictation and meeting fixtures, silence/hallucination rate, multi-speaker overlap, names,
   numbers, punctuation, and paste-format checks.
4. The exact per-model audio-policy contract is implemented for the current model and benchmark catalog.
   Installed-model routing and an independently benchmarked optional denoiser remain.
5. Prototype Nemotron/noisy-Qwen adapters out of process; only retain them if they beat the leaders under the
   same references without unacceptable RAM, startup, energy, licensing, or crash costs.
6. Verify model/dataset licenses and provenance, then implement a signed catalog, resumable downloads,
   hardware routing, deletion, rollback, and disk/RAM warnings.
7. Only after those gates expose Automatic / Roman Hinglish / Natural Script / Translate to English in the
   UI. Never silently force the decoder language or translation task.

## Reproduce

Follow [`benchmarks/speech/README.md`](../benchmarks/speech/README.md). Raw reports contain transcripts and
stay private; only aggregate, non-transcript evidence belongs in Git.
