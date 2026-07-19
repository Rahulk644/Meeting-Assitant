"use strict";

const { test } = require("node:test");
const assert = require("node:assert/strict");
const { aiProviderBase, fetchProviderModels, sanitizeDeveloperAiModel } = require("../ai-provider.js");

test("provider endpoints are fixed or restricted to HTTPS and loopback", () => {
  assert.equal(aiProviderBase("openai"), "https://api.openai.com/v1");
  assert.equal(aiProviderBase("custom", "https://models.example/v1/"), "https://models.example/v1");
  assert.equal(aiProviderBase("custom", "http://127.0.0.1:11434/v1"), "http://127.0.0.1:11434/v1");
  assert.throws(() => aiProviderBase("custom", "http://models.example/v1"), /HTTPS or a loopback/);
  assert.throws(() => aiProviderBase("unknown"), /Unknown API provider/);
});

test("developer model catalog is bounded, normalized, and authenticated", async () => {
  let request;
  const fakeFetch = async (url, options) => {
    request = { url, options };
    return new Response(JSON.stringify({ data: [{ id: "zeta" }, { id: "alpha" }, { id: "alpha" }, {}, { id: "x".repeat(161) }] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };
  const models = await fetchProviderModels("groq", "", "secret-key", fakeFetch);
  assert.deepEqual(models, ["alpha", "zeta"]);
  assert.equal(request.url, "https://api.groq.com/openai/v1/models");
  assert.equal(request.options.headers.Authorization, "Bearer secret-key");
  await assert.rejects(() => fetchProviderModels("groq", "", "", fakeFetch), error => error.status === 409);
});

test("developer key state never survives without an in-memory key", () => {
  const keys = Object.create(null);
  const stale = JSON.parse(sanitizeDeveloperAiModel(JSON.stringify({ provider: "openai", key: "", keySaved: true }), keys));
  assert.equal(stale.keySaved, false);
  const imported = JSON.parse(sanitizeDeveloperAiModel(JSON.stringify({ provider: "openai", key: "secret-key", keySaved: false }), keys));
  assert.equal(imported.key, "");
  assert.equal(imported.keySaved, true);
  assert.equal(keys.openai, "secret-key");
});
