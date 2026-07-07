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
    if (!cur) startMeeting(lastState);
    cur.lines.push({ speaker: obj.speaker, text: obj.text, ts: obj.ts || Date.now(), self: !!obj.self });
  } else if (obj.type === "recording") {
    if (!cur) startMeeting(lastState);
    cur.video = obj.file; saveMeeting(cur); // meeting-tap tells us the call's screen-recording file
  } else if (obj.type === "state" && obj.state) {
    if (obj.state.inMeeting === false) scheduleFinalize();          // debounced — a brief drop won't split the call
    else if (obj.state.inMeeting === true) { cancelFinalize(); ensureWhisper(); if (!cur) startMeeting(obj.state); else updateMeta(obj.state); }
    else updateMeta(obj.state);
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
const WHISPER_MODEL = path.join(__dirname, "..", "stt", "whisper", "ggml-large-v3-turbo-q8_0.bin");
const WHISPER_IDLE_MS = 2 * 60 * 1000;
let whisperProc = null, whisperIdle = null; // the whisper-server child WE started (null if not ours)
async function ensureWhisper() {
  if (whisperIdle) { clearTimeout(whisperIdle); whisperIdle = null; }
  if (whisperProc) return;                                        // ours, already up
  if (await portOpen("127.0.0.1", WHISPER_PORT)) return;          // someone else's (dev) — leave it
  const bin = findBin("whisper-server");
  if (bin === "whisper-server" || !fs.existsSync(WHISPER_MODEL)) return; // not installed here — skip
  whisperProc = spawn(bin, ["-m", WHISPER_MODEL, "--host", "127.0.0.1", "--port", String(WHISPER_PORT), "-t", "4"], { stdio: "ignore" });
  whisperProc.on("exit", () => { whisperProc = null; });
  console.error("whisper-server: starting (call detected)…");
}
function unloadWhisperSoon() { // only unloads the one WE started
  if (!whisperProc || whisperIdle) return;
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

http.createServer((req, res) => {
  if (req.url === "/" || req.url === "/index.html") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    return res.end(fs.readFileSync(path.join(__dirname, "index.html")));
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
        broadcast(obj);
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
  if (req.url === "/api/state" && req.method === "GET") { // meeting-tap polls this to gate capture + video mode
    res.writeHead(200, { "Content-Type": "application/json" });
    return res.end(JSON.stringify(lastState || {}));
  }
  if (req.url === "/api/config" && req.method === "POST") { // UI sets the screen-recording mode (off/always/share)
    readBody(req).then(b => {
      try {
        const j = JSON.parse(b);
        if (typeof j.videoMode === "string") broadcast({ type: "state", state: { videoMode: j.videoMode } });
        if (typeof j.translate === "boolean") broadcast({ type: "state", state: { translate: j.translate } });
        if (typeof j.language === "string") broadcast({ type: "state", state: { language: j.language } });
      } catch {}
      res.writeHead(204); res.end();
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
      let llm; try { llm = JSON.parse(b).llm; } catch {}
      return chat([
        { role: "system", content: "You summarise meeting transcripts for a busy professional. Be concise, specific, neutral — no preamble." },
        { role: "user", content: `Summarise this meeting as 3–5 short bullet points, then a line "Action items:" with each as "- <owner>: <task>" (or "None"). Transcript:\n\n${transcriptText(one)}` },
      ], { maxTokens: 650, llm });
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
      try { broadcast(JSON.parse(line)); } catch {}
    }
  });
  p.on("exit", (c) => {
    console.error(`meeting-state exited (${c})`);
    if (!msShutdown) setTimeout(startMeetingState, 1000); // died mid-session — respawn so the app doesn't go blind to calls
  });
}

