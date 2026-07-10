#!/usr/bin/env node
// Live speaker-attributed transcript app.
//   node server.js  -> spawn ../meeting-state/meeting-state, stream its JSON to the GUI + relay captions.
// No dependencies. Open http://localhost:8787
const http = require("http");
const fs = require("fs");
const net = require("net");
const path = require("path");
const { spawn, execFile } = require("child_process");

const PORT = process.env.PORT || 8787; // respect preview's assigned port; 8787 for manual runs
const clients = new Set();
let lastState = {}; // so a client connecting mid-meeting gets current title/participants/speaker
let msProc = null, msShutdown = false; // meeting-state child + shutdown flag (respawn it if it dies mid-session)
const MEETINGS_DIR = path.join(__dirname, "..", "data", "meetings");
const REC_DIR = path.join(MEETINGS_DIR, "rec"); // screen recordings (.mp4), written by meeting-tap
// Resolve tool paths explicitly — a packaged (GUI-spawned) node has no shell PATH.
function findBin(name) { for (const d of ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]) { const p = path.join(d, name); if (fs.existsSync(p)) return p; } return name; }
const FFMPEG = findBin("ffmpeg"); // for meeting thumbnails from the screen recording
fs.mkdirSync(REC_DIR, { recursive: true });
let cur = null; // the in-progress meeting being recorded to disk

// ---- recording gate: auto-detection composed with a manual override ----------------------------
// `detected` = the raw "in a call" that the capturers (meeting-tap / meeting-state) self-report.
// `manual`   = the UI's override: null = follow auto-detection, 'on' = force record, 'paused' = hold
//              (mic released, meeting kept open), 'off' = stopped. The two compose into the effective
//              recording state, which drives the meeting lifecycle AND what the UI/tray show. A manual
//              stop/pause auto-clears back to auto once the real call actually ends, so the NEXT call
//              still records itself.
let detected = false, manual = null, manualDetected = false, manualHard = false; // manualDetected = the call state when `manual` was set; manualHard = a genuine manual recording with no call to fall back to
let autoDetect = true; // the "Detect meetings automatically" setting — off = only manual start records
let unloadIdle = true; // "Unload model when idle" setting — off = keep whisper hot between calls
let vocabPrompt = ""; // custom vocabulary → whisper initial prompt (handed to meeting-tap via /api/state)
// Dictation (Handy-style push-to-talk): the config is broadcast into /api/state so meeting-tap reads it
// the same way it reads video/translate/vocab; results come back via POST /ingest {type:'dictation'}.
let dictationCfg = { enabled: false, trigger: { fn: true }, mode: "ptt", language: "auto", translate: false, sound: true, punctuation: "auto", clipboard: "restore", cancel: 53 };
let recentDictations = []; // last N dictation results — in-memory, session-lived (dictated text stays off disk)
// Feature modules — everything is OFF until the user opts in (onboarding), so NO macOS permission is
// requested until a module is enabled. meeting-tap/meeting-state gate their capability use on these.
let modules = { dictation: false, meetings: false, writing: false };
let dictStopSeq = 0; // bumped by POST /api/dictation/stop (pill/tray Stop) → meeting-tap finishes the current dictation
function effectiveRecording() { return manual === "on" ? true : (manual === "paused" || manual === "off") ? false : (autoDetect && detected); }
function broadcastCall() { // push the composed call state to the UI (and lastState, for reconnects)
  const rec = effectiveRecording();
  broadcast({ type: "state", state: { inMeeting: rec || manual === "paused", paused: manual === "paused", startedAt: cur ? cur.startedAt : null } });
}
function recomputeLifecycle(action) {
  const rec = effectiveRecording();
  if (rec) { cancelFinalize(); ensureWhisper(); if (!cur) startMeeting(lastState); }
  else if (cur) {
    if (manual === "paused") cancelFinalize();     // paused — keep the meeting open, just stop capturing
    else if (action === "stop") finalizeMeeting();  // manual stop — end it now
    else scheduleFinalize();                        // auto call-end — debounced (a brief drop won't split it)
  }
}
function setDetected(d) {
  if (d === detected) return;
  detected = d;
  // A call-relative override (pause/resume/stop) is scoped to the call state it was made in; once that
  // changes — the call ends, or a new one is detected — hand control back to auto so the next call records
  // itself. This MUST cover resume (manual==='on') too, or a resumed auto call never finalizes when it ends.
  // A genuine manual recording (manualHard: started with no call to fall back to) is exempt — it persists
  // until the user stops it, so a stray detection doesn't hijack it.
  if (manual !== null && !manualHard && detected !== manualDetected) manual = null;
  recomputeLifecycle(); broadcastCall();
}
function setManual(action) {
  if (action === "start")       { manual = "on";     manualHard = !(autoDetect && detected); } // hard only when there's no auto call under it
  else if (action === "resume") { manual = "on";     manualHard = false; }                     // resume is always call-relative
  else if (action === "pause")  { manual = "paused"; manualHard = false; }
  else if (action === "stop")   { manual = "off";    manualHard = false; }
  else if (action === "auto")   { manual = null;     manualHard = false; }
  else return false;
  manualDetected = detected; // remember the call state at the moment of the override
  recomputeLifecycle(action); broadcastCall();
  return true;
}
// Raw state from a capturer: peel off inMeeting (→ detection), pass the rest (title/app/participants) through.
function ingest(obj) {
  if (obj && obj.type === "dictation") { // a finished dictation from meeting-tap → keep recent + push to the view
    if (obj.text) { recentDictations.unshift({ text: String(obj.text).slice(0, 4000), ts: obj.ts || Date.now(), ms: obj.ms || 0 }); recentDictations = recentDictations.slice(0, 50); }
    broadcast(obj); return;
  }
  if (obj && obj.type === "dictation-state") { broadcast(obj); return; } // listening on/off → drives the UI indicator
  if (obj && obj.type === "state" && obj.state && "inMeeting" in obj.state) {
    const { inMeeting, ...rest } = obj.state;
    if (Object.keys(rest).length) broadcast({ type: "state", state: rest });
    setDetected(inMeeting === true);
  } else broadcast(obj);
}

