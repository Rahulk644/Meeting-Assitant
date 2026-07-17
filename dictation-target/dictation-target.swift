// Minimal focused-target classifier for dictation. It reads only the focused app/role and up to eight
// characters immediately before the insertion point. It never walks the Accessibility tree, captures a
// screenshot, emits field contents, writes to disk, or uses the network.

import AppKit
import ApplicationServices
import Foundation

private struct Result: Codable {
    let kind: String
    let secure: Bool
    let startsSentence: Bool
    let liveEditable: Bool
}

private struct Target {
    let result: Result
    let element: AXUIElement?
    let insertion: CFRange?
    let identity: TargetIdentity?
}

// Web and Electron editors commonly replace their Accessibility element after every edit. Retaining only
// the original AXUIElement therefore makes the second live revision look as though focus was lost. Keep a
// content-free identity for the composer so each revision can safely reacquire its fresh focused element.
private struct TargetIdentity {
    let processIdentifier: pid_t
    let role: String
    let subrole: String
    let identifier: String
    let domIdentifier: String
    let title: String
    let description: String
    let placeholder: String
}

private struct SessionCommand: Decodable {
    let op: String
    let text: String?
}

private struct SessionResponse: Codable {
    let ok: Bool
    let active: Bool
    let error: String?
    let unsafeFallback: Bool
}

private func axGet(_ element: AXUIElement, _ attribute: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success ? value : nil
}

private func axString(_ element: AXUIElement, _ attribute: String) -> String? {
    axGet(element, attribute) as? String
}

private func axElement(_ element: AXUIElement, _ attribute: String) -> AXUIElement? {
    guard let value = axGet(element, attribute), CFGetTypeID(value) == AXUIElementGetTypeID() else { return nil }
    return (value as! AXUIElement)
}

private func selectedRange(_ element: AXUIElement) -> CFRange? {
    guard let raw = axGet(element, kAXSelectedTextRangeAttribute as String),
          CFGetTypeID(raw) == AXValueGetTypeID() else { return nil }
    var range = CFRange(location: 0, length: 0)
    return AXValueGetValue(raw as! AXValue, .cfRange, &range) ? range : nil
}

private func precedingText(_ element: AXUIElement, range: CFRange) -> String? {
    guard range.location > 0 else { return "" }
    var requested = CFRange(location: max(0, range.location - 8), length: min(8, range.location))
    guard let parameter = AXValueCreate(.cfRange, &requested) else { return nil }
    var value: CFTypeRef?
    guard AXUIElementCopyParameterizedAttributeValue(
        element,
        kAXStringForRangeParameterizedAttribute as CFString,
        parameter,
        &value
    ) == .success else { return nil }
    return value as? String
}

private func stringForRange(_ element: AXUIElement, range: CFRange) -> String? {
    var requested = range
    guard let parameter = AXValueCreate(.cfRange, &requested) else { return nil }
    var value: CFTypeRef?
    guard AXUIElementCopyParameterizedAttributeValue(
        element,
        kAXStringForRangeParameterizedAttribute as CFString,
        parameter,
        &value
    ) == .success else { return nil }
    return value as? String
}

private func isSettable(_ element: AXUIElement, _ attribute: String) -> Bool {
    var settable = DarwinBoolean(false)
    return AXUIElementIsAttributeSettable(element, attribute as CFString, &settable) == .success
        && settable.boolValue
}

private func setSelectedRange(_ element: AXUIElement, _ range: CFRange) -> Bool {
    var range = range
    guard let value = AXValueCreate(.cfRange, &range) else { return false }
    return AXUIElementSetAttributeValue(
        element,
        kAXSelectedTextRangeAttribute as CFString,
        value
    ) == .success
}

private func sameRange(_ lhs: CFRange?, _ rhs: CFRange) -> Bool {
    lhs?.location == rhs.location && lhs?.length == rhs.length
}

