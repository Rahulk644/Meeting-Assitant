// agent-action.swift
//
// Narrow macOS Accessibility executor for the first Agents mutation milestone.
// It can apply one text draft to an explicitly captured disposable TextEdit
// control or undo that exact change. Every operation revalidates the process,
// app, window, control, and content fingerprint. Requests arrive through stdin so
// draft/original text never appears in process arguments; responses are JSON and
// the helper immediately exits.

import AppKit
import ApplicationServices
import CryptoKit
import Foundation

private let maximumTextLength = 64_000
private let allowedPreviewBundleId = "com.apple.TextEdit"
private let dashboardBundleId = "com.myassistant.desktop"

private struct ExpectedTarget: Codable {
    let bundleId: String
    let processIdentifier: Int32
    let windowTitle: String?
    let focusedRole: String
    let focusedLabel: String?
    let focusedFingerprint: String
}

private struct UndoPayload: Codable {
    let expected: ExpectedTarget
    let originalValue: String
    let originalSelectionLocation: Int?
    let originalSelectionLength: Int?
    let appliedContentFingerprint: String
}

private struct ActionRequest: Decodable {
    let operation: String
    let action: String?
    let text: String?
    let expected: ExpectedTarget
    let undo: UndoPayload?
}

private struct ActionResponse: Encodable {
    let ok: Bool
    let status: String
    let verified: Bool
    let undo: UndoPayload?
    let error: String?
}

private func emit(_ response: ActionResponse) {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    guard let data = try? encoder.encode(response) else {
        print(#"{"ok":false,"status":"encoding_error","verified":false}"#)
        return
    }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0A]))
}

private func failure(_ status: String, _ error: String) -> ActionResponse {
    ActionResponse(ok: false, status: status, verified: false, undo: nil, error: error)
}

private func axGet(_ element: AXUIElement, _ attribute: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success ? value : nil
}

private func axString(_ element: AXUIElement, _ attribute: String) -> String? {
    axGet(element, attribute) as? String
}

private func axElement(_ element: AXUIElement, _ attribute: String) -> AXUIElement? {
    guard let raw = axGet(element, attribute), CFGetTypeID(raw) == AXUIElementGetTypeID() else { return nil }
    return (raw as! AXUIElement)
}

private func axElements(_ element: AXUIElement, _ attribute: String) -> [AXUIElement] {
    (axGet(element, attribute) as? [AXUIElement]) ?? []
}

private func selectedRange(_ element: AXUIElement) -> CFRange? {
    guard let raw = axGet(element, kAXSelectedTextRangeAttribute as String),
          CFGetTypeID(raw) == AXValueGetTypeID() else { return nil }
    var range = CFRange(location: 0, length: 0)
    return AXValueGetValue(raw as! AXValue, .cfRange, &range) ? range : nil
}

private func isAttributeSettable(_ element: AXUIElement, _ attribute: String) -> Bool {
    var settable = DarwinBoolean(false)
    return AXUIElementIsAttributeSettable(element, attribute as CFString, &settable) == .success
        && settable.boolValue
}

private func isValueSettable(_ element: AXUIElement) -> Bool {
    isAttributeSettable(element, kAXValueAttribute as String)
        || isAttributeSettable(element, kAXSelectedTextAttribute as String)
}

