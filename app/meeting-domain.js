"use strict";

const GENERIC_TITLE = /^(untitled meeting|impromptu meeting|meeting|zoom meeting|teams meeting|webex meeting|new meeting|personal meeting room)$/i;
const CHROME_TITLE = /(zoom floating video window|share status|participants|waiting room|audio settings|professional audio|meeting controls)/i;
const FILLER_START = /^(hi|hello|hey|okay|ok|um|uh|thanks|thank you|good morning|good afternoon|good evening)[,!.\s-]*/i;

function cleanText(value, max = 4000) {
  return String(value == null ? "" : value).replace(/\s+/g, " ").trim().slice(0, max);
}

function isSubstantiveLine(line) {
  if (!line || line.excluded) return false;
  const text = cleanText(line.text);
  return text.length >= 8 && /[A-Za-z\p{L}]/u.test(text) && !/^\[(music|silence|inaudible)\]$/i.test(text);
}

function includedLines(meeting) {
  return Array.isArray(meeting?.lines) ? meeting.lines.filter((line) => line && !line.excluded && cleanText(line.text)) : [];
}

function substantiveLines(meeting) { return includedLines(meeting).filter(isSubstantiveLine); }

function transcriptText(meeting, cap = 16000) {
  return includedLines(meeting)
    .map((line) => `${cleanText(line.speaker || "Unknown speaker", 120)}: ${cleanText(line.text)}`)
    .join("\n").slice(0, cap);
}

function validDetectedTitle(value) {
  const title = cleanText(value, 120);
  return !!title && !GENERIC_TITLE.test(title) && !CHROME_TITLE.test(title) && title.length >= 3;
}

function deriveTopicTitle(meeting) {
  for (const line of substantiveLines(meeting)) {
    let text = cleanText(line.text, 160).replace(FILLER_START, "").trim();
    text = text.split(/(?<=[.!?])\s/)[0].replace(/[.!?]+$/, "").trim();
    const words = text.split(/\s+/).filter(Boolean);
    if (words.length < 4) continue;
    const short = words.slice(0, 10).join(" ");
    return short.charAt(0).toUpperCase() + short.slice(1) + (words.length > 10 ? "…" : "");
  }
  return "";
}

function appLabel(app) {
  const key = cleanText(app, 40).toLowerCase();
  return ({ zoom: "Zoom", teams: "Microsoft Teams", meet: "Google Meet", webex: "Webex", facetime: "FaceTime", slack: "Slack Huddle", discord: "Discord", jitsi: "Jitsi Meet", whereby: "Whereby", chime: "Amazon Chime", ringcentral: "RingCentral", goto: "GoTo Meeting", riverside: "Riverside", around: "Around", whatsapp: "WhatsApp", telegram: "Telegram", signal: "Signal" })[key]
    || cleanText(app, 40) || "Meeting";
}

function fallbackTitle(meeting) {
  return "Impromptu meeting";
}

function resolvedTitle(meeting) {
  if (meeting?.titleSource === "user" && cleanText(meeting.title, 120)) return cleanText(meeting.title, 120);
  if (validDetectedTitle(meeting?.calendarTitle)) return cleanText(meeting.calendarTitle, 120);
  if (validDetectedTitle(meeting?.title)) return cleanText(meeting.title, 120);
  return fallbackTitle(meeting);
}

function ensureLineIds(meeting) {
  if (!Array.isArray(meeting.lines)) meeting.lines = [];
  meeting.lines.forEach((line, index) => {
    if (!line.id) line.id = `line-${Number(line.ts) || Number(meeting.startedAt) || 0}-${index}`;
    if (!line.speaker) line.speaker = "Unknown speaker";
    if (!line.speakerSource) line.speakerSource = line.self ? "user" : "unknown";
    if (!line.speakerConfidence) line.speakerConfidence = line.self || line.speakerSource === "user_correction" ? "certain" : "unknown";
  });
  return meeting;
}

function canGenerateNotes(meeting) {
  const lines = substantiveLines(meeting);
  const words = lines.reduce((count, line) => count + cleanText(line.text).split(/\s+/).filter(Boolean).length, 0);
  const duration = Math.max(0, (Number(meeting?.endedAt) || 0) - (Number(meeting?.startedAt) || 0));
  if (lines.length < 3) return { ok: false, reason: "Not enough substantive transcript lines." };
  if (words < 30) return { ok: false, reason: "The transcript is too short to generate useful notes." };
  if (duration && duration < 30_000) return { ok: false, reason: "The meeting was too short to generate useful notes." };
  return { ok: true, reason: "" };
}