private func adjustedSelection(
    _ selection: CFRange,
    replacing range: CFRange,
    replacementLength: Int
) -> CFRange {
    let oldEnd = range.location + range.length
    let delta = replacementLength - range.length
    let mapPosition: (Int) -> Int = { position in
        if position <= range.location { return position }
        if position >= oldEnd { return position + delta }
        return range.location + replacementLength
    }
    let start = mapPosition(selection.location)
    let end = mapPosition(selection.location + selection.length)
    return CFRange(location: max(0, start), length: max(0, end - start))
}

private func postUnicodeText(_ text: String) -> Bool {
    guard let source = CGEventSource(stateID: .combinedSessionState) else { return false }
    let units = Array(text.utf16)
    var start = 0
    while start < units.count {
        var end = min(start + 64, units.count)
        // Do not split a surrogate pair between synthesized events.
        if end < units.count, units[end - 1] >= 0xD800, units[end - 1] <= 0xDBFF { end -= 1 }
        let chunk = Array(units[start..<end])
        guard let down = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: true),
              let up = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: false) else { return false }
        chunk.withUnsafeBufferPointer {
            down.keyboardSetUnicodeString(stringLength: $0.count, unicodeString: $0.baseAddress)
        }
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
        start = end
    }
    return true
}

private func postDelete() -> Bool {
    guard let source = CGEventSource(stateID: .combinedSessionState),
          let down = CGEvent(keyboardEventSource: source, virtualKey: 51, keyDown: true),
          let up = CGEvent(keyboardEventSource: source, virtualKey: 51, keyDown: false) else { return false }
    down.post(tap: .cghidEventTap)
    up.post(tap: .cghidEventTap)
    return true
}

// Setting AXSelectedText reports success in several controlled web editors while the application silently
// discards the edit. Selection through AX plus genuine Unicode keyboard input updates both native and web
// editor state, so React/Electron sees the same input events as normal typing.
private func replaceRangeThroughInput(_ element: AXUIElement, range: CFRange, with text: String) -> Bool {
    guard setSelectedRange(element, range) else { return false }
    if text.isEmpty { return range.length == 0 || postDelete() }
    return postUnicodeText(text)
}

private func startsSentence(_ focused: AXUIElement) -> Bool {
    guard let range = selectedRange(focused) else { return false }
    guard range.location > 0 else { return true }
    guard let before = precedingText(focused, range: range) else { return false }
    if before.hasSuffix("\n") { return true }
    guard let last = before.trimmingCharacters(in: .whitespaces).last else { return true }
    return ".!?।".contains(last)
}

private func isSecure(role: String, subrole: String, label: String) -> Bool {
    let structure = "\(role) \(subrole)".lowercased()
    if structure.contains("securetextfield") || structure.contains("password") { return true }
    let semantic = label.lowercased()
    return ["password", "passcode", "pin", "secret", "api key", "credit card", "cvv", "ssn"]
        .contains { semantic.contains($0) }
}

private func isCodeApp(bundleId: String, name: String) -> Bool {
    let ids = [
        "com.apple.Terminal", "com.googlecode.iterm2", "com.microsoft.VSCode", "com.microsoft.VSCodeInsiders",
        "dev.zed.Zed", "com.todesktop.230313mzl4w4u92", "com.jetbrains.intellij", "com.jetbrains.AppCode",
        "com.jetbrains.CLion", "com.jetbrains.PyCharm", "com.sublimetext.4",
    ]
    if ids.contains(where: { bundleId == $0 || bundleId.hasPrefix($0 + ".") }) { return true }
    let lower = name.lowercased()
    return ["terminal", "iterm", "visual studio code", "cursor", "zed", "xcode"].contains { lower.contains($0) }
}

private func focusedTarget() -> (app: NSRunningApplication, element: AXUIElement)? {
    #if LIVE_EDIT_TEST
    // Compiled only by the integration test binary. It lets the test exercise exact AX reconciliation in
    // a synthetic TextEdit document while Codex itself must remain the frontmost app during tool calls.
    if let bundleId = ProcessInfo.processInfo.environment["DICTATION_TARGET_TEST_BUNDLE_ID"],
       let app = NSRunningApplication.runningApplications(withBundleIdentifier: bundleId).first {
        let appElement = AXUIElementCreateApplication(app.processIdentifier)
        if let focused = axElement(appElement, kAXFocusedUIElementAttribute as String) {
            return (app, focused)
        }
    }
    #endif
    // The system-wide focused-application attribute intermittently returns no element for a background CLI
    // helper even while another app is visibly frontmost. NSWorkspace is the stable source of app identity;
    // query that app's AX root for the focused control without walking its accessibility tree.
    guard let app = NSWorkspace.shared.frontmostApplication else { return nil }
    let appElement = AXUIElementCreateApplication(app.processIdentifier)
    guard let focused = axElement(appElement, kAXFocusedUIElementAttribute as String) else { return nil }
    return (app, focused)
}

