// meeting-state.swift
//
// Meeting-state layer: reads LIVE meeting UI state from native meeting apps via the
// macOS Accessibility API and streams newline-delimited JSON to a parent process
// (Node/Electron). This is the open-source rebuild of Fathom's `Common/` module
// (AXObserverManager / ParticipantMonitor / MuteMonitor / ScreenShareMonitor).
//
// Build:  swiftc -O meeting-state.swift -o meeting-state
// Run:
//   ./meeting-state              # watch: stream JSON `state` lines on every change
//   ./meeting-state --once       # print one snapshot as JSON, exit
//   ./meeting-state --dump       # dump the AX tree of the live meeting app (to stderr)
//                                #   -> use this ON A REAL CALL to build participant/
//                                #      active-speaker selectors, then extend snapshot()
//   ./meeting-state --selftest   # validate the derive logic + JSON plumbing, exit 0/1
//   flags: --interval <sec>  --depth <n>
//
// Coverage (honest):
//   Zoom native (us.zoom.xos): inMeeting / selfMuted / videoOn / sharing  = RELIABLE
//   Teams (com.microsoft.teams2): inMeeting = best-effort; controls via --dump
//   Google Meet (browser): NOT here -> use a Chrome extension (DOM), AX is the wrong tool
//   participants / activeSpeaker: intentionally not guessed -> build via --dump (see README)
//
// License: MIT. Written fresh from public Accessibility APIs; no vendor code copied.

import AppKit
import ApplicationServices
import Foundation

// MARK: - AX attribute constants (raw names, import-agnostic)

private let kChildren = "AXChildren", kTitle = "AXTitle", kRole = "AXRole",
            kValue = "AXValue", kMenuBar = "AXMenuBar", kWindows = "AXWindows",
            kDesc = "AXDescription"

// MARK: - AX helpers

private func axGet(_ el: AXUIElement, _ attr: String) -> CFTypeRef? {
    var v: CFTypeRef?
    return AXUIElementCopyAttributeValue(el, attr as CFString, &v) == .success ? v : nil
}
private func axStr(_ el: AXUIElement, _ attr: String) -> String? { axGet(el, attr) as? String }
private func axKids(_ el: AXUIElement) -> [AXUIElement] {
    (axGet(el, kChildren) as? [AXUIElement]) ?? []
}
private func axElem(_ el: AXUIElement, _ attr: String) -> AXUIElement? {
    guard let raw = axGet(el, attr), CFGetTypeID(raw) == AXUIElementGetTypeID() else { return nil }
    return (raw as! AXUIElement)
}
private func axTrusted(prompt: Bool) -> Bool {
    let key = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
    return AXIsProcessTrustedWithOptions([key: prompt] as CFDictionary)
}
private func err(_ s: String) { FileHandle.standardError.write((s + "\n").data(using: .utf8)!) }
private func nowMs() -> Int { Int(Date().timeIntervalSince1970 * 1000) }

// MARK: - meeting apps

private struct MeetingApp { let key: String; let bundles: [String] }
private let MEETING_APPS = [
    MeetingApp(key: "zoom",  bundles: ["us.zoom.xos"]),
    MeetingApp(key: "teams", bundles: ["com.microsoft.teams2", "com.microsoft.teams"]),
]

/// First running meeting app (ponytail: first-wins; add priority ordering if you run two at once).
private func runningMeetingApp() -> (key: String, pid: pid_t)? {
    for ra in NSWorkspace.shared.runningApplications {
        guard let b = ra.bundleIdentifier else { continue }
        for app in MEETING_APPS where app.bundles.contains(b) {
            return (app.key, ra.processIdentifier)
        }
    }
    return nil
}

// MARK: - state

struct State: Codable, Equatable {
    var app = ""
    var inMeeting = false
    var selfMuted: Bool? = nil     // true = your mic is muted
    var videoOn: Bool? = nil
    var sharing: Bool? = nil       // true = YOU are screen-sharing (hide the overlay!)
    var participants: [String] = []   // best-effort; empty until you build selectors via --dump
    var activeSpeaker: String? = nil
}

// MARK: - control derivation (the reliable signal: read the app's menu-bar toggle labels)

/// Collect every menu-item title in the app menu bar. No menu opening required —
/// the labels live in the AX hierarchy for VoiceOver, readable while backgrounded.
private func menuTitles(_ appEl: AXUIElement) -> [String] {
    guard let mb = axElem(appEl, kMenuBar) else { return [] }
    var titles: [String] = []
    for top in axKids(mb) {          // AXMenuBarItem  ("Meeting", "View", ...)
        for menu in axKids(top) {    // its AXMenu
            for item in axKids(menu) {   // AXMenuItem
                if let t = axStr(item, kTitle), !t.isEmpty { titles.append(t) }
            }
        }
    }
    return titles
}

