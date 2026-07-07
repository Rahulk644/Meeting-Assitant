// meeting-tap.swift (v3) — continuous meeting transcription from TWO sources.
//   • System audio (ScreenCaptureKit), scoped to the meeting app  → the other participants.
//   • Microphone   (AVAudioEngine)                                → you.
// Both → energy-VAD → per-utterance WAV → hot whisper-server → caption POSTed to the GUI.
//
// Build:  swiftc -O -target arm64-apple-macos13.0 meeting-tap.swift -o meeting-tap \
//           -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia \
//           -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker Info.plist
//
// Run:    meeting-tap <guiPort=8787> <whisperURL> [--app auto|all|<id>] [--mic on|off] [--me <name>]
//         meeting-tap --list-apps       # capturable apps + bundle ids (pick one for --app)
//
//   --app auto  (default) captures only the running native meeting app (Zoom/Teams/Webex/FaceTime),
//               so background audio (a YouTube tab) is NOT transcribed. If none is running it falls
//               back to the whole system. Google Meet lives in a browser — pass --app <browser bundle>.
//   --mic on    (default) also transcribes your own mic, labeled "<name> (Me)". Needs Microphone
//               permission on the launching terminal.  --mic off to skip it.
//   --me <name> your label (default: the macOS full name).  --rms / --mic-rms tune the VAD gates
//               (system 0.006, mic 0.02 — the mic is higher because a room is noisier than the feed).
//   --min-conf 0.5   drop utterances below this mean word-confidence (kills noise hallucinations).
//   --expressions    keep Whisper's *sigh* / (laughs) cues as [sigh] tags for a sentiment read.
//   --gate on|off    on (default) captures ONLY while you're in a call (polls the server's meeting-state,
//                    releasing the mic otherwise); off = always capture (Google Meet — AX can't detect it).
//   NOTE: use HEADPHONES. On speakers, the mic re-hears the remote voices → duplicate/misattributed
//         lines. Headphones give a clean split: mic = you, system audio = them.
//
// ponytail: energy-threshold VAD (not Silero) — simplest thing that segments; swap in Silero if it
// mis-segments in noisy calls. Core Audio process taps (Fathom's per-app method) need the macOS 14.2+
// SDK, which this toolchain (13.3) lacks — hence ScreenCaptureKit app-scoping instead.

import Foundation
import ScreenCaptureKit
import AVFoundation
import CoreMedia

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
let GATE = opt("--gate", "on") != "off"                      // capture ONLY while in a call (polls /api/state)
let VIDEO_MODE0 = opt("--video", "off")                      // off | always | share — server config overrides live
let REC_DIR = opt("--rec-dir", "data/meetings/rec")          // where screen recordings (.mp4) are written
let VIDEO_FPS: Int32 = 10                                    // screen content doesn't need 30fps; 10 keeps files small
let ME = { let n = NSFullUserName(); return opt("--me", n.isEmpty ? "Me" : n) }()  // shown as "<name> (Me)"

// VAD / segmentation tuning
let SR = 16000.0
let SILENCE_HANGOVER = 0.7          // seconds of quiet that ends an utterance
let MIN_SEG = 0.5                   // ignore blips shorter than this
let MAX_SEG = Double(opt("--max-seg", "4")) ?? 4.0   // force-flush long speech every N s (lower = snappier, less lag)

