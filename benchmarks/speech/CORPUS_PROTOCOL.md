# Private speech-corpus protocol

This protocol defines the evidence required for Roman Hinglish and real-world noise decisions. It exists to
prevent machine transliteration, played noise, unverified model output, or unconsented recordings from being
presented as human or field evidence.

All audio, annotations, consent mappings, transcript-bearing manifests, and raw reports stay outside Git in
`~/.cache/my-assistant-bench` by default. Only aggregate results without speech text may be committed.

## Human Roman Hinglish

The current source is the pinned 500-clip CoSHE evaluation set. CoSHE is CC-BY-NC-4.0,
evaluation-only; it is never training data or a release asset.

```sh
./script/benchmark_speech.sh prepare-full-data
./script/benchmark_speech.sh create-roman-pack
open ~/.cache/my-assistant-bench/annotation-packs/human-roman-coshe-500/annotate.html
```

The pack is blind: it contains clip IDs and audio paths but not the mixed-script source references. The
first annotator listens and writes natural Latin-script Hinglish exactly as spoken. ASR, LLM cleanup,
translation, and machine transliteration are prohibited. A second person listens independently, corrects
the reference, records confidence, and marks it approved. Pseudonymous transcriber and reviewer IDs must be
different on every approved clip.

Export `annotations.jsonl`, then finalize it:

```sh
python3 benchmarks/speech/corpus.py finalize \
  --pack ~/.cache/my-assistant-bench/annotation-packs/human-roman-coshe-500 \
  --annotations /private/path/annotations.jsonl \
  --out ~/.cache/my-assistant-bench/corpora/human-roman-coshe-500/manifest.jsonl
```

Finalization rejects empty speech text, Devanagari/non-Latin letters, control characters, missing confidence,
unapproved rows, unknown IDs, and same-person review. `--allow-partial` is useful for an engineering
checkpoint but never satisfies the 500-clip promotion gate.

## Field-recorded noise

“Real” means the speaker and the acoustic background were captured together by the test device in the named
environment. Playing café audio from YouTube, mixing a noise file after capture, TTS, or recording a speaker
beside a loudspeaker is controlled/synthetic stress evidence and belongs in a separate corpus.

Required primary conditions are clean control, café, wind, and traffic. The full gate also covers fan,
echo/reverberation, overlapping speakers, and silence. Before promotion, collect at least 20 approved clips
per primary condition, at least three consenting speakers, at least two Mac microphone/device classes, and
both short dictation and 20–60 second conversational speech. Keep exact addresses and legal names out of
metadata.

Every speech fixture requires:

- a pseudonymous speaker ID and separately stored consent-record ID;
- general location type, capture device, UTC timestamp, condition, duration, and audio SHA-256;
- the `field-recorded` attestation;
- a blind human Roman transcript plus independent reviewer and confidence;
- no personally identifying content in the prompt or transcript.

[`field_prompts.jsonl`](field_prompts.jsonl) provides versioned dictation, meeting, spontaneous-speech, and
controlled-overlap prompts. Supplying `--prompt-id hf-001` loads its text automatically. These prompts guide
coverage; they are not references. The double-reviewed transcript of what was actually said remains truth.

List devices and capture a fixture:

```sh
./script/benchmark_speech.sh capture-real list-devices

./script/benchmark_speech.sh capture-real capture \
  --condition cafe --speaker-id speaker-03 --consent-id consent-03 \
  --device 0 --location-type cafe --duration 20 \
  --prompt-id hf-001 \
  --attest-field-recorded
```

Existing field recordings may be normalized to 16 kHz mono PCM16 with `capture-real import`; the same
attestation and metadata are mandatory. Silence fixtures use `speaker-id=none` and
`consent-id=not-applicable`.

After capture, build the same blind review UI and finalize the resulting annotations:

```sh
./script/benchmark_speech.sh create-real-pack
open ~/.cache/my-assistant-bench/annotation-packs/real-noise/annotate.html

python3 benchmarks/speech/corpus.py finalize \
  --pack ~/.cache/my-assistant-bench/annotation-packs/real-noise \
  --annotations /private/path/real-noise-annotations.jsonl \
  --out ~/.cache/my-assistant-bench/corpora/real-noise-human/manifest.jsonl
```

An approved silence fixture has an intentionally empty reference and is scored by false-positive output,
not WER. Synthetic wind/babble remains useful for deterministic regression boundaries, but it cannot satisfy
this field gate. CHiME real café/street speech is valuable external English evidence but uses LDC distribution
terms and is not silently vendored into this project.

## Reporting

Run style-matched models only. Roman models are scored against the human Roman manifest; natural-script
models use the original mixed-script references. Every promotion report includes corpus revision/hash,
model revision/hash, audio policy, samples, total duration, WER and character-F2, empty/truncated/degenerate
rates, real-time factor, startup, peak RSS, hypothesis instability across at least three runs, and failures
by condition. Report limitations next to the number.

## External-source boundary

- [OpenSLR-104](https://www.openslr.org/104/) is a useful CC BY-SA 4.0 real Hindi-English speech corpus,
  but its Hindi words are transcribed in Devanagari. It can extend the natural-script track; it does not
  replace human Roman annotation.
- [AI4Bharat IndicVoices](https://huggingface.co/datasets/ai4bharat/IndicVoices) is CC BY 4.0 natural,
  spontaneous Indian speech collected across outdoor, school, hotel, and home environments. Its QA metadata
  can supplement real-world Hindi/code-mix robustness after the operator accepts the official gated access
  terms. It does not identify our exact café/wind/traffic product conditions, does not provide human Roman
  references, and must not be substituted with a mirror whose license or derivation is unclear.
- [CHiME-3](https://www.chimechallenge.org/challenges/chime3/data) contains real English speech recorded in
  café, bus, pedestrian, and street-junction conditions. Distribution is through LDC terms, so it may be an
  externally licensed robustness control but is not vendored or relabeled as our Hinglish field corpus.
- Public clips with noise played from YouTube, a generator, or a loudspeaker remain controlled-noise tests
  even if microphone-recorded in one take. They do not satisfy the field-recorded attestation.
