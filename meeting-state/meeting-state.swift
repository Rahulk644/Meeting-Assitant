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
    if key == "zoom" { enableManualAX(appEl) }   // unlock Zoom's embedded UI for name reading
    deriveControls(from: menuTitles(appEl), into: &s)
    if windowInMeeting(appEl, appKey: key) { s.inMeeting = true }
    if key == "zoom" {
        let (names, active) = zoomNames(appEl)
        s.participants = names
        s.activeSpeaker = active
        if !names.isEmpty { s.inMeeting = true }
    }
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

// Zoom renders its meeting UI in an embedded-Chromium layer that is INVISIBLE to Accessibility
// until you set AXManualAccessibility=true on the process. We do that to read participant names.
private func enableManualAX(_ el: AXUIElement) {
    AXUIElementSetAttributeValue(el, "AXManualAccessibility" as CFString, kCFBooleanTrue)
}

// ---- Zoom participant / active-speaker names ----------------------------------
// Zoom labels each video tile with an element DESCRIPTION shaped like
//   "Rahul Khatri, Computer audio muted, Video off"      (a tile)
//   "View Rahul Khatri's profile"                        (its profile button)
// The always-on-top "zoom floating video window" follows the ACTIVE SPEAKER in speaker
// view — so the name in that window's tile is who's talking right now. Other windows
// (participants panel, gallery) give the roster.

private func validName(_ s: String) -> Bool {
    let n = s.trimmingCharacters(in: .whitespaces)
    guard n.count >= 2, n.count <= 40, n.rangeOfCharacter(from: .letters) != nil else { return false }
    guard n.split(separator: " ").count <= 5 else { return false }   // names are 1–5 words; Zoom UI hints run long
    // Zoom labels non-people ("Set up professional audio in Audio settings", "Waiting for host…") with the same
    // kind of AX string as tiles. Reject anything containing a UI/state word — by WHOLE word, so real names that
    // merely embed one as a substring ("Sharen", "Mike") survive.
    // ponytail: English word-list; add locale variants if names ever come through localized.
    let tokens = Set(n.lowercased().split { !$0.isLetter }.map(String.init))
    let uiWords: Set<String> = ["zoom","audio","video","mute","unmute","muted","share","sharing",
        "settings","setting","microphone","mic","speaker","camera","waiting","participants",
        "participant","reactions","reaction","profile","record","recording","screen","gallery"]
    return tokens.isDisjoint(with: uiWords)
}

/// Drop Zoom's trailing annotation so the same person dedupes:
/// "Rahul Khatri (Me)" / "Rahul Khatri (Host, me)" -> "Rahul Khatri".
private func canonName(_ s: String) -> String {
    let base = s.firstIndex(of: "(").map { String(s[..<$0]) } ?? s
    return base.trimmingCharacters(in: .whitespaces)
}

/// "Rahul Khatri, Computer audio muted, Video off" -> "Rahul Khatri"
private func nameFromTileDesc(_ d: String) -> String? {
    let low = d.lowercased()
    guard d.contains(","), low.contains("audio") || low.contains("video") || low.contains("muted") else { return nil }
    let name = canonName(d.components(separatedBy: ",").first!.trimmingCharacters(in: .whitespaces))
    return validName(name) ? name : nil
}

/// "View Rahul Khatri's profile" -> "Rahul Khatri"  (handles straight + curly apostrophe)
private func nameFromProfileDesc(_ d: String) -> String? {
    guard d.hasPrefix("View ") else { return nil }
    var name = String(d.dropFirst(5))
    for suffix in ["'s profile", "\u{2019}s profile", " profile"] {
        if let r = name.range(of: suffix) { name = String(name[..<r.lowerBound]); break }
    }
    name = canonName(name.trimmingCharacters(in: .whitespaces))
    return validName(name) ? name : nil
}

/// "Talking: rajat prakash" -> "Rajat Prakash". Zoom shows this AXValue for the active speaker in the
/// floating video window — and it's the ONE name still reachable while YOU are screen-sharing (which
/// collapses the video tiles the other selectors read). Authoritative for who's speaking now.
private func nameFromTalking(_ s: String) -> String? {
    guard s.lowercased().hasPrefix("talking:") else { return nil }
    let raw = canonName(String(s.dropFirst("talking:".count)).trimmingCharacters(in: .whitespaces))
    return validName(raw) ? raw.capitalized : nil
}

