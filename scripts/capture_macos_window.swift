// Actual macOS window evidence, including native chrome or an owned native panel.
// This tool never requests or changes screen-recording permission. Workspace
// client pixels are separate; the Open-panel scene requires a real OS capture.
import CoreGraphics
import Darwin
import Foundation

func output(_ value: [String: Any]) {
    let bytes = try! JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
    FileHandle.standardOutput.write(bytes)
    FileHandle.standardOutput.write(Data([10]))
}

func modeDescription(_ mode: CGDisplayMode) -> [String: Any] {
    return [
        "mode_id": mode.ioDisplayModeID,
        "width": mode.width,
        "height": mode.height,
        "pixel_width": mode.pixelWidth,
        "pixel_height": mode.pixelHeight,
        "refresh_hz": mode.refreshRate,
        "usable_for_desktop": mode.isUsableForDesktopGUI(),
    ]
}

func displayBounds(_ display: CGDirectDisplayID) -> [Double] {
    let bounds = CGDisplayBounds(display)
    return [Double(bounds.origin.x), Double(bounds.origin.y), Double(bounds.width), Double(bounds.height)]
}

func describeDesktop() {
    let display = CGMainDisplayID()
    guard let mode = CGDisplayCopyDisplayMode(display) else {
        output(["status": "unavailable", "reason": "CoreGraphics did not report the current display mode"])
        return
    }
    output([
        "status": "observed",
        "process_id": ProcessInfo.processInfo.processIdentifier,
        "display_id": display,
        "mode": modeDescription(mode),
        "bounds": displayBounds(display),
    ])
}

