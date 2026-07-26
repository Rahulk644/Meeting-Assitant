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
const {
  FORBIDDEN_PRODUCTION_PATTERNS,
  stripAgentResearch,
} = require("../../script/prepare_release_html.cjs");

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

test("production HTML retains only the disabled Agents label", () => {
  const source = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  const production = stripAgentResearch(source);
  assert.match(production, /data-view="agents" disabled aria-disabled="true"/);
  assert.match(production, /Coming soon/);
  for (const [label, pattern] of FORBIDDEN_PRODUCTION_PATTERNS) {
    assert.doesNotMatch(production, pattern, label);
  }
});

test("the default Tauri ACL exposes no Agent command", () => {
  const permissions = fs.readFileSync(
    path.join(__dirname, "../../desktop/src-tauri/permissions/app-commands.toml"),
    "utf8",
  );
  assert.doesNotMatch(permissions, /set_agent_shortcut|agent[-_]/);
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

test("Meetings Beta exposes independent capture sources and grounded live actions", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  const native = fs.readFileSync(path.join(__dirname, "../../desktop/src-tauri/src/meeting_detection.rs"), "utf8");
  const macAdapter = fs.readFileSync(path.join(__dirname, "../../stt/meeting-tap.swift"), "utf8");
  assert.equal(CAPABILITIES.liveMeetingAssist.state, "shipped");
  assert.equal(CAPABILITIES.automaticMeetingDetection.state, "shipped");
  assert.match(html, /id:'desktop_meet'.*Desktop meeting apps/);
  assert.match(html, /id:'browser_meet'.*Browsers and huddles/);
  assert.match(html, /cfg\(\{desktopMeet:v\}\)/);
  assert.match(html, /cfg\(\{browserMeet:v\}\)/);
  for (const action of ["catch_up", "summary", "actions", "follow_ups"]) {
    assert.match(html, new RegExp(`data-live-assist="${action}"`));
  }
  assert.match(html, /fetch\('\/api\/meeting\/live-assist'/);
  assert.match(native, /pub struct MeetingDetector/);
  assert.doesNotMatch(native, /objc2|tauri::|CoreAudio|ScreenCaptureKit/);
  assert.match(macAdapter, /\/api\/meeting\/observation/);
  assert.doesNotMatch(macAdapter, /struct MeetingEdgeTracker/);
});

test("Meetings Beta makes the current outside-app recording journey explicit", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "index.html"), "utf8");
  const journey = fs.readFileSync(path.join(__dirname, "../../docs/MEETINGS_USER_JOURNEY.md"), "utf8");
  const core = fs.readFileSync(path.join(__dirname, "../../desktop/src-tauri/src/local_server.rs"), "utf8");
  const pill = fs.readFileSync(path.join(__dirname, "../../desktop/src-tauri/src/native_meeting_pill.rs"), "utf8");
  const adapter = fs.readFileSync(path.join(__dirname, "../../stt/meeting-tap.swift"), "utf8");
  assert.match(html, /Recording locally/);
  assert.match(html, /macOS notification and the animated menu-bar waveform/);
  assert.match(html, /inside My Assistant while recording/);
  assert.match(html, /Call ended\. Finalizing the local transcript/);
  for (const phase of ["starting", "recording", "needs_attention", "pausing", "paused", "finalizing"]) {
    assert.match(core, new RegExp(`"${phase}"`));
    assert.match(html, new RegExp(`${phase}`));
  }
  assert.match(core, /microphone == "ready"[\s\S]*meeting_audio == "ready"/);
  assert.match(adapter, /"type": "capture-status"/);
  assert.match(adapter, /channelReady\("microphone"/);
  assert.match(adapter, /channelReady\("meetingAudio"/);
  assert.match(adapter, /shouldRestartSystemAudio/);
  assert.match(adapter, /shouldRecycleCaptureProcess/);
  assert.match(adapter, /initial remote silence is not a dropout/);
  assert.match(
    fs.readFileSync(path.join(__dirname, "../../desktop/src-tauri/src/lib.rs"), "utf8"),
    /supervise_meeting_capture_helper/,
  );
  assert.match(pill, /NSWindowStyleMask::NonactivatingPanel/);
  assert.match(pill, /setMovableByWindowBackground\(true\)/);
  assert.match(pill, /What did I miss\?/);
  assert.match(pill, /transcript_lines/);
  assert.match(pill, /dispatch_control\("pause"\)/);
  assert.match(pill, /dispatch_control\("resume"\)/);
  assert.match(pill, /dispatch_control\("stop"\)/);
  assert.match(journey, /Zoom · Starting local capture/);
  assert.match(journey, /Zoom · Recording locally · Mic \+ meeting audio ready/);
  assert.match(journey, /two to four seconds/);
  assert.match(journey, /four to six\s+seconds/);
});

test("the speech model stays cold at boot and warms only after real capture starts", () => {
  const native = fs.readFileSync(path.join(__dirname, "../../desktop/src-tauri/src/lib.rs"), "utf8");
  const settings = native.slice(native.indexOf("fn set_dictation("), native.indexOf("// Tray \"Stop\""));
  const capture = native.slice(native.indexOf("fn dictation_start()"), native.indexOf("fn dictation_stop_rec()"));
  assert.doesNotMatch(settings, /warm_speech_model\(\)/);
  assert.match(capture, /spawn_capture[\s\S]*warm_speech_model\(\)/);
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
