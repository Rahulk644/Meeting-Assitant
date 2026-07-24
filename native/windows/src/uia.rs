//! UI Automation — the Windows analogue of the macOS Accessibility (AX) reads. Backs three helpers:
//! `agent-context` (bounded read-only snapshot), `dictation-target` (focused-field classify), and
//! `meeting-state` (toolbar control labels + window title).
//!
//! The **portable, tested** parts (role→AX mapping, secure/code classification) live at the top and
//! drive the `--selftest`s. The **native UIA walks** are `#[cfg(windows)]`, written against the
//! `uiautomation` crate; the exact method surface of 0.24 (and especially the batched `CacheRequest`
//! read needed to hit the 0.75 s budget) is marked TODO(on-device). Every native path is wrapped so
//! any error degrades to a contract-valid result, and `#[cfg(not(windows))]` fallbacks return the
//! same well-formed shapes so host builds compile.

use helpers_core::protocol::{DictationClassification, FieldKind, Snapshot, PRIVACY_TAGS};
#[cfg(windows)]
use helpers_core::protocol::{Element, MAX_CHILDREN_PER_ELEMENT, MAX_DEPTH, MAX_ELEMENTS};

// ---------------------------------------------------------------------------
// Portable classification helpers (tested)
// ---------------------------------------------------------------------------

/// Map a UIA control-type name to the `AX*` role token the core expects (PROTOCOL §3 mapping table).
pub fn ax_role_for(control_type_name: &str) -> String {
    match control_type_name {
        "Edit" => "AXTextField",
        "Document" => "AXTextArea",
        "ComboBox" => "AXComboBox",
        "Window" => "AXWindow",
        "Button" | "SplitButton" => "AXButton",
        "Text" => "AXStaticText",
        "CheckBox" => "AXCheckBox",
        "RadioButton" => "AXRadioButton",
        "Hyperlink" => "AXLink",
        "List" => "AXList",
        "ListItem" => "AXRow",
        "MenuItem" => "AXMenuItem",
        "Menu" | "MenuBar" => "AXMenu",
        "Image" => "AXImage",
        "Group" | "Pane" => "AXGroup",
        "TabItem" => "AXRadioButton",
        "Tab" => "AXTabGroup",
        "ScrollBar" => "AXScrollBar",
        "Slider" => "AXSlider",
        "ToolBar" => "AXToolbar",
        "Table" | "DataGrid" => "AXTable",
        "Tree" => "AXOutline",
        "Spinner" => "AXIncrementor",
        "ProgressBar" => "AXProgressIndicator",
        _ => "AXUnknown",
    }
    .to_string()
}

/// AX roles we treat as editable text targets (dictation + agent-action gates).
pub fn is_editable_role(ax_role: &str) -> bool {
    matches!(
        ax_role,
        "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField"
    )
}

/// Secure-field detection: an explicit password flag, a structural role hint, or a semantic label.
/// Ports the Swift `isSecure`.
pub fn is_secure(ax_role: &str, label: &str, is_password: bool) -> bool {
    if is_password {
        return true;
    }
    let structural = ax_role.to_lowercase();
    if structural.contains("securetextfield") || structural.contains("password") {
        return true;
    }
    let semantic = label.to_lowercase();
    [
        "password",
        "passcode",
        "pin",
        "secret",
        "api key",
        "credit card",
        "cvv",
        "ssn",
    ]
    .iter()
    .any(|t| semantic.contains(t))
}

/// Code/terminal apps by exe name — the Windows analogue of the Swift `isCodeApp` bundle-ID list.
pub fn is_code_exe(exe: &str) -> bool {
    const CODE: &[&str] = &[
        "code.exe",
        "code - insiders.exe",
        "cursor.exe",
        "zed.exe",
        "windowsterminal.exe",
        "wt.exe",
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "conhost.exe",
        "alacritty.exe",
        "wezterm-gui.exe",
        "hyper.exe",
        "idea64.exe",
        "pycharm64.exe",
        "clion64.exe",
        "webstorm64.exe",
        "goland64.exe",
        "rider64.exe",
        "sublime_text.exe",
        "gvim.exe",
        "nvim.exe",
    ];
    CODE.iter().any(|c| c.eq_ignore_ascii_case(exe))
}

