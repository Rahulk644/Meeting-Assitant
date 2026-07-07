#!/usr/bin/env node
// Live speaker-attributed transcript app.
//   node server.js         -> spawn ../meeting-state/meeting-state, stream its JSON to the GUI
//   node server.js --demo  -> emit scripted sample lines so you can see the GUI with no call
// No dependencies. Open http://localhost:8787
const http = require("http");
const fs = require("fs");
const net = require("net");
const path = require("path");
const { spawn } = require("child_process");

const DEMO = process.argv.includes("--demo");
const INGEST = process.argv.includes("--ingest"); // captions arrive via POST /ingest (from meeting-tap)
const PORT = process.env.PORT || 8787; // respect preview's assigned port; 8787 for manual runs
const clients = new Set();
let lastState = {}; // so a client connecting mid-meeting gets current title/participants/speaker
const MEETINGS_DIR = path.join(__dirname, "..", "data", "meetings");
fs.mkdirSync(MEETINGS_DIR, { recursive: true });
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
}
function record(obj) {
  if (obj.type === "caption") {
    if (!cur) startMeeting(lastState);
    cur.lines.push({ speaker: obj.speaker, text: obj.text, ts: obj.ts || Date.now(), self: !!obj.self });
  } else if (obj.type === "state" && obj.state) {
    if (obj.state.inMeeting === false) finalizeMeeting();
    else if (obj.state.inMeeting === true && !cur) startMeeting(obj.state);
    else updateMeta(obj.state);
  }
}
setInterval(() => { if (cur && cur.lines.length) saveMeeting(cur); }, 10000); // flush so a kill mid-call doesn't lose it

function meetingSummary(m) {
  return { id: m.id, title: m.title, app: m.app, startedAt: m.startedAt, endedAt: m.endedAt,
           participants: m.participants, lineCount: m.lines.length,
           snippet: ((m.lines.find(l => l.text && l.text.length > 1) || {}).text || "").slice(0, 140),
           summarized: !!m.summary, live: m.endedAt == null };
}
function listMeetings() {
  const out = [];
  try {
    for (const f of fs.readdirSync(MEETINGS_DIR)) {
      if (!f.endsWith(".json")) continue;
      try { out.push(meetingSummary(JSON.parse(fs.readFileSync(path.join(MEETINGS_DIR, f))))); } catch {}
    }
  } catch {}
  if (cur && cur.lines.length && !out.some(x => x.id === cur.id)) out.push(meetingSummary(cur));
  return out.sort((a, b) => b.startedAt - a.startedAt);
}
function readMeeting(id) {
  if (cur && cur.id === id) return cur;
  try { return JSON.parse(fs.readFileSync(path.join(MEETINGS_DIR, id + ".json"))); } catch { return null; }
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
["SIGINT", "SIGTERM"].forEach((sig) => process.on(sig, () => { if (llmProc) llmProc.kill(); process.exit(0); }));
process.on("exit", () => { if (llmProc) llmProc.kill(); });

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
  console.log(`\n  Live transcript → http://localhost:${PORT}   ${DEMO ? "(demo)" : "(live)"}\n`);
  if (DEMO) { startDemo(); return; }
  if (INGEST) lastState = { inMeeting: true, title: "Live transcript" }; // captions POST to /ingest
  startMeetingState(); // state + active-speaker (names). Harmless if not built yet.
});

// LIVE: forward meeting-state's NDJSON (state + caption events) straight through.
function startMeetingState() {
  const bin = path.join(__dirname, "..", "meeting-state", "meeting-state");
  if (!fs.existsSync(bin)) {
    console.error(`! meeting-state not built. Run: (cd ../meeting-state && swiftc -O meeting-state.swift -o meeting-state)`);
    return;
  }
  const p = spawn(bin, [], { stdio: ["ignore", "pipe", "inherit"] });
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
  p.on("exit", (c) => console.error(`meeting-state exited (${c})`));
}

// DEMO: a scripted snippet using real teammate names so the GUI looks like a real standup.
function startDemo() {
  const YOU = "You";
  const script = [
    ["Vikram Prasanna", "The export bug — it fails if the file's been modified but not saved."],
    [YOU, "So we auto-save before export? That's the clean fix."],
    ["John Lee", "I hit that in testing too — it loses the work silently."],
    ["Rajat Prakash", "Let's land it before the Valencia launch Monday."],
    ["Vikram Prasanna", "Agreed. Mohit takes the auto-save, I'll fix the dashboard string."],
    [YOU, "Perfect. Ashi, can you add the default refresh status on the LTI dashboard?"],
    ["Ashi Khetan", "Yep, pushing that today."],
  ];
  const participants = ["You", "Vikram Prasanna", "John Lee", "Rajat Prakash", "Ashi Khetan"];
  const first = () =>
    broadcast({ type: "state", state: { app: "zoom", inMeeting: true, title: "PREP Canvas LTI : Debrief", participants } });
  setTimeout(first, 400);
  let i = 0;
  const tick = () => {
    const [speaker, text] = script[i % script.length];
    broadcast({ type: "state", state: { app: "zoom", inMeeting: true, activeSpeaker: speaker, participants } });
    broadcast({ type: "caption", speaker, text, ts: Date.now(), self: speaker === YOU });
    i++;
    setTimeout(tick, 2600);
  };
  setTimeout(tick, 900);
}