private func clean(_ raw: String?, limit: Int = 280) -> String? {
    guard let raw else { return nil }
    let value = raw
        .replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
        .trimmingCharacters(in: .whitespacesAndNewlines)
    guard !value.isEmpty else { return nil }
    return value.count <= limit ? value : String(value.prefix(limit)) + "…"
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

private func isSecure(role: String, subrole: String?, label: String?) -> Bool {
    let structural = "\(role) \(subrole ?? "")".lowercased()
    if structural.contains("securetextfield") || structural.contains("password") { return true }
    let semantic = (label ?? "").lowercased()
    return ["password", "passcode", "pin", "secret", "api key", "credit card", "cvv", "ssn"]
        .contains { semantic.contains($0) }
}

private func isEditable(_ element: AXUIElement, role: String, secure: Bool) -> Bool {
    !secure
        && ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"].contains(role)
        && isValueSettable(element)
}

private func digest(_ value: String) -> String {
    SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
}

private func contentFingerprint(role: String, label: String?, value: String) -> String {
    digest("v1\u{1f}\(role)\u{1f}\(label ?? "")\u{1f}\(value)")
}

private func stateFingerprint(role: String, label: String?, value: String, range: CFRange?) -> String {
    digest("v1\u{1f}\(role)\u{1f}\(label ?? "")\u{1f}\(value)\u{1f}\(range?.location ?? -1):\(range?.length ?? -1)")
}

private func focusedWindow(_ appElement: AXUIElement) -> AXUIElement? {
    axElement(appElement, kAXFocusedWindowAttribute as String)
        ?? axElements(appElement, kAXWindowsAttribute as String).first
}

private struct LiveTarget {
    let element: AXUIElement
    let role: String
    let label: String?
    let value: String
    let range: CFRange?
}

private enum TargetResolution {
    case success(LiveTarget)
    case failure(ActionResponse)
}

private func isActionGateWindow(
    ownerPID: Int32,
    layer: Int,
    alpha: Double,
    bounds: CGRect,
    bundleId: String?,
    isRegularApp: Bool
) -> Bool {
    ownerPID > 0
        && ownerPID != getpid()
        && layer == 0
        && alpha > 0
        && bounds.width >= 80
        && bounds.height >= 60
        && bundleId != dashboardBundleId
        && isRegularApp
}

private func appBehindDashboardProcessIdentifier() -> Int32? {
    guard let windows = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements],
        kCGNullWindowID
    ) as? [[String: Any]] else { return nil }

    for window in windows {
        guard let ownerNumber = window[kCGWindowOwnerPID as String] as? NSNumber else { continue }
        let ownerPID = ownerNumber.int32Value
        let layer = (window[kCGWindowLayer as String] as? NSNumber)?.intValue ?? -1
        let alpha = (window[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 1
        var bounds = CGRect.zero
        guard let boundsDictionary = window[kCGWindowBounds as String] as? NSDictionary,
              CGRectMakeWithDictionaryRepresentation(boundsDictionary, &bounds),
              let app = NSRunningApplication(processIdentifier: ownerPID),
              !app.isTerminated,
              isActionGateWindow(
                  ownerPID: ownerPID,
                  layer: layer,
                  alpha: alpha,
                  bounds: bounds,
                  bundleId: app.bundleIdentifier,
                  isRegularApp: app.activationPolicy == .regular
              ) else { continue }
        return ownerPID
    }
    return nil
}

private func resolveTarget(_ expected: ExpectedTarget, requireOriginalFingerprint: Bool) -> TargetResolution {
    guard AXIsProcessTrusted() else { return .failure(failure("permission_required", "Accessibility permission is required.")) }
    guard expected.bundleId == allowedPreviewBundleId else {
        return .failure(failure(
            "unsupported_target",
            "Guarded Apply is currently limited to a disposable TextEdit document."
        ))
    }
    guard expected.processIdentifier > 0,
          let app = NSRunningApplication(processIdentifier: expected.processIdentifier),
          !app.isTerminated,
          app.bundleIdentifier == expected.bundleId else {
        return .failure(failure("stale_target", "The exact target app process is no longer available. Prepare the draft again."))
    }
    // The dashboard necessarily becomes frontmost when its Apply button is clicked. Resolve the first
    // ordinary on-screen window behind it before activating anything: it must still belong to the exact
    // captured TextEdit process. This makes an intervening app switch fail closed instead of silently
    // returning to an older target.
    guard appBehindDashboardProcessIdentifier() == app.processIdentifier else {
        return .failure(failure("stale_target", "Another app was opened after this preview. Prepare the draft again."))
    }
    // Bring the already revalidated target forward so rich editors such as TextEdit accept their
    // selected-text Accessibility mutation and the user sees the result immediately.
    guard app.activate(options: []) else {
        return .failure(failure("activation_failed", "The target app could not be brought forward safely."))
    }
    Thread.sleep(forTimeInterval: 0.08)
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else {
        return .failure(failure("stale_target", "Another app took focus before the edit could be applied."))
    }
    let appElement = AXUIElementCreateApplication(app.processIdentifier)
    AXUIElementSetMessagingTimeout(appElement, 0.2)
    guard let window = focusedWindow(appElement) else {
        return .failure(failure("stale_target", "The target window is no longer available."))
    }
    let title = clean(axString(window, kAXTitleAttribute as String), limit: 180)
    if title != expected.windowTitle {
        return .failure(failure("stale_target", "The target window changed. Prepare the draft again."))
    }
    guard let element = axElement(appElement, kAXFocusedUIElementAttribute as String) else {
        return .failure(failure("stale_target", "The editable target lost focus."))
    }
    AXUIElementSetMessagingTimeout(element, 0.2)
    let role = axString(element, kAXRoleAttribute as String) ?? "AXUnknown"
    let elementLabel = label(element)
    let secure = isSecure(role: role, subrole: axString(element, kAXSubroleAttribute as String), label: elementLabel)
    guard role == expected.focusedRole, elementLabel == expected.focusedLabel,
          isEditable(element, role: role, secure: secure) else {
        return .failure(failure("stale_target", "The focused control changed or is not safely editable."))
    }
    guard let value = axString(element, kAXValueAttribute as String), value.count <= maximumTextLength else {
        return .failure(failure("unsupported_target", "The focused control did not expose a bounded text value."))
    }
    let range = selectedRange(element)
    if requireOriginalFingerprint,
       stateFingerprint(role: role, label: elementLabel, value: value, range: range) != expected.focusedFingerprint {
        return .failure(failure("stale_target", "The text or selection changed. Prepare the draft again."))
    }
    return .success(LiveTarget(element: element, role: role, label: elementLabel, value: value, range: range))
}

private func replacing(_ value: String, range: CFRange, with text: String) -> String? {
    let source = value as NSString
    guard range.location >= 0, range.length >= 0,
          range.location <= source.length, range.location + range.length <= source.length else { return nil }
    let result = NSMutableString(string: value)
    result.replaceCharacters(in: NSRange(location: range.location, length: range.length), with: text)
    return result.length <= maximumTextLength ? result as String : nil
}

private func valueEquals(_ target: LiveTarget, _ value: String) -> Bool {
    Thread.sleep(forTimeInterval: 0.03)
    return axString(target.element, kAXValueAttribute as String) == value
}

private func setSelectedText(_ target: LiveTarget, _ text: String, expectedValue: String) -> Bool {
    guard isAttributeSettable(target.element, kAXSelectedTextAttribute as String),
          AXUIElementSetAttributeValue(
              target.element,
              kAXSelectedTextAttribute as CFString,
              text as CFString
          ) == .success else { return false }
    return valueEquals(target, expectedValue)
}

private func setWholeValue(_ target: LiveTarget, _ value: String) -> Bool {
    guard isAttributeSettable(target.element, kAXValueAttribute as String),
          AXUIElementSetAttributeValue(
              target.element,
              kAXValueAttribute as CFString,
              value as CFString
          ) == .success else { return false }
    return valueEquals(target, value)
}

private func setSelection(_ element: AXUIElement, _ range: CFRange) -> Bool {
    guard isAttributeSettable(element, kAXSelectedTextRangeAttribute as String) else { return false }
    var mutableRange = range
    guard let value = AXValueCreate(.cfRange, &mutableRange) else { return false }
    return AXUIElementSetAttributeValue(
        element,
        kAXSelectedTextRangeAttribute as CFString,
        value
    ) == .success
}

private func selectionEquals(_ element: AXUIElement, _ expected: CFRange) -> Bool {
    guard let actual = selectedRange(element) else { return false }
    return actual.location == expected.location && actual.length == expected.length
}

private func replaceWholeText(_ target: LiveTarget, _ value: String, restoreSelection: CFRange?) -> Bool {
    if !setWholeValue(target, value) {
        let previousSelection = selectedRange(target.element)
        guard let currentValue = axString(target.element, kAXValueAttribute as String) else { return false }
        let fullRange = CFRange(location: 0, length: (currentValue as NSString).length)
        guard setSelection(target.element, fullRange),
              setSelectedText(target, value, expectedValue: value) else {
            if let previousSelection { _ = setSelection(target.element, previousSelection) }
            return false
        }
    }
    if let restoreSelection {
        guard setSelection(target.element, restoreSelection),
              selectionEquals(target.element, restoreSelection) else { return false }
    }
    return true
}

private enum MutationDisposition: Equatable {
    case verified
    case unchanged
    case unexpected(String)
    case unreadable
}

private func mutationDisposition(original: String, expected: String, actual: String?) -> MutationDisposition {
    guard let actual else { return .unreadable }
    if actual == expected { return .verified }
    if actual == original { return .unchanged }
    return .unexpected(actual)
}

private func rollbackUnexpectedMutation(
    _ target: LiveTarget,
    actualValue: String,
    originalSelection: CFRange?,
    rolledBackStatus: String,
    rolledBackMessage: String
) -> ActionResponse {
    let changed = LiveTarget(
        element: target.element,
        role: target.role,
        label: target.label,
        value: actualValue,
        range: selectedRange(target.element)
    )
    if replaceWholeText(changed, target.value, restoreSelection: originalSelection) {
        return failure(rolledBackStatus, rolledBackMessage)
    }
    return failure(
        "verification_failed",
        "The target changed unexpectedly and exact rollback could not be verified. Review the field immediately."
    )
}

private func apply(_ request: ActionRequest) -> ActionResponse {
    guard let action = request.action, ["draft_text", "replace_selection"].contains(action),
          let text = request.text, !text.isEmpty, text.count <= maximumTextLength else {
        return failure("invalid_request", "Only a non-empty bounded draft or selection replacement can be applied.")
    }
    let target: LiveTarget
    switch resolveTarget(request.expected, requireOriginalFingerprint: true) {
    case .success(let value): target = value
    case .failure(let error): return error
    }
    let range: CFRange
    if let selected = target.range {
        range = selected
    } else if target.value.isEmpty {
        range = CFRange(location: 0, length: 0)
    } else {
        return failure("unsupported_target", "The focused control did not expose a cursor or selection.")
    }
    if action == "replace_selection" && range.length == 0 {
        return failure("stale_target", "There is no longer selected text to replace.")
    }
    guard let next = replacing(target.value, range: range, with: text) else {
        return failure("invalid_request", "The proposed edit exceeded the safe text boundary.")
    }
    if !setSelectedText(target, text, expectedValue: next) {
        switch mutationDisposition(
            original: target.value,
            expected: next,
            actual: axString(target.element, kAXValueAttribute as String)
        ) {
        case .verified:
            break
        case .unreadable:
            return failure("verification_failed", "The target value became unreadable after Apply. Review the field immediately.")
        case .unexpected(let actual):
            return rollbackUnexpectedMutation(
                target,
                actualValue: actual,
                originalSelection: target.range,
                rolledBackStatus: "apply_rolled_back",
                rolledBackMessage: "The target transformed the draft, so the original text and selection were restored."
            )
        case .unchanged:
            if !setWholeValue(target, next) {
                switch mutationDisposition(
                    original: target.value,
                    expected: next,
                    actual: axString(target.element, kAXValueAttribute as String)
                ) {
                case .verified:
                    break
                case .unchanged:
                    return failure("apply_failed", "The target app rejected the text change without changing the field.")
                case .unreadable:
                    return failure("verification_failed", "The target value became unreadable after Apply. Review the field immediately.")
                case .unexpected(let actual):
                    return rollbackUnexpectedMutation(
                        target,
                        actualValue: actual,
                        originalSelection: target.range,
                        rolledBackStatus: "apply_rolled_back",
                        rolledBackMessage: "The target transformed the draft, so the original text and selection were restored."
                    )
                }
            }
        }
    }
    let undo = UndoPayload(
        expected: request.expected,
        originalValue: target.value,
        originalSelectionLocation: target.range?.location,
        originalSelectionLength: target.range?.length,
        appliedContentFingerprint: contentFingerprint(role: target.role, label: target.label, value: next)
    )
    return ActionResponse(ok: true, status: "applied", verified: true, undo: undo, error: nil)
}

private func undo(_ request: ActionRequest) -> ActionResponse {
    guard let undo = request.undo, undo.originalValue.count <= maximumTextLength else {
        return failure("invalid_request", "The undo record is unavailable.")
    }
    let target: LiveTarget
    switch resolveTarget(request.expected, requireOriginalFingerprint: false) {
    case .success(let value): target = value
    case .failure(let error): return error
    }
    guard contentFingerprint(role: target.role, label: target.label, value: target.value)
        == undo.appliedContentFingerprint else {
        return failure("stale_target", "The field changed after Apply, so automatic undo was cancelled.")
    }
    let originalSelection: CFRange?
    if let location = undo.originalSelectionLocation, let length = undo.originalSelectionLength {
        originalSelection = CFRange(location: location, length: length)
    } else {
        originalSelection = nil
    }
    let appliedSelection = target.range
    guard replaceWholeText(target, undo.originalValue, restoreSelection: originalSelection) else {
        switch mutationDisposition(
            original: target.value,
            expected: undo.originalValue,
            actual: axString(target.element, kAXValueAttribute as String)
        ) {
        case .verified:
            return failure(
                "verification_failed",
                "Undo restored the original text but could not verify the original selection. Review TextEdit immediately."
            )
        case .unchanged:
            return failure("undo_failed", "The target app rejected Undo without changing the field.")
        case .unreadable:
            return failure("verification_failed", "The target value became unreadable during Undo. Review the field immediately.")
        case .unexpected(let actual):
            return rollbackUnexpectedMutation(
                target,
                actualValue: actual,
                originalSelection: appliedSelection,
                rolledBackStatus: "undo_rolled_back",
                rolledBackMessage: "Undo could not be verified, so the applied text was restored."
            )
        }
    }
    return ActionResponse(ok: true, status: "undone", verified: true, undo: nil, error: nil)
}

private func selfTest() -> Bool {
    let emoji = "A🙂B"
    let replaced = replacing(emoji, range: CFRange(location: 1, length: 2), with: "ok")
    return replaced == "AokB"
        && replacing("hello", range: CFRange(location: 5, length: 0), with: "!") == "hello!"
        && replacing("hello", range: CFRange(location: 6, length: 0), with: "!") == nil
        && contentFingerprint(role: "AXTextArea", label: "Body", value: "x")
            == contentFingerprint(role: "AXTextArea", label: "Body", value: "x")
        && stateFingerprint(role: "AXTextArea", label: nil, value: "x", range: CFRange(location: 0, length: 0))
            != stateFingerprint(role: "AXTextArea", label: nil, value: "x", range: CFRange(location: 1, length: 0))
        && mutationDisposition(original: "before", expected: "after", actual: "after") == .verified
        && mutationDisposition(original: "before", expected: "after", actual: "before") == .unchanged
        && mutationDisposition(original: "before", expected: "after", actual: nil) == .unreadable
        && mutationDisposition(original: "before", expected: "after", actual: "changed") == .unexpected("changed")
        && isActionGateWindow(
            ownerPID: 42,
            layer: 0,
            alpha: 1,
            bounds: CGRect(x: 0, y: 0, width: 800, height: 600),
            bundleId: allowedPreviewBundleId,
            isRegularApp: true
        )
        && !isActionGateWindow(
            ownerPID: 42,
            layer: 0,
            alpha: 1,
            bounds: CGRect(x: 0, y: 0, width: 800, height: 600),
            bundleId: dashboardBundleId,
            isRegularApp: true
        )
        && !isActionGateWindow(
            ownerPID: 42,
            layer: 0,
            alpha: 1,
            bounds: CGRect(x: 0, y: 0, width: 800, height: 600),
            bundleId: "com.example.accessibility-helper",
            isRegularApp: false
        )
}

if CommandLine.arguments.contains("--selftest") {
    if selfTest() {
        print("agent-action self-test passed")
        exit(0)
    }
    fputs("agent-action self-test failed\n", stderr)
    exit(1)
}

guard CommandLine.arguments.contains("--request") else {
    fputs("usage: agent-action --request | --selftest\n", stderr)
    exit(2)
}

do {
    let data = FileHandle.standardInput.readDataToEndOfFile()
    guard data.count <= 256 * 1024 else {
        emit(failure("invalid_request", "The action request exceeded the local limit."))
        exit(0)
    }
    let request = try JSONDecoder().decode(ActionRequest.self, from: data)
    switch request.operation {
    case "apply": emit(apply(request))
    case "undo": emit(undo(request))
    default: emit(failure("invalid_request", "The action operation was unsupported."))
    }
} catch {
    emit(failure("invalid_request", "The action request was invalid."))
}