/// Portable identity of a focused control, captured at classify time and re-checked on each dictation
/// session op (Windows-side reacquisition matches the Swift `TargetIdentity`).
#[derive(Debug, Clone)]
pub struct FocusIdentity {
    pub pid: u32, // Windows process IDs are DWORD (u32); uiautomation get_process_id() -> Result<u32>
    pub ax_role: String,
    pub name: String,
    pub exe: String,
}

// ---------------------------------------------------------------------------
// dictation-target: classify the focused field
// ---------------------------------------------------------------------------

/// Classify the focused element for dictation. Returns the wire `DictationClassification` plus, when
/// live-editable, a [`FocusIdentity`] the session loop reuses.
#[cfg(windows)]
pub fn classify_focused() -> (DictationClassification, Option<FocusIdentity>) {
    match classify_focused_inner() {
        Ok(v) => v,
        Err(_) => (
            DictationClassification {
                kind: FieldKind::Unknown,
                secure: false,
                starts_sentence: false,
                live_editable: false,
            },
            None,
        ),
    }
}

#[cfg(windows)]
fn classify_focused_inner(
) -> Result<(DictationClassification, Option<FocusIdentity>), Box<dyn std::error::Error>> {
    use uiautomation::UIAutomation;

    let automation = UIAutomation::new()?;
    let focused = automation.get_focused_element()?;

    let pid = focused.get_process_id().unwrap_or(0);
    let exe = if pid > 0 {
        crate::procs::exe_for_pid(pid as u32).unwrap_or_default()
    } else {
        String::new()
    };
    // TODO(on-device): confirm ControlType → name. `get_control_type()` yields a `ControlType`;
    // `format!("{ct:?}")` gives "Edit"/"Document"/… which `ax_role_for` maps. Verify the Debug names.
    let ct_name = focused
        .get_control_type()
        .map(|ct| format!("{ct:?}"))
        .unwrap_or_else(|_| "Unknown".into());
    let ax_role = ax_role_for(&ct_name);
    let name = focused.get_name().unwrap_or_default();
    // TODO(on-device): read IsPassword via get_property_value(UIProperty::IsPassword) → bool.
    let is_password = read_is_password(&focused);

    // Secure fails closed.
    if is_secure(&ax_role, &name, is_password) {
        return Ok((
            DictationClassification {
                kind: FieldKind::Secure,
                secure: true,
                starts_sentence: false,
                live_editable: false,
            },
            None,
        ));
    }
    // Code/terminal app.
    if is_code_exe(&exe) {
        return Ok((
            DictationClassification {
                kind: FieldKind::CodeOrTerminal,
                secure: false,
                starts_sentence: false,
                live_editable: false,
            },
            None,
        ));
    }
    // Non-editable role.
    if !is_editable_role(&ax_role) {
        return Ok((
            DictationClassification {
                kind: FieldKind::Unknown,
                secure: false,
                starts_sentence: false,
                live_editable: false,
            },
            None,
        ));
    }

    // Editable natural-language field. `liveEditable` requires a stable caret we can drive; on
    // Windows we can inject Unicode, but a guaranteed provisional-span reconciliation needs UIA
    // TextPattern. We set liveEditable when the field exposes a TextPattern/ValuePattern.
    let live_editable = supports_text_or_value(&focused);
    let starts_sentence = starts_sentence(&focused);

    let identity = if live_editable {
        Some(FocusIdentity {
            pid,
            ax_role: ax_role.clone(),
            name: name.clone(),
            exe,
        })
    } else {
        None
    };

    Ok((
        DictationClassification {
            kind: FieldKind::NaturalLanguage,
            secure: false,
            starts_sentence,
            live_editable,
        },
        identity,
    ))
}

