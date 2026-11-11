// Spike, throwaway: records one window of a running macOS app with ScreenCaptureKit, as a
// still (PNG) or a film (MP4), and lists it in $FEARLEZZ_OUT/proof.jsonl.
//
//   look-record still --bundle com.example.app --out home.png [--id home]
//   look-record film  --pid 1234 --out tap.mp4 --seconds 3 [--id tap]
//   look-record still --pid 1234 --out content.png --crop-top 28   leaves out the title bar
//   look-record check         prints whether this process may record the screen
//
// The window is captured on its own (desktopIndependentWindow), so it may be covered by other
// windows. A window that is minimized or on another Space gives no fresh frames.

import AppKit
import CoreGraphics
import Foundation
import ScreenCaptureKit
import UniformTypeIdentifiers

let launched = Date()

/// Seconds since launch, for the timing lines on stderr.
func since() -> String { String(format: "%.2f s", Date().timeIntervalSince(launched)) }

struct Options {
    var mode = ""
    var bundle: String?
    var pid: pid_t?
    var title: String?
    var out = ""
    var id: String?
    var seconds = 3.0
    var scale = 2.0
    /// Points cut from the top, to leave out the title bar: its buttons change with focus and
    /// macOS draws a recording badge in it while a film records.
    var cropTop = 0.0
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("look-record: \(message)\n".utf8))
    exit(1)
}

func parse() -> Options {
    var o = Options()
    var args = CommandLine.arguments.dropFirst()
    guard let mode = args.popFirst() else { fail("usage: look-record still|film|check ...") }
    o.mode = mode
    while let flag = args.popFirst() {
        guard let value = args.popFirst() else { fail("\(flag) needs a value") }
        switch flag {
        case "--bundle": o.bundle = value
        case "--pid": o.pid = pid_t(value)
        case "--title": o.title = value
        case "--out": o.out = value
        case "--id": o.id = value
        case "--seconds": o.seconds = Double(value) ?? 3
        case "--scale": o.scale = Double(value) ?? 2
        case "--crop-top": o.cropTop = Double(value) ?? 0
        default: fail("unknown flag \(flag)")
        }
    }
    return o
}

/// The largest window of the app, so tool palettes and menus do not win.
func findWindow(_ o: Options) async throws -> SCWindow {
    let content = try await SCShareableContent.excludingDesktopWindows(
        false, onScreenWindowsOnly: false)
    let windows = content.windows.filter { w in
        guard let app = w.owningApplication else { return false }
        if let pid = o.pid, app.processID != pid { return false }
        if let bundle = o.bundle, app.bundleIdentifier != bundle { return false }
        if let title = o.title, !(w.title ?? "").contains(title) { return false }
        return w.windowLayer == 0 && w.frame.width > 50 && w.frame.height > 50
    }
    guard let window = windows.max(by: { $0.frame.width * $0.frame.height < $1.frame.width * $1.frame.height })
    else { fail("no window found for the app; is it running, with a window open?") }
    return window
}

func configuration(_ window: SCWindow, _ o: Options) -> SCStreamConfiguration {
    let c = SCStreamConfiguration()
    let height = window.frame.height - o.cropTop
    c.sourceRect = CGRect(x: 0, y: o.cropTop, width: window.frame.width, height: height)
    c.width = Int(window.frame.width * o.scale)
    c.height = Int(height * o.scale)
    c.showsCursor = false
    c.ignoreShadowsSingleWindow = true
    c.minimumFrameInterval = CMTime(value: 1, timescale: 30)
    return c
}

func appendProof(kind: String, file: URL, id: String?) {
    guard let dir = ProcessInfo.processInfo.environment["FEARLEZZ_OUT"] else { return }
    let outDir = URL(fileURLWithPath: dir)
    var name = file.path
    if file.path.hasPrefix(outDir.path + "/") { name = String(file.path.dropFirst(outDir.path.count + 1)) }
    var entry: [String: String] = ["kind": kind, "file": name]
    if let id { entry["id"] = id }
    let line = try! JSONSerialization.data(withJSONObject: entry, options: [.sortedKeys]) + Data("\n".utf8)
    let proof = outDir.appendingPathComponent("proof.jsonl")
    if let h = try? FileHandle(forWritingTo: proof) {
        h.seekToEndOfFile()
        h.write(line)
        try? h.close()
    } else {
        try! line.write(to: proof)
    }
}

