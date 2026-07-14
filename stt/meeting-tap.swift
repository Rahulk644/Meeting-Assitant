// meeting-tap.swift (v4) — continuous meeting transcription from TWO sources.
//   • Other participants: a Core Audio process tap (audio-only, "System Audio Recording Only" — NO Screen
//     Recording), scoped to the meeting app. ScreenCaptureKit is used instead only when recording video.
//   • Microphone   (AVAudioEngine)                                → you.
// Both → energy-VAD splits on natural pauses → WHOLE-utterance WAV → on-demand transcribe-server
// (transcribe.cpp on the Metal GPU) → caption POSTed to the GUI. Feeding whole utterances (not fixed-length
// chunks) is what makes it accurate — a mid-sentence cut severs context and the model hallucinates.
//
// Build:  swiftc -O -target arm64-apple-macos14.4 meeting-tap.swift -o meeting-tap \
//           -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia -framework CoreAudio -framework CoreImage \
//           -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker Info.plist
//
// Run:    meeting-tap <guiPort=8787> <whisperURL> [--app auto|all|<id>] [--mic on|off] [--me <name>]
//         meeting-tap --list-apps       # capturable apps + bundle ids (pick one for --app)
//
//   --app auto  (default) captures a dedicated meeting app, or a browser/chat app only when active audio
//               and meeting-window evidence agree. Background browser audio does not start recording.
//   --mic on    (default) also transcribes your own mic, labeled "<name> (Me)". Needs Microphone
//               permission on the launching terminal.  --mic off to skip it.
//   --me <name> your label (default: the macOS full name).  --rms / --mic-rms tune the VAD gates
//               (system 0.006, mic 0.02 — the mic is higher because a room is noisier than the feed).
//   --min-conf 0.5   drop utterances below this mean word-confidence (kills noise hallucinations).
//   --expressions    keep Whisper's *sigh* / (laughs) cues as [sigh] tags for a sentiment read.
//   --gate on|off    on (default) captures ONLY while you're in a call (Core Audio plus meeting-only AX
//                    evidence where needed — no Screen Recording); off = always capture (manual record).
//   NOTE: use HEADPHONES. On speakers, the mic re-hears the remote voices → duplicate/misattributed
//         lines. Headphones give a clean split: mic = you, system audio = them.
//
// ponytail: the SEGMENTER is a simple energy threshold — it only decides utterance BOUNDARIES (speech vs
// pause) and gates out sub-threshold silence/noise upstream; transcribe-server then does the anti-
// hallucination (temperature=0, no fallback) on what reaches it. The audio-only path uses Core Audio process
// taps (Fathom's per-app method, macOS 14.2+) so meetings never touch Screen Recording — that's video-only.

import Foundation
import ScreenCaptureKit
import AVFoundation
import CoreMedia
import CoreAudio       // Core Audio process taps (macOS 14.2+) — "System Audio Recording Only", no screen recording
import CoreImage       // downscaled first-frame JPEG for the meeting library; no bundled ffmpeg subprocess
import AppKit          // NSFullUserName (the "(Me)" label). Dictation's hotkey + ⌘V paste live in the app
                       // process itself (see desktop/src-tauri) — this binary only records meeting audio.
import ApplicationServices // meeting-only AX evidence for browser tabs and multipurpose call apps
import Darwin          // kill(pid, 0): fail closed if the parent app crashes instead of becoming an orphan

func err(_ s: String) { FileHandle.standardError.write((s + "\n").data(using: .utf8)!) }
func f2(_ v: Float) -> String { String(format: "%.4f", v) }
func f1(_ v: Double) -> String { String(format: "%.1f", v) }

// ---- args ----
let A = CommandLine.arguments
func opt(_ name: String, _ def: String) -> String {
    if let i = A.firstIndex(of: name), i + 1 < A.count { return A[i + 1] }
    return def
}
let LIST_APPS = A.contains("--list-apps")
let guiPort    = (A.count > 1 && !A[1].hasPrefix("--")) ? A[1] : "8787"
let whisperURL = (A.count > 2 && !A[2].hasPrefix("--")) ? A[2] : "http://127.0.0.1:8091/inference"
let APP_SEL = opt("--app", "auto")            // auto | all | <bundleID>
let MIC_ON  = opt("--mic", "on") != "off"
let SPEECH_RMS = Float(opt("--rms", "0.006")) ?? 0.006       // system-audio gate (the Zoom feed is clean)
let MIC_RMS    = Float(opt("--mic-rms", "0.02")) ?? 0.02     // mic gate — higher: a room is noisier than the feed
let MIN_CONF   = Float(opt("--min-conf", "0.5")) ?? 0.5      // drop utterances below this mean word-confidence
let EXPRESSIONS = A.contains("--expressions")                // keep *sigh*/(laughs) as [sigh] sentiment cues
let GATE = opt("--gate", "on") != "off"                      // capture ONLY while in a call (self-detected from the window list)
let VIDEO_MODE0 = opt("--video", "off")                      // off | always | share — server config overrides live
let REC_DIR = opt("--rec-dir", "data/meetings/rec")          // where screen recordings (.mp4) are written
let TEMP_DIR = opt("--temp-dir", FileManager.default.temporaryDirectory.path)
let GUI_TOKEN = ProcessInfo.processInfo.environment["ASSISTANT_GUI_TOKEN"] ?? opt("--gui-token", "")
let STT_TOKEN = ProcessInfo.processInfo.environment["ASSISTANT_STT_TOKEN"] ?? opt("--stt-token", "")
let PARENT_PID = pid_t(ProcessInfo.processInfo.environment["ASSISTANT_PARENT_PID"] ?? "") ?? 0
let VIDEO_FPS: Int32 = 10                                    // screen content doesn't need 30fps; 10 keeps files small
let ME = { let n = NSFullUserName(); return opt("--me", n.isEmpty ? "Me" : n) }()  // shown as "<name> (Me)"
try? FileManager.default.createDirectory(atPath: TEMP_DIR, withIntermediateDirectories: true,
                                         attributes: [.posixPermissions: 0o700])

// Keep control-plane HTTP inside this process. The previous implementation launched /usr/bin/curl
// every two seconds for configuration and again for every state/caption event. URLSession reuses
// connections and avoids thousands of short-lived processes during a long meeting.
let netSession: URLSession = {
    let cfg = URLSessionConfiguration.ephemeral
    cfg.timeoutIntervalForRequest = 2
    cfg.httpMaximumConnectionsPerHost = 2
    return URLSession(configuration: cfg)
}()

func postJSON(_ object: [String: Any], path: String = "/ingest") {
    guard let body = try? JSONSerialization.data(withJSONObject: object),
          let url = URL(string: "http://127.0.0.1:\(guiPort)\(path)") else { return }
    var req = URLRequest(url: url)
    req.httpMethod = "POST"
    req.setValue("application/json", forHTTPHeaderField: "Content-Type")
    if !GUI_TOKEN.isEmpty { req.setValue("Bearer \(GUI_TOKEN)", forHTTPHeaderField: "Authorization") }
    req.httpBody = body
    netSession.dataTask(with: req).resume()
}

