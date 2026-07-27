#!/usr/bin/env node
// Live speaker-attributed transcript app.
//   node server.js  -> spawn ../meeting-state/meeting-state, stream its JSON to the GUI + relay captions.
// No dependencies. Open http://localhost:8787
const http = require("http");
const crypto = require("crypto");
const fs = require("fs");
const net = require("net");
const path = require("path");
const { spawn, execFile } = require("child_process");
const { Gate, MeetingDetector } = require("./gate.js");
const { fetchProviderModels, providerError, sanitizeDeveloperAiModel } = require("./ai-provider.js");
const { injectMultipartFields } = require("./multipart.js");
const { releasePayload } = require("./release.js");
const { DEFAULT_MODEL_ID, SPEECH_MODELS, speechModel } = require("./speech-models.js");
const { SpeakerAttribution } = require("./speaker-attribution.js");
const { appThumbnailSvg } = require("./thumbnail.js");
const {
  applyCorrection,
  canGenerateNotes,
  ensureLineIds,
  isAuthoritativeMeetingDetection,
  resolvedTitle,
  transcriptText: correctedTranscriptText,
  validDetectedTitle,
} = require("./meeting-domain.js");

const PORT = process.env.PORT || 8787; // respect preview's assigned port; 8787 for manual runs
const GUI_TOKEN = process.env.ASSISTANT_GUI_TOKEN || crypto.randomBytes(32).toString("hex");
const RELEASE = releasePayload(process.env.ASSISTANT_RELEASE_CHANNEL); // defaults to production; no URL override
const clients = new Set();
let lastState = {}; // so a client connecting mid-meeting gets current title/participants/speaker
let msProc = null, msShutdown = false; // meeting-state child + shutdown flag (respawn it if it dies mid-session)
// Writable data root: packaged builds pass ASSISTANT_DATA_DIR (~/Library/Application Support/My Assistant/data,
// since the .app bundle itself is read-only); dev / ./run.sh fall back to the repo's data/.
const DATA_ROOT = process.env.ASSISTANT_DATA_DIR || path.join(__dirname, "..", "data");
const MEETINGS_DIR = path.join(DATA_ROOT, "meetings");
const REC_DIR = path.join(MEETINGS_DIR, "rec"); // screen recordings (.mp4), written by meeting-tap
const AGENT_CONTEXT = path.join(__dirname, "..", "agent-context", "agent-context");
// Resolve tool paths explicitly — a packaged (GUI-spawned) node has no shell PATH.
function findBin(name) { for (const d of ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]) { const p = path.join(d, name); if (fs.existsSync(p)) return p; } return name; }
const FFMPEG = findBin("ffmpeg"); // for meeting thumbnails from the screen recording
fs.mkdirSync(REC_DIR, { recursive: true, mode: 0o700 });
for (const dir of [DATA_ROOT, MEETINGS_DIR, REC_DIR]) { try { fs.chmodSync(dir, 0o700); } catch {} }
let cur = null; // the in-progress meeting being recorded to disk

// Recording gate: composes auto-detection with the manual override (see gate.js). Drives the meeting
// lifecycle and what the UI/tray show.
const gate = new Gate(true);
const meetingDetector = new MeetingDetector();
const speakerAttribution = new SpeakerAttribution();
let unloadIdle = true; // "Unload model when idle" setting — off = keep whisper hot between calls
let whisperIdleMinutes = 10; // user-configurable; 10 minutes balances back-to-back speed with RAM recovery
let vocabPrompt = ""; // custom vocabulary → whisper initial prompt (handed to meeting-tap via /api/state)
// Dictation: only the bits the SERVER applies at transcription time live here — the spoken language and
// the punctuation behaviour. Everything the desktop app owns (sound, volume, clipboard, overlay, the
// rebindable hotkey, push-to-talk) is set directly in that process via its set_dictation command, not
// through here. Results come back via POST /ingest {type:'dictation'}.
let dictationCfg = { enabled: false, language: "auto", punctuation: "auto" };
let recentDictations = []; // last N dictation results — in-memory, session-lived (dictated text stays off disk)
// Feature modules — everything is OFF until the user opts in (onboarding), so NO macOS permission is
// requested until a module is enabled. meeting-tap/meeting-state gate their capability use on these.
let modules = { dictation: true, meetings: true, writing: false };
let dictStopSeq = 0; // bumped by POST /api/dictation/stop (pill/tray Stop) → meeting-tap finishes the current dictation
// Durable settings — WKWebView does NOT persist localStorage for the http://localhost origin Tauri loads
// (storage is bound to the tauri:// custom protocol, not an external URL), so EVERY UI setting was lost on
// relaunch (modules reset to off → dictation dead). Fix: the UI mirrors every setting write here and
// rehydrates on boot. settings.json is a flat mirror of the UI's localStorage keys, values stored verbatim.
const SETTINGS_FILE = path.join(DATA_ROOT, "settings.json");
let settingsBlob = {};
try { settingsBlob = JSON.parse(fs.readFileSync(SETTINGS_FILE, "utf8")) || {}; } catch { settingsBlob = {}; }
const devProviderKeys = Object.create(null); // developer harness only: never persist provider secrets
const sanitizeAiModel = raw => sanitizeDeveloperAiModel(raw, devProviderKeys);
let _settingsTimer = null;
function saveSettings() {
  clearTimeout(_settingsTimer);
  _settingsTimer = setTimeout(() => {
    try {
      const tmp = SETTINGS_FILE + ".tmp";
      fs.writeFileSync(tmp, JSON.stringify(settingsBlob), { mode: 0o600 });
      fs.renameSync(tmp, SETTINGS_FILE);
      fs.chmodSync(SETTINGS_FILE, 0o600);
    }
    catch (e) { console.error("settings save failed:", e.message); }
  }, 300);
}
if (settingsBlob["set.aiModel"]) {
  const before = settingsBlob["set.aiModel"];
  settingsBlob["set.aiModel"] = sanitizeAiModel(before);
  if (settingsBlob["set.aiModel"] !== before) saveSettings();
}
// Restore module gating from durable settings immediately, so meeting-tap sees the right state on restart
// (before the UI boots and re-pushes it) instead of the all-off default above.
try {
  const m = settingsBlob["set.modules"] && JSON.parse(settingsBlob["set.modules"]);
  if (m && typeof m === "object") { modules = { dictation: m.dictation !== false, meetings: m.meetings !== false, writing: !!m.writing }; dictationCfg.enabled = !!modules.dictation; }
} catch {}
const effectiveRecording = () => gate.recording;
function broadcastCall() { // push the composed call state to the UI (and lastState, for reconnects)
  broadcast({ type: "state", state: { inMeeting: gate.recording || gate.paused, paused: gate.paused, startedAt: cur ? cur.startedAt : null } });
}
function recomputeLifecycle(action) {
  if (gate.recording) { cancelFinalize(); ensureWhisper(); if (!cur) startMeeting(lastState); }
  else if (cur) {
    if (gate.paused) cancelFinalize();             // paused — keep the meeting open, just stop capturing
    else if (action === "stop") finalizeMeeting(); // manual stop — end it now
    else scheduleFinalize();                       // auto call-end — debounced so a brief drop won't split it
  }
}
function setDetected(d) { if (gate.setDetected(d)) { recomputeLifecycle(); broadcastCall(); } }
function setManual(action) {
  if (!gate.setManual(action)) return false;
  recomputeLifecycle(action); broadcastCall();
  return true;
}
// Raw state from a helper: Core Audio alone owns automatic meeting start/end. Accessibility is
// enrichment only; an open Zoom process must never start a recording. Peel inMeeting off every
// helper event so only the server's composed gate state reaches the UI.
function ingest(obj) {
  if (obj && obj.type === "state") speakerAttribution.update(obj);
  if (obj && obj.type === "caption") speakerAttribution.attribute(obj);
  if (obj && obj.type === "dictation") { // a finished dictation from meeting-tap → keep recent + push to the view
    if (obj.text) { recentDictations.unshift({ text: String(obj.text).slice(0, 4000), ts: obj.ts || Date.now(), ms: obj.ms || 0 }); recentDictations = recentDictations.slice(0, 50); }
    broadcast(obj); return;
  }
  if (obj && obj.type === "dictation-state") { broadcast(obj); return; } // listening on/off → drives the UI indicator
  if (obj && obj.type === "state" && obj.state && "inMeeting" in obj.state) {
    const { inMeeting, ...rest } = obj.state;
    if (Object.keys(rest).length) broadcast({ type: "state", state: rest });
    if (isAuthoritativeMeetingDetection(obj)) setDetected(inMeeting === true);
  } else broadcast(obj);
}

