"use strict";

// Server-owned product release contract. The browser receives only the resolved
// channel and booleans; it cannot promote a capability through query parameters,
// localStorage, or a stale saved route.
const CAPABILITIES = Object.freeze({
  meetings:                 { state: "shipped", verification: "meeting lifecycle and library tests" },
  dictation:                 { state: "shipped", verification: "native dictation settings and hotkey path" },
  settings:                  { state: "shipped", verification: "production settings-handler contract" },
  writingAssistant:          { state: "preview", verification: "implementation and native shortcut required" },
  agents:                    { state: "preview", verification: "agent runtime required" },
  agentFoundation:           { state: "shipped", verification: "honest read-only Agents foundation surface" },
  agentContextInspector:     { state: "shipped", verification: "bounded on-demand native Accessibility snapshot and helper self-test" },
  feedbackSupport:           { state: "hidden", verification: "working destinations required" },
  guide:                     { state: "hidden", verification: "published guide required" },
  shareFriend:               { state: "hidden", verification: "working share destination required" },
  pro:                       { state: "blocked", verification: "billing and entitlement verification required" },

  meetingRecord:             { state: "shipped", verification: "manual and automatic gate tests" },
  automaticRecording:        { state: "shipped", verification: "recording gate defaults and call-edge tests" },
  meetingDetectedNotification:{ state: "shipped", verification: "Core Audio rising-edge and native notification tests" },
  liveTranscript:            { state: "shipped", verification: "SSE transcript path" },
  callPanel:                 { state: "shipped", verification: "recording-state UI contract" },
  scratchpad:                { state: "shipped", verification: "meeting notes persistence route" },
  pauseResumeStop:           { state: "shipped", verification: "recording gate transition tests" },
  meetingLibrary:            { state: "shipped", verification: "meeting summary/list domain tests" },
  meetingThumbnails:         { state: "shipped", verification: "recording-frame and platform-fallback endpoint tests" },
  meetingRename:             { state: "shipped", verification: "title persistence endpoint" },
  meetingExport:             { state: "shipped", verification: "client Markdown export handler" },
  meetingSummary:            { state: "shipped", verification: "summary endpoint and retry states" },
  meetingActions:            { state: "shipped", verification: "summary action extraction" },
  meetingTranscript:         { state: "shipped", verification: "saved transcript detail" },
  meetingAsk:                { state: "shipped", verification: "corrected transcript ask endpoint" },
  meetingUpload:             { state: "preview", verification: "upload and transcription pipeline required" },
  meetingShare:              { state: "blocked", verification: "sharing backend and destination required" },
  calendarView:              { state: "preview", verification: "calendar view implementation required" },
  folders:                   { state: "preview", verification: "folder persistence required" },
  googleCalendarSource:      { state: "blocked", verification: "OAuth and calendar sync required" },
  fathomSource:              { state: "blocked", verification: "import adapter required" },
  meetingShortcut:           { state: "blocked", verification: "native registration required" },
  storageLimits:             { state: "blocked", verification: "storage enforcement required" },
  recordingNotice:           { state: "hidden", verification: "supported-chat proof and duplicate prevention required" },

  dictationSystem:           { state: "shipped", verification: "native recorder and paste path" },
  appleSpeechAnalyzer:       { state: "blocked", verification: "native adapter, asset lifecycle, fallback, and benchmark required" },
  curatedSpeechPicker:       { state: "shipped", verification: "pinned catalog, verified lifecycle, selection routing, and production UI contract" },
  customVocabulary:          { state: "shipped", verification: "vocabulary config route" },
  languagePunctuation:       { state: "shipped", verification: "dictation config application" },
  clipboardBehavior:         { state: "shipped", verification: "native clipboard modes" },
  dictationOverlay:          { state: "shipped", verification: "native overlay settings" },
  soundVolume:               { state: "shipped", verification: "native sound and volume settings" },
  dictationShortcut:         { state: "shipped", verification: "native shortcut registration" },
  recentDictations:          { state: "shipped", verification: "session dictation feed" },
  dictationUpload:           { state: "preview", verification: "voice-note upload pipeline required" },
  documentationLink:         { state: "hidden", verification: "published documentation destination required" },

  appearance:                { state: "shipped", verification: "theme handler" },
  accent:                    { state: "shipped", verification: "accent handler" },
  trayVisibility:            { state: "shipped", verification: "native tray visibility command" },
  screenShareInvisibility:   { state: "shipped", verification: "native content protection command" },
  permissions:               { state: "shipped", verification: "permission rows and recovery actions" },
  liveMeetingIndicator:      { state: "shipped", verification: "call panel visibility handler" },
  automaticMeetingDetection: { state: "shipped", verification: "recording gate config" },
  deeperMeetingReasoning:    { state: "shipped", verification: "summary request option" },
  transcriptionLanguage:     { state: "shipped", verification: "capture config handler" },
  translation:               { state: "shipped", verification: "capability-gated Whisper translation task and incompatible-model fail-closed routing" },
  videoRecording:            { state: "shipped", verification: "screen-recording mode handler" },
  meetingTemplates:          { state: "shipped", verification: "summary prompt templates" },
  aiProvider:                { state: "shipped", verification: "local and OpenAI-compatible summary path" },
  unloadModel:               { state: "shipped", verification: "model idle config handler" },
  profileAccount:            { state: "hidden", verification: "local identity-only treatment or account backend required" },
  startupBehavior:           { state: "blocked", verification: "native launch-at-login behavior required" },
  updater:                   { state: "blocked", verification: "signed update service required" },
  writingSettings:           { state: "preview", verification: "writing assistant runtime required" },
  meetingSource:             { state: "blocked", verification: "source adapters required" },
  whatsNew:                  { state: "blocked", verification: "release-notes behavior required" },
  experimental:              { state: "preview", verification: "individual experiment release gates required" },
  supportCreator:            { state: "hidden", verification: "real support destination required" },

  participantCorrections:    { state: "shipped", verification: "meeting correction domain tests" },
  transcriptCorrections:     { state: "shipped", verification: "meeting correction domain tests" },
  automaticTitles:           { state: "shipped", verification: "meeting-title domain tests" },
  automaticNotes:            { state: "shipped", verification: "post-call queue and recoverable state" },
  accessibilitySemantics:    { state: "shipped", verification: "production markup contract" },
});

