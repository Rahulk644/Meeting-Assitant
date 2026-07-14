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
//   Browser/chat calls: Core Audio first proves active call-shaped audio; this helper then reads only
//   the matching meeting window for title/control enrichment. It never scans while there is no call.
//   participants / activeSpeaker: best-effort with source/role/context evidence on every accepted name
//
// License: MIT. Written fresh from public Accessibility APIs; no vendor code copied.

import AppKit
import ApplicationServices
import Foundation
import Darwin          // kill(pid, 0): stop this helper if the parent app crashes

private let parentPID = pid_t(ProcessInfo.processInfo.environment["ASSISTANT_PARENT_PID"] ?? "") ?? 0
private func parentIsAlive() -> Bool {
    guard parentPID > 1 else { return true } // standalone self-tests and dumps have no parent contract
    if kill(parentPID, 0) == 0 { return true }
    return errno == EPERM
}

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
    MeetingApp(key: "zoom", bundles: ["us.zoom.xos"]),
    MeetingApp(key: "teams", bundles: ["com.microsoft.teams2", "com.microsoft.teams"]),
    MeetingApp(key: "webex", bundles: ["com.cisco.webexmeetingsapp", "com.cisco.webex2"]),
    MeetingApp(key: "facetime", bundles: ["com.apple.FaceTime"]),
    MeetingApp(key: "slack", bundles: ["com.tinyspeck.slackmacgap"]),
    MeetingApp(key: "discord", bundles: ["com.hnc.Discord"]),
    MeetingApp(key: "goto", bundles: ["com.logmein.GoToMeeting", "com.goto.GoTo"]),
    MeetingApp(key: "ringcentral", bundles: ["com.ringcentral.glip", "com.ringcentral.RingCentral"]),
    MeetingApp(key: "around", bundles: ["com.getaround.Around"]),
    MeetingApp(key: "whatsapp", bundles: ["net.whatsapp.WhatsApp"]),
    MeetingApp(key: "telegram", bundles: ["ru.keepcoder.Telegram"]),
    MeetingApp(key: "signal", bundles: ["org.whispersystems.signal-desktop"]),
    MeetingApp(key: "browser", bundles: ["com.apple.Safari", "com.google.Chrome", "com.google.Chrome.beta",
        "com.google.Chrome.canary", "com.microsoft.edgemac", "com.microsoft.edgemac.Beta",
        "com.microsoft.edgemac.Dev", "company.thebrowser.Browser", "com.brave.Browser",
        "org.mozilla.firefox", "org.chromium.Chromium", "com.operasoftware.Opera", "com.vivaldi.Vivaldi"]),
]

/// First running meeting app (ponytail: first-wins; add priority ordering if you run two at once).
private func runningMeetingApp(preferredKey: String = "", preferredBundle: String = "") -> (key: String, pid: pid_t)? {
    if !preferredBundle.isEmpty,
       let running = NSRunningApplication.runningApplications(withBundleIdentifier: preferredBundle).first {
        let key = MEETING_APPS.first(where: { $0.bundles.contains(preferredBundle) })?.key ?? preferredKey
        return (key.isEmpty ? "call" : key, running.processIdentifier)
    }
    for ra in NSWorkspace.shared.runningApplications {
        guard let b = ra.bundleIdentifier else { continue }
        for app in MEETING_APPS where app.bundles.contains(b) && (preferredKey.isEmpty || app.key == preferredKey) {
            return (app.key, ra.processIdentifier)
        }
    }
    return nil
}

// MARK: - state