// VAD / segmentation tuning
let SR = 16000.0
let SILENCE_HANGOVER = 0.7          // seconds of quiet that ends an utterance
let MIN_SEG = 0.5                   // ignore blips shorter than this
// Safety cap only — the REAL utterance boundary is SILENCE_HANGOVER (a natural pause). Whisper needs the
// whole utterance for context; chopping at 4 s severed words mid-sentence and made it hallucinate. Handy
// feeds the entire utterance (25 s is fine — under Whisper's 30 s window). Cut only a nonstop monologue.
let MAX_SEG = Double(opt("--max-seg", "25")) ?? 25.0

// Dedicated call apps are safe to gate from active Core Audio alone. Multipurpose communication apps
// and browsers need BOTH active audio and meeting-shaped Accessibility evidence, so a Slack notification
// or a YouTube tab cannot start a recording. If Accessibility is off, every platform remains available
// through the explicit Record button; only conservative auto-detection is reduced.
let DEDICATED_MEETING_APPS: [(key: String, bundles: Set<String>)] = [
    ("zoom", ["us.zoom.xos"]),
    ("teams", ["com.microsoft.teams", "com.microsoft.teams2"]),
    ("webex", ["com.cisco.webexmeetingsapp", "com.cisco.webex2"]),
    ("facetime", ["com.apple.FaceTime"]),
    ("goto", ["com.logmein.GoToMeeting", "com.goto.GoTo"]),
    ("ringcentral", ["com.ringcentral.glip", "com.ringcentral.RingCentral"]),
    ("around", ["com.getaround.Around"]),
    ("skype", ["com.skype.skype"]),
]
let EVIDENCE_MEETING_APPS: [(key: String, bundles: Set<String>)] = [
    ("slack", ["com.tinyspeck.slackmacgap"]),
    ("discord", ["com.hnc.Discord"]),
    ("whatsapp", ["net.whatsapp.WhatsApp"]),
    ("telegram", ["ru.keepcoder.Telegram"]),
    ("signal", ["org.whispersystems.signal-desktop"]),
]
let BROWSER_BUNDLES: Set<String> = [
    "com.apple.Safari", "com.google.Chrome", "com.google.Chrome.beta", "com.google.Chrome.canary",
    "com.microsoft.edgemac", "com.microsoft.edgemac.Beta", "com.microsoft.edgemac.Dev",
    "company.thebrowser.Browser", "com.brave.Browser", "org.mozilla.firefox", "org.chromium.Chromium",
    "com.operasoftware.Opera", "com.vivaldi.Vivaldi",
]
let KNOWN_CALL_BUNDLES = Set(DEDICATED_MEETING_APPS.flatMap(\.bundles)
    + EVIDENCE_MEETING_APPS.flatMap(\.bundles) + Array(BROWSER_BUNDLES))

func dedicatedKey(_ bundle: String) -> String? {
    DEDICATED_MEETING_APPS.first(where: { $0.bundles.contains(bundle) })?.key
}
func evidenceKey(_ bundle: String) -> String? {
    EVIDENCE_MEETING_APPS.first(where: { $0.bundles.contains(bundle) })?.key
}
func axText(_ element: AXUIElement, _ attribute: CFString) -> String? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, attribute, &value) == .success else { return nil }
    if let string = value as? String { return string }
    if let url = value as? URL { return url.absoluteString }
    return nil
}
func axChildren(_ element: AXUIElement) -> [AXUIElement] {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &value) == .success else { return [] }
    return value as? [AXUIElement] ?? []
}
func collectMeetingEvidence(_ element: AXUIElement, depth: Int, values: inout [String]) {
    guard depth >= 0, values.count < 120 else { return }
    for attribute in [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute, kAXDocumentAttribute, kAXURLAttribute] {
        if let value = axText(element, attribute as CFString), !value.isEmpty { values.append(value) }
    }
    if depth > 0 {
        for child in axChildren(element).prefix(40) {
            collectMeetingEvidence(child, depth: depth - 1, values: &values)
            if values.count >= 120 { break }
        }
    }
}
func meetingWindowEvidence(_ bundle: String) -> (text: String, titles: [String])? {
    guard AXIsProcessTrusted(),
          let app = NSRunningApplication.runningApplications(withBundleIdentifier: bundle).first else { return nil }
    let appElement = AXUIElementCreateApplication(app.processIdentifier)
    var rawWindows: CFTypeRef?
    guard AXUIElementCopyAttributeValue(appElement, kAXWindowsAttribute as CFString, &rawWindows) == .success,
          let windows = rawWindows as? [AXUIElement] else { return nil }
    var values: [String] = [], titles: [String] = []
    for window in windows.prefix(12) {
        if let title = axText(window, kAXTitleAttribute as CFString), !title.isEmpty { titles.append(title) }
        collectMeetingEvidence(window, depth: 3, values: &values)
    }
    return (values.joined(separator: "\n").lowercased(), titles)
}
func platformFromEvidence(_ text: String, bundle: String) -> String? {
    let text = text.lowercased()
    if text.contains("meet.google.com/") || text.contains("google meet") { return "meet" }
    if text.contains("zoom.us/wc/") || text.contains("zoom.us/j/") || text.contains("zoom meeting") { return "zoom" }
    if (text.contains("teams.microsoft.com") && (text.contains("meeting") || text.contains("call")))
        || text.contains("microsoft teams meeting") { return "teams" }
    if text.contains("webex.com/meet") || text.contains("webex meeting") { return "webex" }
    if text.contains("slack huddle") || text.contains("in a huddle") || text.contains("huddle window") { return "slack" }
    if text.contains("discord") && (text.contains("voice connected") || text.contains("in call") || text.contains("call with")) { return "discord" }
    if text.contains("meet.jit.si/") || text.contains("jitsi meet") { return "jitsi" }
    if text.contains("whereby.com/") || text.contains("whereby meeting") { return "whereby" }
    if text.contains("chime.aws/") || text.contains("amazon chime") { return "chime" }
    if text.contains("app.gather.town/") || text.contains("gather meeting") { return "gather" }
    if text.contains("riverside.fm/studio") || text.contains("riverside studio") { return "riverside" }
    if text.contains("around.co/") || text.contains("around meeting") { return "around" }
    if text.contains("butter.us/") || text.contains("butter session") { return "butter" }
    if text.contains("call") || text.contains("voice chat") {
        return evidenceKey(bundle)
    }
    return nil
}
func usefulMeetingTitle(_ titles: [String], key: String) -> String {
    let generic = ["meeting", "zoom meeting", "microsoft teams", "webex", "facetime", "slack", "discord"]
    for raw in titles {
        var title = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        for suffix in [" - Google Chrome", " — Mozilla Firefox", " - Microsoft Edge", " — Safari", " | Microsoft Teams"] {
            if title.hasSuffix(suffix) { title.removeLast(suffix.count) }
        }
        let lower = title.lowercased()
        if title.count >= 3, title.count <= 120, !generic.contains(lower), !lower.contains("floating video window"),
           !lower.contains("meeting controls"), !lower.contains("audio settings"), !lower.contains("http") {
            return title
        }
    }
    return ""
}