// Native meeting apps we auto-scope to (browsers excluded on purpose — a browser may just be YouTube).
let NATIVE_MEETING: Set<String> = [
    "us.zoom.xos",                               // Zoom
    "com.microsoft.teams", "com.microsoft.teams2",
    "com.cisco.webexmeetingsapp",                // Webex
    "com.apple.FaceTime",
]

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
        let wav = URL(fileURLWithPath: "/tmp/mt_\(tag)_seg_\(idx % 4).wav")   // small ring of temp files
        writeWav16(samples, to: wav)
        let secs = f1(Double(samples.count) / SR)
        guard let (raw, conf) = transcribe(wav.path) else { return }
        if conf < MIN_CONF {   // low mean word-probability = noise/hallucination (the "*Sigh*" case)
            err("\(tag) seg#\(idx) \(secs)s dropped: conf \(f2(conf)) < \(f2(MIN_CONF)) raw='\(raw.prefix(40))'")
            return
        }
        let clean = EXPRESSIONS ? keepExpressions(raw) : cleanTranscript(raw)
        err("\(tag) seg#\(idx) \(secs)s conf=\(f2(conf)) → '\(clean)'")
        guard clean.count > 1 else { return }
        postCaption(clean)
    }

    func postCaption(_ text: String) {
        let obj: [String: Any] = ["type": "caption", "speaker": speaker, "text": text, "self": isSelf,
                                  "ts": Int(Date().timeIntervalSince1970 * 1000)]
        guard let data = try? JSONSerialization.data(withJSONObject: obj) else { return }
        let f = "/tmp/mt_cap_\(tag).json"; try? data.write(to: URL(fileURLWithPath: f))
        _ = curl(["-s", "-X", "POST", "http://127.0.0.1:\(guiPort)/ingest",
                  "-H", "Content-Type: application/json", "--data-binary", "@\(f)"])
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

// MARK: - screen recording (ScreenCaptureKit video → H.264 .mp4 via AVAssetWriter)

final class VideoRecorder {
    let url: URL
    private var writer: AVAssetWriter?
    private var input: AVAssetWriterInput?
    private var started = false
    private(set) var wroteAny = false
    var record = false                          // gated by mode (always) / share-state (share)
    private let q = DispatchQueue(label: "video.write")

    init(dir: String, name: String) {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        url = URL(fileURLWithPath: (dir as NSString).appendingPathComponent(name))
        try? FileManager.default.removeItem(at: url)
    }
    func append(_ sb: CMSampleBuffer) {
        guard record, CMSampleBufferDataIsReady(sb) else { return }
        q.sync {
            if writer == nil { setup(sb) }
            guard let w = writer, let inp = input, w.status == .writing, inp.isReadyForMoreMediaData else { return }
            if !started { w.startSession(atSourceTime: CMSampleBufferGetPresentationTimeStamp(sb)); started = true }
            inp.append(sb); wroteAny = true
        }
    }
    private func setup(_ sb: CMSampleBuffer) {
        guard let fmt = CMSampleBufferGetFormatDescription(sb) else { return }
        let dim = CMVideoFormatDescriptionGetDimensions(fmt)
        guard let w = try? AVAssetWriter(url: url, fileType: .mp4) else { err("video: writer init failed"); return }
        let s: [String: Any] = [AVVideoCodecKey: AVVideoCodecType.h264,
                                AVVideoWidthKey: Int(dim.width), AVVideoHeightKey: Int(dim.height)]
        let inp = AVAssetWriterInput(mediaType: .video, outputSettings: s)
        inp.expectsMediaDataInRealTime = true
        if w.canAdd(inp) { w.add(inp) }
        w.startWriting()
        writer = w; input = inp
        err(">> screen recording → \(url.lastPathComponent) (\(dim.width)x\(dim.height))")
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
    try? d.write(to: url)
}

// Ask whisper-server for verbose_json → (full text, mean per-word confidence). Whisper reports a
// probability per word; averaged, it separates real speech (~0.9) from noise/hallucination (low).
func transcribe(_ wavPath: String) -> (text: String, conf: Float)? {
    // Language is EXPLICIT (Settings → default English), not auto — like Handy. `auto` misfires on
    // code-switched speech and, with translate, hallucinates a wrong script (the "کن کن کن" bug). With the
    // language pinned, both transcribe and translate are accurate. translate (→ English) is an opt-in toggle.
    var args = ["-s", whisperURL, "-F", "file=@\(wavPath)", "-F", "temperature=0", "-F", "response_format=verbose_json", "-F", "language=\(curLanguage)"]
    if curTranslate { args += ["-F", "translate=true"] }
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

func makeFilter(_ display: SCDisplay, _ apps: [SCRunningApplication]) -> SCContentFilter {
    let whole = SCContentFilter(display: display, excludingWindows: [])
    switch APP_SEL {
    case "all":
        err("capturing full system audio (--app all)")
        return whole
    case "auto":
        if let m = apps.first(where: { NATIVE_MEETING.contains($0.bundleIdentifier) }) {
            err("auto: scoping audio to \(m.applicationName) [\(m.bundleIdentifier)]")
            return SCContentFilter(display: display, including: [m], exceptingWindows: [])
        }
        err("auto: no native meeting app running — capturing full system audio (background audio will be transcribed; pass --app <bundleID>, see --list-apps)")
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

// Capture is GATED on being in a real call: the mic + system taps run only while the server's
// meeting-state reports inMeeting, and are torn down (releasing the mic) otherwise — so nothing is
// captured when you're not in a meeting. `--gate off` = always capture (Google Meet, which the
// Accessibility layer can't see).
final class Capture {
    var stream: SCStream?, sckOut: SCKOut?, videoOut: VideoOut?, rec: VideoRecorder?, mic: Mic?
    var running = false, mode = "off", recFile = ""
    func start(mode: String) {
        guard !running else { return }
        running = true; self.mode = mode
        let wantVideo = mode == "always" || mode == "share"
        err(">> in a call — starting capture (video: \(mode))")
        sysSeg.reset(); micSeg.reset()
        if MIC_ON { let m = Mic(micSeg); m.start(); mic = m }
        SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) { content, error in
            guard self.running else { return }                 // call ended during the async fetch
            if let error = error { err("SCShareableContent error: \(error)"); return }
            guard let display = content?.displays.first else { err("no display"); return }
            let cfg = SCStreamConfiguration()
            cfg.capturesAudio = true
            cfg.excludesCurrentProcessAudio = false            // we play no audio; excluding wedges capture
            cfg.sampleRate = 16000; cfg.channelCount = 1
            if wantVideo {
                cfg.width = Int(display.width); cfg.height = Int(display.height)
                cfg.minimumFrameInterval = CMTime(value: 1, timescale: VIDEO_FPS)
                cfg.pixelFormat = kCVPixelFormatType_32BGRA
                cfg.queueDepth = 6
            } else {
                cfg.width = 2; cfg.height = 2
                cfg.minimumFrameInterval = CMTime(value: 1, timescale: 1)
            }
            let out = SCKOut(sysSeg)
            do {
                let stream = SCStream(filter: makeFilter(display, content?.applications ?? []), configuration: cfg, delegate: out)
                try stream.addStreamOutput(out, type: .audio, sampleHandlerQueue: DispatchQueue(label: "audio"))
                if wantVideo {
                    let stamp = Int(Date().timeIntervalSince1970 * 1000)
                    let file = "\(stamp).mp4"
                    let r = VideoRecorder(dir: REC_DIR, name: file)
                    r.record = (mode == "always") ? true : curSharing   // share mode: only while a screen is shared
                    let vout = VideoOut(r)
                    try stream.addStreamOutput(vout, type: .screen, sampleHandlerQueue: DispatchQueue(label: "video"))
                    self.rec = r; self.videoOut = vout; self.recFile = file
                    postRecording(file, stamp)                          // reserve cur.video on the server
                }
                stream.startCapture { e in if let e = e { err("startCapture error: \(e)") } }
                self.stream = stream; self.sckOut = out
                err(">> capture started (audio\(wantVideo ? "+video" : ""))")
            } catch { err("SCStream setup failed: \(error)") }
        }
    }
    func updateShare() { if mode == "share", let r = rec { r.record = curSharing } }
    func stop() {
        guard running else { return }
        running = false
        err(">> not in a call — stopping capture")
        mic?.stop(); mic = nil
        stream?.stopCapture { _ in }
        stream = nil; sckOut = nil; videoOut = nil
        rec?.finish { ok in err(ok ? ">> screen recording saved" : ">> no screen recording written") }
        rec = nil
    }
}

// Tell the server which file the current call's screen recording is in (it stamps cur.video).
func postRecording(_ file: String, _ startedAt: Int) {
    let obj: [String: Any] = ["type": "recording", "file": file, "startedAt": startedAt]
    guard let data = try? JSONSerialization.data(withJSONObject: obj) else { return }
    let f = "/tmp/mt_rec.json"; try? data.write(to: URL(fileURLWithPath: f))
    _ = curl(["-s", "-X", "POST", "http://127.0.0.1:\(guiPort)/ingest",
              "-H", "Content-Type: application/json", "--data-binary", "@\(f)"])
}

let cap = Capture()
var wasInCall = false, curSharing = false, curVideoMode = VIDEO_MODE0, curTranslate = false, curLanguage = "en"
// Poll the server ONLY for UI-set config (video mode + share state). inMeeting is SELF-DETECTED below.
func fetchConfig() -> (sharing: Bool, videoMode: String, translate: Bool, language: String) {
    let out = curl(["-s", "--max-time", "2", "http://127.0.0.1:\(guiPort)/api/state"])
    guard let d = out.data(using: .utf8),
          let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any] else {
        return (curSharing, curVideoMode, curTranslate, curLanguage)
    }
    return ((o["sharing"] as? Bool) ?? false, (o["videoMode"] as? String) ?? VIDEO_MODE0, (o["translate"] as? Bool) ?? false, (o["language"] as? String) ?? "en")
}
// Self-detect "in a call" from the WINDOW LIST via ScreenCaptureKit — needs Screen Recording (which
// capture needs anyway), NOT Accessibility. A native meeting app showing its in-call window = we're in a
// meeting. This is what lets the PACKAGED app detect calls itself: the AX-based meeting-state can't run
// as a grandchild-of-the-app under macOS TCC, but the capturer is a direct child so its grants work.
// Returns the app key ("zoom"/"teams"/"call") or nil when not in a call.
var detectErrLogged = false
func detectMeetingApp(_ done: @escaping (String?) -> Void) {
    SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) { content, error in
        if let error = error {   // almost always: Screen Recording not granted to this app
            if !detectErrLogged { err(">> can't read the window list to detect the call — grant Screen Recording to Assistant. (\(error.localizedDescription))"); detectErrLogged = true }
            done(nil); return
        }
        var hit: String? = nil
        for w in (content?.windows ?? []) {
            guard let bid = w.owningApplication?.bundleIdentifier, NATIVE_MEETING.contains(bid) else { continue }
            let t = (w.title ?? "").lowercased()
            // Zoom hides the "Zoom Meeting" window while YOU screen-share, leaving the floating video /
            // share-statusbar windows — match those too, so sharing doesn't read as "call ended".
            let inCall = (bid == "us.zoom.xos")
                ? (t.contains("zoom meeting") || t.contains("floating video") || t.contains("share statusbar"))
                : t.contains("meeting")
            if inCall { hit = (bid == "us.zoom.xos") ? "zoom" : (bid.hasPrefix("com.microsoft.teams") ? "teams" : "call"); break }
        }
        done(hit)
    }
}
// Push inMeeting to the server so the UI flips live + shows the recording indicator.
func postInMeeting(_ on: Bool, _ app: String) {
    let st: [String: Any] = on ? ["inMeeting": true, "app": app] : ["inMeeting": false]
    guard let data = try? JSONSerialization.data(withJSONObject: ["type": "state", "state": st] as [String: Any]) else { return }
    let f = "/tmp/mt_state.json"; try? data.write(to: URL(fileURLWithPath: f))
    _ = curl(["-s", "-X", "POST", "http://127.0.0.1:\(guiPort)/ingest", "-H", "Content-Type: application/json", "--data-binary", "@\(f)"])
}
func setInCall(_ app: String?) {
    let inCall = app != nil
    guard inCall != wasInCall else { return }
    wasInCall = inCall
    postInMeeting(inCall, app ?? "")
    inCall ? cap.start(mode: curVideoMode) : cap.stop()
}
func tick() {
    let cfg = fetchConfig()
    curSharing = cfg.sharing; curVideoMode = cfg.videoMode; curTranslate = cfg.translate; curLanguage = cfg.language
    cap.updateShare()
    if !GATE { setInCall("call"); return }                   // --gate off: always capture
    detectMeetingApp { app in DispatchQueue.main.async { setInCall(app) } }
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
