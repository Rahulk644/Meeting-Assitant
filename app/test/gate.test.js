"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { Gate } = require("../gate.js");

test("fresh gate is idle", () => {
  const g = new Gate();
  assert.equal(g.recording, false);
  assert.equal(g.paused, false);
});

test("auto-detection drives recording when no override is set", () => {
  const g = new Gate(true);
  assert.equal(g.setDetected(true), true, "detection changed");
  assert.equal(g.recording, true);
  assert.equal(g.setDetected(true), false, "no change on repeat");
  g.setDetected(false);
  assert.equal(g.recording, false);
});

test("autoDetect off ignores detection — only manual records", () => {
  const g = new Gate(true);
  g.setAutoDetect(false);
  g.setDetected(true);
  assert.equal(g.recording, false);
  g.setManual("start");
  assert.equal(g.recording, true);
});

test("manual override truth table", () => {
  const cases = [
    // action     detected  expect recording  expect paused
    ["start", true, true, false],
    ["pause", true, false, true],
    ["resume", true, true, false],
    ["stop", true, false, false],
    ["auto", true, true, false], // auto + detected → follows detection
    ["auto", false, false, false],
  ];
  for (const [action, detected, rec, paused] of cases) {
    const g = new Gate(true);
    g.setDetected(detected);
    assert.equal(g.setManual(action), true, `${action} accepted`);
    assert.equal(g.recording, rec, `${action}@${detected} → recording=${rec}`);
    assert.equal(g.paused, paused, `${action}@${detected} → paused=${paused}`);
  }
});

test("unknown action is rejected and leaves state untouched", () => {
  const g = new Gate(true);
  g.setDetected(true);
  assert.equal(g.setManual("frobnicate"), false);
  assert.equal(g.recording, true); // still following auto-detect
});

test("a call-relative override expires when that call ends, so the next call auto-records", () => {
  const g = new Gate(true);
  g.setDetected(true); // in a call
  g.setManual("pause"); // hold this call
  assert.equal(g.recording, false);
  g.setDetected(false); // call ends → pause override should clear
  assert.equal(g.manual, null);
  g.setDetected(true); // next call
  assert.equal(g.recording, true, "next call records itself again");
});

test("a resumed auto-call still finalizes when the call ends", () => {
  const g = new Gate(true);
  g.setDetected(true);
  g.setManual("pause");
  g.setManual("resume"); // resume is call-relative, not a hard recording
  assert.equal(g.recording, true);
  g.setDetected(false); // call ends
  assert.equal(g.recording, false, "resumed call finalizes with the call");
});

test("a hard manual recording (no call under it) survives stray detections", () => {
  const g = new Gate(true);
  g.setManual("start"); // detected=false → this is a HARD recording
  assert.equal(g.recording, true);
  g.setDetected(true); // stray detection must not hijack it
  assert.equal(g.recording, true);
  g.setDetected(false); // …and its disappearance must not stop it
  assert.equal(g.recording, true);
  g.setManual("stop"); // only the user stops it
  assert.equal(g.recording, false);
});

test("a manual start DURING a call ends with that call", () => {
  const g = new Gate(true);
  g.setDetected(true); // call active → start is soft, not hard
  g.setManual("start");
  g.setDetected(false);
  assert.equal(g.recording, false);
});