if A.contains("--selftest") {
    func check(_ condition: Bool, _ message: String) {
        if !condition { err("SELFTEST FAIL: \(message)"); exit(1) }
    }
    check(platformFromEvidence("https://meet.google.com/abc-defg-hij", bundle: "com.google.Chrome") == "meet", "Google Meet URL")
    check(platformFromEvidence("YouTube — Google Chrome", bundle: "com.google.Chrome") == nil, "ordinary browser audio rejected")
    check(platformFromEvidence("Slack huddle window", bundle: "com.tinyspeck.slackmacgap") == "slack", "Slack huddle")
    check(platformFromEvidence("Discord voice connected", bundle: "com.hnc.Discord") == "discord", "Discord voice")
    check(usefulMeetingTitle(["Zoom Meeting"], key: "zoom").isEmpty, "generic title rejected")
    check(usefulMeetingTitle(["Customer launch review - Google Chrome"], key: "meet") == "Customer launch review", "browser suffix removed")
    err("SELFTEST OK")
    exit(0)
}

// MARK: - one energy-VAD segmenter → whisper → caption. Reused by both audio sources.

final class Segmenter {
    let speaker: String, isSelf: Bool, tag: String, threshold: Float
    var seg = [Float](), inSpeech = false, silentDur = 0.0, segIdx = 0
    var bufN = 0; var peak: Float = 0; var announced = false
    let procQ: DispatchQueue

    init(speaker: String, isSelf: Bool, tag: String, threshold: Float) {
        self.speaker = speaker; self.isSelf = isSelf; self.tag = tag; self.threshold = threshold
        self.procQ = DispatchQueue(label: "stt.process.\(tag)")
    }
    func reset() { seg.removeAll(keepingCapacity: true); inSpeech = false; silentDur = 0 }   // between calls

    func feed(_ s: [Float]) {
        guard !s.isEmpty else { return }
        let dur = Double(s.count) / SR
        let rms = (s.reduce(0) { $0 + $1 * $1 } / Float(s.count)).squareRoot()
        if !announced { announced = true; err("\(tag): first buffer \(s.count) samples rms=\(f2(rms)) (threshold=\(f2(threshold)))") }
        bufN += 1; peak = max(peak, rms)
        if bufN % 500 == 0 { err("\(tag): · listening (peak \(f2(peak)))"); peak = 0 }
        if rms > threshold {
            if !inSpeech { inSpeech = true; seg.removeAll(keepingCapacity: true) }
            seg.append(contentsOf: s); silentDur = 0
        } else if inSpeech {
            seg.append(contentsOf: s); silentDur += dur
            if silentDur >= SILENCE_HANGOVER { flush() }
        }
        if inSpeech && Double(seg.count) / SR >= MAX_SEG { flush() }
    }

    func flush() {
        let samples = seg; seg.removeAll(keepingCapacity: true); inSpeech = false; silentDur = 0
        guard Double(samples.count) / SR >= MIN_SEG else { return }
        segIdx += 1; let idx = segIdx
        procQ.async { self.process(samples, idx) }
    }

    func process(_ samples: [Float], _ idx: Int) {
        let wav = URL(fileURLWithPath: TEMP_DIR).appendingPathComponent("meeting-\(tag)-\(idx)-\(UUID().uuidString).wav")
        writeWav16(samples, to: wav)
        defer { try? FileManager.default.removeItem(at: wav) }
        let secs = f1(Double(samples.count) / SR)
        guard let (raw, conf) = transcribe(wav.path) else { return }
        if conf < MIN_CONF {   // low mean word-probability = noise/hallucination (the "*Sigh*" case)
            err("\(tag) seg#\(idx) \(secs)s dropped: confidence below threshold")
            return
        }
        let clean = EXPRESSIONS ? keepExpressions(raw) : cleanTranscript(raw)
        err("\(tag) seg#\(idx) \(secs)s accepted (\(clean.count) characters)")
        guard clean.count > 1 else { return }
        postCaption(clean)
    }

    func postCaption(_ text: String) {
        let obj: [String: Any] = ["type": "caption", "speaker": speaker, "text": text, "self": isSelf,
                                  "ts": Int(Date().timeIntervalSince1970 * 1000)]
        postJSON(obj)
    }
}

// MARK: - system audio (ScreenCaptureKit) → segmenter

final class SCKOut: NSObject, SCStreamOutput, SCStreamDelegate {
    let seg: Segmenter
    init(_ seg: Segmenter) { self.seg = seg }
    func stream(_ stream: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, CMSampleBufferDataIsReady(sb), let s = floatSamples(sb) else { return }
        seg.feed(s)
    }
    func stream(_ stream: SCStream, didStopWithError error: Error) { err("sys stream stopped: \(error)") }
}

func floatSamples(_ sb: CMSampleBuffer) -> [Float]? {
    guard let fmt = CMSampleBufferGetFormatDescription(sb),
          let asbdP = CMAudioFormatDescriptionGetStreamBasicDescription(fmt) else { return nil }
    var asbd = asbdP.pointee
    guard let avf = AVAudioFormat(streamDescription: &asbd) else { return nil }
    let n = CMSampleBufferGetNumSamples(sb)
    guard n > 0, let pcm = AVAudioPCMBuffer(pcmFormat: avf, frameCapacity: AVAudioFrameCount(n)) else { return nil }
    pcm.frameLength = AVAudioFrameCount(n)
    guard CMSampleBufferCopyPCMDataIntoAudioBufferList(sb, at: 0, frameCount: Int32(n), into: pcm.mutableAudioBufferList) == noErr else { return nil }
    if let fc = pcm.floatChannelData { return Array(UnsafeBufferPointer(start: fc[0], count: Int(n))) }
    if let ic = pcm.int16ChannelData { return (0..<Int(n)).map { Float(ic[0][$0]) / 32768.0 } }   // Int16 SCK buffers
    return nil
}

// MARK: - microphone (AVAudioEngine) → segmenter.  Resamples hardware rate → 16k mono.

