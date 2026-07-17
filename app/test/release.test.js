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

test("hidden, Agents, and unknown saved routes fall back to a shipped surface", () => {
  const production = capabilityMap("production");
  assert.equal(sanitizeRoute("writing", production), "meetings");
  assert.equal(sanitizeRoute("agents", production), "meetings");
  assert.equal(production.agents, false, "Agents is outside the initial product");
  assert.equal(production.agentContextInspector, false);
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

test("Agents stays visibly disabled and does not initialize at boot", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  assert.match(html, /data-view="agents" disabled aria-disabled="true"/);
  assert.match(html, /Coming soon/);
  const boot = html.slice(html.indexOf("async function boot()"), html.indexOf("\/\* ---------- on-device speech models"));
  assert.doesNotMatch(boot, /initAgents\(\)|syncAgentShortcut\(\)|ensureAgentRuntime\(\)/);
});

test("Agents cannot be restored through a saved route", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  assert.match(html, /function safeView\(name\).*hasCap\(cap\)\?name:'meetings'/);
  assert.match(html, /if\(n\.dataset\.view==='agents'\).*n\.disabled=true/);
});

test("settings model libraries remain visibly layered above their scrim", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  const modalCss = html.match(/\.mcardm\{([^}]*)\}/)?.[1] || "";
  assert.match(modalCss, /position:relative/);
  assert.match(modalCss, /z-index:1/);
  assert.match(modalCss, /opacity:1/);
  assert.match(modalCss, /visibility:visible/);
  assert.match(html, /@keyframes modalRise/);
  assert.match(html, /<div class="mscrim" id="mScrim" hidden>[\s\S]*?<div class="mcardm" id="mCard"/);
});

test("screen-share privacy is deterministically restored on every native boot", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  assert.match(html, /if\(inTauri\(\)\) setStealth\(store\.get\('stealth',false\)\)/);
});

test("live dictation is selected-model capability gated and has no preview-model setup", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  const pill = fs.readFileSync(path.join(__dirname, "../../desktop/src-tauri/src/native_pill.rs"), "utf8");
  assert.match(html, /model\.supportsLiveStreaming/);
  assert.match(html, /Live in text fields/);
  assert.doesNotMatch(html, /live_transcript_model|\/api\/live-model|onLiveModelProgress|Moonshine/);
  assert.doesNotMatch(pill, /set_live|live_transcript|LIVE_WIDTH/);
});

test("live cleanup is opt-in, stable-chunk only, and Smart Polish remains after stop", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  const native = fs.readFileSync(path.join(__dirname, "../../desktop/src-tauri/src/lib.rs"), "utf8");
  assert.match(html, /cleanup_timing:'After I finish'/);
  assert.match(html, /options:\['After I finish','While I speak'\]/);
  assert.match(html, /supportsLiveStreaming\|\|!store\.get\('smart_polish',false\)/);
  assert.match(html, /Smart Polish still waits until you stop/);
  assert.match(native, /snapshot\.committed/);
  assert.match(native, /DictFeedback::PostProcessing/);
  assert.doesNotMatch(native, /DictFeedback::Polishing/);
});

test("preview Agent actions remain TextEdit-isolated and report unknown final state honestly", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  assert.match(html, /Target: /);
  assert.match(html, /result\.actionBlockedReason/);
  assert.match(html, /verification_failed/);
  assert.match(html, /exact captured disposable TextEdit process/);
  assert.doesNotMatch(html, /Apply cancelled safely/);
});
