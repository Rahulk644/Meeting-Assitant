// agent-context.swift
//
// Bounded, read-only macOS Accessibility snapshot for the Agents foundation.
// It is invoked on demand, prints one JSON object, and exits. It never captures
// audio, takes screenshots, changes another app, writes to disk, or uses the network.
//
// Build: swiftc -O agent-context.swift -o agent-context
// Run:   ./agent-context --once | ./agent-context --selftest

import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

private let maxElements = 180
private let maxVisitedElements = 1_200
private let maxDepth = 12
private let maxChildrenPerElement = 80
private let captureBudget: TimeInterval = 0.75

private struct Frame: Codable {
    let x: Double
    let y: Double
    let width: Double
    let height: Double
}

private struct Element: Codable {
    let id: String
    let role: String
    let subrole: String?
    let label: String?
    let value: String?
    let frame: Frame?
    let enabled: Bool
    let focused: Bool
    let editable: Bool
    let secure: Bool
    let actions: [String]
}

private struct Snapshot: Codable {
    let ok: Bool
    let status: String
    let snapshotId: String?
    let capturedAt: Int
    let source: String
    let appName: String?
    let bundleId: String?
    let windowTitle: String?
    let focusedRole: String?
    let focusedLabel: String?
    let selectedText: String?
    let editable: Bool
    let elementCount: Int
    let truncated: Bool
    let elements: [Element]
    let privacy: [String]
    let error: String?
}

private struct Pending {
    let element: AXUIElement
    let depth: Int
}

private func nowMs() -> Int {
    Int(Date().timeIntervalSince1970 * 1000)
}

private func emit(_ snapshot: Snapshot) {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    guard let data = try? encoder.encode(snapshot) else {
        print(#"{"ok":false,"status":"encoding_error"}"#)
        return
    }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0A]))
}

private func axGet(_ element: AXUIElement, _ attribute: String) -> CFTypeRef? {
    var value: CFTypeRef?
    let result = AXUIElementCopyAttributeValue(element, attribute as CFString, &value)
    return result == .success ? value : nil
}

private func axString(_ element: AXUIElement, _ attribute: String) -> String? {
    axGet(element, attribute) as? String
}

private func axBool(_ element: AXUIElement, _ attribute: String, default fallback: Bool) -> Bool {
    (axGet(element, attribute) as? NSNumber)?.boolValue ?? fallback
}

private func axElement(_ element: AXUIElement, _ attribute: String) -> AXUIElement? {
    guard let raw = axGet(element, attribute), CFGetTypeID(raw) == AXUIElementGetTypeID() else {
        return nil
    }
    return (raw as! AXUIElement)
}

private func axElements(_ element: AXUIElement, _ attribute: String) -> [AXUIElement] {
    (axGet(element, attribute) as? [AXUIElement]) ?? []
}

private func axFrame(_ element: AXUIElement) -> Frame? {
    guard let positionValue = axGet(element, kAXPositionAttribute as String),
          let sizeValue = axGet(element, kAXSizeAttribute as String),
          CFGetTypeID(positionValue) == AXValueGetTypeID(),
          CFGetTypeID(sizeValue) == AXValueGetTypeID() else { return nil }
    var point = CGPoint.zero
    var size = CGSize.zero
    guard AXValueGetValue(positionValue as! AXValue, .cgPoint, &point),
          AXValueGetValue(sizeValue as! AXValue, .cgSize, &size) else { return nil }
    return Frame(x: point.x, y: point.y, width: size.width, height: size.height)
}

private func actions(_ element: AXUIElement) -> [String] {
    var names: CFArray?
    guard AXUIElementCopyActionNames(element, &names) == .success,
          let values = names as? [String] else { return [] }
    return Array(values.prefix(12))
}

private func isValueSettable(_ element: AXUIElement) -> Bool {
    var settable = DarwinBoolean(false)
    return AXUIElementIsAttributeSettable(element, kAXValueAttribute as CFString, &settable) == .success
        && settable.boolValue
}

private func isEditableControl(_ element: AXUIElement, role: String, secure: Bool) -> Bool {
    guard !secure else { return false }
    let editableRoles = ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"]
    return editableRoles.contains(role) && isValueSettable(element)
}

private func clean(_ raw: String?, limit: Int = 280) -> String? {
    guard let raw else { return nil }
    let value = raw
        .replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
        .trimmingCharacters(in: .whitespacesAndNewlines)
    guard !value.isEmpty else { return nil }
    if value.count <= limit { return value }
    return String(value.prefix(limit)) + "…"
}