function broadcast(obj) {
  if (obj.type === "state" && obj.state) lastState = { ...lastState, ...obj.state };
  record(obj);
  const line = `data: ${JSON.stringify(obj)}\n\n`;
  for (const res of clients) res.write(line);
}
function writePrivateJson(file, value) {
  fs.writeFileSync(file, JSON.stringify(value), { mode: 0o600 });
  fs.chmodSync(file, 0o600);
}

// ---- meeting recording (history) ---------------------------------------------
// Accumulate live captions + state into the current meeting; persist JSON to data/meetings/.
function startMeeting(state) {
  cur = { id: String(Date.now()), title: (state && state.title) || "Untitled meeting",
          app: (state && state.app) || "", startedAt: Date.now(), endedAt: null,
          participants: (state && state.participants) ? [...state.participants] : [],
          participantEvidence: (state && state.participantEvidence) ? [...state.participantEvidence] : [],
          transcriptRevision: 0, lines: [] };
}
function updateMeta(state) {
  if (!cur || !state) return;
  if (state.title) cur.title = state.title;
  if (state.app) cur.app = state.app;
  if (state.participants && state.participants.length)
    cur.participants = [...new Set([...cur.participants, ...state.participants])];
  if (state.participantEvidence && state.participantEvidence.length) cur.participantEvidence = state.participantEvidence;
}
function saveMeeting(m) {
  try { ensureLineIds(m); writePrivateJson(path.join(MEETINGS_DIR, m.id + ".json"), m); }
  catch (e) { console.error("meeting save failed:", e.message); }
}
function finalizeMeeting() {
  if (!cur) return;
  cur.endedAt = Date.now();
  let finalizedId = null, finalizedSummaryState = "skipped";
  if (cur.lines.length) {
    if (!validDetectedTitle(cur.title)) { cur.title = resolvedTitle(cur); cur.titleSource = cur.title.includes(" — ") ? "fallback" : "local_topic"; }
    const noteGate = canGenerateNotes(cur);
    cur.summaryState = noteGate.ok ? "queued" : "skipped";
    finalizedSummaryState = cur.summaryState;
    cur.summaryError = noteGate.ok ? "" : noteGate.reason;
    saveMeeting(cur);
    finalizedId = cur.id;
  } // skip empty meetings
  cur = null;
  speakerAttribution.reset();
  delete lastState.startedAt; // meeting over — drop the echoed start time
  for (const key of ["activeSpeaker", "activeSpeakerEvidence", "selfName", "participants", "participantEvidence", "ignoredParticipantEvidence"]) delete lastState[key];
  unloadWhisperSoon(); // free ~1.5 GB after the call
  if (finalizedId) broadcast({ type: "meeting-updated", id: finalizedId, summaryState: finalizedSummaryState });
  if (finalizedId) queueAutomaticSummary(finalizedId);
}
// Debounce finalize: brief inMeeting=false blips (a screen-share toggle, a view switch, one missed
// detection poll) must NOT end the call and split it into a second meeting. Only a sustained absence
// finalizes; if inMeeting comes back within the grace window, it's the same meeting.
let finalizeTimer = null;
const FINALIZE_GRACE_MS = 15000; // brief drops only — the capturer now also detects the call while you screen-share
function scheduleFinalize() { if (!finalizeTimer && cur) finalizeTimer = setTimeout(() => { finalizeTimer = null; finalizeMeeting(); }, FINALIZE_GRACE_MS); }
function cancelFinalize() { if (finalizeTimer) { clearTimeout(finalizeTimer); finalizeTimer = null; } }
function record(obj) {
  if (obj.type === "caption") {
    if (!effectiveRecording()) return;                             // dropped while paused / stopped / not in a call
    if (!cur) startMeeting(lastState);
    const index = cur.lines.length, ts = obj.ts || Date.now();
    const endedAt = Number.isSafeInteger(Number(obj.endedAt)) ? Number(obj.endedAt) : ts;
    const requestedStart = Number(obj.startedAt);
    const startedAt = Number.isSafeInteger(requestedStart) && requestedStart <= endedAt && endedAt - requestedStart <= 60_000 ? requestedStart : endedAt;
    cur.lines.push({
      id: `line-${ts}-${index}`,
      speaker: obj.speaker || "Unknown speaker",
      speakerSource: obj.speakerSource || (obj.self ? "user" : "unknown"),
      speakerContext: obj.speakerContext || "",
      speakerConfidence: obj.speakerConfidence || (obj.self ? "certain" : "unknown"),
      text: obj.text,
      startedAt,
      endedAt,
      ts,
      self: !!obj.self,
      excluded: false,
    });
  } else if (obj.type === "recording") {
    if (!cur) startMeeting(lastState);
    cur.video = obj.file; saveMeeting(cur); // meeting-tap tells us the call's screen-recording file
  } else if (obj.type === "state" && obj.state) {
    updateMeta(obj.state);                                         // names/app only — inMeeting is handled by setDetected()
  }
}
setInterval(() => { if (cur && cur.lines.length) saveMeeting(cur); }, 10000); // flush so a kill mid-call doesn't lose it

function meetingSummary(m) {
  ensureLineIds(m);
  const visibleLines = m.lines.filter(l => !l.excluded);
  return { id: m.id, title: resolvedTitle(m), app: m.app, startedAt: m.startedAt, endedAt: m.endedAt,
           participants: m.participants, lineCount: visibleLines.length,
           snippet: ((visibleLines.find(l => l.text && l.text.length > 1) || {}).text || "").slice(0, 140),
           summarized: !!m.summary, hasVideo: !!(m.video && fs.existsSync(path.join(REC_DIR, m.video))),
           summaryState: m.summaryState || (m.summary ? "ready" : "idle"), summaryError: m.summaryError || "",
           summaryStale: !!m.summaryStale,
           live: m.endedAt == null };
}
// A server killed mid-call leaves a meeting with endedAt=null; on restart it isn't actually live, so
// finalize any such orphans (else they show "LIVE" forever). Only the in-memory `cur` is ever live.
function finalizeOrphans() {
  try {
    for (const f of fs.readdirSync(MEETINGS_DIR)) {
      if (!f.endsWith(".json")) continue;
      const p = path.join(MEETINGS_DIR, f);
      try {
        const m = JSON.parse(fs.readFileSync(p));
        if (m.endedAt == null) {
          m.endedAt = (m.lines && m.lines.length && m.lines[m.lines.length - 1].ts) || m.startedAt || Date.now();
          writePrivateJson(p, m);
        }
      } catch {}
    }
  } catch {}
}
function listMeetings() {
  const out = [];
  try {
    for (const f of fs.readdirSync(MEETINGS_DIR)) {
      if (!f.endsWith(".json")) continue;
      try { const s = meetingSummary(JSON.parse(fs.readFileSync(path.join(MEETINGS_DIR, f)))); s.live = false; out.push(s); } catch {}
    }
  } catch {}
  if (cur && cur.lines.length && !out.some(x => x.id === cur.id)) out.push(meetingSummary(cur));
  return out.sort((a, b) => b.startedAt - a.startedAt);
}
function readMeeting(id) {
  if (cur && cur.id === id) return cur;
  let m; try { m = JSON.parse(fs.readFileSync(path.join(MEETINGS_DIR, id + ".json"))); } catch { return null; }
  ensureLineIds(m);
  if (m && m.video && !fs.existsSync(path.join(REC_DIR, m.video))) delete m.video; // recording missing → no player
  return m;
}
function meetingForClient(m) { return { ...m, title: resolvedTitle(m) }; }
function persistMeeting(m) { if (cur && cur.id === m.id) cur = m; saveMeeting(m); }