function broadcast(obj) {
  if (obj.type === "state" && obj.state) lastState = { ...lastState, ...obj.state };
  record(obj);
  const line = `data: ${JSON.stringify(obj)}\n\n`;
  for (const res of clients) res.write(line);
}

// ---- meeting recording (history) ---------------------------------------------
// Accumulate live captions + state into the current meeting; persist JSON to data/meetings/.
function startMeeting(state) {
  cur = { id: String(Date.now()), title: (state && state.title) || "Untitled meeting",
          app: (state && state.app) || "", startedAt: Date.now(), endedAt: null,
          participants: (state && state.participants) ? [...state.participants] : [], lines: [] };
}
function updateMeta(state) {
  if (!cur || !state) return;
  if (state.title) cur.title = state.title;
  if (state.app) cur.app = state.app;
  if (state.participants && state.participants.length)
    cur.participants = [...new Set([...cur.participants, ...state.participants])];
}
function saveMeeting(m) {
  try { fs.writeFileSync(path.join(MEETINGS_DIR, m.id + ".json"), JSON.stringify(m)); }
  catch (e) { console.error("meeting save failed:", e.message); }
}
function finalizeMeeting() {
  if (!cur) return;
  cur.endedAt = Date.now();
  if (cur.lines.length) saveMeeting(cur); // skip empty meetings
  cur = null;
  delete lastState.startedAt; // meeting over — drop the echoed start time
  unloadWhisperSoon(); // free ~1.5 GB after the call
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
    cur.lines.push({ speaker: obj.speaker, text: obj.text, ts: obj.ts || Date.now(), self: !!obj.self });
  } else if (obj.type === "recording") {
    if (!cur) startMeeting(lastState);
    cur.video = obj.file; saveMeeting(cur); // meeting-tap tells us the call's screen-recording file
  } else if (obj.type === "state" && obj.state) {
    updateMeta(obj.state);                                         // names/app only — inMeeting is handled by setDetected()
  }
}
setInterval(() => { if (cur && cur.lines.length) saveMeeting(cur); }, 10000); // flush so a kill mid-call doesn't lose it