final class Mic {
    let engine = AVAudioEngine()
    let seg: Segmenter
    init(_ seg: Segmenter) { self.seg = seg }
    func start() {
        let input = engine.inputNode
        let inFmt = input.inputFormat(forBus: 0)
        guard inFmt.sampleRate > 0, inFmt.channelCount > 0 else {
            err("mic: no input format — grant Microphone permission to your terminal, then rerun (or --mic off)"); return
        }
        guard let outFmt = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: SR, channels: 1, interleaved: false),
              let conv = AVAudioConverter(from: inFmt, to: outFmt) else { err("mic: converter init failed"); return }
        input.installTap(onBus: 0, bufferSize: 4096, format: inFmt) { [weak self] buf, _ in
            guard let self = self else { return }
            let cap = AVAudioFrameCount(Double(buf.frameLength) * SR / inFmt.sampleRate) + 1024
            guard let out = AVAudioPCMBuffer(pcmFormat: outFmt, frameCapacity: cap) else { return }
            var e: NSError?; var fed = false
            conv.convert(to: out, error: &e) { _, st in
                if fed { st.pointee = .noDataNow; return nil }
                fed = true; st.pointee = .haveData; return buf
            }
            if let ch = out.floatChannelData, out.frameLength > 0 {
                self.seg.feed(Array(UnsafeBufferPointer(start: ch[0], count: Int(out.frameLength))))
            }
        }
        engine.prepare()
        do { try engine.start(); err(">> mic capture started (\(Int(inFmt.sampleRate))Hz \(inFmt.channelCount)ch → 16k mono)") }
        catch { err("mic: engine.start failed: \(error) — grant Microphone permission, or --mic off") }
    }
    func stop() {
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        err(">> mic capture stopped (mic released)")
    }
}

// MARK: - system audio via a Core Audio process tap (macOS 14.2+) → segmenter.
// Captures the meeting app's OUTPUT (the other participants) WITHOUT Screen Recording — this is the
// "System Audio Recording Only" TCC category (Fathom's audio-only path). ScreenCaptureKit is used ONLY when
// we also record video. Chain: tap the app's process → wrap in a PRIVATE aggregate device → read via an IO
// block → resample to 16k mono → the shared Segmenter. Call detection (below) reads the same process list,
// so an audio-only meeting needs NEITHER Screen Recording to hear others NOR to know you're in a call.

let CA_SYS = AudioObjectID(kAudioObjectSystemObject)
func caAddr(_ sel: AudioObjectPropertySelector, _ scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress(mSelector: sel, mScope: scope, mElement: kAudioObjectPropertyElementMain)
}
func caProcessList() -> [AudioObjectID] {
    var a = caAddr(kAudioHardwarePropertyProcessObjectList); var size: UInt32 = 0
    guard AudioObjectGetPropertyDataSize(CA_SYS, &a, 0, nil, &size) == noErr, size > 0 else { return [] }
    var ids = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
    guard AudioObjectGetPropertyData(CA_SYS, &a, 0, nil, &size, &ids) == noErr else { return [] }
    return ids
}
func caString(_ obj: AudioObjectID, _ sel: AudioObjectPropertySelector) -> String? {
    var a = caAddr(sel); var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size); var out: Unmanaged<CFString>?
    guard AudioObjectGetPropertyData(obj, &a, 0, nil, &size, &out) == noErr, let u = out else { return nil }
    return u.takeRetainedValue() as String
}
func caBool(_ obj: AudioObjectID, _ sel: AudioObjectPropertySelector) -> Bool {
    var a = caAddr(sel); var size = UInt32(MemoryLayout<UInt32>.size); var v: UInt32 = 0
    guard AudioObjectGetPropertyData(obj, &a, 0, nil, &size, &v) == noErr else { return false }
    return v != 0
}
func caAOIDForPID(_ pid: pid_t) -> AudioObjectID? {
    var a = caAddr(kAudioHardwarePropertyTranslatePIDToProcessObject)
    var inPID = pid; var out = AudioObjectID(kAudioObjectUnknown); var size = UInt32(MemoryLayout<AudioObjectID>.size)
    let st = AudioObjectGetPropertyData(CA_SYS, &a, UInt32(MemoryLayout<pid_t>.size), &inPID, &size, &out)
    return (st == noErr && out != kAudioObjectUnknown) ? out : nil
}
func caDeviceInputFormat(_ dev: AudioObjectID) -> AudioStreamBasicDescription? {
    var a = caAddr(kAudioDevicePropertyStreamFormat, kAudioObjectPropertyScopeInput)
    var f = AudioStreamBasicDescription(); var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    guard AudioObjectGetPropertyData(dev, &a, 0, nil, &size, &f) == noErr else { return nil }
    return f
}

final class ProcessTap {
    private var tapID = AudioObjectID(kAudioObjectUnknown)
    private var aggID = AudioObjectID(kAudioObjectUnknown)
    private var procID: AudioDeviceIOProcID?
    private var conv: AVAudioConverter?
    private var srcFmt: AVAudioFormat?
    private let outFmt = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: SR, channels: 1, interleaved: false)!
    let seg: Segmenter
    init(_ seg: Segmenter) { self.seg = seg }

    // aoids: the process object IDs to capture (the meeting app). Empty = global (all output except ours).
    func start(aoids: [AudioObjectID]) -> Bool {
        let desc: CATapDescription = aoids.isEmpty
            ? CATapDescription(monoGlobalTapButExcludeProcesses: caAOIDForPID(getpid()).map { [$0] } ?? [])
            : CATapDescription(monoMixdownOfProcesses: aoids)
        desc.name = "My Assistant meeting audio"; desc.isPrivate = true; desc.muteBehavior = .unmuted

        var st = AudioHardwareCreateProcessTap(desc, &tapID)
        guard st == noErr, tapID != kAudioObjectUnknown else { err("tap: create failed (\(st)) — grant 'System Audio Recording Only' to My Assistant"); return false }
        guard let uid = caString(tapID, kAudioTapPropertyUID) else { err("tap: no UID"); stop(); return false }
        let dict: [String: Any] = [
            kAudioAggregateDeviceNameKey: "My Assistant Tap",
            kAudioAggregateDeviceUIDKey: "com.myassistant.tap.\(getpid())",
            kAudioAggregateDeviceIsPrivateKey: true,
            kAudioAggregateDeviceTapAutoStartKey: true,
            kAudioAggregateDeviceTapListKey: [[kAudioSubTapUIDKey: uid, kAudioSubTapDriftCompensationKey: true]],
        ]
        st = AudioHardwareCreateAggregateDevice(dict as CFDictionary, &aggID)
        guard st == noErr, aggID != kAudioObjectUnknown else { err("tap: aggregate device failed (\(st))"); stop(); return false }
        guard var asbd = caDeviceInputFormat(aggID), let src = AVAudioFormat(streamDescription: &asbd),
              let c = AVAudioConverter(from: src, to: outFmt) else { err("tap: converter init failed"); stop(); return false }
        srcFmt = src; conv = c
        st = AudioDeviceCreateIOProcIDWithBlock(&procID, aggID, nil) { [weak self] _, inData, _, _, _ in
            guard let self = self, let src = self.srcFmt, let conv = self.conv else { return }
            let abl = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: inData))
            guard abl.count > 0, let mdata = abl[0].mData, abl[0].mDataByteSize > 0 else { return }
            let bytes = Int(abl[0].mDataByteSize)
            let frames = bytes / (MemoryLayout<Float>.size * Int(max(1, src.channelCount)))
            guard frames > 0, let inBuf = AVAudioPCMBuffer(pcmFormat: src, frameCapacity: AVAudioFrameCount(frames)) else { return }
            inBuf.frameLength = AVAudioFrameCount(frames)
            if let dst = inBuf.floatChannelData { memcpy(dst[0], mdata, bytes) }
            let cap = AVAudioFrameCount(Double(frames) * SR / src.sampleRate) + 1024
            guard let ob = AVAudioPCMBuffer(pcmFormat: self.outFmt, frameCapacity: cap) else { return }
            var e: NSError?; var fed = false
            conv.convert(to: ob, error: &e) { _, s in if fed { s.pointee = .noDataNow; return nil }; fed = true; s.pointee = .haveData; return inBuf }
            if let ch = ob.floatChannelData, ob.frameLength > 0 {
                self.seg.feed(Array(UnsafeBufferPointer(start: ch[0], count: Int(ob.frameLength))))
            }
        }
        guard st == noErr, procID != nil else { err("tap: IOProc create failed (\(st))"); stop(); return false }
        st = AudioDeviceStart(aggID, procID)
        guard st == noErr else { err("tap: device start failed (\(st))"); stop(); return false }
        err(">> system-audio tap started (Core Audio — no screen recording) \(Int(src.sampleRate))Hz \(src.channelCount)ch")
        return true
    }
    func stop() {
        if let p = procID { AudioDeviceStop(aggID, p); AudioDeviceDestroyIOProcID(aggID, p); procID = nil }
        if aggID != kAudioObjectUnknown { AudioHardwareDestroyAggregateDevice(aggID); aggID = kAudioObjectUnknown }
        if tapID != kAudioObjectUnknown { AudioHardwareDestroyProcessTap(tapID); tapID = kAudioObjectUnknown }
        conv = nil; srcFmt = nil
    }
}