#[cfg(windows)]
fn read_is_password(el: &uiautomation::UIElement) -> bool {
    use uiautomation::types::UIProperty;
    // uiautomation implements `TryInto<bool> for Variant` directly (not TryFrom), so convert via
    // `try_into()`. A non-secure/unreadable field yields false; `is_secure()` additionally applies the
    // label/role heuristic, so this only needs to surface the explicit IsPassword flag when present.
    match el.get_property_value(UIProperty::IsPassword) {
        Ok(v) => v.try_into().unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(windows)]
fn supports_text_or_value(el: &uiautomation::UIElement) -> bool {
    use uiautomation::patterns::{UITextPattern, UIValuePattern};
    // TODO(on-device): confirm get_pattern generic form.
    el.get_pattern::<UITextPattern>().is_ok() || el.get_pattern::<UIValuePattern>().is_ok()
}

#[cfg(windows)]
fn starts_sentence(el: &uiautomation::UIElement) -> bool {
    // TODO(on-device): read the ≤8 chars preceding the caret via TextPattern and apply the Swift
    // rule (newline or trailing ".!?।" ⇒ true; empty ⇒ true). Conservatively true when unknown so a
    // new dictation capitalises its first word.
    let _ = el;
    true
}

#[cfg(not(windows))]
pub fn classify_focused() -> (DictationClassification, Option<FocusIdentity>) {
    crate::degrade("UIA focused-element classify");
    (
        DictationClassification {
            kind: FieldKind::Unknown,
            secure: false,
            starts_sentence: false,
            live_editable: false, // conservatively not live-editable off-device
        },
        None,
    )
}

// ---------------------------------------------------------------------------
// agent-context: bounded read-only snapshot
// ---------------------------------------------------------------------------

fn privacy() -> Vec<String> {
    PRIVACY_TAGS.iter().map(|s| s.to_string()).collect()
}

/// A well-formed degraded snapshot (used by fallbacks and error paths).
pub fn degraded_snapshot(status: &str, source: &str, error: Option<&str>) -> Snapshot {
    Snapshot {
        ok: status == "ready",
        status: status.to_string(),
        snapshot_id: uuid::Uuid::new_v4().simple().to_string(),
        captured_at: helpers_core::epoch_ms(),
        source: source.to_string(),
        app_name: None,
        bundle_id: None,
        process_identifier: None,
        window_title: None,
        focused_role: None,
        focused_label: None,
        focused_fingerprint: None,
        selection_location: None,
        selection_length: None,
        selected_text: None,
        editable: false,
        element_count: 0,
        truncated: false,
        elements: Vec::new(),
        privacy: privacy(),
        error: error.map(|e| e.to_string()),
    }
}

/// Capture one bounded snapshot of the foreground app window.
#[cfg(windows)]
pub fn snapshot() -> Snapshot {
    match snapshot_inner() {
        Ok(s) => s,
        Err(_) => degraded_snapshot("no_window", "frontmost_app", Some("UI Automation read failed.")),
    }
}

#[cfg(windows)]
fn snapshot_inner() -> Result<Snapshot, Box<dyn std::error::Error>> {
    use std::time::Instant;
    use uiautomation::UIAutomation;

    let our_pid = std::process::id();
    let Some(fg_pid) = crate::window::foreground_pid().filter(|p| *p != our_pid) else {
        return Ok(degraded_snapshot("no_target", "frontmost_app", Some("No inspectable app window was found.")));
    };

    let automation = UIAutomation::new()?;
    let focused = automation.get_focused_element().ok();

    // Root of the foreground window. TODO(on-device): resolve the element for `fg_pid`'s top window
    // via `automation.element_from_handle(HWND)` using the window::activate/enumerate HWND, or walk
    // from the root element filtering by ProcessId. We use the focused element's ancestor window as a
    // pragmatic root; verify on-device.
    let root = focused.clone().unwrap_or(automation.get_root_element()?);

    let app_name = crate::procs::exe_for_pid(fg_pid);
    let window_title = crate::window::titles_for_pid(fg_pid).into_iter().next();

    // BFS with PROTOCOL budgets. TODO(on-device): batch reads with a UIA CacheRequest
    // (`automation.create_cache_request()` + add_property + get with cache) — critical to stay inside
    // the 0.75 s budget. Direct property reads below are correct but slower.
    let deadline = Instant::now() + std::time::Duration::from_millis(helpers_core::protocol::CAPTURE_BUDGET_MS);
    let walker = automation.get_control_view_walker()?;

    let mut queue: Vec<(uiautomation::UIElement, usize)> = vec![(root.clone(), 0)];
    let mut cursor = 0;
    let mut output: Vec<Element> = Vec::new();
    let mut truncated = false;

    while cursor < queue.len() {
        if output.len() >= MAX_ELEMENTS || Instant::now() >= deadline {
            truncated = true;
            break;
        }
        let (el, depth) = queue[cursor].clone();
        cursor += 1;

        let candidate = make_element(&el, output.len());
        let meaningful = candidate.label.is_some()
            || candidate.value.is_some()
            || candidate.focused
            || candidate.editable
            || !candidate.actions.is_empty()
            || candidate.role == "AXWindow";
        if meaningful {
            output.push(candidate);
        }

        if depth >= MAX_DEPTH {
            continue;
        }
        // Collect children via the tree walker.
        if let Ok(mut child) = walker.get_first_child(&el) {
            let mut n = 0;
            loop {
                if n >= MAX_CHILDREN_PER_ELEMENT {
                    truncated = true;
                    break;
                }
                queue.push((child.clone(), depth + 1));
                n += 1;
                match walker.get_next_sibling(&child) {
                    Ok(sib) => child = sib,
                    Err(_) => break,
                }
            }
        }
    }

    // Focused element details + fingerprint.
    let (focused_role, focused_label, focused_fp, sel_loc, sel_len, selected_text, editable) =
        match &focused {
            Some(f) => focused_details(f),
            None => (None, None, None, None, None, None, false),
        };

    Ok(Snapshot {
        ok: true,
        status: "ready".into(),
        snapshot_id: uuid::Uuid::new_v4().simple().to_string(),
        captured_at: helpers_core::epoch_ms(),
        source: "frontmost_app".into(),
        app_name,
        bundle_id: None, // Windows has no bundle id; exe name is carried in app_name.
        process_identifier: Some(fg_pid as i64),
        window_title,
        focused_role,
        focused_label,
        focused_fingerprint: focused_fp,
        selection_location: sel_loc,
        selection_length: sel_len,
        selected_text,
        editable,
        element_count: output.len(),
        truncated,
        elements: output,
        privacy: privacy(),
        error: None,
    })
}

#[cfg(windows)]
fn make_element(el: &uiautomation::UIElement, idx: usize) -> Element {
    use helpers_core::protocol::Frame;
    let ct_name = el
        .get_control_type()
        .map(|ct| format!("{ct:?}"))
        .unwrap_or_else(|_| "Unknown".into());
    let role = ax_role_for(&ct_name);
    let name = el.get_name().unwrap_or_default();
    let label = if name.is_empty() { None } else { Some(name.clone()) };
    let is_password = read_is_password(el);
    let secure = is_secure(&role, &name, is_password);
    let focused = el.has_keyboard_focus().unwrap_or(false);
    let editable = !secure && is_editable_role(&role) && supports_text_or_value(el);
    let value = if secure {
        None
    } else {
        read_value(el)
    };
    // TODO(on-device): get_bounding_rectangle() → Rect fields (left/top/width/height).
    let frame = el
        .get_bounding_rectangle()
        .map(|r| Frame {
            x: r.get_left() as f64,
            y: r.get_top() as f64,
            width: (r.get_right() - r.get_left()) as f64,
            height: (r.get_bottom() - r.get_top()) as f64,
        })
        .unwrap_or(Frame {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
    Element {
        id: format!("el_{idx}"),
        role,
        subrole: None,
        label: label.map(|l| clean(&l, 160)),
        value,
        frame,
        enabled: el.is_enabled().unwrap_or(true),
        focused,
        editable,
        secure,
        actions: Vec::new(), // UIA exposes patterns, not AX actions; left empty (contract-valid).
    }
}

#[cfg(windows)]
fn read_value(el: &uiautomation::UIElement) -> Option<String> {
    use uiautomation::patterns::UIValuePattern;
    // TODO(on-device): prefer ValuePattern.Value; fall back to Name for read-only controls.
    if let Ok(vp) = el.get_pattern::<UIValuePattern>() {
        if let Ok(v) = vp.get_value() {
            if !v.is_empty() {
                return Some(clean(&v, 280));
            }
        }
    }
    None
}

#[cfg(windows)]
#[allow(clippy::type_complexity)]
fn focused_details(
    f: &uiautomation::UIElement,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    bool,
) {
    let ct_name = f
        .get_control_type()
        .map(|ct| format!("{ct:?}"))
        .unwrap_or_else(|_| "Unknown".into());
    let role = ax_role_for(&ct_name);
    let name = f.get_name().unwrap_or_default();
    let is_password = read_is_password(f);
    let secure = is_secure(&role, &name, is_password);
    let label = if name.is_empty() || secure {
        None
    } else {
        Some(clean(&name, 160))
    };
    let value = if secure { None } else { read_value(f) };
    let editable = !secure && is_editable_role(&role) && supports_text_or_value(f);

    // Selection loc/len via TextPattern is device-specific; leave None unless read succeeds.
    let (sel_loc, sel_len): (Option<i64>, Option<i64>) = (None, None);
    let fp = value.as_ref().map(|v| {
        helpers_core::fingerprint::state_fingerprint(
            &role,
            label.as_deref(),
            v,
            sel_loc.zip(sel_len),
        )
    });
    (Some(role), label, fp, sel_loc, sel_len, None, editable)
}

#[cfg(windows)]
fn clean(s: &str, limit: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        collapsed
    } else {
        let truncated: String = collapsed.chars().take(limit).collect();
        format!("{truncated}…")
    }
}

#[cfg(not(windows))]
pub fn snapshot() -> Snapshot {
    crate::degrade("UIA agent-context snapshot");
    degraded_snapshot("no_window", "frontmost_app", Some("UI Automation is unavailable on this host."))
}

// ---------------------------------------------------------------------------
// meeting-state: control labels + window title + participant names
// ---------------------------------------------------------------------------

/// UIA reads for a meeting app process.
#[derive(Debug, Clone, Default)]
pub struct MeetingReads {
    pub button_labels: Vec<String>,
    pub window_title: Option<String>,
    pub participants: Vec<String>,
    pub active_speaker: Option<String>,
}

#[cfg(windows)]
pub fn read_meeting(pid: u32) -> MeetingReads {
    read_meeting_inner(pid).unwrap_or_default()
}

#[cfg(windows)]
fn read_meeting_inner(pid: u32) -> Result<MeetingReads, Box<dyn std::error::Error>> {
    use uiautomation::UIAutomation;

    let automation = UIAutomation::new()?;
    let walker = automation.get_control_view_walker()?;
    let root = automation.get_root_element()?;

    let mut reads = MeetingReads {
        window_title: crate::window::titles_for_pid(pid).into_iter().next(),
        ..Default::default()
    };

    // BFS from the desktop root, restricted to windows owned by `pid`, harvesting Button names.
    // TODO(on-device): scope more tightly with a CacheRequest + ProcessId condition; and add the
    // participant/active-speaker name harvesting (Zoom/Teams render names in embedded web content —
    // this is the most device-specific selector work, deferred like the Swift `--dump` workflow).
    let mut queue = vec![(root, 0usize)];
    let mut cursor = 0;
    let mut visited = 0;
    while cursor < queue.len() && visited < 4000 {
        let (el, depth) = queue[cursor].clone();
        cursor += 1;
        visited += 1;

        let el_pid = el.get_process_id().unwrap_or(0);
        let ct_name = el
            .get_control_type()
            .map(|ct| format!("{ct:?}"))
            .unwrap_or_default();
        if el_pid == pid && (ct_name == "Button" || ct_name == "MenuItem") {
            if let Ok(name) = el.get_name() {
                if !name.is_empty() {
                    reads.button_labels.push(name);
                }
            }
        }

        if depth >= 20 {
            continue;
        }
        if let Ok(mut child) = walker.get_first_child(&el) {
            loop {
                queue.push((child.clone(), depth + 1));
                match walker.get_next_sibling(&child) {
                    Ok(sib) => child = sib,
                    Err(_) => break,
                }
            }
        }
    }
    Ok(reads)
}

#[cfg(not(windows))]
pub fn read_meeting(pid: u32) -> MeetingReads {
    crate::degrade("UIA meeting-state read");
    let _ = pid;
    MeetingReads::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_mapping() {
        assert_eq!(ax_role_for("Edit"), "AXTextField");
        assert_eq!(ax_role_for("Document"), "AXTextArea");
        assert_eq!(ax_role_for("ComboBox"), "AXComboBox");
        assert_eq!(ax_role_for("Window"), "AXWindow");
        assert_eq!(ax_role_for("Nonsense"), "AXUnknown");
    }

    #[test]
    fn secure_and_code_detection() {
        assert!(is_secure("AXTextField", "", true));
        assert!(is_secure("AXSecureTextField", "", false));
        assert!(is_secure("AXTextField", "API key", false));
        assert!(!is_secure("AXTextArea", "Message", false));
        assert!(is_code_exe("Code.exe"));
        assert!(is_code_exe("WindowsTerminal.exe"));
        assert!(!is_code_exe("notepad.exe"));
    }
}