/// A meeting app shows a toggle labeled with the ACTION, not the current state:
///   muted   -> menu says "Unmute Audio"      unmuted -> "Mute Audio"
///   sharing -> "Stop Share"                   not     -> "Start Share"
///   video   -> "Stop Video"                   off     -> "Start Video"
/// (Localized/English-only. ponytail: English labels; add locale maps if you go global.)
func deriveControls(from titles: [String], into s: inout State) {
    let low = titles.map { $0.lowercased() }
    func has(_ needles: String...) -> Bool {
        low.contains { t in needles.allSatisfy { t.contains($0) } }
    }
    if has("unmute", "audio")      { s.selfMuted = true }   // check unmute first ("unmute" contains "mute")
    else if has("mute", "audio")   { s.selfMuted = false }

    if has("stop", "video")        { s.videoOn = true }
    else if has("start", "video")  { s.videoOn = false }

    if has("stop", "share")        { s.sharing = true }
    else if has("start", "share")  { s.sharing = false }

    // Presence of in-call controls implies we're in a meeting.
    if s.selfMuted != nil || s.sharing != nil { s.inMeeting = true }
}

/// Secondary in-meeting signal from window titles (works even if menus read empty).
private func windowInMeeting(_ appEl: AXUIElement, appKey: String) -> Bool {
    for w in (axGet(appEl, kWindows) as? [AXUIElement] ?? []) {
        let t = (axStr(w, kTitle) ?? "").lowercased()
        if appKey == "zoom",  t.contains("zoom meeting") { return true }
        if appKey == "teams", t.contains("meeting") || t.contains("| microsoft teams") { return true }
    }
    return false
}

private func snapshot() -> State {
    var s = State()
    guard let (key, pid) = runningMeetingApp() else { return s }
    s.app = key
    let appEl = AXUIElementCreateApplication(pid)
    deriveControls(from: menuTitles(appEl), into: &s)
    if windowInMeeting(appEl, appKey: key) { s.inMeeting = true }
    return s
}

// MARK: - output

private struct Line: Encodable {
    let type: String
    var axTrusted: Bool? = nil
    var msg: String? = nil
    var ts: Int? = nil
    var state: State? = nil
    var speaker: String? = nil
    var text: String? = nil
    var isSelf: Bool? = nil
    enum CodingKeys: String, CodingKey {
        case type, axTrusted, msg, ts, state, speaker, text
        case isSelf = "self"
    }
}
private func emit(_ line: Line) {
    if let d = try? JSONEncoder().encode(line), let str = String(data: d, encoding: .utf8) {
        print(str)   // stdout is unbuffered (setbuf below) so the parent gets lines live
    }
}

// MARK: - AX tree dump (build participant/active-speaker selectors against a REAL call)

private func dumpTree(_ el: AXUIElement, depth: Int, maxDepth: Int) {
    if depth > maxDepth { return }
    let role = axStr(el, kRole) ?? "?"
    var line = String(repeating: "  ", count: depth) + role
    if let t = axStr(el, kTitle), !t.isEmpty { line += "  title=\"\(t)\"" }
    if let v = axGet(el, kValue) as? String, !v.isEmpty { line += "  value=\"\(v)\"" }
    if let d = axStr(el, kDesc), !d.isEmpty { line += "  desc=\"\(d)\"" }
    err(line)
    for k in axKids(el) { dumpTree(k, depth: depth + 1, maxDepth: maxDepth) }
}

/// Collect every text-bearing node (role + value). Used by --probe to discover how the
/// meeting app renders live captions (speaker names, partial vs final lines).
private func collectTextNodes(_ el: AXUIElement, _ out: inout [(String, String)], depth: Int) {
    if depth > 32 { return }
    let role = axStr(el, kRole) ?? "?"
    if role == "AXStaticText" || role == "AXTextArea" || role == "AXTextField" {
        if let v = (axGet(el, kValue) as? String) ?? axStr(el, kTitle) ?? axStr(el, kDesc), !v.isEmpty {
            out.append((role, v))
        }
    }
    for k in axKids(el) { collectTextNodes(k, &out, depth: depth + 1) }
}

// ---- Zoom live-caption reading -------------------------------------------------
// Zoom renders its meeting UI (incl. captions) in an embedded-Chromium layer that is
// INVISIBLE to Accessibility until you set AXManualAccessibility=true on the process.
// Then live captions appear in an AXTable titled/described "Closed caption subtitles".

private func enableManualAX(_ el: AXUIElement) {
    AXUIElementSetAttributeValue(el, "AXManualAccessibility" as CFString, kCFBooleanTrue)
}

// NOTE: live captions now come from on-device Whisper (see ../stt). This helper's job is the
// AX meeting-STATE layer — in-meeting / mute / share (reliable, from menu-bar labels) — plus the
// hooks (enableManualAX, collectTextNodes, --dump/--probe) to add active-speaker NAMES next.

// MARK: - self-test (ponytail runnable check; uses explicit checks, not assert — assert is
//         stripped under -O)

