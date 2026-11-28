// Optional CI evidence of the actual macOS window, including native chrome.
// This tool never requests or changes screen-recording permission. The mandatory
// client screenshot does not depend on this optional OS capture succeeding.
import CoreGraphics
import Darwin
import Foundation

func output(_ value: [String: Any]) {
    let bytes = try! JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
    FileHandle.standardOutput.write(bytes)
    FileHandle.standardOutput.write(Data([10]))
}

guard CommandLine.arguments.count == 3,
      let pid = Int32(CommandLine.arguments[1]) else {
    output(["status": "unavailable", "reason": "expected PID and PNG output path"])
    exit(2)
}
guard CGPreflightScreenCaptureAccess() else {
    output(["status": "unavailable", "reason": "existing screen-recording permission is absent"])
    exit(0)
}
guard let windows = CGWindowListCopyWindowInfo(
    [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
) as? [[String: Any]] else {
    output(["status": "unavailable", "reason": "WindowServer did not return an on-screen window list"])
    exit(0)
}
let candidates = windows.filter {
    ($0[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid
        && ($0[kCGWindowLayer as String] as? NSNumber)?.intValue == 0
}
guard candidates.count == 1,
      let window = candidates.first,
      let windowID = window[kCGWindowNumber as String] as? NSNumber else {
    output([
        "status": "unavailable",
        "reason": "expected exactly one on-screen ordinary window belonging to the capture process",
        "matching_windows": candidates.count,
        "process_id": pid,
    ])
    exit(0)
}
let capture = Process()
capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
capture.arguments = ["-x", "-o", "-l", windowID.stringValue, CommandLine.arguments[2]]
do {
    try capture.run()
    let deadline = Date().addingTimeInterval(10)
    while capture.isRunning && Date() < deadline {
        Thread.sleep(forTimeInterval: 0.05)
    }
    if capture.isRunning {
        // Only our own optional screenshot subprocess is terminated on timeout.
        kill(capture.processIdentifier, SIGKILL)
        capture.waitUntilExit()
        output(["status": "unavailable", "reason": "screencapture exceeded ten seconds"])
        exit(0)
    }
    guard capture.terminationStatus == 0,
          FileManager.default.fileExists(atPath: CommandLine.arguments[2]) else {
        output([
            "status": "unavailable",
            "reason": "screencapture did not produce a PNG",
            "exit_code": capture.terminationStatus,
            "window_id": windowID,
        ])
        exit(0)
    }
    output([
        "status": "captured",
        "capture_kind": "native-window",
        "native_window_chrome": true,
        "shadow_included": false,
        "capture_api": "/usr/sbin/screencapture -x -o -l",
        "process_id": pid,
        "window_id": windowID,
        "window_bounds": window[kCGWindowBounds as String] ?? NSNull(),
        "screen_recording_permission": "already granted; unchanged",
    ])
} catch {
    output(["status": "unavailable", "reason": "screencapture could not start: \(error)"])
}