private func targetIdentity(
    app: NSRunningApplication,
    element: AXUIElement,
    role: String,
    subrole: String
) -> TargetIdentity {
    TargetIdentity(
        processIdentifier: app.processIdentifier,
        role: role,
        subrole: subrole,
        identifier: axString(element, kAXIdentifierAttribute as String) ?? "",
        domIdentifier: axString(element, "AXDOMIdentifier") ?? "",
        title: axString(element, kAXTitleAttribute as String) ?? "",
        description: axString(element, kAXDescriptionAttribute as String) ?? "",
        placeholder: axString(element, kAXPlaceholderValueAttribute as String) ?? ""
    )
}

private func matchesIdentity(
    _ candidate: AXUIElement,
    original: AXUIElement,
    identity: TargetIdentity
) -> Bool {
    var processIdentifier: pid_t = 0
    guard AXUIElementGetPid(candidate, &processIdentifier) == .success,
          processIdentifier == identity.processIdentifier else { return false }
    if CFEqual(candidate, original) { return true }
    guard axString(candidate, kAXRoleAttribute as String) == identity.role,
          (axString(candidate, kAXSubroleAttribute as String) ?? "") == identity.subrole else { return false }

    let identifier = axString(candidate, kAXIdentifierAttribute as String) ?? ""
    if !identity.identifier.isEmpty, identifier == identity.identifier { return true }
    let domIdentifier = axString(candidate, "AXDOMIdentifier") ?? ""
    if !identity.domIdentifier.isEmpty, domIdentifier == identity.domIdentifier { return true }

    // Native fields generally preserve CF identity. This semantic fallback is for a re-rendered web
    // composer; it stays inside the original process and requires every available non-content label to
    // remain identical. The tracked provisional span is checked separately before any mutation.
    let semantics = [
        (identity.title, axString(candidate, kAXTitleAttribute as String) ?? ""),
        (identity.description, axString(candidate, kAXDescriptionAttribute as String) ?? ""),
        (identity.placeholder, axString(candidate, kAXPlaceholderValueAttribute as String) ?? ""),
    ]
    let available = semantics.filter { !$0.0.isEmpty }
    return available.isEmpty || available.allSatisfy { $0.0 == $0.1 }
}

private func focusedSessionElement(
    original: AXUIElement,
    identity: TargetIdentity
) -> AXUIElement? {
    guard let focused = focusedTarget(),
          focused.app.processIdentifier == identity.processIdentifier,
          matchesIdentity(focused.element, original: original, identity: identity) else { return nil }
    return focused.element
}

private func waitForTrackedText(
    original: AXUIElement,
    identity: TargetIdentity,
    range: CFRange,
    text: String,
    expectedCursor: CFRange,
    timeout: TimeInterval = 0.35
) -> AXUIElement? {
    let deadline = Date().addingTimeInterval(timeout)
    repeat {
        if let focused = focusedSessionElement(original: original, identity: identity),
           stringForRange(focused, range: range) == text,
           sameRange(selectedRange(focused), expectedCursor) {
            return focused
        }
        Thread.sleep(forTimeInterval: 0.015)
    } while Date() < deadline
    return nil
}

private func removeTrackedText(
    original: AXUIElement,
    identity: TargetIdentity,
    range: CFRange,
    text: String
) -> Bool {
    guard !text.isEmpty,
          let focused = focusedSessionElement(original: original, identity: identity),
          stringForRange(focused, range: range) == text,
          replaceRangeThroughInput(focused, range: range, with: "") else { return text.isEmpty }
    let cursor = CFRange(location: range.location, length: 0)
    if waitForTrackedText(
        original: original,
        identity: identity,
        range: cursor,
        text: "",
        expectedCursor: cursor
    ) != nil {
        return true
    }
    return false
}

