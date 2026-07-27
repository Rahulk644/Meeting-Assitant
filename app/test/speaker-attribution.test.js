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

test("microphone ownership fails closed until meeting audio proves channel separation", () => {
  const attribution = new SpeakerAttribution();
  const line = attribution.attribute({
    type: "caption",
    speaker: "Rahul Khatri",
    self: true,
    startedAt: 1_500,
    endedAt: 2_500,
    text: "Speech heard while meeting audio is unavailable.",
  }, "starting");
  assert.equal(line.speaker, "Unknown speaker");
  assert.equal(line.self, false);
  assert.equal(line.speakerSource, "microphone_unseparated");
  assert.equal(line.speakerConfidence, "unknown");
});

test("explicit remote talking evidence reclassifies speaker playback heard by the microphone", () => {
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
  const line = attribution.attribute({
    type: "caption",
    speaker: "Rahul Khatri",
    self: true,
    startedAt: 1_200,
    endedAt: 2_800,
    text: "Remote playback heard by the microphone.",
  }, "ready");
  assert.equal(line.speaker, "Aditi");
  assert.equal(line.self, false);
  assert.equal(line.speakerSource, "microphone_playback:active_speaker_label");
  assert.equal(line.speakerConfidence, "high");
});

test("meeting mute evidence prevents microphone playback from being labelled as You", () => {
  const attribution = new SpeakerAttribution();
  attribution.update({
    type: "state",
    source: "accessibility",
    ts: 1_300,
    state: {
      selfMuted: true,
      selfName: "Rahul Khatri",
      activeSpeaker: "Vikram Prasanna",
      activeSpeakerEvidence: {
        name: "Vikram Prasanna",
        source: "active_speaker_tile",
        context: "zoom floating video window",
      },
    },
  }, 1_300);
  const line = attribution.attribute({
    type: "caption",
    speaker: "Rahul Khatri",
    self: true,
    startedAt: 1_200,
    endedAt: 2_800,
    text: "Remote playback heard while the local Zoom participant is muted.",
  }, "ready");
  assert.equal(line.speaker, "Vikram Prasanna");
  assert.equal(line.self, false);
  assert.equal(line.speakerSource, "microphone_playback:active_speaker_tile");
  assert.equal(line.speakerConfidence, "medium");
});

test("meeting mute evidence fails closed when no remote identity is available", () => {
  const attribution = new SpeakerAttribution();
  attribution.update({
    type: "state",
    source: "accessibility",
    ts: 1_300,
    state: {
      selfMuted: true,
    },
  }, 1_300);
  const line = attribution.attribute({
    type: "caption",
    speaker: "Rahul Khatri",
    self: true,
    startedAt: 1_200,
    endedAt: 2_800,
    text: "Audio without identity evidence.",
  }, "ready");
  assert.equal(line.speaker, "Unknown speaker");
  assert.equal(line.self, false);
  assert.equal(line.speakerSource, "microphone_while_muted");
  assert.equal(line.speakerConfidence, "unknown");
});