function meetingSummary(m) {
  return { id: m.id, title: m.title, app: m.app, startedAt: m.startedAt, endedAt: m.endedAt,
           participants: m.participants, lineCount: m.lines.length,
           snippet: ((m.lines.find(l => l.text && l.text.length > 1) || {}).text || "").slice(0, 140),
           summarized: !!m.summary, hasVideo: !!(m.video && fs.existsSync(path.join(REC_DIR, m.video))),
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
          fs.writeFileSync(p, JSON.stringify(m));
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
  if (m && m.video && !fs.existsSync(path.join(REC_DIR, m.video))) delete m.video; // recording missing → no player
  return m;
}

// ---- LLM backend (local-first) -----------------------------------------------
// One OpenAI-compatible client serves every tier. The UI (Settings → AI Model) sends the chosen
// { base, key, model } per request; the LLM_* env vars are the fallback default.
//   Local = LiteRT-LM (Gemma 4 E2B) on 127.0.0.1:9379 — started ON DEMAND (first summary/ask), so it
//           never competes with Whisper for RAM during a call, and unloads after LLM_IDLE_MS idle.
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
// A packaged build no longer keeps whisper-server (~1.5 GB) resident: node spawns it when a call starts
// and kills it WHISPER_IDLE_MS after the call ends, so it's off RAM between calls. In dev, run.sh already
// has it up on :8091 → node sees the open port and neither spawns nor kills it (not ours to manage).
const WHISPER_PORT = +(process.env.WHISPER_PORT || 8091);
// Primary STT engine: transcribe.cpp (handy-computer) held hot behind our transcribe-server — ~half the RAM
// and ~2x the speed of whisper-server on Apple Silicon, and cleaner output (proper punctuation, no dropped
// onsets). Falls back to whisper.cpp's whisper-server if transcribe-server isn't built on this machine.
const TS_BIN = path.join(__dirname, "..", "stt", "transcribe-server", "transcribe-server");
const TS_MODEL = path.join(__dirname, "..", "stt", "whisper", "whisper-large-v3-turbo-Q8_0.gguf");
const WHISPER_MODEL = path.join(__dirname, "..", "stt", "whisper", "ggml-large-v3-turbo-q8_0.bin"); // whisper.cpp fallback
const WHISPER_IDLE_MS = 10 * 60 * 1000; // keep the model hot 10 min so back-to-back dictation is fast (was 2 min → cold reloads felt slow)
let whisperProc = null, whisperIdle = null; // the STT-server child WE started (null if not ours)
async function ensureWhisper() {
  if (whisperIdle) { clearTimeout(whisperIdle); whisperIdle = null; }
  if (whisperProc) return;                                        // ours, already up
  if (await portOpen("127.0.0.1", WHISPER_PORT)) return;          // someone else's (dev) — leave it
  let bin, args;
  if (fs.existsSync(TS_BIN) && fs.existsSync(TS_MODEL)) {         // primary: transcribe.cpp
    bin = TS_BIN;
    args = ["-m", TS_MODEL, "--host", "127.0.0.1", "--port", String(WHISPER_PORT)];
  } else {                                                        // fallback: whisper.cpp — no Silero VAD (it
    const ws = findBin("whisper-server");                         // degraded clean speech: dropped onsets, lost
    if (ws === "whisper-server" || !fs.existsSync(WHISPER_MODEL)) return; // punctuation). Whole-utterance capture
    bin = ws;                                                     // + the mic energy-gate + conf gate handle noise.
    args = ["-m", WHISPER_MODEL, "--host", "127.0.0.1", "--port", String(WHISPER_PORT), "-t", "4", "--suppress-nst"];
  }
  whisperProc = spawn(bin, args, { stdio: "ignore" });
  whisperProc.on("exit", () => { whisperProc = null; });
  console.error(`STT: starting ${path.basename(bin)} (call detected)…`);
}
function unloadWhisperSoon() { // only unloads the one WE started
  if (!whisperProc || whisperIdle || !unloadIdle) return; // setting off → keep the model hot between calls
  whisperIdle = setTimeout(() => { if (whisperProc) { whisperProc.kill(); whisperProc = null; console.error("whisper-server: unloaded (idle)"); } }, WHISPER_IDLE_MS);
}
const killKids = () => { msShutdown = true; if (llmProc) llmProc.kill(); if (msProc) msProc.kill(); if (whisperProc) whisperProc.kill(); };
["SIGINT", "SIGTERM"].forEach((sig) => process.on(sig, () => { killKids(); process.exit(0); }));
process.on("exit", killKids);

async function chat(messages, opts = {}) {
  const llm = opts.llm || {};
  const base = (llm.base || process.env.LLM_BASE || LLM_LOCAL + "/v1").replace(/\/$/, "");
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
function transcriptText(m, cap = 16000) {
  return m.lines.map(l => `${l.speaker}: ${l.text}`).join("\n").slice(0, cap);
}
function readBody(req) {
  return new Promise(resolve => { let b = ""; req.on("data", c => (b += c)); req.on("end", () => resolve(b)); });
}
function readBodyBuffer(req) { // binary-safe (multipart WAV must not be stringified)
  return new Promise(resolve => { const cs = []; req.on("data", c => cs.push(c)); req.on("end", () => resolve(Buffer.concat(cs))); });
}
async function waitPort(host, port, ms) { const t = Date.now() + ms; while (Date.now() < t) { if (await portOpen(host, port)) return true; await new Promise(r => setTimeout(r, 300)); } return false; }

http.createServer((req, res) => {
  if (req.url === "/" || req.url === "/index.html") {
    // no-store: the WebView (WKWebView) otherwise heuristically caches the UI and serves a stale copy after
    // an edit — so "the code is updated but the app behaves like the old version". Always fetch fresh.
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" });
    return res.end(fs.readFileSync(path.join(__dirname, "index.html")));
  }
  if (req.url.split("?")[0] === "/overlay.html") { // the floating Dictation Pill + Menu Bar Tray windows
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8", "Cache-Control": "no-store" });
    return res.end(fs.readFileSync(path.join(__dirname, "overlay.html")));
  }
  if (req.url === "/logo.png") { // brand mark (sidebar, Pro card, recording chip)
    res.writeHead(200, { "Content-Type": "image/png", "Cache-Control": "max-age=86400" });
    return res.end(fs.readFileSync(path.join(__dirname, "logo.png")));
  }
  if (req.url === "/ingest" && req.method === "POST") {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      try {
        const obj = JSON.parse(body);
        // Tag a remote line with whoever meeting-state says is speaking now. "You"/self lines untouched.
        if (obj.type === "caption" && !obj.self && (!obj.speaker || obj.speaker === "Speaker") && lastState.activeSpeaker)
          obj.speaker = lastState.activeSpeaker;
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
    return res.end(JSON.stringify({ ...lastState, manual: manual || "auto", dictation: dictationCfg, modules, dictStop: dictStopSeq }));
  }
  if (req.url === "/api/meeting/control" && req.method === "POST") { // UI start/pause/resume/stop (composes with auto-detect)
    readBody(req).then(b => {
      let action = ""; try { action = String(JSON.parse(b).action || "").toLowerCase(); } catch {}
      const ok = setManual(action);
      res.writeHead(ok ? 200 : 400, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ manual: manual || "auto", inMeeting: effectiveRecording() || manual === "paused", paused: manual === "paused" }));
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
  if (req.url === "/api/config" && req.method === "POST") { // UI sets the screen-recording mode (off/always/share)
    readBody(req).then(b => {
      try {
        const j = JSON.parse(b);
        if (typeof j.videoMode === "string") broadcast({ type: "state", state: { videoMode: j.videoMode } });
        if (typeof j.translate === "boolean") broadcast({ type: "state", state: { translate: j.translate } });
        if (typeof j.language === "string") broadcast({ type: "state", state: { language: j.language } });
        if (typeof j.autoDetect === "boolean") { autoDetect = j.autoDetect; broadcast({ type: "state", state: { autoDetect } }); recomputeLifecycle(); broadcastCall(); } // off → ignore self-detection (manual only)
        if (typeof j.unloadIdle === "boolean") unloadIdle = j.unloadIdle; // off → keep whisper hot
        if (typeof j.vocab === "string") { vocabPrompt = j.vocab.slice(0, 800); broadcast({ type: "state", state: { vocab: vocabPrompt } }); } // custom vocabulary → meeting-tap
        if (j.dictation && typeof j.dictation === "object") { dictationCfg = { ...dictationCfg, ...j.dictation }; broadcast({ type: "state", state: { dictation: dictationCfg } }); } // Handy-style dictation config → meeting-tap
        if (j.modules && typeof j.modules === "object") { // feature opt-in → gates every permission-touching capability
          modules = { ...modules, ...j.modules };
          dictationCfg.enabled = !!modules.dictation;    // the dictation tap follows the module
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
        // Dictation POSTs only the WAV; inject a language so speech isn't written in the wrong script.
        // A/B-verified on a real clip: language=auto MIS-detects short Hinglish ("mera naam Rahul hai") as
        // Tamil ("மேர் நாம்…") — that IS the bug; language=en yields clean "My name is Rahul." Handy pins en
        // too. Honor an explicit Settings language; default (or "auto") → en for reliable English output.
        let reqBody = buf;
        const bm = /boundary=(?:"([^"]+)"|([^";,\s]+))/i.exec(ct);
        if (bm) {
          const b = bm[1] || bm[2];
          // Match Handy's ACTUAL RUNTIME (from its debug log), not just its settings file. Handy's
          // settings say translate_to_english=true, but its log shows every run is `task=Transcribe,
          // language=en` — the Translate task only engages for a NON-English source; with the language
          // pinned to en it always Transcribes. We were forcing translate=true (the Translate task),
          // which is lower-quality for English/Hinglish audio — the reason our text read worse than
          // Handy's. So: language=en + translate=false (Transcribe), byte-for-byte what Handy runs.
          const lang = (dictationCfg.language && dictationCfg.language !== "auto") ? dictationCfg.language : "en";
          // Match Handy's runtime: language=en + Transcribe task (translate=false). We deliberately do NOT
          // enable Whisper's temperature fallback — without VAD it hallucinates fluent garbage on any silence
          // in the clip (A/B-tested: fallback gave ZERO accuracy gain on clean speech but risks the repetition
          // loop on quiet audio), so dictation uses the same anti-hallucination greedy decode as meetings.
          const fields =
            `--${b}\r\nContent-Disposition: form-data; name="language"\r\n\r\n${lang}\r\n` +
            `--${b}\r\nContent-Disposition: form-data; name="translate"\r\n\r\nfalse\r\n` +
            (vocabPrompt ? `--${b}\r\nContent-Disposition: form-data; name="prompt"\r\n\r\nVocabulary: ${vocabPrompt}\r\n` : "") +
            `--${b}--\r\n`;
          const cut = buf.lastIndexOf(Buffer.from(`--${b}--`));
          reqBody = Buffer.concat([cut >= 0 ? buf.subarray(0, cut) : buf, Buffer.from(fields)]);
        }
        const r = await fetch(`http://127.0.0.1:${WHISPER_PORT}/inference`, { method: "POST", headers: { "Content-Type": ct }, body: reqBody });
        const body = await r.text();
        if (!r.ok) throw new Error(`${r.status} ${body.slice(0, 160)}`);
        let text = ""; try { text = String(JSON.parse(body).text || "").trim(); } catch {}
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
  if (req.url === "/api/open-settings" && req.method === "POST") { // deep-link a macOS Privacy pane (permission checklist)
    readBody(req).then(b => {
      let pane = ""; try { pane = String(JSON.parse(b).pane || ""); } catch {}
      const PANES = { Privacy_Microphone: 1, Privacy_ScreenCapture: 1, Privacy_Accessibility: 1, Privacy_ListenEvent: 1 }; // whitelist
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
    return res.end(JSON.stringify(one));
  }
  const mtitle = req.url.match(/^\/api\/meetings\/([\w-]+)\/title$/); // rename a meeting
  if (mtitle && req.method === "POST") {
    readBody(req).then(b => {
      let t; try { t = String(JSON.parse(b).title || "").trim().slice(0, 120); } catch {}
      if (!t) { res.writeHead(400); return res.end(); }
      if (cur && cur.id === mtitle[1]) { cur.title = t; saveMeeting(cur); }
      else {
        const p = path.join(MEETINGS_DIR, mtitle[1] + ".json");
        if (!fs.existsSync(p)) { res.writeHead(404); return res.end(); }
        try { const m = JSON.parse(fs.readFileSync(p)); m.title = t; fs.writeFileSync(p, JSON.stringify(m)); } catch { res.writeHead(500); return res.end(); }
      }
      res.writeHead(200, { "Content-Type": "application/json" }); res.end(JSON.stringify({ title: t }));
    });
    return;
  }
  const mthumb = req.url.match(/^\/api\/meetings\/([\w-]+)\/thumb$/); // a frame of the screen recording (generated once, cached)
  if (mthumb && req.method === "GET") {
    const m = readMeeting(mthumb[1]);
    if (!m || !m.video) { res.writeHead(404); return res.end(); }
    const video = path.join(REC_DIR, m.video), thumb = video.replace(/\.\w+$/, ".jpg");
    const send = () => { res.writeHead(200, { "Content-Type": "image/jpeg", "Cache-Control": "max-age=86400" }); fs.createReadStream(thumb).pipe(res); };
    if (fs.existsSync(thumb)) return send();
    if (!fs.existsSync(video)) { res.writeHead(404); return res.end(); }
    execFile(FFMPEG, ["-y", "-ss", "1", "-i", video, "-frames:v", "1", "-vf", "scale=480:-1", thumb], () => {
      if (fs.existsSync(thumb)) send(); else { res.writeHead(404); res.end(); }
    });
    return;
  }
  const msum = req.url.match(/^\/api\/meetings\/([\w-]+)\/summary$/);
  if (msum && req.method === "POST") {
    const one = readMeeting(msum[1]);
    if (!one) { res.writeHead(404); return res.end(); }
    readBody(req).then(b => {
      let llm, deeper = false, template = ""; try { const j = JSON.parse(b); llm = j.llm; deeper = !!j.deeper; template = String(j.template || ""); } catch {}
      const instruction = template.trim() || `Summarise this meeting as 3–5 short bullet points, then a line "Action items:" with each as "- <owner>: <task>" (or "None").`;
      const sys = "You summarise meeting transcripts for a busy professional. Be concise, specific, neutral — no preamble."
        + (deeper ? " Reason carefully through the discussion — capture decisions, owners, and nuance you might otherwise miss." : "");
      return chat([
        { role: "system", content: sys },
        { role: "user", content: `${instruction}\n\n${one.notes ? `The note-taker's own scratchpad from the call (weight these points):\n${one.notes}\n\n` : ""}Transcript:\n\n${transcriptText(one)}` },
      ], { maxTokens: deeper ? 1100 : 650, temperature: deeper ? 0.2 : 0.3, llm });
    }).then(text => {
      one.summary = text; saveMeeting(one);
      res.writeHead(200, { "Content-Type": "application/json" }); res.end(JSON.stringify({ summary: text }));
    }).catch(e => { res.writeHead(502, { "Content-Type": "application/json" }); res.end(JSON.stringify({ error: String(e.message || e) })); });
    return;
  }
  const mask = req.url.match(/^\/api\/meetings\/([\w-]+)\/ask$/);
  if (mask && req.method === "POST") {
    const one = readMeeting(mask[1]);
    if (!one) { res.writeHead(404); return res.end(); }
    readBody(req).then(b => {
      let q = "", llm; try { const j = JSON.parse(b); q = j.question || ""; llm = j.llm; } catch {}
      return chat([
        { role: "system", content: "Answer questions about this meeting using ONLY its transcript. If it isn't covered, say so briefly." },
        { role: "user", content: `Transcript:\n${transcriptText(one)}\n\nQuestion: ${q}` },
      ], { maxTokens: 500, llm });
    }).then(text => { res.writeHead(200, { "Content-Type": "application/json" }); res.end(JSON.stringify({ answer: text })); })
      .catch(e => { res.writeHead(502, { "Content-Type": "application/json" }); res.end(JSON.stringify({ error: String(e.message || e) })); });
    return;
  }
  res.writeHead(404);
  res.end();
}).listen(PORT, () => {
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

