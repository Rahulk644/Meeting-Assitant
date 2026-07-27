"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const { SpeakerAttribution } = require("../speaker-attribution.js");

test("speaker attribution maps the dominant provenanced participant over the speech interval", () => {
  const attribution = new SpeakerAttribution();
  attribution.update({
    type: "state",
    source: "accessibility",
    ts: 1_300,
    state: {
      selfName: "Rahul Khatri",
      activeSpeaker: "Aditi",
      activeSpeakerEvidence: {
        name: "Aditi",
        source: "active_speaker_label",
        context: "Zoom Meeting",
      },
    },
  }, 1_300);
  attribution.update({
    type: "state",
    source: "accessibility",
    ts: 3_300,
    state: {
      selfName: "Rahul Khatri",
      activeSpeaker: "Rajat",
      activeSpeakerEvidence: {
        name: "Rajat",
        source: "active_speaker_tile",
        context: "zoom floating video window",
      },
    },
  }, 3_300);

  const line = attribution.attribute({
    type: "caption",
    speaker: "Speaker",
    self: false,
    startedAt: 1_200,
    endedAt: 2_800,
    text: "We should ship this week.",
  });
  assert.equal(line.speaker, "Aditi");
  assert.equal(line.speakerSource, "active_speaker_label");
  assert.equal(line.speakerConfidence, "high");
});

test("speaker attribution filters self evidence and keeps ambiguous turns unnamed", () => {
  const attribution = new SpeakerAttribution();
  attribution.update({
    type: "state",
    source: "accessibility",
    ts: 1_300,
    state: {
      selfName: "Rahul Khatri",
      activeSpeaker: "Rahul Khatri",
      activeSpeakerEvidence: {
        name: "Rahul Khatri",
        source: "active_speaker_label",
        context: "Zoom Meeting",
      },
    },
  }, 1_300);
  assert.equal(attribution.attribute({
    type: "caption",
    speaker: "Speaker",
    self: false,
    startedAt: 1_200,
    endedAt: 2_800,
  }).speaker, "Unknown speaker");
  assert.equal(attribution.attribute({
    type: "caption",
    speaker: "Rahul Khatri",
    self: true,
    startedAt: 1_200,
    endedAt: 2_800,
  }).speaker, "You");
});

test("speaker evidence is scoped to one meeting", () => {
  const attribution = new SpeakerAttribution();
  attribution.update({
    type: "state",
    source: "accessibility",
    ts: 1_300,
    state: {
      activeSpeaker: "Aditi",
      activeSpeakerEvidence: {
        name: "Aditi",
        source: "active_speaker_label",
        context: "Zoom Meeting",
      },
    },
  }, 1_300);
  attribution.reset();
  assert.equal(attribution.attribute({
    type: "caption",
    speaker: "Speaker",
    self: false,
    startedAt: 1_500,
    endedAt: 2_500,
  }).speaker, "Unknown speaker");
});