private func classifyTarget() -> Target {
    // Without trusted target evidence, fail closed so Rust skips contextual processing and text history.
    guard AXIsProcessTrusted() else {
        return Target(
            result: Result(kind: "secure", secure: true, startsSentence: false, liveEditable: false),
            element: nil,
            insertion: nil,
            identity: nil
        )
    }
    guard let focusedTarget = focusedTarget() else {
        return Target(
            result: Result(kind: "unknown", secure: false, startsSentence: false, liveEditable: false),
            element: nil,
            insertion: nil,
            identity: nil
        )
    }
    let focused = focusedTarget.element
    let app = focusedTarget.app
    let bundleId = app.bundleIdentifier ?? ""
    let name = app.localizedName ?? ""
    let role = axString(focused, kAXRoleAttribute as String) ?? ""
    let subrole = axString(focused, kAXSubroleAttribute as String) ?? ""
    let label = [
        axString(focused, kAXTitleAttribute as String),
        axString(focused, kAXDescriptionAttribute as String),
        axString(focused, kAXPlaceholderValueAttribute as String),
    ].compactMap { $0 }.joined(separator: " ")
    let secure = isSecure(role: role, subrole: subrole, label: label)
    if secure {
        return Target(
            result: Result(kind: "secure", secure: true, startsSentence: false, liveEditable: false),
            element: nil,
            insertion: nil,
            identity: nil
        )
    }
    if isCodeApp(bundleId: bundleId, name: name) {
        return Target(
            result: Result(kind: "codeOrTerminal", secure: false, startsSentence: false, liveEditable: false),
            element: nil,
            insertion: nil,
            identity: nil
        )
    }
    let editableRoles = ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"]
    guard editableRoles.contains(role) else {
        return Target(
            result: Result(kind: "unknown", secure: false, startsSentence: false, liveEditable: false),
            element: nil,
            insertion: nil,
            identity: nil
        )
    }
    let insertion = selectedRange(focused)
    let liveEditable = insertion?.length == 0
        && isSettable(focused, kAXSelectedTextAttribute as String)
        && isSettable(focused, kAXSelectedTextRangeAttribute as String)
        && insertion.flatMap { stringForRange(focused, range: $0) } != nil
    return Target(
        result: Result(
            kind: "naturalLanguage",
            secure: false,
            startsSentence: startsSentence(focused),
            liveEditable: liveEditable
        ),
        element: liveEditable ? focused : nil,
        insertion: liveEditable ? insertion : nil,
        identity: liveEditable
            ? targetIdentity(app: app, element: focused, role: role, subrole: subrole)
            : nil
    )
}

private func emit<T: Encodable>(_ value: T) {
    let encoder = JSONEncoder()
    if let data = try? encoder.encode(value) {
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([0x0A]))
    }
}

