#!/usr/bin/env node
// Live speaker-attributed transcript app.
//   node server.js         -> spawn ../meeting-state/meeting-state, stream its JSON to the GUI
//   node server.js --demo  -> emit scripted sample lines so you can see the GUI with no call
// No dependencies. Open http://localhost:8787
const http = require("http");
const fs = require("fs");
const path = require("path");
const { spawn } = require("child_process");

const DEMO = process.argv.includes("--demo");
const INGEST = process.argv.includes("--ingest"); // captions arrive via POST /ingest (from meeting-tap)
const PORT = process.env.PORT || 8787; // respect preview's assigned port; 8787 for manual runs
const clients = new Set();
let lastState = {}; // so a client connecting mid-meeting gets current title/participants/speaker

function broadcast(obj) {
  if (obj.type === "state" && obj.state) lastState = { ...lastState, ...obj.state };
  const line = `data: ${JSON.stringify(obj)}\n\n`;
  for (const res of clients) res.write(line);
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