// MARK: - screen recording (ScreenCaptureKit video → H.264 .mp4 via AVAssetWriter)

final class VideoRecorder {
    let url: URL
    let thumbnailURL: URL
    private var writer: AVAssetWriter?
    private var input: AVAssetWriterInput?
    private var started = false
    private var wroteThumbnail = false
    private(set) var wroteAny = false
    var record = false                          // gated by mode (always) / share-state (share)
    private let q = DispatchQueue(label: "video.write")

    init(dir: String, name: String) {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        url = URL(fileURLWithPath: (dir as NSString).appendingPathComponent(name))
        thumbnailURL = url.deletingPathExtension().appendingPathExtension("jpg")
        try? FileManager.default.removeItem(at: url)
        try? FileManager.default.removeItem(at: thumbnailURL)
    }
    func append(_ sb: CMSampleBuffer) {
        guard record, CMSampleBufferDataIsReady(sb) else { return }
        q.sync {
            if writer == nil { setup(sb) }
            guard let w = writer, let inp = input, w.status == .writing, inp.isReadyForMoreMediaData else { return }
            if !started { w.startSession(atSourceTime: CMSampleBufferGetPresentationTimeStamp(sb)); started = true }
            inp.append(sb); wroteAny = true
            if !wroteThumbnail { writeThumbnail(sb) }
        }
    }
    private func writeThumbnail(_ sb: CMSampleBuffer) {
        guard let pixelBuffer = CMSampleBufferGetImageBuffer(sb) else { return }
        let source = CIImage(cvPixelBuffer: pixelBuffer)
        let width = max(1, source.extent.width)
        let scale = min(1, 480 / width)
        let image = source.transformed(by: CGAffineTransform(scaleX: scale, y: scale))
        let context = CIContext(options: [.cacheIntermediates: false])
        guard let cg = context.createCGImage(image, from: image.extent) else { return }
        let bitmap = NSBitmapImageRep(cgImage: cg)
        guard let jpeg = bitmap.representation(using: .jpeg, properties: [.compressionFactor: 0.76]) else { return }
        do {
            try jpeg.write(to: thumbnailURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: thumbnailURL.path)
            wroteThumbnail = true
        } catch {
            err("video: thumbnail write failed: \(error)")
        }
    }
    private func setup(_ sb: CMSampleBuffer) {
        guard let fmt = CMSampleBufferGetFormatDescription(sb) else { return }
        let dim = CMVideoFormatDescriptionGetDimensions(fmt)
        guard let w = try? AVAssetWriter(url: url, fileType: .mp4) else { err("video: writer init failed"); return }
        // Screen content is mostly static → cap at 4 Mbps with sparse keyframes; hardware H.264 (VideoToolbox,
        // the default on Apple Silicon) does the encode. Keeps a long-meeting file small and the encoder light.
        let compression: [String: Any] = [
            AVVideoAverageBitRateKey: 4_000_000,
            AVVideoMaxKeyFrameIntervalKey: Int(VIDEO_FPS) * 10,   // a keyframe every ~10 s
            AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
        ]
        let s: [String: Any] = [AVVideoCodecKey: AVVideoCodecType.h264,
                                AVVideoWidthKey: Int(dim.width), AVVideoHeightKey: Int(dim.height),
                                AVVideoCompressionPropertiesKey: compression]
        let inp = AVAssetWriterInput(mediaType: .video, outputSettings: s)
        inp.expectsMediaDataInRealTime = true
        if w.canAdd(inp) { w.add(inp) }
        // Flush a movie fragment every 10 s: RAM stays flat over a long call, and a half-written file (hard
        // crash / power loss) is still playable up to the last fragment — "record in chunks", but one file.
        w.movieFragmentInterval = CMTime(value: 10, timescale: 1)
        w.startWriting()
        writer = w; input = inp
        err(">> screen recording → \(url.lastPathComponent) (\(dim.width)x\(dim.height)) @\(VIDEO_FPS)fps ≤4Mbps")
    }
    func finish(_ done: @escaping (Bool) -> Void) {
        q.async {
            guard let w = self.writer, self.started, self.wroteAny else { done(false); return }
            self.input?.markAsFinished()
            w.finishWriting { done(w.status == .completed) }
        }
    }
}

