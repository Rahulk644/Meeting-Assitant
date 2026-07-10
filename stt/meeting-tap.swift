// meeting-tap.swift (v3) — continuous meeting transcription from TWO sources.
//   • System audio (ScreenCaptureKit), scoped to the meeting app  → the other participants.
//   • Microphone   (AVAudioEngine)                                → you.
// Both → energy-VAD splits on natural pauses → WHOLE-utterance WAV → on-demand transcribe-server
// (transcribe.cpp on the Metal GPU) → caption POSTed to the GUI. Feeding whole utterances (not fixed-length
// chunks) is what makes it accurate — a mid-sentence cut severs context and the model hallucinates.
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
//   --gate on|off    on (default) captures ONLY while you're in a call (self-detected from the SCK window
//                    list, releasing the mic otherwise); off = always capture (Google Meet — no native window).
//   NOTE: use HEADPHONES. On speakers, the mic re-hears the remote voices → duplicate/misattributed
//         lines. Headphones give a clean split: mic = you, system audio = them.
//
// ponytail: the SEGMENTER is a simple energy threshold — it only decides utterance BOUNDARIES (speech vs
// pause) and gates out sub-threshold silence/noise upstream; transcribe-server then does the anti-
// hallucination (temperature=0, no fallback) on what reaches it. Core Audio process taps (Fathom's per-app
// method) need the macOS 14.2+ SDK, which this toolchain (13.3) lacks — hence ScreenCaptureKit app-scoping.

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
let GATE = opt("--gate", "on") != "off"                      // capture ONLY while in a call (self-detected from the window list)
let VIDEO_MODE0 = opt("--video", "off")                      // off | always | share — server config overrides live
let REC_DIR = opt("--rec-dir", "data/meetings/rec")          // where screen recordings (.mp4) are written
let VIDEO_FPS: Int32 = 10                                    // screen content doesn't need 30fps; 10 keeps files small
let ME = { let n = NSFullUserName(); return opt("--me", n.isEmpty ? "Me" : n) }()  // shown as "<name> (Me)"

// VAD / segmentation tuning
let SR = 16000.0
let SILENCE_HANGOVER = 0.7          // seconds of quiet that ends an utterance
let MIN_SEG = 0.5                   // ignore blips shorter than this
// Safety cap only — the REAL utterance boundary is SILENCE_HANGOVER (a natural pause). Whisper needs the
// whole utterance for context; chopping at 4 s severed words mid-sentence and made it hallucinate. Handy
// feeds the entire utterance (25 s is fine — under Whisper's 30 s window). Cut only a nonstop monologue.
let MAX_SEG = Double(opt("--max-seg", "25")) ?? 25.0

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
    try? d.write(to: url)
}

// Ask whisper-server for verbose_json → (full text, mean per-word confidence). Whisper reports a
// probability per word; averaged, it separates real speech (~0.9) from noise/hallucination (low).
func transcribe(_ wavPath: String) -> (text: String, conf: Float)? {
    // Two independent knobs: `language` = DETECTION (default `auto` — Rahul speaks Hindi + English, so let
    // Whisper detect per utterance), `translate` = OUTPUT (default on → English). The old "auto hallucinates
    // a wrong script" (کن کن کن) was really our 4 s fragments starving detection — on whole utterances `auto`
    // is accurate (A/B-verified against Handy's own WAVs). Both overridable in Settings → Meetings.
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

// Capture is GATED on being in a real call: the mic + system taps run only while we self-detect a call
// in the ScreenCaptureKit window list (see detectMeetingApp), and are torn down (releasing the mic)
// otherwise — so nothing is captured when you're not in a meeting. `--gate off` = always capture
// (Google Meet, whose window we can't yet recognize).
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
                // Cap capture at 1080p tall (preserve aspect, even dims for H.264). A meeting recording
                // doesn't need full retina, and fewer pixels = less capture bandwidth AND less encoder work
                // on long calls — the main lever against lag. Downscale is done by SCK on the GPU (free).
                let sc = min(1.0, 1080.0 / Double(display.height))
                cfg.width  = Int(Double(display.width)  * sc) & ~1
                cfg.height = Int(Double(display.height) * sc) & ~1
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
var wasInCall = false, curSharing = false, curVideoMode = VIDEO_MODE0, curTranslate = true, curLanguage = "auto", curManual = "auto"
// Poll the server for UI-set config (video mode + share state) AND the manual override (start/pause/stop).
// inMeeting is SELF-DETECTED below; `manual` lets the UI force capture on/off on top of that — e.g. manually
// record a Google Meet call (no native window to auto-detect), or pause/stop while still in a call.
func fetchConfig() -> (sharing: Bool, videoMode: String, translate: Bool, language: String, manual: String) {
    let out = curl(["-s", "--max-time", "2", "http://127.0.0.1:\(guiPort)/api/state"])
    guard let d = out.data(using: .utf8),
          let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any] else {
        return (curSharing, curVideoMode, curTranslate, curLanguage, curManual)
    }
    return ((o["sharing"] as? Bool) ?? false, (o["videoMode"] as? String) ?? VIDEO_MODE0, (o["translate"] as? Bool) ?? true, (o["language"] as? String) ?? "auto", (o["manual"] as? String) ?? "auto")
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
            // Zoom keeps an OFF-SCREEN "zoom floating video window" alive even when idle (no call), so matching
            // it unconditionally false-detected a call whenever Zoom was merely open. The real "Zoom Meeting"
            // window exists ONLY during a call → match it unconditionally. The floating-video / share-statusbar
            // windows (the screen-share case, where Zoom hides the main window) count ONLY while on screen —
            // during a share they're visible; idle they sit off-screen.
            let inCall = (bid == "us.zoom.xos")
                ? (t.contains("zoom meeting")
                   || (w.isOnScreen && (t.contains("floating video") || t.contains("share statusbar"))))
                : (w.isOnScreen && t.contains("meeting"))
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
// Report raw call detection to the server on change. The server composes this with the manual override
// to drive the UI, the meeting lifecycle, and re-arming auto-detection after a manual pause/stop.
func reportDetected(_ inCall: Bool, _ app: String) {
    guard inCall != wasInCall else { return }
    wasInCall = inCall
    postInMeeting(inCall, app)
}
// Decide capture: the manual override wins over auto-detection. start()/stop() are idempotent, so this
// runs every tick and only acts on a change — 'paused'/'off' release the mic even mid-call; 'on' forces
// capture even with no detected call (manual record).
func gate(_ app: String?) {
    let detected = app != nil
    reportDetected(detected, app ?? "")
    let capture = (curManual == "on") || (curManual == "auto" && detected)
    capture ? cap.start(mode: curVideoMode) : cap.stop()
}
func tick() {
    let cfg = fetchConfig()
    curSharing = cfg.sharing; curVideoMode = cfg.videoMode; curTranslate = cfg.translate; curLanguage = cfg.language; curManual = cfg.manual
    cap.updateShare()
    if !GATE { gate("call"); return }                        // --gate off: always capture (still honors manual)
    detectMeetingApp { app in DispatchQueue.main.async { gate(app) } }
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
