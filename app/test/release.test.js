"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const {
  CAPABILITIES,
  PRODUCTION_SETTINGS,
  HANDLED_SETTINGS,
  PRODUCTION_CTAS,
  HANDLED_CTAS,
  capabilityMap,
  releasePayload,
  sanitizeRoute,
} = require("../release.js");

test("production exposes only shipped capabilities and preview preserves the complete design", () => {
  const production = capabilityMap("production"), preview = capabilityMap("preview");
  for (const [key, meta] of Object.entries(CAPABILITIES)) {
    assert.equal(production[key], meta.state === "shipped", `${key} production visibility`);
    assert.equal(preview[key], true, `${key} preview visibility`);
    assert.ok(meta.verification, `${key} has named verification`);
  }
});

test("release channel is fail-closed", () => {
  assert.equal(releasePayload("PREVIEW").channel, "preview");
  assert.equal(releasePayload("anything-else").channel, "production");
  assert.equal(releasePayload(undefined).channel, "production");
});

test("hidden and unknown saved routes fall back to Meetings", () => {
  const production = capabilityMap("production");
  assert.equal(sanitizeRoute("writing", production), "meetings");
  assert.equal(sanitizeRoute("agents", production), "meetings");
  assert.equal(sanitizeRoute("unknown", production), "meetings");
  assert.equal(sanitizeRoute("dictation", production), "dictation");
});

test("every production-visible setting has an application handler", () => {
  const handled = new Set(HANDLED_SETTINGS);
  for (const id of PRODUCTION_SETTINGS) assert.ok(handled.has(id), `${id} needs a handler`);
});

test("every production-visible CTA has a functional handler", () => {
  const handled = new Set(HANDLED_CTAS);
  for (const id of PRODUCTION_CTAS) assert.ok(handled.has(id), `${id} needs a handler`);
});

test("every release-matrix capability is registered in the server-owned contract", () => {
  const matrix = fs.readFileSync(path.join(__dirname, "..", "..", "PRODUCT_RELEASE_MATRIX.md"), "utf8");
  const matrixKeys = new Set([...matrix.matchAll(/^\| `([^`]+)` \|/gm)].map((match) => match[1]));
  for (const key of matrixKeys) assert.ok(CAPABILITIES[key], `${key} is missing from app/release.js`);
});