func outURL(_ o: Options) -> URL {
    guard !o.out.isEmpty else { fail("--out is required") }
    if o.out.hasPrefix("/") { return URL(fileURLWithPath: o.out) }
    let base = ProcessInfo.processInfo.environment["FEARLEZZ_OUT"] ?? FileManager.default.currentDirectoryPath
    return URL(fileURLWithPath: base).appendingPathComponent(o.out)
}

func still(_ o: Options) async throws {
    let window = try await findWindow(o)
    let filter = SCContentFilter(desktopIndependentWindow: window)
    let start = Date()
    let image = try await SCScreenshotManager.captureImage(
        contentFilter: filter, configuration: configuration(window, o))
    let url = outURL(o)
    guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { fail("cannot write \(url.path)") }
    CGImageDestinationAddImage(dest, image, nil)
    CGImageDestinationFinalize(dest)
    let ms = Int(Date().timeIntervalSince(start) * 1000)
    print("still \(image.width)x\(image.height) of \"\(window.title ?? "")\" in \(ms) ms: \(url.path)")
    appendProof(kind: "still", file: url, id: o.id)
}

final class Recording: NSObject, SCRecordingOutputDelegate, SCStreamDelegate {
    var finished: CheckedContinuation<Void, Error>?
    func recordingOutputDidFinishRecording(_ recordingOutput: SCRecordingOutput) {
        finished?.resume()
        finished = nil
    }
    func recordingOutput(_ recordingOutput: SCRecordingOutput, didFailWithError error: Error) {
        finished?.resume(throwing: error)
        finished = nil
    }
    func stream(_ stream: SCStream, didStopWithError error: Error) {
        finished?.resume(throwing: error)
        finished = nil
    }
}

func film(_ o: Options) async throws {
    let window = try await findWindow(o)
    FileHandle.standardError.write(Data("window found at \(since())\n".utf8))
    let filter = SCContentFilter(desktopIndependentWindow: window)
    let url = outURL(o)
    try? FileManager.default.removeItem(at: url)
    let rc = SCRecordingOutputConfiguration()
    rc.outputURL = url
    rc.outputFileType = .mp4
    rc.videoCodecType = .h264
    let delegate = Recording()
    let stream = SCStream(filter: filter, configuration: configuration(window, o), delegate: delegate)
    let output = SCRecordingOutput(configuration: rc, delegate: delegate)
    try stream.addRecordingOutput(output)
    let start = Date()
    try await stream.startCapture()
    let started = Date().timeIntervalSince(start)
    try await Task.sleep(nanoseconds: UInt64(o.seconds * 1_000_000_000))
    try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
        delegate.finished = c
        Task { try? await stream.stopCapture() }
    }
    let total = Date().timeIntervalSince(start)
    let size = (try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? Int) ?? 0
    print(String(format: "film of \"%@\": start %.2f s, total %.2f s, %d KB: %@",
                 window.title ?? "", started, total, size / 1024, url.path))
    appendProof(kind: "film", file: url, id: o.id)
    FileHandle.standardError.write(Data("done at \(since())\n".utf8))
}

let o = parse()
// A command line tool has no window server connection until AppKit makes one;
// ScreenCaptureKit asserts in CGS_REQUIRE_INIT without it.
_ = NSApplication.shared
let main = Task {
    do {
        switch o.mode {
        case "check":
            // CGPreflightScreenCaptureAccess never prompts; it says whether the responsible
            // process (usually the terminal app) is allowed.
            let ok = CGPreflightScreenCaptureAccess()
            print(ok ? "screen recording: allowed" : "screen recording: not allowed")
            exit(ok ? 0 : 1)
        case "still": try await still(o)
        case "film": try await film(o)
        default: fail("unknown mode \(o.mode); use still, film or check")
        }
        exit(0)
    } catch {
        fail("\(error.localizedDescription) (\(error))")
    }
}
RunLoop.main.run()