private func runSession(_ target: Target) {
    emit(target.result)
    guard target.result.liveEditable,
          let original = target.element,
          let insertion = target.insertion,
          let identity = target.identity else { return }

    var current = ""
    while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let command = try? JSONDecoder().decode(SessionCommand.self, from: data) else {
            emit(SessionResponse(
                ok: false,
                active: !current.isEmpty,
                error: "invalidCommand",
                unsafeFallback: !current.isEmpty
            ))
            continue
        }
        let trackedRange = CFRange(location: insertion.location, length: current.utf16.count)
        let expectedCursor = CFRange(location: trackedRange.location + trackedRange.length, length: 0)

        switch command.op {
        case "preview":
            let next = String((command.text ?? "").prefix(4_000))
            guard let focused = focusedSessionElement(original: original, identity: identity) else {
                emit(SessionResponse(
                    ok: false,
                    active: !current.isEmpty,
                    error: "targetChanged",
                    unsafeFallback: true
                ))
                continue
            }
            guard stringForRange(focused, range: trackedRange) == current else {
                emit(SessionResponse(
                    ok: false,
                    active: !current.isEmpty,
                    error: "provisionalTextChanged",
                    unsafeFallback: true
                ))
                continue
            }
            guard sameRange(selectedRange(focused), expectedCursor) else {
                emit(SessionResponse(
                    ok: false,
                    active: !current.isEmpty,
                    error: "cursorChanged",
                    unsafeFallback: true
                ))
                continue
            }
            guard replaceRangeThroughInput(focused, range: trackedRange, with: next) else {
                emit(SessionResponse(
                    ok: false,
                    active: !current.isEmpty,
                    error: "previewEditRejected",
                    unsafeFallback: false
                ))
                continue
            }
            let nextRange = CFRange(location: insertion.location, length: next.utf16.count)
            let nextCursor = CFRange(location: nextRange.location + nextRange.length, length: 0)
            guard waitForTrackedText(
                original: original,
                identity: identity,
                range: nextRange,
                text: next,
                expectedCursor: nextCursor
            ) != nil else {
                emit(SessionResponse(
                    ok: false,
                    active: !current.isEmpty,
                    error: "previewEditNotObserved",
                    unsafeFallback: !current.isEmpty
                ))
                continue
            }
            current = next
            emit(SessionResponse(ok: true, active: !current.isEmpty, error: nil, unsafeFallback: false))
        case "commit":
            let final = String((command.text ?? "").prefix(20_000))
            guard let focused = focusedSessionElement(original: original, identity: identity),
                  stringForRange(focused, range: trackedRange) == current else {
                emit(SessionResponse(
                    ok: false,
                    active: !current.isEmpty,
                    error: "provisionalTextUnavailable",
                    unsafeFallback: true
                ))
                return
            }
            let previousSelection = selectedRange(focused)
            guard replaceRangeThroughInput(focused, range: trackedRange, with: final) else {
                emit(SessionResponse(
                    ok: false,
                    active: !current.isEmpty,
                    error: "finalEditRejected",
                    unsafeFallback: true
                ))
                return
            }
            let finalRange = CFRange(location: insertion.location, length: final.utf16.count)
            let finalCursor = CFRange(location: finalRange.location + finalRange.length, length: 0)
            guard let resolved = waitForTrackedText(
                original: original,
                identity: identity,
                range: finalRange,
                text: final,
                expectedCursor: finalCursor,
                timeout: 0.6
            ) else {
                emit(SessionResponse(
                    ok: false,
                    active: !current.isEmpty,
                    error: "finalEditNotObserved",
                    unsafeFallback: true
                ))
                return
            }
            if let previousSelection, !sameRange(previousSelection, expectedCursor) {
                _ = setSelectedRange(
                    resolved,
                    adjustedSelection(previousSelection, replacing: trackedRange, replacementLength: final.utf16.count)
                )
            }
            emit(SessionResponse(ok: true, active: false, error: nil, unsafeFallback: false))
            return
        case "cancel":
            let removed = removeTrackedText(
                original: original,
                identity: identity,
                range: trackedRange,
                text: current
            )
            emit(SessionResponse(
                ok: removed,
                active: !removed && !current.isEmpty,
                error: removed ? nil : "previewRemovalRejected",
                unsafeFallback: !removed && !current.isEmpty
            ))
            return
        default:
            emit(SessionResponse(
                ok: false,
                active: !current.isEmpty,
                error: "unknownCommand",
                unsafeFallback: !current.isEmpty
            ))
        }
    }
    let trackedRange = CFRange(location: insertion.location, length: current.utf16.count)
    if !current.isEmpty {
        _ = removeTrackedText(
        original: original,
        identity: identity,
        range: trackedRange,
        text: current
        )
    }
}

if CommandLine.arguments.contains("--selftest") {
    precondition(isCodeApp(bundleId: "com.apple.Terminal", name: "Terminal"))
    precondition(isSecure(role: "AXSecureTextField", subrole: "", label: ""))
    precondition(!isSecure(role: "AXTextArea", subrole: "", label: "Message"))
    print("dictation-target selftest ok")
} else if CommandLine.arguments.contains("--session") {
    runSession(classifyTarget())
} else {
    emit(classifyTarget().result)
}