final class VideoOut: NSObject, SCStreamOutput {
    let rec: VideoRecorder
    init(_ rec: VideoRecorder) { self.rec = rec }
    func stream(_ s: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen else { return }
        // append only "complete" frames (skip idle/blank ones SCK emits when nothing changed)
        guard let att = CMSampleBufferGetSampleAttachmentsArray(sb, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let raw = att.first?[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete else { return }
        rec.append(sb)
    }
}

// MARK: - shared helpers

func writeWav16(_ samples: [Float], to url: URL) {
    let dataSize = samples.count * 2
    func le32(_ v: UInt32) -> [UInt8] { [UInt8(v & 0xff), UInt8((v >> 8) & 0xff), UInt8((v >> 16) & 0xff), UInt8((v >> 24) & 0xff)] }
    func le16(_ v: UInt16) -> [UInt8] { [UInt8(v & 0xff), UInt8((v >> 8) & 0xff)] }
    var d = Data()
    d.append(contentsOf: Array("RIFF".utf8)); d.append(contentsOf: le32(UInt32(36 + dataSize)))
    d.append(contentsOf: Array("WAVE".utf8)); d.append(contentsOf: Array("fmt ".utf8))
    d.append(contentsOf: le32(16)); d.append(contentsOf: le16(1)); d.append(contentsOf: le16(1))   // PCM, mono
    d.append(contentsOf: le32(16000)); d.append(contentsOf: le32(32000))                            // rate, byte-rate
    d.append(contentsOf: le16(2)); d.append(contentsOf: le16(16))                                   // block align, bits
    d.append(contentsOf: Array("data".utf8)); d.append(contentsOf: le32(UInt32(dataSize)))
    d.reserveCapacity(44 + dataSize)
    for f in samples { d.append(contentsOf: le16(UInt16(bitPattern: Int16(max(-1, min(1, f)) * 32767)))) }
    _ = FileManager.default.createFile(atPath: url.path, contents: nil, attributes: [.posixPermissions: 0o600])
    if let handle = try? FileHandle(forWritingTo: url) {
        try? handle.write(contentsOf: d)
        try? handle.close()
    }
}

// Ask whisper-server for verbose_json → (full text, mean per-word confidence). Whisper reports a
// probability per word; averaged, it separates real speech (~0.9) from noise/hallucination (low).
func transcribe(_ wavPath: String) -> (text: String, conf: Float)? {
    // Two independent knobs: `language` = DETECTION (default `auto` — Rahul speaks Hindi + English, so let
    // Whisper detect per utterance), `translate` = OUTPUT (default on → English). The old "auto hallucinates
    // a wrong script" (کن کن کن) was really our 4 s fragments starving detection — on whole utterances `auto`
    // is accurate (A/B-verified against Handy's own WAVs). Both overridable in Settings → Meetings.
    var args = ["-s", whisperURL]
    if !STT_TOKEN.isEmpty { args += ["-H", "Authorization: Bearer \(STT_TOKEN)"] }
    args += ["-F", "file=@\(wavPath)", "--form-string", "temperature=0", "--form-string", "response_format=verbose_json", "--form-string", "language=\(curLanguage)"]
    if curTranslate { args += ["--form-string", "translate=true"] }
    if !curVocab.isEmpty { args += ["--form-string", "prompt=\(curVocab)"] } // form-string prevents @file expansion
    let out = curl(args)
    guard let data = out.data(using: .utf8),
          let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
    let text = (obj["text"] as? String) ?? ""
    var probs: [Double] = []
    for seg in (obj["segments"] as? [[String: Any]] ?? []) {
        for w in (seg["words"] as? [[String: Any]] ?? []) {
            if let p = w["probability"] as? Double { probs.append(p) }
        }
    }
    let conf = probs.isEmpty ? 1.0 : probs.reduce(0, +) / Double(probs.count)   // no word probs → don't penalize
    return (text, Float(conf))
}

// Expression capture (--expressions): keep Whisper's paralinguistic cues (*sigh*, (laughs)) as [sigh]
// tags for a sentiment read, instead of stripping them. Trustworthy only because low-confidence noise
// is already dropped upstream. ponytail: a cheap proxy — real emotion sensing wants a dedicated model.
func keepExpressions(_ raw: String) -> String {
    var t = raw.replacingOccurrences(of: "[BLANK_AUDIO]", with: "")
    t = t.replacingOccurrences(of: "\\*([^*]+)\\*", with: "[$1]", options: .regularExpression)
    t = t.replacingOccurrences(of: "\\(([^)]+)\\)", with: "[$1]", options: .regularExpression)
    return t.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
}

// Whisper hallucinates non-speech as annotations on quiet/noisy audio: [BLANK_AUDIO], *sigh*,
// (music), *click*. Strip those; drop whatever's left with no letters/digits (e.g. "- -", ".").
func cleanTranscript(_ raw: String) -> String {
    var t = raw
    for pat in ["\\[[^\\]]*\\]", "\\*[^*]*\\*", "\\([^)]*\\)"] {
        t = t.replacingOccurrences(of: pat, with: " ", options: .regularExpression)
    }
    t = t.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
         .trimmingCharacters(in: .whitespacesAndNewlines)
    return t.range(of: "[\\p{L}\\p{N}]", options: .regularExpression) == nil ? "" : t
}

@discardableResult
func curl(_ argv: [String]) -> String {
    let p = Process(); p.executableURL = URL(fileURLWithPath: "/usr/bin/curl"); p.arguments = argv
    let out = Pipe(); p.standardOutput = out; p.standardError = FileHandle.nullDevice
    do { try p.run() } catch { return "" }
    p.waitUntilExit()
    return String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
}

// MARK: - content filter (which app's audio to capture)

func makeFilter(_ display: SCDisplay, _ apps: [SCRunningApplication], bundle: String) -> SCContentFilter {
    let whole = SCContentFilter(display: display, excludingWindows: [])
    switch APP_SEL {
    case "all":
        err("capturing full system audio (--app all)")
        return whole
    case "auto":
        if let m = apps.first(where: { $0.bundleIdentifier == bundle }) {
            err("auto: scoping audio to \(m.applicationName) [\(m.bundleIdentifier)]")
            return SCContentFilter(display: display, including: [m], exceptingWindows: [])
        }
        err("auto: detected meeting app is unavailable to ScreenCaptureKit — capturing full system audio")
        return whole
    default:
        if let m = apps.first(where: { $0.bundleIdentifier == APP_SEL }) {
            err("scoping audio to \(m.applicationName) [\(APP_SEL)]")
            return SCContentFilter(display: display, including: [m], exceptingWindows: [])
        }
        err("app \(APP_SEL) not running — capturing full system audio")
        return whole
    }
}

// MARK: - start

let sysSeg = Segmenter(speaker: "Speaker", isSelf: false, tag: "sys", threshold: SPEECH_RMS)
let micSeg = Segmenter(speaker: ME, isSelf: true, tag: "mic", threshold: MIC_RMS)

// --list-apps: enumerate capturable apps + bundle ids, then exit.
if LIST_APPS {
    SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) { content, _ in
        for a in (content?.applications ?? []).sorted(by: { $0.applicationName.lowercased() < $1.applicationName.lowercased() }) {
            print("\(a.bundleIdentifier)\t\(a.applicationName)")
        }
        exit(0)
    }
    RunLoop.main.run()
}

// Capture is GATED on being in a real call: the mic + system taps run only while we self-detect a call
// in the ScreenCaptureKit window list (see detectMeetingApp), and are torn down (releasing the mic)
// otherwise — so nothing is captured when you're not in a meeting. `--gate off` = always capture
// (Google Meet, whose window we can't yet recognize).
final class Capture {
    var stream: SCStream?, sckOut: SCKOut?, videoOut: VideoOut?, rec: VideoRecorder?, mic: Mic?, tap: ProcessTap?
    var running = false, mode = "off", recFile = "", targetBundle = ""
    // aoids: the meeting app's Core Audio process objects (for the audio-only tap). Empty = global.
    func start(mode: String, aoids: [AudioObjectID], bundle: String) {
        guard !running else { return }
        running = true; self.mode = mode; targetBundle = bundle
        let wantVideo = mode == "always" || mode == "share"
        err(">> in a call — starting capture (video: \(mode))")
        sysSeg.reset(); micSeg.reset()
        if MIC_ON { let m = Mic(micSeg); m.start(); mic = m }
        // Two paths, each asking for the MINIMUM permission (Fathom's model):
        //   audio-only → Core Audio tap  → "System Audio Recording Only"      (default; no Screen Recording)
        //   video      → ScreenCaptureKit → "Screen & System Audio Recording"  (only when you record video)
        if wantVideo { startSCK(bundle: bundle) } else { startTap(aoids) }
    }
    private func startTap(_ aoids: [AudioObjectID]) {
        let t = ProcessTap(sysSeg)
        if t.start(aoids: aoids) { self.tap = t; err(">> capture started (audio-only, Core Audio tap)") }
        else { self.tap = nil }   // start() logged why; the mic still runs, and the next tick re-arms the tap
    }
    private func startSCK(bundle: String) {
        SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) { content, error in
            guard self.running else { return }                 // call ended during the async fetch
            if let error = error { err("SCShareableContent error: \(error) — grant Screen Recording (video mode)"); return }
            guard let display = content?.displays.first else { err("no display"); return }
            let cfg = SCStreamConfiguration()
            cfg.capturesAudio = true
            cfg.excludesCurrentProcessAudio = false            // we play no audio; excluding wedges capture
            cfg.sampleRate = 16000; cfg.channelCount = 1
            // Cap capture at 1080p tall (preserve aspect, even dims for H.264). A meeting recording doesn't
            // need full retina, and fewer pixels = less capture bandwidth AND less encoder work on long calls.
            let sc = min(1.0, 1080.0 / Double(display.height))
            cfg.width  = Int(Double(display.width)  * sc) & ~1
            cfg.height = Int(Double(display.height) * sc) & ~1
            cfg.minimumFrameInterval = CMTime(value: 1, timescale: VIDEO_FPS)
            cfg.pixelFormat = kCVPixelFormatType_32BGRA
            cfg.queueDepth = 6
            let out = SCKOut(sysSeg)
            do {
                let stream = SCStream(filter: makeFilter(display, content?.applications ?? [], bundle: bundle), configuration: cfg, delegate: out)
                try stream.addStreamOutput(out, type: .audio, sampleHandlerQueue: DispatchQueue(label: "audio"))
                let stamp = Int(Date().timeIntervalSince1970 * 1000)
                let file = "\(stamp).mp4"
                let r = VideoRecorder(dir: REC_DIR, name: file)
                r.record = (self.mode == "always") ? true : curSharing   // share mode: only while a screen is shared
                let vout = VideoOut(r)
                try stream.addStreamOutput(vout, type: .screen, sampleHandlerQueue: DispatchQueue(label: "video"))
                self.rec = r; self.videoOut = vout; self.recFile = file
                postRecording(file, stamp)                          // reserve cur.video on the server
                stream.startCapture { e in if let e = e { err("startCapture error: \(e)") } }
                self.stream = stream; self.sckOut = out
                err(">> capture started (audio+video, ScreenCaptureKit)")
            } catch { err("SCStream setup failed: \(error)") }
        }
    }
    func updateShare() { if mode == "share", let r = rec { r.record = curSharing } }
    func stop() {
        guard running else { return }
        running = false
        err(">> not in a call — stopping capture")
        mic?.stop(); mic = nil
        tap?.stop(); tap = nil
        stream?.stopCapture { _ in }
        stream = nil; sckOut = nil; videoOut = nil
        rec?.finish { ok in err(ok ? ">> screen recording saved" : ">> no screen recording written") }
        rec = nil
        targetBundle = ""
    }
}