private func isSecure(role: String, subrole: String?, label: String?) -> Bool {
    let structural = "\(role) \(subrole ?? "")".lowercased()
    if structural.contains("securetextfield") || structural.contains("password") { return true }
    let semantic = (label ?? "").lowercased()
    let terms = ["password", "passcode", "pin", "secret", "api key", "credit card", "cvv", "ssn"]
    return terms.contains { semantic.contains($0) }
}

private func readableValue(_ element: AXUIElement, secure: Bool) -> String? {
    if secure { return nil }
    guard let raw = axGet(element, kAXValueAttribute as String) else { return nil }
    if let text = raw as? String { return clean(text) }
    if let number = raw as? NSNumber { return clean(number.stringValue) }
    return nil
}

private func label(_ element: AXUIElement) -> String? {
    let candidates = [
        axString(element, kAXTitleAttribute as String),
        axString(element, kAXDescriptionAttribute as String),
        axString(element, kAXHelpAttribute as String),
        axString(element, kAXPlaceholderValueAttribute as String),
    ]
    var seen = Set<String>()
    let parts = candidates.compactMap { clean($0, limit: 160) }.filter { seen.insert($0).inserted }
    return parts.isEmpty ? nil : parts.joined(separator: " · ")
}

private func isOurApp(_ app: NSRunningApplication) -> Bool {
    app.bundleIdentifier == "com.myassistant.desktop" || app.localizedName == "My Assistant"
}

private func topmostAppBehindAssistant() -> NSRunningApplication? {
    let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
    guard let rows = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
        return nil
    }
    for row in rows {
        guard (row[kCGWindowLayer as String] as? NSNumber)?.intValue == 0,
              let pidValue = row[kCGWindowOwnerPID as String] as? NSNumber else { continue }
        if let alpha = (row[kCGWindowAlpha as String] as? NSNumber)?.doubleValue, alpha <= 0 { continue }
        if let bounds = row[kCGWindowBounds as String] as? [String: Any],
           let width = (bounds["Width"] as? NSNumber)?.doubleValue,
           let height = (bounds["Height"] as? NSNumber)?.doubleValue,
           (width < 80 || height < 50) { continue }
        guard let app = NSRunningApplication(processIdentifier: pid_t(pidValue.int32Value)),
              !isOurApp(app), app.activationPolicy == .regular else { continue }
        return app
    }
    return nil
}

private func targetApp() -> (NSRunningApplication?, String) {
    if let frontmost = NSWorkspace.shared.frontmostApplication, !isOurApp(frontmost) {
        return (frontmost, "frontmost_app")
    }
    return (topmostAppBehindAssistant(), "app_behind_assistant")
}

private func focusedWindow(_ appElement: AXUIElement) -> AXUIElement? {
    axElement(appElement, kAXFocusedWindowAttribute as String)
        ?? axElements(appElement, kAXWindowsAttribute as String).first
}

private func makeElement(_ element: AXUIElement, id: String) -> Element {
    let role = axString(element, kAXRoleAttribute as String) ?? "AXUnknown"
    let subrole = clean(axString(element, kAXSubroleAttribute as String), limit: 80)
    let elementLabel = label(element)
    let secure = isSecure(role: role, subrole: subrole, label: elementLabel)
    let focused = axBool(element, kAXFocusedAttribute as String, default: false)
    return Element(
        id: id,
        role: role,
        subrole: subrole,
        label: secure ? clean(elementLabel, limit: 80) : elementLabel,
        value: readableValue(element, secure: secure),
        frame: axFrame(element),
        enabled: axBool(element, kAXEnabledAttribute as String, default: true),
        focused: focused,
        editable: isEditableControl(element, role: role, secure: secure),
        secure: secure,
        actions: actions(element)
    )
}