func fitDesktop() {
    guard ProcessInfo.processInfo.environment["GITHUB_ACTIONS"] == "true" else {
        output(["status": "not-requested", "reason": "mode selection is restricted to ephemeral CI"])
        return
    }
    let display = CGMainDisplayID()
    guard let before = CGDisplayCopyDisplayMode(display),
          let modes = CGDisplayCopyAllDisplayModes(display, nil) as? [CGDisplayMode] else {
        output(["status": "unavailable", "reason": "CoreGraphics did not return display modes"])
        return
    }
    var report: [String: Any] = [
        "api": "CGDisplayCopyAllDisplayModes + CGConfigureDisplayWithDisplayMode + CGCompleteDisplayConfiguration",
        "requested_configuration_scope": "current-login-session",
        "permanent_configuration_changed": false,
        "process_id": ProcessInfo.processInfo.processIdentifier,
        "display_id": display,
        "requested_scene_points": [1280, 800],
        "minimum_desktop_mode": [1440, 900],
        "preferred_desktop_mode": [1600, 1000],
        "before": modeDescription(before),
        "before_bounds": displayBounds(display),
        "available_modes": modes.prefix(256).map(modeDescription),
        "available_modes_truncated": modes.count > 256,
    ]
    // Leave space for the actual menu bar, Dock and native window decorations.
    // These are supported display modes, not virtual/constructed image sizes.
    if before.width >= 1600 && before.height >= 1000
        && CGDisplayBounds(display).width >= 1600 && CGDisplayBounds(display).height >= 1000 {
        report["status"] = "already-sufficient"
        report["after"] = modeDescription(before)
        report["after_bounds"] = displayBounds(display)
        output(report)
        return
    }
    let candidates = modes.filter {
        $0.isUsableForDesktopGUI() && $0.width >= 1440 && $0.height >= 900
            && $0.pixelWidth > 0 && $0.pixelHeight > 0
            && $0.pixelWidth <= 8192 && $0.pixelHeight <= 8192
            && $0.pixelWidth * $0.pixelHeight <= 16 * 1024 * 1024
    }.sorted {
        let leftPreferred = $0.width >= 1600 && $0.height >= 1000
        let rightPreferred = $1.width >= 1600 && $1.height >= 1000
        if leftPreferred != rightPreferred { return leftPreferred }
        let leftArea = $0.width * $0.height
        let rightArea = $1.width * $1.height
        if leftArea != rightArea { return leftArea < rightArea }
        return $0.pixelWidth * $0.pixelHeight < $1.pixelWidth * $1.pixelHeight
    }
    guard let selected = candidates.first else {
        report["status"] = "unavailable"
        report["reason"] = "runner exposes no supported larger desktop mode"
        report["after"] = modeDescription(before)
        report["after_bounds"] = displayBounds(display)
        output(report)
        return
    }
    report["selected"] = modeDescription(selected)
    // CGDisplaySetDisplayMode reverts when its calling process exits. The app
    // starts after this helper exits, so retain the supported mode for this
    // ephemeral CI login session without changing permanent display settings.
    var configuration: CGDisplayConfigRef?
    let beginResult = CGBeginDisplayConfiguration(&configuration)
    report["begin_configuration_result"] = beginResult.rawValue
    guard beginResult == .success, let configuration = configuration else {
        report["status"] = "unavailable"
        report["reason"] = "CoreGraphics could not begin the display configuration transaction"
        report["after_bounds"] = displayBounds(display)
        output(report)
        return
    }
    let configureResult = CGConfigureDisplayWithDisplayMode(configuration, display, selected, nil)
    report["configure_mode_result"] = configureResult.rawValue
    guard configureResult == .success else {
        _ = CGCancelDisplayConfiguration(configuration)
        report["status"] = "unavailable"
        report["reason"] = "CoreGraphics declined the supported display mode"
        report["after_bounds"] = displayBounds(display)
        output(report)
        return
    }
    // The configuration token is consumed even if completing the transaction fails.
    let result = CGCompleteDisplayConfiguration(configuration, .forSession)
    report["coregraphics_result"] = result.rawValue
    if result != .success {
        report["status"] = "unavailable"
        report["reason"] = "CoreGraphics declined the supported display-mode change"
    } else {
        report["applied_configuration_scope"] = "current-login-session"
        let deadline = Date().addingTimeInterval(2)
        while Date() < deadline {
            if let observed = CGDisplayCopyDisplayMode(display),
               observed.ioDisplayModeID == selected.ioDisplayModeID,
               CGDisplayBounds(display).width >= 1440,
               CGDisplayBounds(display).height >= 900 {
                break
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        }
        report["status"] = "changed"
    }
    if let after = CGDisplayCopyDisplayMode(display) {
        report["after"] = modeDescription(after)
        if result == .success && (after.ioDisplayModeID != selected.ioDisplayModeID
            || CGDisplayBounds(display).width < 1440 || CGDisplayBounds(display).height < 900) {
            report["status"] = "unavailable"
            report["reason"] = "requested display mode was not observed within two seconds"
        }
    } else {
        report["status"] = "unavailable"
        report["reason"] = "CoreGraphics did not report the resulting display mode"
    }
    report["after_bounds"] = displayBounds(display)
    output(report)
}

if CommandLine.arguments.count == 2 && CommandLine.arguments[1] == "--fit-desktop" {
    fitDesktop()
    exit(0)
}

if CommandLine.arguments.count == 2 && CommandLine.arguments[1] == "--describe-desktop" {
    describeDesktop()
    exit(0)
}

let captureSheet = CommandLine.arguments.count == 5 && CommandLine.arguments[4] == "--sheet"
guard (3...4).contains(CommandLine.arguments.count) || captureSheet,
      let pid = Int32(CommandLine.arguments[1]) else {
    output(["status": "unavailable", "reason": "expected PID, PNG path and optional window ID [--sheet]"])
    exit(2)
}
let requestedWindowID: UInt32?
if CommandLine.arguments.count >= 4 {
    guard let parsed = UInt32(CommandLine.arguments[3]), parsed != kCGNullWindowID else {
        output(["status": "unavailable", "reason": "invalid requested CoreGraphics window ID"])
        exit(2)
    }
    requestedWindowID = parsed
} else {
    requestedWindowID = nil
}
guard CGPreflightScreenCaptureAccess() else {
    output(["status": "unavailable", "reason": "existing screen-recording permission is absent"])
    exit(0)
}
// Attachment and a window number precede WindowServer presentation. Observe the
// exact owned window until it is visible with stable bounds, rather than sleeping
// for an assumed sheet-animation duration or capturing some other app window.
let visibilityStarted = ProcessInfo.processInfo.systemUptime
let visibilityDeadline = visibilityStarted + 3
var selectedWindow: [String: Any]?
var previousBounds: [Double]?
var stableSince = visibilityStarted
var matchingWindows = 0
var lastCandidateBounds: Any = NSNull()
var lastCandidateAlpha: Any = NSNull()
while ProcessInfo.processInfo.systemUptime < visibilityDeadline {
    let now = ProcessInfo.processInfo.systemUptime
    let windows = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
    ) as? [[String: Any]] ?? []
    let candidates = windows.filter { window in
        guard (window[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == pid else {
            return false
        }
        if let requestedWindowID = requestedWindowID {
            return (window[kCGWindowNumber as String] as? NSNumber)?.uint32Value == requestedWindowID
        }
        return (window[kCGWindowLayer as String] as? NSNumber)?.intValue == 0
    }
    matchingWindows = candidates.count
    lastCandidateBounds = candidates.first?[kCGWindowBounds as String] ?? NSNull()
    lastCandidateAlpha = candidates.first?[kCGWindowAlpha as String] ?? NSNull()
    if candidates.count == 1, let candidate = candidates.first,
       let bounds = candidate[kCGWindowBounds as String] as? [String: NSNumber],
       let x = bounds["X"]?.doubleValue, let y = bounds["Y"]?.doubleValue,
       let width = bounds["Width"]?.doubleValue, let height = bounds["Height"]?.doubleValue,
       (captureSheet ? width > 0 && height > 0 : width >= 320 && height >= 200),
       (candidate[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 0 >= 0.99 {
        let observed = [x, y, width, height]
        if observed != previousBounds {
            previousBounds = observed
            stableSince = now
        } else if now - stableSince >= 0.15 {
            selectedWindow = candidate
            break
        }
    } else {
        previousBounds = nil
        stableSince = now
    }
    RunLoop.current.run(until: Date().addingTimeInterval(0.05))
}
guard let window = selectedWindow,
      let windowID = window[kCGWindowNumber as String] as? NSNumber else {
    output([
        "status": "unavailable",
        "reason": "requested native window was not uniquely visible with stable bounds within three seconds",
        "matching_windows": matchingWindows,
        "process_id": pid,
        "last_candidate_bounds": lastCandidateBounds,
        "last_candidate_alpha": lastCandidateAlpha,
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
        "selection": requestedWindowID == nil ? "single-ordinary-window" : "explicit-owned-window-id",
        "capture_presentation": captureSheet ? "sheet" : "window",
        "visible_stable_bounds_seconds": 0.15,
        "visibility_wait_seconds": ProcessInfo.processInfo.systemUptime - visibilityStarted,
        "window_bounds": window[kCGWindowBounds as String] ?? NSNull(),
        "screen_recording_permission": "already granted; unchanged",
    ])
} catch {
    output(["status": "unavailable", "reason": "screencapture could not start: \(error)"])
}
