// meeting-tap.swift (v2) — continuous meeting transcription.
// Captures system audio (ScreenCaptureKit) → energy-VAD segments into utterances →
// POSTs each utterance to a hot whisper-server → POSTs the caption to the GUI server.
//
// Build:  swiftc -O -target arm64-apple-macos13.0 meeting-tap.swift -o meeting-tap \
//           -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia \
//           -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker Info.plist
//         codesign --force --sign - meeting-tap   (then packaged into meeting-tap.app)
// Run (as the app, for Screen-Recording permission):
//         open meeting-tap.app --args <guiPort=8787> <whisperURL=http://127.0.0.1:8091/inference>
//
// ponytail: energy-threshold VAD (not Silero yet) — simplest thing that segments; swap in
// Silero (vad-rs / a coreml silero) if the energy gate mis-segments in noisy calls.

import Foundation
import ScreenCaptureKit
import AVFoundation
import CoreMedia

func err(_ s: String) { FileHandle.standardError.write((s + "\n").data(using: .utf8)!) }

let args = CommandLine.arguments
let guiPort = args.count > 1 ? args[1] : "8787"
let whisperURL = args.count > 2 ? args[2] : "http://127.0.0.1:8091/inference"

// VAD / segmentation tuning
let SR = 16000.0
let SPEECH_RMS: Float = CommandLine.arguments.count > 3 ? (Float(CommandLine.arguments[3]) ?? 0.006) : 0.006  // arg3 tunable
let SILENCE_HANGOVER = 0.7          // seconds of quiet that ends an utterance
let MIN_SEG = 0.5                   // ignore blips shorter than this
let MAX_SEG = 12.0                  // force-flush very long speech

final class Rec: NSObject, SCStreamOutput, SCStreamDelegate {
    var seg = [Float]()
    var inSpeech = false
    var silentDur = 0.0
    var segIdx = 0
    var bufN = 0
    var peak: Float = 0
    var announced = false
    let procQ = DispatchQueue(label: "stt.process")   // WAV+curl off the audio thread

    func stream(_ stream: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, CMSampleBufferDataIsReady(sb), let s = floatSamples(sb), !s.isEmpty else { return }
        let dur = Double(s.count) / SR
        let rms = (s.reduce(0) { $0 + $1 * $1 } / Float(s.count)).squareRoot()
        if !announced { announced = true; err("first buffer: \(s.count) samples, rms=\(String(format: "%.4f", rms)) (threshold=\(SPEECH_RMS))") }
        bufN += 1; peak = max(peak, rms)
        if bufN % 500 == 0 { err("· listening (peak \(String(format: "%.3f", peak)))"); peak = 0 }   // ~10s heartbeat
        if rms > SPEECH_RMS {
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
        segIdx += 1
        let idx = segIdx
        err("flush seg#\(idx) len=\(String(format: "%.1f", Double(samples.count) / SR))s")
        procQ.async { self.process(samples, idx) }
    }

    func process(_ samples: [Float], _ idx: Int) {
        let wav = URL(fileURLWithPath: "/tmp/mt_seg_\(idx % 4).wav")   // small ring of temp files
        writeWav16(samples, to: wav)
        let text = curl(["-s", whisperURL, "-F", "file=@\(wav.path)", "-F", "temperature=0", "-F", "response_format=text"])
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let clean = text.replacingOccurrences(of: "[BLANK_AUDIO]", with: "").trimmingCharacters(in: .whitespacesAndNewlines)
        err("seg#\(idx) \(String(format: "%.1f", Double(samples.count) / SR))s → '\(clean)'")
        guard clean.count > 1 else { return }
        postCaption(clean)
    }

    func postCaption(_ text: String) {
        let obj: [String: Any] = ["type": "caption", "speaker": "Speaker", "text": text, "self": false]
        guard let data = try? JSONSerialization.data(withJSONObject: obj) else { return }
        let f = "/tmp/mt_cap.json"; try? data.write(to: URL(fileURLWithPath: f))
        _ = curl(["-s", "-X", "POST", "http://127.0.0.1:\(guiPort)/ingest",
                  "-H", "Content-Type: application/json", "--data-binary", "@\(f)"])
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) { err("stream stopped: \(error)") }
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
    if let ic = pcm.int16ChannelData { return (0..<Int(n)).map { Float(ic[0][$0]) / 32768.0 } }   // handle Int16 SCK buffers
    return nil
}

// Plain 16-bit PCM mono WAV writer (AVAudioFile mangles Int16 buffers → write bytes ourselves).
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

@discardableResult
func curl(_ argv: [String]) -> String {
    let p = Process(); p.executableURL = URL(fileURLWithPath: "/usr/bin/curl"); p.arguments = argv
    let out = Pipe(); p.standardOutput = out; p.standardError = FileHandle.nullDevice
    do { try p.run() } catch { return "" }
    p.waitUntilExit()
    return String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
}

// MARK: - start continuous capture

let rec = Rec()
let started = DispatchSemaphore(value: 0)
var theStream: SCStream?

SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) { content, error in
    if let error = error { err("SCShareableContent error: \(error)"); exit(1) }
    guard let display = content?.displays.first else { err("no display"); exit(1) }
    let cfg = SCStreamConfiguration()
    cfg.capturesAudio = true
    cfg.excludesCurrentProcessAudio = false   // we play no audio; excluding wedges capture on some states
    cfg.sampleRate = 16000
    cfg.channelCount = 1
    cfg.width = 2; cfg.height = 2
    cfg.minimumFrameInterval = CMTime(value: 1, timescale: 1)
    do {
        let stream = SCStream(filter: SCContentFilter(display: display, excludingWindows: []), configuration: cfg, delegate: rec)
        try stream.addStreamOutput(rec, type: .audio, sampleHandlerQueue: DispatchQueue(label: "audio"))
        theStream = stream
        stream.startCapture { e in
            if let e = e { err("startCapture error: \(e)"); exit(1) }
            err(">> live transcription started (gui:\(guiPort) whisper:\(whisperURL))")
            started.signal()
        }
    } catch { err("SCStream setup failed: \(error)"); exit(1) }
}

started.wait()
RunLoop.main.run()   // run until killed