// Backward-compatible ingest events are authoritative only when they came from the audio subsystem.
// Current adapters use /api/meeting/observation and the shared detector instead. Accessibility enrichment
// still cannot start a call by itself; an idle Zoom process may expose meeting-shaped controls.
function isAuthoritativeMeetingDetection(event) {
  return event?.type === "state"
    && event?.source === "coreaudio"
    && typeof event?.state?.inMeeting === "boolean";
}

function invalidateDerived(meeting, reason) {
  meeting.transcriptRevision = (Number(meeting.transcriptRevision) || 0) + 1;
  meeting.lastCorrectionAt = Date.now();
  meeting.lastCorrectionReason = reason;
  if (meeting.summary) {
    meeting.summaryStale = true;
    meeting.summaryState = "stale";
  }
}

function uniqueNames(values) {
  return [...new Set(values.map((v) => cleanText(v, 120)).filter(Boolean))];
}

function requireName(value, field) {
  const name = cleanText(value, 120);
  if (!name) throw new Error(`${field} is required`);
  return name;
}

function findLine(meeting, correction) {
  const lines = meeting.lines || [];
  if (correction.lineId) return lines.find((line) => line.id === correction.lineId);
  const index = Number(correction.lineIndex);
  return Number.isInteger(index) && index >= 0 ? lines[index] : null;
}

function applyCorrection(meeting, correction) {
  if (!meeting || typeof meeting !== "object") throw new Error("meeting is required");
  if (!correction || typeof correction !== "object") throw new Error("correction is required");
  ensureLineIds(meeting);
  if (!Array.isArray(meeting.participants)) meeting.participants = [];
  const operation = cleanText(correction.operation, 60);

  if (operation === "rename_participant") {
    const from = requireName(correction.from, "from"), to = requireName(correction.to, "to");
    meeting.participants = uniqueNames(meeting.participants.map((name) => name === from ? to : name).concat(to));
    meeting.lines.forEach((line) => { if (line.speaker === from) { line.speaker = to; line.speakerSource = "user_correction"; line.speakerConfidence = "certain"; } });
  } else if (operation === "remove_participant") {
    const name = requireName(correction.name, "name");
    meeting.participants = meeting.participants.filter((value) => value !== name);
    meeting.lines.forEach((line) => { if (line.speaker === name) { line.speaker = "Unknown speaker"; line.speakerSource = "user_correction"; line.speakerConfidence = "unknown"; } });
  } else if (operation === "merge_speakers") {
    const from = requireName(correction.from, "from"), into = requireName(correction.into, "into");
    if (from === into) throw new Error("speaker identities must be different");
    meeting.participants = uniqueNames(meeting.participants.filter((name) => name !== from).concat(into));
    meeting.lines.forEach((line) => { if (line.speaker === from) { line.speaker = into; line.speakerSource = "user_correction"; line.speakerConfidence = "certain"; } });
  } else if (["edit_transcript", "reassign_line", "exclude_line"].includes(operation)) {
    const line = findLine(meeting, correction);
    if (!line) throw new Error("transcript line not found");
    if (operation === "edit_transcript") {
      const text = cleanText(correction.text);
      if (!text) throw new Error("text is required");
      line.text = text;
    } else if (operation === "reassign_line") {
      const speaker = requireName(correction.speaker, "speaker");
      line.speaker = speaker;
      line.self = speaker === "You";
      line.speakerSource = "user_correction";
      line.speakerConfidence = "certain";
      if (!line.self && speaker !== "Unknown speaker") meeting.participants = uniqueNames(meeting.participants.concat(speaker));
    } else {
      line.excluded = correction.excluded !== false;
    }
  } else {
    throw new Error("unknown correction operation");
  }

  invalidateDerived(meeting, operation);
  return meeting;
}

module.exports = {
  cleanText,
  isSubstantiveLine,
  includedLines,
  substantiveLines,
  transcriptText,
  validDetectedTitle,
  deriveTopicTitle,
  fallbackTitle,
  resolvedTitle,
  ensureLineIds,
  canGenerateNotes,
  isAuthoritativeMeetingDetection,
  applyCorrection,
};