// ---- LLM backend (local-first) -----------------------------------------------
// One OpenAI-compatible client serves every tier. The UI (Settings → AI Model) sends the chosen
// { base, key, model } per request; the LLM_* env vars are the fallback default.
//   Local = LiteRT-LM (Gemma 4 E2B) on 127.0.0.1:9379 — started ON DEMAND (first summary/ask), so it
//           is not loaded during the call. A post-call summary can overlap with Whisper if the user keeps
//           Whisper warm; that speed/RAM tradeoff is controlled by the idle-timeout setting.
//   API   = any OpenAI-compatible provider (Groq / OpenAI / Cerebras …) via a user-supplied key.
const LLM_LOCAL = "http://127.0.0.1:" + (process.env.LLM_PORT || "9379"); // on-device server origin
const LLM_IDLE_MS = 5 * 60 * 1000;
let llmProc = null, llmIdle = null; // the LiteRT-LM child WE started (null if already up / not ours)
const isLocal = (base) => base.startsWith(LLM_LOCAL);

function portOpen(host, port, ms = 800) {
  return new Promise((res) => {
    const s = net.connect(port, host);
    const done = (ok) => { s.destroy(); res(ok); };
    s.once("connect", () => done(true));
    s.once("error", () => done(false));
    s.setTimeout(ms, () => done(false));
  });
}
// Ensure the on-device LLM is listening; spawn `litert-lm serve` and wait for it if not.
async function ensureLocalLLM(base) {
  const u = new URL(base), port = +u.port || 80;
  if (await portOpen(u.hostname, port)) return; // already up (run.sh / user / a previous request)
  if (!llmProc) {
    const bin = process.env.LITERT_BIN || path.join(process.env.HOME || "", ".local/bin/litert-lm");
    if (!fs.existsSync(bin)) throw new Error("on-device model not set up — run ./setup.sh, or pick an API model in Settings");
    llmProc = spawn(bin, ["serve", "--host", u.hostname, "--port", String(port)], { stdio: "ignore" });
    llmProc.on("exit", () => { llmProc = null; });
  }
  for (let i = 0; i < 90; i++) { if (await portOpen(u.hostname, port)) return; await new Promise((r) => setTimeout(r, 1000)); }
  throw new Error("on-device LLM did not start");
}
function touchLLM() { // unload our LLM after idle so it doesn't hold ~1 GB between calls
  if (!llmProc) return;
  clearTimeout(llmIdle);
  llmIdle = setTimeout(() => { if (llmProc) { llmProc.kill(); llmProc = null; } }, LLM_IDLE_MS);
}

// ---- Whisper (transcription) — started on demand, unloaded when idle -----------------------------
// A packaged build no longer keeps whisper-server (~1.5 GB) permanently resident: node spawns it when a
// call starts and kills it after the user-selected idle timeout. In dev, run.sh already
// has it up on :8091 → node sees the open port and neither spawns nor kills it (not ours to manage).
const WHISPER_PORT = +(process.env.WHISPER_PORT || 8091);
// Primary STT engine: transcribe.cpp (handy-computer) held hot behind our transcribe-server — ~half the RAM
// and ~2x the speed of whisper-server on Apple Silicon, and cleaner output (proper punctuation, no dropped
// onsets). Falls back to whisper.cpp's whisper-server if transcribe-server isn't built on this machine.
const TS_BIN = path.join(__dirname, "..", "stt", "transcribe-server", "transcribe-server");
// Model files aren't bundled in the app. The shipped Rust service owns the authoritative lifecycle; this
// developer harness mirrors it so ./run.sh can exercise the same model-library UI and runtime routing.
const LEGACY_GGML_NAME = "ggml-large-v3-turbo-q8_0.bin";
const MODELS_DIR = process.env.ASSISTANT_MODELS_DIR || path.join(__dirname, "..", "stt", "whisper");
function modelDirs() {
  const d = [MODELS_DIR];
  if (process.env.ASSISTANT_REPO) d.push(path.join(process.env.ASSISTANT_REPO, "stt", "whisper"));
  d.push(path.join(__dirname, "..", "stt", "whisper"));
  return [...new Set(d)];
}
function findModel(name) { for (const d of modelDirs()) { const p = path.join(d, name); if (fs.existsSync(p)) return p; } return null; }
let activeSpeechModelId = speechModel(settingsBlob["set.speechModel"]) ? settingsBlob["set.speechModel"] : DEFAULT_MODEL_ID;
let modelDownload = null;
function activeSpeechModel() { return speechModel(activeSpeechModelId) || speechModel(DEFAULT_MODEL_ID); }
function modelPath(spec) { return findModel(spec.filename) || (spec.id === DEFAULT_MODEL_ID ? findModel(LEGACY_GGML_NAME) : null); }
function modelPresent() { return !!modelPath(activeSpeechModel()); }
function partialBytes(spec) { try { return Math.min(fs.statSync(path.join(MODELS_DIR, spec.filename + ".part")).size, spec.sizeBytes); } catch { return 0; } }
function speechModelStatus() {
  const active = activeSpeechModel();
  return {
    present: !!modelPath(active), downloading: !!modelDownload, downloadingModelId: modelDownload?.id || null,
    activeModelId: active.id, activeModelName: active.name,
    models: SPEECH_MODELS.map(spec => ({ ...spec, url: undefined, sha256: undefined, filename: undefined,
      downloaded: !!modelPath(spec), active: spec.id === active.id, downloading: modelDownload?.id === spec.id, partialBytes: partialBytes(spec) })),
  };
}
function selectSpeechModel(spec) {
  if (!modelPath(spec)) throw new Error("download this model first");
  if (gate.recording || gate.paused) throw new Error("stop the meeting before switching speech models");
  if (whisperIdle) { clearTimeout(whisperIdle); whisperIdle = null; }
  if (whisperProc) { whisperProc.kill(); whisperProc = null; }
  activeSpeechModelId = spec.id; settingsBlob["set.speechModel"] = spec.id; saveSettings();
  if (!spec.supportsTranslation) broadcast({ type: "state", state: { translate: false } });
}
async function hashFile(file) {
  const hash = crypto.createHash("sha256");
  await new Promise((resolve, reject) => { const input = fs.createReadStream(file); input.on("data", c => hash.update(c)); input.on("end", resolve); input.on("error", reject); });
  return hash.digest("hex");
}
async function startModelDownload(spec, selectAfter = false) {
  if (modelDownload) throw new Error("another model is downloading");
  if (modelPath(spec)) { if (selectAfter) selectSpeechModel(spec); return; }
  fs.mkdirSync(MODELS_DIR, { recursive: true, mode: 0o700 });
  const dest = path.join(MODELS_DIR, spec.filename), tmp = dest + ".part", controller = new AbortController();
  modelDownload = { id: spec.id, controller }; broadcast({ type: "model-progress", modelId: spec.id, pct: 0, bytes: partialBytes(spec), total: spec.sizeBytes });
  try {
    let existing = partialBytes(spec); if (existing >= spec.sizeBytes) existing = 0;
    const r = await fetch(spec.url, { headers: existing ? { Range: `bytes=${existing}-` } : {}, signal: controller.signal });
    if ((!r.ok && r.status !== 206) || !r.body) throw new Error("HTTP " + r.status);
    const append = existing > 0 && r.status === 206; if (!append) existing = 0;
    let got = existing, lastPct = -1; const out = fs.createWriteStream(tmp, { flags: append ? "a" : "w", mode: 0o600 });
    const stream = require("stream").Readable.fromWeb(r.body);
    stream.on("data", c => { got += c.length; const pct = Math.min(99, Math.floor(got * 100 / spec.sizeBytes)); if (pct !== lastPct) { lastPct = pct; broadcast({ type: "model-progress", modelId: spec.id, pct, bytes: got, total: spec.sizeBytes }); } });
    await new Promise((resolve, reject) => { stream.pipe(out); out.on("finish", resolve); out.on("error", reject); stream.on("error", reject); });
    const size = fs.statSync(tmp).size, digest = await hashFile(tmp);
    if (size !== spec.sizeBytes || digest !== spec.sha256) { try { fs.unlinkSync(tmp); } catch {} throw new Error("downloaded model failed size/SHA-256 verification"); }
    fs.chmodSync(tmp, 0o600); fs.renameSync(tmp, dest);
    const selected = selectAfter && !gate.recording && !gate.paused; if (selected) selectSpeechModel(spec);
    broadcast({ type: "model-progress", modelId: spec.id, pct: 100, bytes: spec.sizeBytes, total: spec.sizeBytes, done: true, selected });
  } catch (e) {
    const cancelled = e.name === "AbortError"; broadcast({ type: "model-progress", modelId: spec.id, error: cancelled ? "download cancelled" : String(e.message || e), cancelled });
    if (!cancelled) console.error("model: download failed:", e.message || e);
  } finally { modelDownload = null; }
}
let whisperProc = null, whisperIdle = null; // the STT-server child WE started (null if not ours)
async function ensureWhisper() {
  if (whisperIdle) { clearTimeout(whisperIdle); whisperIdle = null; }
  if (whisperProc) return;                                        // ours, already up
  if (await portOpen("127.0.0.1", WHISPER_PORT)) return;          // someone else's (dev) — leave it
  let bin, args;
  const spec = activeSpeechModel(), tsModel = modelPath(spec), ggml = spec.id === DEFAULT_MODEL_ID ? findModel(LEGACY_GGML_NAME) : null;
  if (fs.existsSync(TS_BIN) && tsModel) {                          // primary: transcribe.cpp
    bin = TS_BIN;
    args = ["-m", tsModel, "--host", "127.0.0.1", "--port", String(WHISPER_PORT)];
  } else {                                                        // fallback: whisper.cpp — no Silero VAD (it
    const ws = findBin("whisper-server");                         // degraded clean speech: dropped onsets, lost
    if (ws === "whisper-server" || !ggml) return;                 // punctuation). Whole-utterance capture
    bin = ws;                                                     // + the mic energy-gate + conf gate handle noise.
    args = ["-m", ggml, "--host", "127.0.0.1", "--port", String(WHISPER_PORT), "-t", "4", "--suppress-nst"];
  }
  whisperProc = spawn(bin, args, { stdio: "ignore" });
  whisperProc.on("exit", () => { whisperProc = null; });
  console.error(`STT: starting ${path.basename(bin)} (call detected)…`);
}
function unloadWhisperSoon() { // only unloads the one WE started
  if (!whisperProc || whisperIdle || !unloadIdle) return; // setting off → keep the model hot between calls
  const idleMs = whisperIdleMinutes * 60 * 1000;
  whisperIdle = setTimeout(() => { if (whisperProc) { whisperProc.kill(); whisperProc = null; console.error("whisper-server: unloaded (idle)"); } }, idleMs);
}
const killKids = () => { msShutdown = true; if (modelDownload) modelDownload.controller.abort(); if (llmProc) llmProc.kill(); if (msProc) msProc.kill(); if (whisperProc) whisperProc.kill(); };
["SIGINT", "SIGTERM"].forEach((sig) => process.on(sig, () => { killKids(); process.exit(0); }));
process.on("exit", killKids);

