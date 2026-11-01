// Small macOS helper for the Native Kiro UI test (used by --ocr). No AI
// involved: it finds a process's main window and OCRs a PNG with Vision.
//
//   uitool window <pid>        -> CGWindowID of the process's largest on-screen window
//   uitool ocr <png>           -> recognised text, one line per observation
import AppKit
import Foundation
import Vision

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(2)
}

let args = CommandLine.arguments
guard args.count >= 3 else { fail("usage: uitool window|ocr <arg>") }

switch args[1] {
case "window":
    guard let pid = Int32(args[2]) else { fail("bad pid") }
    let info = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
    let mine = info.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
    let area = { (w: [String: Any]) -> Double in
        let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
        return (b["Width"] ?? 0) * (b["Height"] ?? 0)
    }
    guard let best = mine.max(by: { area($0) < area($1) }), let id = best[kCGWindowNumber as String] as? Int else {
        exit(1)
    }
    print(id)

case "ocr":
    guard let image = NSImage(contentsOfFile: args[2]),
          let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { fail("cannot read \(args[2])") }
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = false
    do { try VNImageRequestHandler(cgImage: cg).perform([request]) } catch { fail("ocr failed: \(error)") }
    for observation in request.results ?? [] {
        if let text = observation.topCandidates(1).first?.string { print(text) }
    }

default:
    fail("unknown command \(args[1])")
}