const PREVIEW_ONLY_STATES = new Set(["preview", "hidden", "blocked"]);

function normalizeChannel(raw) {
  return String(raw || "").toLowerCase() === "preview" ? "preview" : "production";
}

function capabilityMap(channel) {
  const resolved = normalizeChannel(channel);
  return Object.fromEntries(Object.entries(CAPABILITIES).map(([key, meta]) => [
    key,
    resolved === "preview" ? true : meta.state === "shipped",
  ]));
}

function releasePayload(channel) {
  const resolved = normalizeChannel(channel);
  return {
    channel: resolved,
    capabilities: capabilityMap(resolved),
    states: Object.fromEntries(Object.entries(CAPABILITIES).map(([key, meta]) => [key, meta.state])),
    routes: ROUTE_CAPABILITY,
    settingCapabilities: SETTING_CAPABILITY,
    panelCapabilities: PANEL_CAPABILITY,
    staticSurfaces: STATIC_SURFACE_CAPABILITY,
  };
}

function isCapabilityEnabled(capabilities, key) {
  return Object.prototype.hasOwnProperty.call(CAPABILITIES, key) && capabilities?.[key] === true;
}

const ROUTE_CAPABILITY = Object.freeze({
  meetings: "meetings",
  dictation: "dictation",
  writing: "writingAssistant",
  agents: "agentFoundation",
  feedback: "feedbackSupport",
  guide: "guide",
  share: "shareFriend",
  settings: "settings",
});

function sanitizeRoute(route, capabilities) {
  const key = ROUTE_CAPABILITY[route];
  return key && isCapabilityEnabled(capabilities, key) ? route : "meetings";
}