/// Walk the app's windows harvesting tile/profile names. The floating video window's tile
/// is the active speaker; all harvested names form the roster.
private func zoomNames(_ appEl: AXUIElement) -> (participants: [String], active: String?) {
    var names: [String] = [], active: String? = nil
    func walk(_ el: AXUIElement, floating: Bool, depth: Int) {
        if depth > 30 { return }
        let inFloat = floating || (axStr(el, kTitle) ?? "").lowercased().contains("floating video")
        if let d = axStr(el, kDesc), let n = nameFromTileDesc(d) ?? nameFromProfileDesc(d) {
            if !names.contains(n) { names.append(n) }
            if inFloat && active == nil { active = n }   // first tile in the floating window = who's on screen
        }
        if let v = axGet(el, kValue) as? String, let n = nameFromTalking(v) {   // "Talking: X" — works even while sharing
            if !names.contains(n) { names.append(n) }
            active = n
        }
        for k in axKids(el) { walk(k, floating: inFloat, depth: depth + 1) }
    }
    for w in (axGet(appEl, kWindows) as? [AXUIElement] ?? []) { walk(w, floating: false, depth: 0) }
    return (names, active)
}

// NOTE: live caption TEXT comes from on-device Whisper (see ../stt). This layer's job is meeting
// STATE — in-meeting / mute / share (from menu-bar labels) and participant/active-speaker NAMES.

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

    // 3. Name filtering — reject Zoom UI chrome, keep real names, canonicalize "(Me)"
    check(!validName("Set up professional audio in Audio settings"), "reject UI-chrome as a name")
    check(!validName("Waiting for host to start the meeting"), "reject waiting-room hint")
    check(validName("Rahul Khatri"), "accept a real name")
    check(validName("Sharen"), "keep a name that merely embeds a ui-word substring")
    check(canonName("Rahul Khatri (Me)") == "Rahul Khatri", "canon strips (Me)")
    check(canonName("Rahul Khatri (Host, me)") == "Rahul Khatri", "canon strips (Host, me)")
    check(nameFromTileDesc("Rahul Khatri (Me), Computer audio muted") == "Rahul Khatri", "tile desc -> canonical name")
    check(nameFromTileDesc("Set up professional audio in Audio settings, Muted") == nil, "tile-shaped UI hint rejected")

    // 4. Detection runs without crashing
    _ = runningMeetingApp()
    err("SELFTEST OK")
    exit(0)
}

// MARK: - main

setbuf(stdout, nil)   // unbuffered: parent receives each JSON line immediately

enum Mode { case watch, once, dump, selftest }
var mode = Mode.watch
var interval = 1.0
var maxDepth = 14

let args = CommandLine.arguments
var i = 1
while i < args.count {
    switch args[i] {
    case "--once": mode = .once
    case "--dump": mode = .dump
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
    if key == "zoom" { enableManualAX(appEl); Thread.sleep(forTimeInterval: 0.5) }  // unlock embedded UI, let it populate
    err("-- menu titles --")
    for t in menuTitles(appEl) { err("  \(t)") }
    err("-- window trees (build participant/active-speaker selectors from here) --")
    for w in (axGet(appEl, kWindows) as? [AXUIElement] ?? []) { dumpTree(w, depth: 1, maxDepth: maxDepth) }
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
            // meeting-state is now spawned as a DIRECT child of the app, so Accessibility CAN be granted to it
            // (via the app's grant). Request it once per launch; once granted, a valid app signature makes it
            // stick, so it won't nag again. Detection still works without it (the capturer self-detects); this
            // only enables speaker names.
            emit(Line(type: "status", axTrusted: false, msg: "Grant Accessibility for speaker names: System Settings > Privacy & Security > Accessibility"))
            if !prompted { _ = axTrusted(prompt: true); prompted = true }
            Thread.sleep(forTimeInterval: max(interval, 3))
            continue
        }
        let s = snapshot()
        if s != last { emit(Line(type: "state", ts: nowMs(), state: s)); last = s }
        Thread.sleep(forTimeInterval: interval)
    }
}