struct ParticipantEvidence: Codable, Equatable {
    var name: String
    var source: String       // participant_list | video_tile | profile_button | active_speaker_tile | active_speaker_label
    var role: String         // source AX element role
    var context: String      // containing Zoom window title
}
struct IgnoredParticipantEvidence: Codable, Equatable {
    var text: String
    var source: String       // zoom_interface_text
    var role: String
    var context: String
    var reason: String
}
struct State: Codable, Equatable {
    var app = ""
    var title = ""
    var inMeeting = false
    var selfMuted: Bool? = nil     // true = your mic is muted
    var videoOn: Bool? = nil
    var sharing: Bool? = nil       // true = YOU are screen-sharing (hide the overlay!)
    var participants: [String] = []
    var participantEvidence: [ParticipantEvidence] = []
    var activeSpeaker: String? = nil
    var activeSpeakerEvidence: ParticipantEvidence? = nil
    var ignoredParticipantEvidence: [IgnoredParticipantEvidence] = []
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

private func usefulWindowTitle(_ appEl: AXUIElement, appKey: String) -> String {
    let generic = ["meeting", "zoom meeting", "microsoft teams", "webex", "facetime", "slack", "discord"]
    for window in (axGet(appEl, kWindows) as? [AXUIElement] ?? []) {
        var title = (axStr(window, kTitle) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        for suffix in [" - Google Chrome", " — Mozilla Firefox", " - Microsoft Edge", " — Safari", " | Microsoft Teams"] {
            if title.hasSuffix(suffix) { title.removeLast(suffix.count) }
        }
        let lower = title.lowercased()
        if title.count >= 3, title.count <= 120, !generic.contains(lower),
           !lower.contains("floating video window"), !lower.contains("meeting controls"),
           !lower.contains("audio settings"), !lower.contains("http") { return title }
    }
    return ""
}

private func snapshot(preferredKey: String = "", preferredBundle: String = "") -> State {
    var s = State()
    guard let (key, pid) = runningMeetingApp(preferredKey: preferredKey, preferredBundle: preferredBundle) else { return s }
    s.app = preferredKey.isEmpty || preferredKey == "call" ? key : preferredKey
    let appEl = AXUIElementCreateApplication(pid)
    if key == "zoom" { enableManualAX(appEl) }   // unlock Zoom's embedded UI for name reading
    s.title = usefulWindowTitle(appEl, appKey: s.app)
    deriveControls(from: menuTitles(appEl), into: &s)
    if windowInMeeting(appEl, appKey: key) { s.inMeeting = true }
    if key == "zoom" {
        let (names, evidence, active, activeEvidence, ignored) = zoomNames(appEl)
        s.participants = names
        s.participantEvidence = evidence
        s.activeSpeaker = active
        s.activeSpeakerEvidence = activeEvidence
        s.ignoredParticipantEvidence = ignored
        if !names.isEmpty { s.inMeeting = true }
    }
    return s
}

// MARK: - output

private struct Line: Encodable {
    let type: String
    var source: String? = nil
    var axTrusted: Bool? = nil
    var msg: String? = nil
    var ts: Int? = nil
    var state: State? = nil
    var speaker: String? = nil
    var text: String? = nil
    var isSelf: Bool? = nil
    enum CodingKeys: String, CodingKey {
        case type, source, axTrusted, msg, ts, state, speaker, text
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

private func participantSource(context: String, floating: Bool, profile: Bool) -> String {
    if context.lowercased().contains("participant") { return "participant_list" }
    if floating { return "active_speaker_tile" }
    return profile ? "profile_button" : "video_tile"
}

/// Walk the app's windows harvesting tile/profile names with provenance. The source AX role and
/// containing window title travel with every name, so downstream code can distinguish confirmed
/// participant-list evidence from active-speaker labels and ordinary video tiles. Tile-shaped Zoom
/// interface text is retained only as rejected evidence; it never enters the participant roster.
private func zoomNames(_ appEl: AXUIElement) -> (participants: [String], evidence: [ParticipantEvidence], active: String?, activeEvidence: ParticipantEvidence?, ignored: [IgnoredParticipantEvidence]) {
    var names: [String] = [], evidence: [ParticipantEvidence] = [], active: String? = nil
    var activeEvidence: ParticipantEvidence? = nil, ignored: [IgnoredParticipantEvidence] = []
    func add(_ ev: ParticipantEvidence) {
        if !names.contains(ev.name) { names.append(ev.name) }
        if !evidence.contains(ev) { evidence.append(ev) }
    }
    func reject(_ text: String, role: String, context: String) {
        guard ignored.count < 20 else { return }
        let item = IgnoredParticipantEvidence(text: String(text.prefix(160)), source: "zoom_interface_text", role: role, context: context, reason: "not_a_person")
        if !ignored.contains(item) { ignored.append(item) }
    }
    func walk(_ el: AXUIElement, floating: Bool, context: String, depth: Int) {
        if depth > 30 { return }
        let title = axStr(el, kTitle) ?? ""
        let nextContext = title.isEmpty ? context : title
        let inFloat = floating || nextContext.lowercased().contains("floating video")
        let role = axStr(el, kRole) ?? "unknown"
        if let d = axStr(el, kDesc) {
            let fromTile = nameFromTileDesc(d), fromProfile = nameFromProfileDesc(d)
            if let n = fromTile ?? fromProfile {
                let ev = ParticipantEvidence(name: n, source: participantSource(context: nextContext, floating: inFloat, profile: fromProfile != nil), role: role, context: nextContext)
                add(ev)
                if inFloat && active == nil { active = n; activeEvidence = ev } // first tile in the floating window = who's on screen
            } else {
                let low = d.lowercased()
                let tileShaped = d.contains(",") && (low.contains("audio") || low.contains("video") || low.contains("muted"))
                if tileShaped || d.hasPrefix("View ") { reject(d, role: role, context: nextContext) }
            }
        }
        if let v = axGet(el, kValue) as? String { // "Talking: X" — works even while sharing
            if let n = nameFromTalking(v) {
                let ev = ParticipantEvidence(name: n, source: "active_speaker_label", role: role, context: nextContext)
                add(ev); active = n; activeEvidence = ev
            } else if v.lowercased().hasPrefix("talking:") { reject(v, role: role, context: nextContext) }
        }
        for k in axKids(el) { walk(k, floating: inFloat, context: nextContext, depth: depth + 1) }
    }
    for w in (axGet(appEl, kWindows) as? [AXUIElement] ?? []) {
        let context = axStr(w, kTitle) ?? ""
        walk(w, floating: false, context: context, depth: 0)
    }
    return (names, evidence, active, activeEvidence, ignored)
}

// NOTE: live caption TEXT comes from on-device Whisper (see ../stt). This layer's job is meeting
// STATE — in-meeting / mute / share (from menu-bar labels) and participant/active-speaker NAMES.

// MARK: - meeting-only Accessibility observation

// meeting-tap publishes this internal distributed notification from its Core Audio call detector.
// It is intentionally distinct from a user-visible macOS notification: this event tells the AX
// enrichment layer when it may wake up. No real call means no AX tree reads at all.
private let meetingActivityNotification = Notification.Name("com.myassistant.desktop.meeting-activity")

private func axObserverCallback(
    _ observer: AXObserver,
    _ element: AXUIElement,
    _ notification: CFString,
    _ refcon: UnsafeMutableRawPointer?
) {
    guard let refcon else { return }
    Unmanaged<MeetingActivityMonitor>.fromOpaque(refcon).takeUnretainedValue().scheduleSnapshot()
}

private final class MeetingActivityMonitor {
    private let interval: TimeInterval
    private var activityToken: NSObjectProtocol?
    private var timer: Timer?
    private var active = false
    private var activeApp = ""
    private var activeBundle = ""
    private var lastHeartbeat = Date.distantPast
    private var lastState: State?
    private var permissionStatusSent = false
    private var observer: AXObserver?
    private var observerPID: pid_t = 0
    private var observedElement: AXUIElement?
    private var pendingSnapshot: DispatchWorkItem?

    init(interval: TimeInterval) {
        // Zoom's embedded Chromium layer does not emit every participant/speaker mutation through AX.
        // Keep an in-call reconciliation pass as a safety net, but never run it while idle.
        self.interval = max(interval, 2.0)
    }

    func run() -> Never {
        activityToken = DistributedNotificationCenter.default().addObserver(
            forName: meetingActivityNotification,
            object: nil,
            queue: .main
        ) { [weak self] note in self?.handleActivity(note) }

        timer = Timer.scheduledTimer(withTimeInterval: interval, repeats: true) { [weak self] _ in
            self?.reconcile()
        }
        RunLoop.main.run()
        fatalError("RunLoop exited unexpectedly")
    }

    private func handleActivity(_ note: Notification) {
        let on = (note.userInfo?["active"] as? NSNumber)?.boolValue
            ?? (note.userInfo?["active"] as? Bool)
            ?? false
        let app = (note.userInfo?["app"] as? String) ?? ""
        let bundle = (note.userInfo?["bundle"] as? String) ?? ""
        lastHeartbeat = Date()
        if !on {
            deactivate()
            return
        }

        let changed = !active || activeApp != app || activeBundle != bundle
        active = true
        activeApp = app
        activeBundle = bundle
        if changed {
            lastState = nil
            installObserverIfPossible()
            snapshotNow()
        }
    }

    private func reconcile() {
        if !parentIsAlive() { exit(0) }
        guard active else { return }
        // meeting-tap heartbeats every two seconds. If it dies or the server disables Meetings,
        // fail closed and put Accessibility back to sleep instead of scanning forever.
        if Date().timeIntervalSince(lastHeartbeat) > max(8.0, interval * 3) {
            deactivate()
            return
        }
        installObserverIfPossible()
        snapshotNow()
    }

    func scheduleSnapshot() {
        guard active else { return }
        pendingSnapshot?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.snapshotNow() }
        pendingSnapshot = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15, execute: work)
    }

    private func snapshotNow() {
        guard active else { return }
        guard axTrusted(prompt: false) else {
            removeObserver()
            if !permissionStatusSent {
                emit(Line(type: "status", source: "accessibility", axTrusted: false,
                          msg: "Accessibility off — speaker names disabled for this meeting"))
                permissionStatusSent = true
            }
            return
        }
        permissionStatusSent = false
        installObserverIfPossible()
        let state = snapshot(preferredKey: activeApp, preferredBundle: activeBundle)
        if state != lastState {
            emit(Line(type: "state", source: "accessibility", ts: nowMs(), state: state))
            lastState = state
        }
    }

    private func installObserverIfPossible() {
        guard active, axTrusted(prompt: false),
              let (_, pid) = runningMeetingApp(preferredKey: activeApp, preferredBundle: activeBundle) else {
            removeObserver()
            return
        }
        if observer != nil, observerPID == pid { return }
        removeObserver()

        var created: AXObserver?
        guard AXObserverCreate(pid, axObserverCallback, &created) == .success, let created else { return }
        let appElement = AXUIElementCreateApplication(pid)
        let refcon = Unmanaged.passUnretained(self).toOpaque()
        let names: [CFString] = [
            kAXWindowCreatedNotification as CFString,
            kAXFocusedWindowChangedNotification as CFString,
            kAXTitleChangedNotification as CFString,
            kAXValueChangedNotification as CFString,
            kAXSelectedChildrenChangedNotification as CFString,
            kAXUIElementDestroyedNotification as CFString,
        ]
        for name in names { _ = AXObserverAddNotification(created, appElement, name, refcon) }
        // Register the current windows too. App-level notifications cover lifecycle; window-level
        // notifications catch most mute/share/control changes in native meeting UIs.
        for window in (axGet(appElement, kWindows) as? [AXUIElement] ?? []) {
            for name in names { _ = AXObserverAddNotification(created, window, name, refcon) }
        }
        CFRunLoopAddSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(created), .defaultMode)
        observer = created
        observerPID = pid
        observedElement = appElement
    }

    private func removeObserver() {
        if let observer {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), .defaultMode)
        }
        observer = nil
        observerPID = 0
        observedElement = nil
    }

    private func deactivate() {
        guard active || lastState != nil else { return }
        active = false
        activeApp = ""
        activeBundle = ""
        pendingSnapshot?.cancel()
        pendingSnapshot = nil
        removeObserver()
        // Clear stale participant/mute/share metadata. The server deliberately ignores this helper's
        // inMeeting field; only Core Audio is allowed to end the actual meeting lifecycle.
        let empty = State()
        if lastState != empty {
            emit(Line(type: "state", source: "accessibility", ts: nowMs(), state: empty))
        }
        lastState = nil
    }
}

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
    check(participantSource(context: "Participants", floating: false, profile: true) == "participant_list", "participant panel provenance")
    check(participantSource(context: "zoom floating video window", floating: true, profile: false) == "active_speaker_tile", "active tile provenance")

    // 4. Detection runs without crashing
    _ = runningMeetingApp()
    check(State().title.isEmpty, "default title is empty")
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
    guard let (key, pid) = runningMeetingApp() else { err("No supported meeting app running."); exit(1) }
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
    emit(Line(type: "state", source: "accessibility", ts: nowMs(), state: snapshot()))
    exit(0)

case .watch:
    // Core Audio activity is the wake-up signal. AXObserver handles eventful native UI changes;
    // the meeting-only timer reconciles Zoom's embedded UI, which does not expose every mutation.
    let monitor = MeetingActivityMonitor(interval: interval)
    monitor.run()
}