async function chat(messages, opts = {}) {
  const llm = opts.llm || {};
  const rawBase = Object.prototype.hasOwnProperty.call(llm, "base") ? llm.base : (process.env.LLM_BASE || LLM_LOCAL + "/v1");
  if (!rawBase) throw new Error("AI provider endpoint is missing. Choose a provider in Settings → Meetings.");
  const base = String(rawBase).replace(/\/$/, "");
  let parsed;
  try { parsed = new URL(base); } catch { throw new Error("AI provider endpoint is invalid"); }
  const loopback = parsed.hostname === "127.0.0.1" || parsed.hostname === "localhost";
  if (parsed.username || parsed.password || (parsed.protocol !== "https:" && !(loopback && parsed.protocol === "http:"))) {
    throw new Error("AI provider endpoints must use HTTPS or a loopback HTTP address");
  }
  const key = llm.key || process.env.LLM_KEY || "";
  const model = llm.model || process.env.LLM_MODEL || "local";
  if (isLocal(base)) await ensureLocalLLM(base);
  try {
    const r = await fetch(base + "/chat/completions", {
      method: "POST",
      headers: { "Content-Type": "application/json", ...(key ? { Authorization: "Bearer " + key } : {}) },
      body: JSON.stringify({ model, messages, temperature: opts.temperature ?? 0.3, max_tokens: opts.maxTokens ?? 700, stream: false }),
    });
    if (!r.ok) throw new Error(`${r.status} ${(await r.text()).slice(0, 160)}`);
    const j = await r.json();
    return (j.choices?.[0]?.message?.content || "").trim();
  } finally {
    if (isLocal(base)) touchLLM();
  }
}