private func capture() -> Snapshot {
    let capturedAt = nowMs()
    guard AXIsProcessTrusted() else {
        return Snapshot(ok: false, status: "permission_required", snapshotId: nil, capturedAt: capturedAt,
                        source: "none", appName: nil, bundleId: nil, windowTitle: nil,
                        focusedRole: nil, focusedLabel: nil, selectedText: nil, editable: false,
                        elementCount: 0, truncated: false, elements: [],
                        privacy: ["read_only", "on_demand", "no_screenshot", "no_network", "no_persistence"],
                        error: "Accessibility permission is required.")
    }
    let (runningApp, source) = targetApp()
    guard let runningApp else {
        return Snapshot(ok: false, status: "no_target", snapshotId: nil, capturedAt: capturedAt,
                        source: source, appName: nil, bundleId: nil, windowTitle: nil,
                        focusedRole: nil, focusedLabel: nil, selectedText: nil, editable: false,
                        elementCount: 0, truncated: false, elements: [],
                        privacy: ["read_only", "on_demand", "no_screenshot", "no_network", "no_persistence"],
                        error: "No inspectable app window was found.")
    }

    let appElement = AXUIElementCreateApplication(runningApp.processIdentifier)
    AXUIElementSetMessagingTimeout(appElement, 0.2)
    guard let window = focusedWindow(appElement) else {
        return Snapshot(ok: false, status: "no_window", snapshotId: nil, capturedAt: capturedAt,
                        source: source, appName: runningApp.localizedName,
                        bundleId: runningApp.bundleIdentifier, windowTitle: nil,
                        focusedRole: nil, focusedLabel: nil, selectedText: nil, editable: false,
                        elementCount: 0, truncated: false, elements: [],
                        privacy: ["read_only", "on_demand", "no_screenshot", "no_network", "no_persistence"],
                        error: "The target app did not expose an Accessibility window.")
    }
    AXUIElementSetMessagingTimeout(window, 0.2)

    let deadline = Date().addingTimeInterval(captureBudget)
    var queue = [Pending(element: window, depth: 0)]
    var cursor = 0
    var output: [Element] = []
    var visited = 0
    var truncated = false
    while cursor < queue.count {
        if output.count >= maxElements || visited >= maxVisitedElements || Date() >= deadline {
            truncated = true
            break
        }
        let pending = queue[cursor]
        cursor += 1
        visited += 1
        AXUIElementSetMessagingTimeout(pending.element, 0.12)
        let candidate = makeElement(pending.element, id: "el_\(output.count)")
        let meaningful = candidate.label != nil || candidate.value != nil || candidate.focused
            || candidate.editable || !candidate.actions.isEmpty || candidate.role == "AXWindow"
        if meaningful { output.append(candidate) }
        guard pending.depth < maxDepth else {
            if !axElements(pending.element, kAXChildrenAttribute as String).isEmpty { truncated = true }
            continue
        }
        let children = axElements(pending.element, kAXChildrenAttribute as String)
        if children.count > maxChildrenPerElement { truncated = true }
        queue.append(contentsOf: children.prefix(maxChildrenPerElement).map {
            Pending(element: $0, depth: pending.depth + 1)
        })
    }

    let focused = axElement(appElement, kAXFocusedUIElementAttribute as String)
    let focusedRole = focused.flatMap { clean(axString($0, kAXRoleAttribute as String), limit: 80) }
    let focusedLabel = focused.flatMap { label($0) }
    let focusedSecure = isSecure(role: focusedRole ?? "", subrole: focused.flatMap {
        axString($0, kAXSubroleAttribute as String)
    }, label: focusedLabel)
    let selected = focusedSecure ? nil : focused.flatMap {
        clean(axString($0, kAXSelectedTextAttribute as String), limit: 500)
    }
    let editable = focused.map {
        isEditableControl($0, role: focusedRole ?? "", secure: focusedSecure)
    } ?? false

    return Snapshot(ok: true, status: "ready", snapshotId: UUID().uuidString.lowercased(),
                    capturedAt: capturedAt, source: source, appName: runningApp.localizedName,
                    bundleId: runningApp.bundleIdentifier,
                    windowTitle: clean(axString(window, kAXTitleAttribute as String), limit: 180),
                    focusedRole: focusedRole, focusedLabel: focusedSecure ? nil : focusedLabel,
                    selectedText: selected, editable: editable, elementCount: output.count,
                    truncated: truncated, elements: output,
                    privacy: ["read_only", "on_demand", "secure_fields_redacted", "no_screenshot", "no_network", "no_persistence"],
                    error: nil)
}

private func selfTest() -> Bool {
    clean("  hello\n  world  ") == "hello world"
        && clean(String(repeating: "a", count: 300))?.hasSuffix("…") == true
        && isSecure(role: "AXTextField", subrole: "AXSecureTextField", label: nil)
        && isSecure(role: "AXTextField", subrole: nil, label: "API key")
        && !isSecure(role: "AXTextArea", subrole: nil, label: "Email body")
}

if CommandLine.arguments.contains("--selftest") {
    if selfTest() {
        print("agent-context self-test passed")
        exit(0)
    }
    fputs("agent-context self-test failed\n", stderr)
    exit(1)
}

guard CommandLine.arguments.contains("--once") else {
    fputs("usage: agent-context --once | --selftest\n", stderr)
    exit(2)
}
emit(capture())
