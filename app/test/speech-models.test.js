"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const { DEFAULT_MODEL_ID, SPEECH_MODELS } = require("../speech-models.js");

test("developer harness and shipped Rust catalog expose the same model IDs", () => {
  const rust = fs.readFileSync(path.join(__dirname, "../../desktop/src-tauri/src/speech_models.rs"), "utf8");
  const rustIds = [...rust.matchAll(/^\s*id:\s*"([a-z0-9.-]+)",/gm)].map(match => match[1]);
  assert.deepEqual([...new Set(rustIds)].sort(), SPEECH_MODELS.map(model => model.id).sort());
});

test("catalog downloads are pinned, verifiable, and include the requested specialists", () => {
  const ids = new Set();
  for (const model of SPEECH_MODELS) {
    assert.ok(!ids.has(model.id), `duplicate ${model.id}`); ids.add(model.id);
    assert.ok(!model.url.includes("/resolve/main/"), `${model.id} is not revision-pinned`);
    assert.match(model.sha256, /^[a-f0-9]{64}$/);
    assert.ok(model.sizeBytes > 1_000_000);
  }
  assert.ok(ids.has(DEFAULT_MODEL_ID));
  for (const required of ["parakeet-unified-en-0.6b", "nemotron-speech-streaming-en-0.6b", "nemotron-3.5-asr-streaming-0.6b", "oriserve-hinglish-apex", "oriserve-hinglish-prime"]) assert.ok(ids.has(required), `missing ${required}`);
});