function storedSetting(key, fallback) {
  try {
    const raw = settingsBlob["set." + key];
    return raw == null ? fallback : JSON.parse(raw);
  } catch { return fallback; }
}
function storedLLM() {
  const c = storedSetting("aiModel", { provider: "local", key: "", model: "", base: "" }) || {};
  const providers = {
    local: { base: LLM_LOCAL + "/v1", model: "gemma4-e2b", needsKey: false },
    groq: { base: "https://api.groq.com/openai/v1", model: "llama-3.3-70b-versatile", needsKey: true },
    openai: { base: "https://api.openai.com/v1", model: "gpt-4o-mini", needsKey: true },
    cerebras: { base: "https://api.cerebras.ai/v1", model: "llama-3.3-70b", needsKey: true },
    custom: { base: c.base || "", model: c.model || "", needsKey: true },
  };
  const p = providers[c.provider] || providers.local;
  return { base: c.provider === "custom" ? (c.base || "") : p.base, key: p.needsKey ? (devProviderKeys[c.provider] || process.env.LLM_KEY || "") : "", model: c.model || p.model };
}
function storedTemplatePrompt() {
  const selected = storedSetting("template_sel", "standard");
  const templates = storedSetting("templates", []);
  const template = Array.isArray(templates) && templates.find(t => t && t.id === selected);
  return String(template?.prompt || "");
}
async function generateSummaryForMeeting(id, opts = {}) {
  const meeting = readMeeting(id);
  if (!meeting) throw new Error("meeting not found");
  const eligible = canGenerateNotes(meeting);
  if (!eligible.ok) {
    meeting.summaryState = "skipped"; meeting.summaryError = eligible.reason; persistMeeting(meeting);
    throw new Error(eligible.reason);
  }
  const revision = Number(meeting.transcriptRevision) || 0;
  meeting.summaryState = "generating"; meeting.summaryError = ""; persistMeeting(meeting);
  const deeper = opts.deeper ?? !!storedSetting("deeper", false);
  const template = String(opts.template ?? storedTemplatePrompt());
  const instruction = template.trim() || `Summarise this meeting as 3–5 short bullet points, then a line "Action items:" with each as "- <owner>: <task>" (or "None").`;
  const sys = "You summarise meeting transcripts for a busy professional. Be concise, specific, neutral — no preamble."
    + (deeper ? " Reason carefully through the discussion — capture decisions, owners, and nuance you might otherwise miss." : "");
  const text = await chat([
    { role: "system", content: sys },
    { role: "user", content: `${instruction}\n\n${meeting.notes ? `The note-taker's own scratchpad from the call (weight these points):\n${meeting.notes}\n\n` : ""}Transcript:\n\n${transcriptText(meeting)}` },
  ], { maxTokens: deeper ? 1100 : 650, temperature: deeper ? 0.2 : 0.3, llm: opts.llm || storedLLM() });
  const latest = readMeeting(id);
  if (!latest) throw new Error("meeting disappeared while notes were generating");
  if ((Number(latest.transcriptRevision) || 0) !== revision) {
    latest.summaryState = "stale"; latest.summaryStale = true;
    latest.summaryError = "The transcript changed while notes were generating. Regenerate notes.";
    persistMeeting(latest);
    broadcast({ type: "meeting-updated", id, summaryState: "stale" });
    throw new Error(latest.summaryError);
  }
  latest.summary = text;
  latest.summaryRevision = revision;
  latest.summaryStale = false;
  latest.summaryState = "ready";
  latest.summaryError = "";
  persistMeeting(latest);
  broadcast({ type: "meeting-updated", id, summaryState: "ready" });
  return latest;
}
function queueAutomaticSummary(id) {
  const meeting = readMeeting(id), eligible = meeting && canGenerateNotes(meeting);
  if (!meeting || !eligible.ok) return;
  setTimeout(() => {
    generateSummaryForMeeting(id).catch((e) => {
      const latest = readMeeting(id);
      if (!latest || latest.summaryState === "stale") return;
      latest.summaryState = "failed";
      latest.summaryError = String(e.message || e).slice(0, 240) || "Notes could not be generated. Try again.";
      persistMeeting(latest); // transcript remains intact; the UI presents a retry
      broadcast({ type: "meeting-updated", id, summaryState: "failed" });
    });
  }, 250);
}
function transcriptText(m, cap = 16000) {
  return correctedTranscriptText(m, cap);
}
function liveAssistSpec(action) {
  return {
    catch_up: ["In at most four bullets, explain what happened in the last minute and what matters right now.", "last_minute"],
    summary: ["Summarize the meeting so far in at most five concrete bullets. Distinguish discussion from decisions.", "meeting_so_far"],
    actions: ["List only action items that were explicitly stated. Include an owner only when the transcript names one. If none are explicit, say so.", "meeting_so_far"],
    follow_ups: ["Suggest up to three concise follow-up questions the operator could ask next, grounded in unresolved points in the transcript.", "meeting_so_far"],
  }[action];
}
function liveAssistExcerpt(meeting, action, now = Date.now()) {
  const cutoff = action === "catch_up" ? now - 60_000 : 0;
  const cap = action === "catch_up" ? 6000 : 14000;
  const selected = []; let chars = 0, lines = meeting.lines || [];
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i], text = String(line.text || "").trim().slice(0, 4000);
    if (line.excluded || !text || (cutoff && Number(line.ts) < cutoff)) continue;
    const value = `${String(line.speaker || "Unknown speaker").slice(0, 120)}: ${text}`;
    selected.push(value); chars += Array.from(value).length + 1;
    if (chars >= cap) break;
  }
  return Array.from(selected.reverse().join("\n")).slice(-cap).join("");
}
function readBody(req) {
  return new Promise(resolve => { let b = ""; req.on("data", c => (b += c)); req.on("end", () => resolve(b)); });
}
function readBodyBuffer(req) { // binary-safe (multipart WAV must not be stringified)
  return new Promise(resolve => { const cs = []; req.on("data", c => cs.push(c)); req.on("end", () => resolve(Buffer.concat(cs))); });
}
async function waitPort(host, port, ms) { const t = Date.now() + ms; while (Date.now() < t) { if (await portOpen(host, port)) return true; await new Promise(r => setTimeout(r, 300)); } return false; }