const SETTING_CAPABILITY = Object.freeze({
  appearance: "appearance", accent: "accent", start_login: "startupBehavior", start_hidden: "startupBehavior",
  stealth: "screenShareInvisibility", permissions: "permissions", version: "updater", show_tray: "trayVisibility",
  overlay_enabled: "dictationOverlay", overlay_pos: "dictationOverlay", dict_enabled: "dictationSystem",
  push_to_talk: "dictationSystem", speech_models: "curatedSpeechPicker", dict_lang: "languagePunctuation", punctuation: "languagePunctuation",
  clipboard: "clipboardBehavior", custom_vocab: "customVocabulary", dict_sound: "soundVolume", volume: "soundVolume",
  sc_start: "dictationShortcut", sc_cancel: "dictationShortcut", wa_selection: "writingSettings",
  wa_noneditable: "writingSettings", wa_translate_lang: "writingSettings", wa_modes: "writingSettings",
  wa_model: "writingSettings", wa_open: "writingSettings", wa_close: "writingSettings", mtg_source: "meetingSource",
  live_indicator: "liveMeetingIndicator", auto_detect: "automaticMeetingDetection", deeper: "deeperMeetingReasoning",
  default_view: "calendarView", folders: "folders", trans_lang: "transcriptionLanguage", translate_en: "translation",
  screen_rec: "videoRecording", audio_storage: "storageLimits", templates: "meetingTemplates", mtg_model: "aiProvider",
  mtg_shortcut: "meetingShortcut", unload_model: "unloadModel", whisper_idle_minutes: "unloadModel", whats_new: "whatsNew",
  support_creator: "supportCreator", copilot: "experimental", browser_meet: "experimental", agents_exp: "experimental",
});

const PANEL_CAPABILITY = Object.freeze({
  profile: "profileAccount", general: "settings", dictation: "dictation", writing: "writingSettings",
  meetings: "meetings", advanced: "unloadModel", experimental: "experimental",
});

const STATIC_SURFACE_CAPABILITY = Object.freeze({
  uploadAudio: "meetingUpload", "det-share": "meetingShare", dictUploadBtn: "dictationUpload",
  docLink: "documentationLink", proCard: "pro",
});

// Contract lists used by tests: every production-visible item must map to code that
// applies it, not just a stored value or a "coming soon" toast.
const PRODUCTION_SETTINGS = Object.freeze(Object.entries(SETTING_CAPABILITY)
  .filter(([, cap]) => CAPABILITIES[cap].state === "shipped").map(([id]) => id));
const HANDLED_SETTINGS = Object.freeze([
  "appearance", "accent", "stealth", "permissions", "show_tray", "overlay_enabled", "overlay_pos",
  "dict_enabled", "push_to_talk", "speech_models", "dict_lang", "punctuation", "clipboard", "custom_vocab", "dict_sound",
  "volume", "sc_start", "sc_cancel", "live_indicator", "auto_detect", "deeper", "trans_lang",
  "translate_en", "screen_rec", "templates", "mtg_model", "unload_model", "whisper_idle_minutes",
]);
const PRODUCTION_CTAS = Object.freeze([
  "meeting-record", "meeting-pause", "meeting-resume", "meeting-stop", "meeting-open", "meeting-rename",
  "meeting-export", "meeting-generate-summary", "meeting-ask", "participant-correct", "transcript-correct",
  "dictation-vocabulary", "settings-open", "agent-context-inspect",
]);
const HANDLED_CTAS = Object.freeze([...PRODUCTION_CTAS]);

module.exports = {
  CAPABILITIES,
  PREVIEW_ONLY_STATES,
  ROUTE_CAPABILITY,
  SETTING_CAPABILITY,
  PANEL_CAPABILITY,
  STATIC_SURFACE_CAPABILITY,
  PRODUCTION_SETTINGS,
  HANDLED_SETTINGS,
  PRODUCTION_CTAS,
  HANDLED_CTAS,
  normalizeChannel,
  capabilityMap,
  releasePayload,
  isCapabilityEnabled,
  sanitizeRoute,
};