private func check(_ cond: Bool, _ msg: String) {
    if !cond { err("SELFTEST FAIL: \(msg)"); exit(1) }
}
private func selftest() -> Never {
    // 1. Codable round-trip
    var s = State(); s.app = "zoom"; s.inMeeting = true; s.selfMuted = true
    s.sharing = false; s.participants = ["Ann", "Bob"]
    let d = try! JSONEncoder().encode(s)
    check((try? JSONDecoder().decode(State.self, from: d)) == s, "state codable round-trip")

    // 2. Control derivation — the load-bearing logic
    var m = State(); deriveControls(from: ["Unmute Audio", "Start Video", "Stop Share"], into: &m)
    check(m.selfMuted == true && m.videoOn == false && m.sharing == true && m.inMeeting, "muted/sharing derive")
    var u = State(); deriveControls(from: ["Mute Audio", "Stop Video", "Start Share"], into: &u)
    check(u.selfMuted == false && u.videoOn == true && u.sharing == false, "unmuted derive")
    var n = State(); deriveControls(from: ["File", "Edit", "Help"], into: &n)
    check(n.selfMuted == nil && !n.inMeeting, "no-meeting derive stays nil")

    // 3. Detection runs without crashing
    _ = runningMeetingApp()
    err("SELFTEST OK")
    exit(0)
}

// MARK: - main

setbuf(stdout, nil)   // unbuffered: parent receives each JSON line immediately

enum Mode { case watch, once, dump, probe, selftest }
var mode = Mode.watch
var interval = 1.0
var maxDepth = 14

let args = CommandLine.arguments
var i = 1
while i < args.count {
    switch args[i] {
    case "--once": mode = .once
    case "--dump": mode = .dump
    case "--probe": mode = .probe
    case "--selftest": mode = .selftest
    case "--interval": if i + 1 < args.count { i += 1; interval = Double(args[i]) ?? 1.0 }
    case "--depth": if i + 1 < args.count { i += 1; maxDepth = Int(args[i]) ?? 14 }
    default: break
    }
    i += 1
}

switch mode {
case .selftest:
    selftest()

case .dump:
    guard axTrusted(prompt: true) else { err("Grant Accessibility permission, then rerun."); exit(1) }
    guard let (key, pid) = runningMeetingApp() else { err("No meeting app (Zoom/Teams) running."); exit(1) }
    err("== \(key) pid=\(pid) ==")
    let appEl = AXUIElementCreateApplication(pid)
    err("-- menu titles --")
    for t in menuTitles(appEl) { err("  \(t)") }
    err("-- window trees (build participant/active-speaker selectors from here) --")
    for w in (axGet(appEl, kWindows) as? [AXUIElement] ?? []) { dumpTree(w, depth: 1, maxDepth: maxDepth) }
    exit(0)

case .probe:
    // The make-or-break test: does Zoom expose its live captions to the Accessibility API?
    // Prints every NEW text node that appears while you speak (with captions ON), tagged by
    // window + role + time — so we can see if/how captions + speaker names are structured.
    guard axTrusted(prompt: true) else { err("Grant Accessibility permission, then rerun."); exit(1) }
    guard let (key, pid) = runningMeetingApp(), key == "zoom" else {
        err("Start a Zoom meeting (New Meeting) with captions ON, then run --probe."); exit(1)
    }
    let appEl = AXUIElementCreateApplication(pid)
    err(">> Probing Zoom caption AX for ~25s. Captions ON? Now speak a few full sentences...")
    var seen = Set<String>()
    let start = Date()
    while Date().timeIntervalSince(start) < 25 {
        for w in (axGet(appEl, kWindows) as? [AXUIElement] ?? []) {
            let wt = axStr(w, kTitle) ?? "(untitled)"
            var nodes: [(String, String)] = []
            collectTextNodes(w, &nodes, depth: 0)
            for (role, val) in nodes where val.count >= 5 {
                if seen.insert(wt + "|" + role + "|" + val).inserted {
                    print("[\(Int(Date().timeIntervalSince(start)))s] win=\"\(wt)\" \(role): \(val)")
                }
            }
        }
        Thread.sleep(forTimeInterval: 1.0)
    }
    err(">> Probe done. Send me the [Ns] lines above.")
    exit(0)

case .once:
    if !axTrusted(prompt: true) { emit(Line(type: "status", axTrusted: false,
        msg: "Accessibility permission required")) ; exit(1) }
    emit(Line(type: "state", ts: nowMs(), state: snapshot()))
    exit(0)

case .watch:
    var last: State? = nil
    var prompted = false
    while true {
        if !axTrusted(prompt: false) {
            emit(Line(type: "status", axTrusted: false,
                      msg: "Grant Accessibility: System Settings > Privacy & Security > Accessibility"))
            if !prompted { _ = axTrusted(prompt: true); prompted = true }
            Thread.sleep(forTimeInterval: interval)
            continue
        }
        let s = snapshot()
        if s != last { emit(Line(type: "state", ts: nowMs(), state: s)); last = s }
        // TODO(names): read the active-speaker NAME from Zoom's AX tree here → state.activeSpeaker
        //              (the GUI already renders it; Whisper provides the words).
        Thread.sleep(forTimeInterval: interval)
    }
}