http.createServer((req, res) => {
  // Loopback-only: this server holds transcripts, notes, and your API key. Reject any non-localhost Host —
  // defence-in-depth against LAN peers and DNS-rebinding, on top of the 127.0.0.1 bind below.
  const host = (req.headers.host || "").split(":")[0];
  if (host && host !== "127.0.0.1" && host !== "localhost") { res.writeHead(403); return res.end(); }
  const requestUrl = new URL(req.url, `http://127.0.0.1:${PORT}`);
  if (req.method === "GET" && requestUrl.pathname === "/" && requestUrl.searchParams.get("bootstrap") === GUI_TOKEN) {
    res.writeHead(302, {
      Location: "/",
      "Set-Cookie": `assistant_session=${GUI_TOKEN}; HttpOnly; SameSite=Strict; Path=/`,
      "Cache-Control": "no-store",
      "Referrer-Policy": "no-referrer",
    });
    return res.end();
  }
  const bearer = req.headers.authorization === `Bearer ${GUI_TOKEN}`;
  const cookie = String(req.headers.cookie || "").split(";").some(v => v.trim() === `assistant_session=${GUI_TOKEN}`);
  const origin = String(req.headers.origin || "");
  const validOrigin = !origin || origin === `http://127.0.0.1:${PORT}` || origin === `http://localhost:${PORT}`;
  const fetchSite = String(req.headers["sec-fetch-site"] || "");
  if ((!bearer && !cookie) || !validOrigin || (fetchSite && fetchSite !== "same-origin" && fetchSite !== "none")) {
    res.writeHead(401, { "Content-Type": "application/json", "Cache-Control": "no-store" });
    return res.end('{"error":"unauthorized"}');
  }
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.setHeader("Referrer-Policy", "no-referrer");
  if (req.url === "/" || req.url === "/index.html") {
    // no-store: the WebView (WKWebView) otherwise heuristically caches the UI and serves a stale copy after
    // an edit — so "the code is updated but the app behaves like the old version". Always fetch fresh.
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" });
    return res.end(fs.readFileSync(path.join(__dirname, "index.html")));
  }
  if (req.url === "/logo.png") { // brand mark (sidebar, Pro card, recording chip)
    res.writeHead(200, { "Content-Type": "image/png", "Cache-Control": "max-age=86400" });
    return res.end(fs.readFileSync(path.join(__dirname, "logo.png")));
  }
  if (req.url === "/favicon.ico") { res.writeHead(204, { "Cache-Control": "max-age=86400" }); return res.end(); }
  if (req.url === "/api/release" && req.method === "GET") {
    res.writeHead(200, { "Content-Type": "application/json", "Cache-Control": "no-store" });
    return res.end(JSON.stringify(RELEASE));
  }
  if (req.url === "/ingest" && req.method === "POST") {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      try {
        const obj = JSON.parse(body);
        ingest(obj);
      } catch {}
      res.writeHead(204); res.end();
    });
    return;
  }
  if (req.url === "/events") {
    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    });
    res.write(": ok\n\n");
    if (Object.keys(lastState).length) res.write(`data: ${JSON.stringify({ type: "state", state: lastState })}\n\n`);
    clients.add(res);
    req.on("close", () => clients.delete(res));
    return;
  }
  if (req.url === "/api/state" && req.method === "GET") { // meeting-tap polls this to gate capture + video mode + manual override
    res.writeHead(200, { "Content-Type": "application/json" });
    return res.end(JSON.stringify({ ...lastState, manual: gate.manual || "auto", dictation: dictationCfg, modules, dictStop: dictStopSeq }));
  }
  if (req.url === "/api/meeting/observation" && req.method === "POST") {
    readBody(req).then(b => {
      try {
        const input = JSON.parse(b || "{}");
        const identity = String(input.identity || "").trim().slice(0, 240);
        let observed = null;
        if (identity) {
          if (!/^[A-Za-z0-9.:_|-]+$/.test(identity)) throw new Error("meeting observation identity contains unsupported characters");
          const family = String(input.family || "");
          const app = String(input.app || "").trim().slice(0, 80);
          const bundle = String(input.bundle || "").trim().slice(0, 180);
          if (!["desktop", "browser"].includes(family) || !app || !bundle) throw new Error("meeting observation is invalid");
          observed = { identity, family, app, bundle, title: String(input.title || "").trim().slice(0, 160) };
        }
        const decision = meetingDetector.update(observed, {
          enabled: !!modules.meetings && gate.autoDetect,
          desktop: lastState.desktopMeet !== false,
          browser: lastState.browserMeet !== false,
        });
        if (decision.active
            && (lastState.app !== decision.active.app || lastState.meetingTitle !== decision.active.title)) {
          broadcast({ type: "state", source: "meeting-observation", state: {
            app: decision.active.app,
            meetingTitle: decision.active.title,
            title: decision.active.title,
          } });
        }
        setDetected(!!decision.active);
        res.writeHead(200, { "Content-Type": "application/json", "Cache-Control": "no-store" });
        res.end(JSON.stringify({
          active: !!decision.active,
          activeIdentity: decision.active?.identity || null,
          app: decision.active?.app || null,
          title: decision.active?.title || null,
          bundle: decision.active?.bundle || null,
          transition: decision.transition,
        }));
      } catch (error) {
        res.writeHead(400, { "Content-Type": "application/json", "Cache-Control": "no-store" });
        res.end(JSON.stringify({ error: String(error.message || error).slice(0, 240) }));
      }
    });
    return;
  }
  if (req.url === "/api/meeting/control" && req.method === "POST") { // UI start/pause/resume/stop (composes with auto-detect)
    readBody(req).then(b => {
      let action = ""; try { action = String(JSON.parse(b).action || "").toLowerCase(); } catch {}
      const ok = setManual(action);
      res.writeHead(ok ? 200 : 400, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ manual: gate.manual || "auto", inMeeting: gate.recording || gate.paused, paused: gate.paused }));
    });
    return;
  }
  if (req.url === "/api/meeting/notes" && req.method === "POST") { // the in-call scratchpad → saved on the current meeting
    readBody(req).then(b => {
      let notes = ""; try { notes = String(JSON.parse(b).notes || ""); } catch {}
      if (cur) { cur.notes = notes.slice(0, 8000); saveMeeting(cur); }
      res.writeHead(cur ? 204 : 409); res.end(); // 409 = no live meeting to attach to
    });
    return;
  }
  if (req.url === "/api/meeting/live-assist" && req.method === "POST") {
    if (!cur) { res.writeHead(409, { "Content-Type": "application/json" }); res.end(JSON.stringify({ error: "No meeting is being recorded" })); return; }
    readBody(req).then(async b => {
      let action = ""; try { action = String(JSON.parse(b).action || "").slice(0, 40); } catch {}
      const spec = liveAssistSpec(action);
      if (!spec) { const error = new Error("unknown live assist action"); error.status = 400; throw error; }
      const excerpt = liveAssistExcerpt(cur, action);
      if (excerpt.trim().split(/\s+/).length < 4) { const error = new Error("Not enough live transcript yet"); error.status = 422; throw error; }
      const scratchpad = cur.notes ? `\n\nOperator scratchpad:\n${String(cur.notes).slice(0, 2400)}` : "";
      const answer = await chat([
        { role: "system", content: "You are a live meeting assistant. Use ONLY the supplied transcript and scratchpad. Be concise and directly useful. Never invent facts, decisions, owners, commitments, or participant intent. If the evidence is insufficient, say so." },
        { role: "user", content: `${spec[0]}\n\nLive transcript:\n${excerpt}${scratchpad}` },
      ], { maxTokens: 320, temperature: 0.2, llm: storedLLM() });
      return { action, scope: spec[1], answer };
    }).then(result => {
      res.writeHead(200, { "Content-Type": "application/json", "Cache-Control": "no-store" });
      res.end(JSON.stringify(result));
    }).catch(error => {
      res.writeHead(error.status || 502, { "Content-Type": "application/json", "Cache-Control": "no-store" });
      res.end(JSON.stringify({ error: String(error.message || error).slice(0, 240) }));
    });
    return;
  }
  if (req.url === "/api/settings" && req.method === "GET") { // durable mirror of the UI's localStorage (WKWebView drops it for the http origin)
    res.writeHead(200, { "Content-Type": "application/json" });
    return res.end(JSON.stringify(settingsBlob));
  }
  if (req.url === "/api/settings" && req.method === "POST") { // the UI mirrors every setting write here so it survives relaunch
    readBody(req).then(b => {
      try {
        const j = JSON.parse(b);
        const patch = j && j.set && typeof j.set === "object" ? j.set : null;
        if (patch) {
          for (const k in patch) {
            if (patch[k] == null) delete settingsBlob[k];
            else settingsBlob[k] = k === "set.aiModel" ? sanitizeAiModel(String(patch[k])) : String(patch[k]);
          }
          saveSettings();
          try { const m = patch["set.modules"] != null && JSON.parse(patch["set.modules"]); if (m && typeof m === "object") { modules = { dictation: m.dictation !== false, meetings: m.meetings !== false, writing: RELEASE.capabilities.writingAssistant ? !!m.writing : false }; dictationCfg.enabled = !!modules.dictation; } } catch {}
        }
      } catch {}
      res.writeHead(204); res.end();
    });
    return;
  }
  if (req.url === "/api/ai/key" && req.method === "POST") { // dev harness parity: memory only, never Keychain or disk
    readBody(req).then(body => {
      try {
        const input = JSON.parse(body || "{}"), provider = String(input.provider || ""), key = String(input.key || "").trim();
        if (!["openai", "groq", "cerebras", "custom"].includes(provider)) throw providerError("Unknown API provider");
        if (key.length < 8 || key.length > 4096 || /[\r\n\0]/.test(key)) throw providerError("The API key format is invalid");
        devProviderKeys[provider] = key;
        res.writeHead(200, { "Content-Type": "application/json", "Cache-Control": "no-store" });
        res.end(JSON.stringify({ saved: true, storage: "memory" }));
      } catch (error) {
        res.writeHead(error.status || 400, { "Content-Type": "application/json", "Cache-Control": "no-store" });
        res.end(JSON.stringify({ error: String(error.message || error) }));
      }
    });
    return;
  }
  if (req.url === "/api/ai/models" && req.method === "POST") { // dev harness parity for provider onboarding
    readBody(req).then(async body => {
      try {
        const input = JSON.parse(body || "{}"), provider = String(input.provider || "");
        const models = await fetchProviderModels(provider, String(input.base || ""), devProviderKeys[provider]);
        res.writeHead(200, { "Content-Type": "application/json", "Cache-Control": "no-store" });
        res.end(JSON.stringify({ provider, models }));
      } catch (error) {
        res.writeHead(error.status || 502, { "Content-Type": "application/json", "Cache-Control": "no-store" });
        res.end(JSON.stringify({ error: String(error.message || error) }));
      }
    });
    return;
  }
  if (req.url === "/api/config" && req.method === "POST") { // UI sets the screen-recording mode (off/always/share)
    readBody(req).then(b => {
      try {
        const j = JSON.parse(b);
        if (typeof j.videoMode === "string") broadcast({ type: "state", state: { videoMode: j.videoMode } });
        if (typeof j.translate === "boolean") broadcast({ type: "state", state: { translate: !!j.translate && !!activeSpeechModel().supportsTranslation && !!RELEASE.capabilities.translation } });
        if (typeof j.language === "string") broadcast({ type: "state", state: { language: j.language } });
        if (typeof j.desktopMeet === "boolean") broadcast({ type: "state", state: { desktopMeet: j.desktopMeet } });
        if (typeof j.browserMeet === "boolean") broadcast({ type: "state", state: { browserMeet: j.browserMeet } });
        if (typeof j.autoDetect === "boolean") {
          gate.setAutoDetect(j.autoDetect);
          if (!j.autoDetect) { meetingDetector.reset(); setDetected(false); }
          broadcast({ type: "state", state: { autoDetect: j.autoDetect } }); recomputeLifecycle(); broadcastCall();
        } // off → ignore raw observations (manual only)
        if (typeof j.unloadIdle === "boolean") unloadIdle = j.unloadIdle; // off → keep whisper hot
        if (Number.isFinite(Number(j.whisperIdleMinutes))) {
          const requested = Number(j.whisperIdleMinutes);
          // Keep the product choices bounded: immediate unload through one hour. A changed setting
          // applies to the next idle period; it never interrupts an active transcription.
          whisperIdleMinutes = Math.min(60, Math.max(0, requested));
        }
        if (typeof j.vocab === "string") { vocabPrompt = j.vocab.slice(0, 800); broadcast({ type: "state", state: { vocab: vocabPrompt } }); } // custom vocabulary → meeting-tap
        if (j.dictation && typeof j.dictation === "object") { dictationCfg = { ...dictationCfg, ...j.dictation }; broadcast({ type: "state", state: { dictation: dictationCfg } }); } // Handy-style dictation config → meeting-tap
        if (j.modules && typeof j.modules === "object") { // feature opt-in → gates every permission-touching capability
            modules = { ...modules, ...j.modules, writing: RELEASE.capabilities.writingAssistant ? !!j.modules.writing : false };
          if (!modules.meetings) { meetingDetector.reset(); setDetected(false); }
          dictationCfg.enabled = !!modules.dictation;    // the dictation tap follows the module
          settingsBlob["set.modules"] = JSON.stringify(modules); saveSettings(); // persist so the boot-time restore (above) survives a relaunch — was RAM-only, so the server booted all-off until the UI re-pushed
          broadcast({ type: "state", state: { modules, dictation: dictationCfg } });
        }
      } catch {}
      res.writeHead(204); res.end();
    });
    return;
  }
  if (req.url === "/api/transcribe" && req.method === "POST") { // dictation: WAV → local STT, ensuring the engine is up first
    const ct = req.headers["content-type"] || "application/octet-stream";
    readBodyBuffer(req).then(async (buf) => {
      try {
        await ensureWhisper();
        if (!await waitPort("127.0.0.1", WHISPER_PORT, 15000)) throw new Error("transcription engine did not start");
        // Preserve Automatic language detection. Forcing the English token turns Hindi/Hinglish into a
        // phonetic English guess and measured far worse than Automatic on the pinned mixed-speech corpus.
        const spec = activeSpeechModel(), requested = dictationCfg.language || "auto";
        const lang = spec.fixedLanguage || (spec.supportedLanguages.includes(requested) ? requested : (spec.supportsLanguageDetection ? "auto" : (spec.supportedLanguages[0] || "en")));
        const reqBody = injectMultipartFields(buf, ct, {
          language: lang,
          translate: "false",
          prompt: vocabPrompt ? `Vocabulary: ${vocabPrompt}` : "",
        });
        const r = await fetch(`http://127.0.0.1:${WHISPER_PORT}/inference`, { method: "POST", headers: { "Content-Type": ct, Authorization: `Bearer ${process.env.ASSISTANT_STT_TOKEN || GUI_TOKEN}` }, body: reqBody });
        const body = await r.text();
        if (!r.ok) throw new Error(`${r.status} ${body.slice(0, 160)}`);
        let text = ""; try { text = String(JSON.parse(body).text || "").trim(); } catch {}
        // "Punctuation behaviour" = Off → strip the punctuation Whisper adds; Automatic leaves it untouched.
        if (dictationCfg.punctuation === "off" && text) text = text.replace(/[.,!?;:…]+/g, "").replace(/\s{2,}/g, " ").trim();
        res.writeHead(200, { "Content-Type": "application/json" }); res.end(JSON.stringify({ text }));
      } catch (e) {
        res.writeHead(502, { "Content-Type": "application/json" }); res.end(JSON.stringify({ error: String(e.message || e) }));
      } finally { if (!cur) unloadWhisperSoon(); } // free the model after dictation, unless a meeting is live
    });
    return;
  }
  if (req.url === "/api/dictations" && req.method === "GET") { // recent dictation results for the Dictation view
    res.writeHead(200, { "Content-Type": "application/json" }); return res.end(JSON.stringify(recentDictations));
  }
  if (req.url === "/api/dictation/stop" && req.method === "POST") { // pill/tray Stop → meeting-tap finishes (transcribes) the current dictation
    dictStopSeq++; res.writeHead(204); return res.end();
  }
  if (req.url === "/api/agent/context" && req.method === "POST") {
    if (!RELEASE.capabilities.agentContextInspector || !fs.existsSync(AGENT_CONTEXT)) {
      res.writeHead(503, { "Content-Type": "application/json" });
      return res.end('{"ok":false,"error":"The native screen-context helper is unavailable."}');
    }
    execFile(AGENT_CONTEXT, ["--once"], { timeout: 1500, maxBuffer: 256 * 1024 }, (error, stdout, stderr) => {
      if (error) {
        res.writeHead(503, { "Content-Type": "application/json" });
        return res.end(JSON.stringify({ ok: false, error: String(stderr || error.message || error).trim().slice(0, 240) }));
      }
      try {
        const snapshot = JSON.parse(stdout);
        res.writeHead(200, { "Content-Type": "application/json", "Cache-Control": "no-store" });
        res.end(JSON.stringify(snapshot));
      } catch {
        res.writeHead(503, { "Content-Type": "application/json" });
        res.end('{"ok":false,"error":"Screen context returned invalid JSON."}');
      }
    });
    return;
  }
  if (req.url === "/api/model/status" && req.method === "GET") { // first-run: is the on-device model here yet?
    res.writeHead(200, { "Content-Type": "application/json" });
    return res.end(JSON.stringify(speechModelStatus()));
  }
  if (req.url === "/api/model/download" && req.method === "POST") {
    readBody(req).then(body => {
      let input = {}; try { input = JSON.parse(body || "{}"); } catch {}
      const spec = speechModel(input.id || activeSpeechModelId); if (!spec) { res.writeHead(400, { "Content-Type": "application/json" }); return res.end('{"error":"unknown speech model"}'); }
      startModelDownload(spec, !!input.select).catch(error => broadcast({ type: "model-progress", modelId: spec.id, error: String(error.message || error) }));
      res.writeHead(202, { "Content-Type": "application/json" }); return res.end(JSON.stringify(speechModelStatus()));
    }); return;
  }
  if (req.url === "/api/model/select" && req.method === "POST") {
    readBody(req).then(body => { try { const spec = speechModel(JSON.parse(body || "{}").id); if (!spec) throw new Error("unknown speech model"); selectSpeechModel(spec); res.writeHead(200, { "Content-Type": "application/json" }); res.end(JSON.stringify(speechModelStatus())); }
      catch (error) { res.writeHead(409, { "Content-Type": "application/json" }); res.end(JSON.stringify({ error: String(error.message || error) })); } }); return;
  }
  if (req.url === "/api/model/delete" && req.method === "POST") {
    readBody(req).then(body => { try { const spec = speechModel(JSON.parse(body || "{}").id); if (!spec) throw new Error("unknown speech model"); if (modelDownload?.id === spec.id) throw new Error("cancel the download before deleting it");
        if (spec.id === activeSpeechModelId && (gate.recording || gate.paused)) throw new Error("stop the meeting before removing the active speech model");
        if (spec.id === activeSpeechModelId && whisperProc) { whisperProc.kill(); whisperProc = null; }
        for (const file of [path.join(MODELS_DIR, spec.filename), path.join(MODELS_DIR, spec.filename + ".part")]) { try { fs.unlinkSync(file); } catch {} }
        if (spec.id === activeSpeechModelId) { const fallback = SPEECH_MODELS.find(candidate => modelPath(candidate)) || speechModel(DEFAULT_MODEL_ID); activeSpeechModelId = fallback.id; settingsBlob["set.speechModel"] = fallback.id; saveSettings(); }
        res.writeHead(200, { "Content-Type": "application/json" }); res.end(JSON.stringify(speechModelStatus()));
      } catch (error) { res.writeHead(409, { "Content-Type": "application/json" }); res.end(JSON.stringify({ error: String(error.message || error) })); } }); return;
  }
  if (req.url === "/api/model/cancel" && req.method === "POST") {
    readBody(req).then(body => { let id = ""; try { id = JSON.parse(body || "{}").id || ""; } catch {}
      if (modelDownload?.id === id) { modelDownload.controller.abort(); res.writeHead(202, { "Content-Type": "application/json" }); return res.end(JSON.stringify({ cancelling: true, id })); }
      res.writeHead(409, { "Content-Type": "application/json" }); return res.end('{"error":"that model is not downloading"}'); }); return;
  }
  if (req.url === "/api/open-settings" && req.method === "POST") { // deep-link a macOS Privacy pane (permission checklist)
    readBody(req).then(b => {
      let pane = ""; try { pane = String(JSON.parse(b).pane || ""); } catch {}
      const PANES = { Privacy_Microphone: 1, Privacy_AudioCapture: 1, Privacy_ScreenCapture: 1, Privacy_Accessibility: 1 }; // whitelist
      if (PANES[pane]) execFile("open", ["x-apple.systempreferences:com.apple.preference.security?" + pane]);
      res.writeHead(PANES[pane] ? 204 : 400); res.end();
    });
    return;
  }
  const rec = req.url.match(/^\/api\/rec\/([\w.\-]+)$/); // serve a saved screen recording (with range for seeking)
  if (rec && req.method === "GET") {
    const file = path.join(REC_DIR, rec[1]);
    if (!file.startsWith(REC_DIR + path.sep) || !fs.existsSync(file)) { res.writeHead(404); return res.end(); }
    const size = fs.statSync(file).size;
    const range = req.headers.range && /bytes=(\d+)-(\d*)/.exec(req.headers.range);
    if (range) {
      const start = parseInt(range[1], 10), end = range[2] ? parseInt(range[2], 10) : size - 1;
      res.writeHead(206, { "Content-Type": "video/mp4", "Accept-Ranges": "bytes", "Content-Range": `bytes ${start}-${end}/${size}`, "Content-Length": end - start + 1 });
      return fs.createReadStream(file, { start, end }).pipe(res);
    }
    res.writeHead(200, { "Content-Type": "video/mp4", "Accept-Ranges": "bytes", "Content-Length": size });
    return fs.createReadStream(file).pipe(res);
  }
  if (req.url === "/api/meetings" && req.method === "GET") {
    res.writeHead(200, { "Content-Type": "application/json" });
    return res.end(JSON.stringify(listMeetings()));
  }
  const mm = req.url.match(/^\/api\/meetings\/([\w-]+)$/);
  if (mm && req.method === "GET") {
    const one = readMeeting(mm[1]);
    if (!one) { res.writeHead(404); return res.end(); }
    res.writeHead(200, { "Content-Type": "application/json" });
    return res.end(JSON.stringify(meetingForClient(one)));
  }
  const mtitle = req.url.match(/^\/api\/meetings\/([\w-]+)\/title$/); // rename a meeting
  if (mtitle && req.method === "POST") {
    readBody(req).then(b => {
      let t; try { t = String(JSON.parse(b).title || "").trim().slice(0, 120); } catch {}
      if (!t) { res.writeHead(400); return res.end(); }
      if (cur && cur.id === mtitle[1]) { cur.title = t; cur.titleSource = "user"; saveMeeting(cur); }
      else {
        const p = path.join(MEETINGS_DIR, mtitle[1] + ".json");
        if (!fs.existsSync(p)) { res.writeHead(404); return res.end(); }
        try { const m = JSON.parse(fs.readFileSync(p)); m.title = t; m.titleSource = "user"; writePrivateJson(p, m); } catch { res.writeHead(500); return res.end(); }
      }
      res.writeHead(200, { "Content-Type": "application/json" }); res.end(JSON.stringify({ title: t }));
    });
    return;
  }
  const mcorr = req.url.match(/^\/api\/meetings\/([\w-]+)\/corrections$/);
  if (mcorr && req.method === "POST") {
    readBody(req).then(b => {
      const meeting = readMeeting(mcorr[1]);
      if (!meeting) { res.writeHead(404); return res.end(); }
      try {
        applyCorrection(meeting, JSON.parse(b));
        persistMeeting(meeting);
        broadcast({ type: "meeting-updated", id: meeting.id, summaryState: meeting.summaryState || "stale" });
        res.writeHead(200, { "Content-Type": "application/json" });
        res.end(JSON.stringify(meetingForClient(meeting)));
      } catch (e) {
        res.writeHead(400, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ error: String(e.message || e) }));
      }
    });
    return;
  }
  const mthumb = req.url.match(/^\/api\/meetings\/([\w-]+)\/thumb$/); // recording frame, then platform tile
  if (mthumb && req.method === "GET") {
    const m = readMeeting(mthumb[1]);
    if (!m) { res.writeHead(404); return res.end(); }
    const fallback = () => {
      res.writeHead(200, { "Content-Type": "image/svg+xml; charset=utf-8", "Cache-Control": "private, max-age=86400" });
      res.end(appThumbnailSvg(m.app));
    };
    if (!m.video) return fallback();
    const video = path.join(REC_DIR, m.video), thumb = video.replace(/\.\w+$/, ".jpg");
    const send = () => { res.writeHead(200, { "Content-Type": "image/jpeg", "Cache-Control": "max-age=86400" }); fs.createReadStream(thumb).pipe(res); };
    if (fs.existsSync(thumb)) return send();
    if (!fs.existsSync(video)) return fallback();
    execFile(FFMPEG, ["-y", "-ss", "1", "-i", video, "-frames:v", "1", "-vf", "scale=480:-1", thumb], () => {
      if (fs.existsSync(thumb)) send(); else fallback();
    });
    return;
  }
  const msum = req.url.match(/^\/api\/meetings\/([\w-]+)\/summary$/);
  if (msum && req.method === "POST") {
    readBody(req).then(b => {
      let opts = {}; try { const j = JSON.parse(b); opts = { deeper: !!j.deeper, template: String(j.template || "") }; } catch {}
      return generateSummaryForMeeting(msum[1], opts);
    }).then(meeting => {
      res.writeHead(200, { "Content-Type": "application/json" }); res.end(JSON.stringify({ summary: meeting.summary, summaryRevision: meeting.summaryRevision }));
    }).catch(e => { res.writeHead(502, { "Content-Type": "application/json" }); res.end(JSON.stringify({ error: String(e.message || e) })); });
    return;
  }
  const mask = req.url.match(/^\/api\/meetings\/([\w-]+)\/ask$/);
  if (mask && req.method === "POST") {
    const one = readMeeting(mask[1]);
    if (!one) { res.writeHead(404); return res.end(); }
    readBody(req).then(b => {
      let q = ""; try { const j = JSON.parse(b); q = j.question || ""; } catch {}
      return chat([
        { role: "system", content: "Answer questions about this meeting using ONLY its transcript. If it isn't covered, say so briefly." },
        { role: "user", content: `Transcript:\n${transcriptText(one)}\n\nQuestion: ${q}` },
      ], { maxTokens: 500, llm: storedLLM() });
    }).then(text => { res.writeHead(200, { "Content-Type": "application/json" }); res.end(JSON.stringify({ answer: text })); })
      .catch(e => { res.writeHead(502, { "Content-Type": "application/json" }); res.end(JSON.stringify({ error: String(e.message || e) })); });
    return;
  }
  res.writeHead(404);
  res.end();
}).listen(PORT, "127.0.0.1", () => {
  console.log(`\n  Live transcript → http://localhost:${PORT}\n`);
  finalizeOrphans(); // meetings a prior (killed) server left open aren't live — close them
  // No hard-coded "inMeeting" — meeting-state (names) + meeting-tap (self-detect) drive "live", only in a real call.
  startMeetingState(); // state + active-speaker (names). Harmless if not built yet.
});

// LIVE: forward meeting-state's NDJSON (state + caption events) straight through.
function startMeetingState() {
  if (process.env.ASSISTANT_RUST_MEETING_STATE === "1") return; // packaged: the app spawns it directly (so it gets the AX grant)
  const bin = path.join(__dirname, "..", "meeting-state", "meeting-state");
  if (!fs.existsSync(bin)) {
    console.error(`! meeting-state not built. Run: (cd ../meeting-state && swiftc -O meeting-state.swift -o meeting-state)`);
    return;
  }
  const p = spawn(bin, [], { stdio: ["ignore", "pipe", "inherit"] });
  msProc = p;
  let buf = "";
  p.stdout.on("data", (d) => {
    buf += d;
    let nl;
    while ((nl = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (!line) continue;
      try { ingest(JSON.parse(line)); } catch {}
    }
  });
  p.on("exit", (c) => {
    console.error(`meeting-state exited (${c})`);
    if (!msShutdown) setTimeout(startMeetingState, 1000); // died mid-session — respawn so the app doesn't go blind to calls
  });
}
