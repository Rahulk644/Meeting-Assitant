"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  applyCorrection,
  canGenerateNotes,
  deriveTopicTitle,
  isAuthoritativeMeetingDetection,
  resolvedTitle,
  transcriptText,
  validDetectedTitle,
} = require("../meeting-domain.js");

function fixture() {
  return {
    id: "m1", title: "Untitled meeting", app: "zoom", startedAt: 1_700_000_000_000,
    endedAt: 1_700_000_600_000, participants: ["Rahul", "Zoom Settings", "Rajat"],
    lines: [
      { id: "a", speaker: "Rahul", self: true, text: "We need to finalize the enterprise launch plan this week." },
      { id: "b", speaker: "Zoom Settings", text: "The pricing decision depends on the customer interviews." },
      { id: "c", speaker: "Rajat", text: "I will complete three customer interviews by Thursday." },
      { id: "d", speaker: "Rajat", text: "Then we can publish the rollout plan with owners." },
    ],
    summary: "Earlier summary", transcriptRevision: 0, summaryRevision: 0,
  };
}

test("generic and Zoom chrome titles are rejected; unscheduled calls stay explicitly impromptu", () => {
  assert.equal(validDetectedTitle("Untitled meeting"), false);
  assert.equal(validDetectedTitle("Impromptu meeting"), false);
  assert.equal(validDetectedTitle("zoom floating video window"), false);
  assert.match(deriveTopicTitle(fixture()), /enterprise launch plan/i);
  assert.equal(resolvedTitle(fixture()), "Impromptu meeting");
});

test("user and calendar titles outrank detected meeting-window titles", () => {
  const scheduled = { ...fixture(), title: "Zoom Meeting", calendarTitle: "Enterprise launch review" };
  assert.equal(resolvedTitle(scheduled), "Enterprise launch review");
  scheduled.title = "Customer renamed title";
  scheduled.titleSource = "user";
  assert.equal(resolvedTitle(scheduled), "Customer renamed title");
});

test("participant and transcript corrections persist in the meeting model and stale derived notes", () => {
  const m = fixture();
  applyCorrection(m, { operation: "rename_participant", from: "Zoom Settings", to: "Aditi" });
  applyCorrection(m, { operation: "reassign_line", lineId: "c", speaker: "Aditi" });
  applyCorrection(m, { operation: "edit_transcript", lineId: "b", text: "Pricing depends on six customer interviews." });
  applyCorrection(m, { operation: "exclude_line", lineId: "d", excluded: true });
  assert.ok(m.participants.includes("Aditi"));
  assert.equal(m.lines.find((line) => line.id === "c").speaker, "Aditi");
  assert.equal(m.lines.find((line) => line.id === "b").text, "Pricing depends on six customer interviews.");
  assert.equal(m.lines.find((line) => line.id === "d").excluded, true);
  assert.equal(m.summaryStale, true);
  assert.equal(m.summaryState, "stale");
  assert.equal(transcriptText(m).includes("rollout plan with owners"), false);
});

test("speaker identities can be merged and removed", () => {
  const m = fixture();
  applyCorrection(m, { operation: "merge_speakers", from: "Rajat", into: "Rahul" });
  assert.equal(m.lines.filter((line) => line.speaker === "Rajat").length, 0);
  applyCorrection(m, { operation: "remove_participant", name: "Rahul" });
  assert.equal(m.participants.includes("Rahul"), false);
  assert.ok(m.lines.some((line) => line.speaker === "Unknown speaker"));
});

test("automatic notes reject short sessions and accept substantive meetings", () => {
  assert.equal(canGenerateNotes({ startedAt: 0, endedAt: 10_000, lines: [{ text: "hello", speaker: "A" }] }).ok, false);
  assert.equal(canGenerateNotes(fixture()).ok, true);
});

test("only Core Audio state can automatically start or end a meeting", () => {
  assert.equal(isAuthoritativeMeetingDetection({
    type: "state", source: "coreaudio", state: { inMeeting: true, app: "zoom" },
  }), true);
  assert.equal(isAuthoritativeMeetingDetection({
    type: "state", source: "accessibility", state: { inMeeting: true, app: "zoom" },
  }), false, "Zoom UI metadata is enrichment, not call detection");
  assert.equal(isAuthoritativeMeetingDetection({
    type: "state", state: { inMeeting: true, app: "zoom" },
  }), false, "untagged legacy state fails closed");
});
