"use strict";

const HISTORY_LIMIT = 600;
const DETECTION_LAG_MS = 300;
const EVIDENCE_TAIL_MS = 10_000;
const MIN_OVERLAP_MS = 200;
const TRUSTED_SOURCES = new Set([
  "active_speaker_label",
  "active_speaker_tile",
  "meet_dom",
  "zoom_ax",
  "extension_active_speaker",
]);

function clean(value, cap) {
  return String(value || "").replace(/[\u0000-\u001f\u007f]/g, " ").replace(/\s+/g, " ").trim().slice(0, cap);
}

function genericSpeaker(value) {
  const normalized = clean(value, 120).toLowerCase();
  return !normalized
    || ["speaker", "unknown", "unknown speaker", "other speaker", "them"].includes(normalized)
    || /^speaker [a-z0-9]+$/i.test(normalized);
}

function confidence(source) {
  if (source === "active_speaker_label" || source === "zoom_ax") return "high";
  if (["active_speaker_tile", "meet_dom", "extension_active_speaker"].includes(source)) return "medium";
  return "unknown";
}

class SpeakerAttribution {
  constructor() {
    this.timeline = [];
    this.evidenceSeenAt = 0;
  }

  reset() {
    this.timeline = [];
    this.evidenceSeenAt = 0;
  }

  update(event, now = Date.now()) {
    if (!event || event.type !== "state" || !["accessibility", "browser-extension"].includes(event.source)) return;
    const snapshot = event.state;
    if (!snapshot || !Object.prototype.hasOwnProperty.call(snapshot, "activeSpeaker")) return;
    const reportedAt = Number(event.ts);
    const observedAt = Number.isSafeInteger(reportedAt) && Math.abs(reportedAt - now) <= 60_000 ? reportedAt : now;
    this.evidenceSeenAt = observedAt;

    const selfName = clean(snapshot.selfName, 120);
    const requested = clean(snapshot.activeSpeaker, 120);
    const evidence = snapshot.activeSpeakerEvidence;
    const evidenceName = clean(evidence?.name, 120);
    const source = clean(evidence?.source, 80);
    const accepted = !genericSpeaker(requested)
      && (!selfName || requested.toLowerCase() !== selfName.toLowerCase())
      && requested.toLowerCase() === evidenceName.toLowerCase()
      && TRUSTED_SOURCES.has(source);
    const next = accepted
      ? { name: requested, source, context: clean(evidence?.context, 240), at: Math.max(0, observedAt - DETECTION_LAG_MS) }
      : { name: null, source: "", context: "", at: observedAt };
    const previous = this.timeline.at(-1);
    if (previous && previous.name === next.name && previous.source === next.source && previous.context === next.context) return;
    this.timeline.push(next);
    if (this.timeline.length > HISTORY_LIMIT) this.timeline.splice(0, this.timeline.length - HISTORY_LIMIT);
  }

  dominant(startedAt, endedAt) {
    if (!this.timeline.length || endedAt < startedAt) return null;
    const durations = new Map();
    this.timeline.forEach((transition, index) => {
      if (!transition.name) return;
      const intervalEnd = this.timeline[index + 1]?.at ?? this.evidenceSeenAt + EVIDENCE_TAIL_MS;
      const overlap = Math.min(endedAt, intervalEnd) - Math.max(startedAt, transition.at);
      if (overlap <= 0) return;
      const previous = durations.get(transition.name) || { duration: 0, index };
      durations.set(transition.name, { duration: previous.duration + overlap, index });
    });
    const ranked = [...durations.entries()].sort((left, right) => right[1].duration - left[1].duration);
    const top = ranked[0];
    if (!top || top[1].duration < MIN_OVERLAP_MS) return null;
    if (ranked[1] && top[1].duration < ranked[1][1].duration * 1.25) return null;
    const transition = this.timeline[top[1].index];
    return { name: top[0], source: transition.source, context: transition.context };
  }

  attribute(caption) {
    if (!caption || caption.type !== "caption") return caption;
    if (caption.self) {
      caption.speaker = "You";
      caption.speakerSource = "user";
      caption.speakerConfidence = "certain";
      delete caption.speakerContext;
      return caption;
    }
    if (!genericSpeaker(caption.speaker)) {
      caption.speakerSource ||= "capturer";
      caption.speakerConfidence ||= "provided";
      return caption;
    }
    const endedAt = Number.isSafeInteger(Number(caption.endedAt))
      ? Number(caption.endedAt)
      : Number.isSafeInteger(Number(caption.ts)) ? Number(caption.ts) : Date.now();
    const requestedStart = Number(caption.startedAt);
    const startedAt = Number.isSafeInteger(requestedStart) && requestedStart <= endedAt && endedAt - requestedStart <= 60_000
      ? requestedStart
      : Math.max(0, endedAt - 1_000);
    const speaker = this.dominant(startedAt, endedAt);
    if (speaker) {
      caption.speaker = speaker.name;
      caption.speakerSource = speaker.source;
      caption.speakerConfidence = confidence(speaker.source);
      if (speaker.context) caption.speakerContext = speaker.context;
    } else {
      caption.speaker = "Unknown speaker";
      caption.speakerSource = "unknown";
      caption.speakerConfidence = "unknown";
      delete caption.speakerContext;
    }
    return caption;
  }
}

module.exports = { SpeakerAttribution, genericSpeaker };