// Tell the server which file the current call's screen recording is in (it stamps cur.video).
func postRecording(_ file: String, _ startedAt: Int) {
    let obj: [String: Any] = ["type": "recording", "file": file, "startedAt": startedAt]
    postJSON(obj)
}

let cap = Capture()
var curModuleMeetings = false   // Meetings opt-in — gates Screen-Recording detection + capture (no opt-in ⇒ no prompt)
var wasInCall = false, wasApp = "", wasTitle = "", curSharing = false, curVideoMode = VIDEO_MODE0, curTranslate = true, curLanguage = "auto", curManual = "auto", curAutoDetect = true, curBrowserMeet = true, curVocab = ""
var configFetchInFlight = false
var parentExitScheduled = false

func parentIsAlive() -> Bool {
    guard PARENT_PID > 1 else { return true } // standalone developer runs do not set a parent contract
    if kill(PARENT_PID, 0) == 0 { return true }
    return errno == EPERM
}
// Fetch UI-set config (video mode + share state) AND the manual override (start/pause/stop) in-process.
// inMeeting is SELF-DETECTED below; `manual` lets the UI force capture on/off on top of that — e.g. manually
// record a Google Meet call (no native window to auto-detect), or pause/stop while still in a call.
func fetchConfig() {
    guard !configFetchInFlight,
          let url = URL(string: "http://127.0.0.1:\(guiPort)/api/state") else { return }
    configFetchInFlight = true
    var request = URLRequest(url: url)
    if !GUI_TOKEN.isEmpty { request.setValue("Bearer \(GUI_TOKEN)", forHTTPHeaderField: "Authorization") }
    netSession.dataTask(with: request) { data, _, _ in
        let object = data.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
        DispatchQueue.main.async {
            configFetchInFlight = false
            if let o = object {
                if let m = o["modules"] as? [String: Any] { curModuleMeetings = (m["meetings"] as? Bool) ?? false }
                curSharing = (o["sharing"] as? Bool) ?? false
                curVideoMode = (o["videoMode"] as? String) ?? VIDEO_MODE0
                curTranslate = (o["translate"] as? Bool) ?? true
                curLanguage = (o["language"] as? String) ?? "auto"
                curManual = (o["manual"] as? String) ?? "auto"
                curAutoDetect = (o["autoDetect"] as? Bool) ?? true
                curBrowserMeet = (o["browserMeet"] as? Bool) ?? true
                curVocab = (o["vocab"] as? String) ?? ""
            }
            applyCaptureGate()
        }
    }.resume()
}
// Self-detect "in a call" from the CORE AUDIO PROCESS LIST — needs NO permission (not Screen Recording,
// not Accessibility): a native meeting app actively running audio I/O = we're in a call. Returns the app
// key + the process objects to tap, or nil. This is the whole point of the audio-only model — we know
// you're in a call AND capture the other participants without ever touching Screen Recording.
typealias CaptureTarget = (key: String, title: String, bundle: String, aoids: [AudioObjectID])
func captureTarget() -> CaptureTarget? {
    // Resolve WHAT to tap from --app: `all` = global system audio; a bundle id = that app; `auto` = a
    // dedicated call app or an audio-active browser/chat app with meeting evidence.
    func procs(_ match: (String) -> Bool) -> [(aoid: AudioObjectID, bundle: String, running: Bool)] {
        caProcessList().compactMap { p in
            guard let b = caString(p, kAudioProcessPropertyBundleID), match(b) else { return nil }
            return (p, b, caBool(p, kAudioProcessPropertyIsRunning))
        }
    }
    switch APP_SEL {
    case "all":
        return ("call", "", "", [])
    case "auto":
        let active = procs { KNOWN_CALL_BUNDLES.contains($0) }.filter { $0.running }
        // Dedicated conferencing apps first: active process audio is a strong call signal.
        for spec in DEDICATED_MEETING_APPS {
            if let first = active.first(where: { spec.bundles.contains($0.bundle) }) {
                let hits = active.filter { $0.bundle == first.bundle }
                let evidence = meetingWindowEvidence(first.bundle)
                return (spec.key, usefulMeetingTitle(evidence?.titles ?? [], key: spec.key), first.bundle, hits.map(\.aoid))
            }
        }
        // Browsers and multipurpose chat apps require a meeting-shaped window or tab as well as audio.
        for first in active where curBrowserMeet {
            guard evidenceKey(first.bundle) != nil || BROWSER_BUNDLES.contains(first.bundle),
                  let evidence = meetingWindowEvidence(first.bundle),
                  let key = platformFromEvidence(evidence.text, bundle: first.bundle) else { continue }
            let hits = active.filter { $0.bundle == first.bundle }
            return (key, usefulMeetingTitle(evidence.titles, key: key), first.bundle, hits.map(\.aoid))
        }
        return GATE ? nil : ("call", "", "", [])
    default:
        let hits = procs { $0 == APP_SEL }.filter { $0.running }
        if !hits.isEmpty {
            let key = dedicatedKey(APP_SEL) ?? evidenceKey(APP_SEL) ?? "call"
            let evidence = meetingWindowEvidence(APP_SEL)
            return (key, usefulMeetingTitle(evidence?.titles ?? [], key: key), APP_SEL, hits.map(\.aoid))
        }
        return GATE ? nil : ("call", "", APP_SEL, [])
    }
}
// Push inMeeting to the server so the UI flips live + shows the recording indicator.
func postInMeeting(_ on: Bool, _ app: String, _ title: String) {
    var st: [String: Any] = on ? ["inMeeting": true, "app": app] : ["inMeeting": false]
    if on, !title.isEmpty { st["meetingTitle"] = title; st["title"] = title }
    postJSON(["type": "state", "source": "coreaudio", "state": st])
}
// Internal cross-process event: the Accessibility helper sleeps until Core Audio proves a call is
// active. This is not a user-facing macOS notification; it is the efficient signal behind one.
let meetingActivityNotification = Notification.Name("com.myassistant.desktop.meeting-activity")
func publishMeetingActivity(_ on: Bool, _ app: String, _ bundle: String) {
    DistributedNotificationCenter.default().postNotificationName(
        meetingActivityNotification,
        object: nil,
        userInfo: ["active": on, "app": app, "bundle": bundle, "ts": Int(Date().timeIntervalSince1970 * 1000)],
        deliverImmediately: true
    )
}
// Report raw call detection to the server on change. The server composes this with the manual override
// to drive the UI, the meeting lifecycle, and re-arming auto-detection after a manual pause/stop.
func reportDetected(_ inCall: Bool, _ app: String, _ title: String) {
    guard inCall != wasInCall || (inCall && (app != wasApp || title != wasTitle)) else { return }
    wasInCall = inCall; wasApp = inCall ? app : ""; wasTitle = inCall ? title : ""
    postInMeeting(inCall, app, title)
}
// Decide capture: the manual override wins over auto-detection. start()/stop() are idempotent, so this
// runs every tick and only acts on a change — 'paused'/'off' release the mic even mid-call; 'on' forces
// capture even with no detected call (manual record).
func gate(_ target: CaptureTarget?) {
    let detected = target != nil
    publishMeetingActivity(detected, target?.key ?? "", target?.bundle ?? "")
    reportDetected(detected, target?.key ?? "", target?.title ?? "")
    let capture = (curManual == "on") || (curManual == "auto" && curAutoDetect && detected)
    if capture { cap.start(mode: curVideoMode, aoids: target?.aoids ?? [], bundle: target?.bundle ?? "") } else { cap.stop() }
}
func applyCaptureGate() {
    cap.updateShare()
    if !curModuleMeetings {
        publishMeetingActivity(false, "", "")
        reportDetected(false, "", "")
        if cap.running { cap.stop() }
        return
    }
    gate(captureTarget())                                            // detect + capture, all from the audio process list
}
func tick() {
    guard parentIsAlive() else {
        if !parentExitScheduled {
            parentExitScheduled = true
            err(">> parent app exited — releasing capture")
            cap.stop()
            DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) { exit(0) }
        }
        return
    }
    fetchConfig()
}

err(">> meeting-tap ready — gate:\(GATE ? "on (captures only during a call)" : "off (always captures)") gui:\(guiPort) whisper:\(whisperURL) app:\(APP_SEL) mic:\(MIC_ON) me:\(ME)")
Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { _ in tick() }
tick()   // check immediately

// On Ctrl-C / SIGTERM, finalize any in-progress screen recording (write the moov atom) before exiting.
var sigSources: [DispatchSourceSignal] = []   // retained for the program's life, else the source never fires
for sig in [SIGINT, SIGTERM] {
    signal(sig, SIG_IGN)
    let src = DispatchSource.makeSignalSource(signal: sig, queue: .main)
    src.setEventHandler { err(">> shutting down — finalizing"); cap.stop(); DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) { exit(0) } }
    src.resume()
    sigSources.append(src)
}

RunLoop.main.run()   // run until killed
