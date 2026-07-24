//! agent-context "brain" — REAL, fully unit-tested. Everything that is OS-agnostic lives here and
//! operates on the [`AxNode`] trait, so the BFS budgets, meaningful-element filter, secure
//! redaction, label composition, fingerprint, and the four status outcomes are verifiable without a
//! Linux desktop (see the `MockNode` tests at the bottom). The only TODO(on-device) piece is the
//! concrete atspi-backed `AxNode` in `atspi_tree.rs`.
//!
//! Contract source: `native/PROTOCOL.md` §3 + the reverse-engineered Swift behavior. Emitted `role`
//! strings are the `AX*` tokens (already mapped from AT-SPI by the adapter via
//! `platform::roles::atspi_role_to_ax`).

use helpers_core::fingerprint::state_fingerprint;
use helpers_core::protocol::{
    Element, Frame, Snapshot, CAPTURE_BUDGET_MS, MAX_CHILDREN_PER_ELEMENT, MAX_DEPTH, MAX_ELEMENTS,
    PRIVACY_TAGS,
};
use platform::text::{clean, compose_label, is_secure};
use std::time::{Duration, Instant};

/// Bound on total nodes visited during the walk (Swift `maxVisitedElements`).
const MAX_VISITED: usize = 1_200;
/// Per-part label cap (Swift `label(_, limit: 160)`).
const LABEL_LIMIT: usize = 160;
/// The label/subrole re-cap applied to secure elements + subroles.
const SHORT_LIMIT: usize = 80;

/// Abstract accessibility node. The concrete impl wraps atspi proxies; a `MockNode` drives tests.
/// `role_ax` returns the already-translated `AX*` token (adapter calls `atspi_role_to_ax`).
pub trait AxNode {
    fn role_ax(&self) -> String;
    fn subrole(&self) -> Option<String>;
    fn title(&self) -> Option<String>;
    fn description(&self) -> Option<String>;
    fn help(&self) -> Option<String>;
    fn placeholder(&self) -> Option<String>;
    fn value(&self) -> Option<String>;
    fn selected_text(&self) -> Option<String>;
    fn frame(&self) -> Option<Frame>;
    fn enabled(&self) -> bool;
    fn focused(&self) -> bool;
    fn value_settable(&self) -> bool;
    /// (location, length) in AT-SPI character offsets, if the node exposes a caret/selection.
    fn selection_range(&self) -> Option<(i64, i64)>;
    fn actions(&self) -> Vec<String>;
    fn children(&self) -> Vec<Box<dyn AxNode>>;
}

/// A resolved, ready-to-walk target (status == "ready").
pub struct ReadyTarget {
    pub source: String,
    pub app_name: Option<String>,
    pub bundle_id: Option<String>,
    pub process_identifier: Option<i64>,
    pub window: Box<dyn AxNode>,
    pub focused: Option<Box<dyn AxNode>>,
}

/// The privacy tag list for the ready path (all six) vs the error path (drops `secure_fields_redacted`).
fn privacy(ready: bool) -> Vec<String> {
    if ready {
        PRIVACY_TAGS.iter().map(|s| s.to_string()).collect()
    } else {
        PRIVACY_TAGS
            .iter()
            .filter(|t| **t != "secure_fields_redacted")
            .map(|s| s.to_string())
            .collect()
    }
}

/// Build one of the three error snapshots (permission_required / no_target / no_window).
pub fn error_snapshot(
    status: &str,
    source: &str,
    captured_at: u64,
    app_name: Option<String>,
    bundle_id: Option<String>,
    process_identifier: Option<i64>,
    error: &str,
) -> Snapshot {
    Snapshot {
        ok: false,
        status: status.to_string(),
        // Frozen struct types this as a required String (macOS omits it on error; empty is the
        // faithful equivalent — the core reads by name and treats "" like absent for error paths).
        snapshot_id: String::new(),
        captured_at,
        source: source.to_string(),
        app_name,
        bundle_id,
        process_identifier,
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
        privacy: privacy(false),
        error: Some(error.to_string()),
    }
}

/// True if the node is an editable text control (role token + not secure + settable value).
fn is_editable(role: &str, secure: bool, value_settable: bool) -> bool {
    !secure && platform::roles::ax_role_is_editable(role) && value_settable
}

/// Compose the four-attribute label ([title, description, help, placeholder], " · ", 160).
fn node_label(n: &dyn AxNode) -> Option<String> {
    compose_label(
        &[n.title(), n.description(), n.help(), n.placeholder()],
        " · ",
        LABEL_LIMIT,
    )
}

/// Build one `Element` from a node, using `index` for its `id`.
fn make_element(n: &dyn AxNode, index: usize) -> Element {
    let role = n.role_ax();
    let subrole = clean(n.subrole().as_deref(), SHORT_LIMIT);
    let raw_label = node_label(n);
    let secure = is_secure(&role, subrole.as_deref(), raw_label.as_deref());

    let label = if secure {
        clean(raw_label.as_deref(), SHORT_LIMIT)
    } else {
        raw_label
    };
    let value = if secure {
        None
    } else {
        clean(n.value().as_deref(), 280)
    };
    let value_settable = n.value_settable();
    let editable = is_editable(&role, secure, value_settable);
    let mut actions = n.actions();
    actions.truncate(12);

    Element {
        id: format!("el_{index}"),
        role,
        subrole,
        label,
        value,
        frame: n.frame().unwrap_or(Frame { x: 0.0, y: 0.0, width: 0.0, height: 0.0 }),
        enabled: n.enabled(),
        focused: n.focused(),
        editable,
        secure,
        actions,
    }
}

/// An element is "meaningful" (worth emitting) per the Swift filter.
fn is_meaningful(e: &Element) -> bool {
    e.label.is_some()
        || e.value.is_some()
        || e.focused
        || e.editable
        || !e.actions.is_empty()
        || e.role == "AXWindow"
}

struct Pending {
    node: Box<dyn AxNode>,
    depth: usize,
}

/// Build the full ready snapshot: focused-element fields + BFS element walk.
pub fn build_snapshot(target: ReadyTarget, captured_at: u64, snapshot_id: String) -> Snapshot {
    // --- focused element fields ---
    let mut focused_role = None;
    let mut focused_label = None;
    let mut focused_fingerprint = None;
    let mut selection_location = None;
    let mut selection_length = None;
    let mut selected_text = None;
    let mut editable = false;

    if let Some(f) = target.focused.as_ref() {
        let role = f.role_ax();
        let subrole = f.subrole();
        let raw_label = node_label(f.as_ref());
        let secure = is_secure(&role, subrole.as_deref(), raw_label.as_deref());

        focused_role = clean(Some(&role), SHORT_LIMIT);
        focused_label = if secure { None } else { raw_label.clone() };

        let range = f.selection_range();
        selection_location = range.map(|(l, _)| l);
        selection_length = range.map(|(_, len)| len);
        selected_text = if secure {
            None
        } else {
            clean(f.selected_text().as_deref(), 500)
        };
        editable = is_editable(&role, secure, f.value_settable());

        // focusedValue is the RAW (untruncated) value; fingerprint only when present & not secure.
        let focused_value = if secure { None } else { f.value() };
        if let Some(v) = focused_value {
            let role_for_fp = if role.is_empty() { "AXUnknown".to_string() } else { role.clone() };
            focused_fingerprint = Some(state_fingerprint(
                &role_for_fp,
                focused_label.as_deref(),
                &v,
                range,
            ));
        }
    }

    // --- window title (read before the window node is consumed by the walk) ---
    let window_title = clean(target.window.title().as_deref(), 180);

    // --- BFS walk from the window ---
    let deadline = Instant::now() + Duration::from_millis(CAPTURE_BUDGET_MS);
    let mut queue: Vec<Pending> = vec![Pending { node: target.window, depth: 0 }];
    let mut cursor = 0usize;
    let mut output: Vec<Element> = Vec::new();
    let mut visited = 0usize;
    let mut truncated = false;

    while cursor < queue.len() {
        if output.len() >= MAX_ELEMENTS || visited >= MAX_VISITED || Instant::now() >= deadline {
            truncated = true;
            break;
        }
        let depth = queue[cursor].depth;
        // Take ownership of this pending node's children lazily; we need the node to build+inspect.
        let node = std::mem::replace(
            &mut queue[cursor].node,
            Box::new(EmptyNode) as Box<dyn AxNode>,
        );
        cursor += 1;
        visited += 1;

        let candidate = make_element(node.as_ref(), output.len());
        let children = node.children();

        if is_meaningful(&candidate) {
            output.push(candidate);
        }

        if depth >= MAX_DEPTH {
            if !children.is_empty() {
                truncated = true;
            }
            continue;
        }
        if children.len() > MAX_CHILDREN_PER_ELEMENT {
            truncated = true;
        }
        for child in children.into_iter().take(MAX_CHILDREN_PER_ELEMENT) {
            queue.push(Pending { node: child, depth: depth + 1 });
        }
    }

    Snapshot {
        ok: true,
        status: "ready".to_string(),
        snapshot_id,
        captured_at,
        source: target.source,
        app_name: target.app_name,
        bundle_id: target.bundle_id,
        process_identifier: target.process_identifier,
        window_title,
        focused_role,
        focused_label,
        focused_fingerprint,
        selection_location,
        selection_length,
        selected_text,
        editable,
        element_count: output.len(),
        truncated,
        elements: output,
        privacy: privacy(true),
        error: None,
    }
}

/// A no-op node used as a placeholder when we `mem::replace` a queue slot mid-walk.
struct EmptyNode;
impl AxNode for EmptyNode {
    fn role_ax(&self) -> String {
        "AXUnknown".into()
    }
    fn subrole(&self) -> Option<String> {
        None
    }
    fn title(&self) -> Option<String> {
        None
    }
    fn description(&self) -> Option<String> {
        None
    }
    fn help(&self) -> Option<String> {
        None
    }
    fn placeholder(&self) -> Option<String> {
        None
    }
    fn value(&self) -> Option<String> {
        None
    }
    fn selected_text(&self) -> Option<String> {
        None
    }
    fn frame(&self) -> Option<Frame> {
        None
    }
    fn enabled(&self) -> bool {
        true
    }
    fn focused(&self) -> bool {
        false
    }
    fn value_settable(&self) -> bool {
        false
    }
    fn selection_range(&self) -> Option<(i64, i64)> {
        None
    }
    fn actions(&self) -> Vec<String> {
        Vec::new()
    }
    fn children(&self) -> Vec<Box<dyn AxNode>> {
        Vec::new()
    }
}

/// The six pure-logic self-test checks (mirrors the Swift `selfTest`).
pub fn selftest() -> bool {
    let c1 = clean(Some("  hello\n  world  "), 280).as_deref() == Some("hello world");
    let c2 = clean(Some(&"a".repeat(300)), 280)
        .map(|s| s.ends_with('…'))
        .unwrap_or(false);
    let c3 = is_secure("AXTextField", Some("AXSecureTextField"), None);
    let c4 = is_secure("AXTextField", None, Some("API key"));
    let c5 = !is_secure("AXTextArea", None, Some("Email body"));
    let c6 = state_fingerprint("AXTextArea", None, "x", Some((0, 0)))
        != state_fingerprint("AXTextArea", None, "x", Some((1, 0)));
    c1 && c2 && c3 && c4 && c5 && c6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct MockNode {
        role: String,
        subrole: Option<String>,
        title: Option<String>,
        value: Option<String>,
        focused: bool,
        settable: bool,
        selection: Option<(i64, i64)>,
        actions: Vec<String>,
        children: Vec<MockNode>,
    }

    impl AxNode for MockNode {
        fn role_ax(&self) -> String {
            self.role.clone()
        }
        fn subrole(&self) -> Option<String> {
            self.subrole.clone()
        }
        fn title(&self) -> Option<String> {
            self.title.clone()
        }
        fn description(&self) -> Option<String> {
            None
        }
        fn help(&self) -> Option<String> {
            None
        }
        fn placeholder(&self) -> Option<String> {
            None
        }
        fn value(&self) -> Option<String> {
            self.value.clone()
        }
        fn selected_text(&self) -> Option<String> {
            None
        }
        fn frame(&self) -> Option<Frame> {
            None
        }
        fn enabled(&self) -> bool {
            true
        }
        fn focused(&self) -> bool {
            self.focused
        }
        fn value_settable(&self) -> bool {
            self.settable
        }
        fn selection_range(&self) -> Option<(i64, i64)> {
            self.selection
        }
        fn actions(&self) -> Vec<String> {
            self.actions.clone()
        }
        fn children(&self) -> Vec<Box<dyn AxNode>> {
            self.children
                .iter()
                .cloned()
                .map(|c| Box::new(c) as Box<dyn AxNode>)
                .collect()
        }
    }

    #[test]
    fn selftest_passes() {
        assert!(selftest());
    }

    #[test]
    fn window_is_always_meaningful_and_bfs_counts() {
        let window = MockNode {
            role: "AXWindow".into(),
            children: vec![
                MockNode { role: "AXStaticText".into(), title: Some("Label".into()), ..Default::default() },
                MockNode { role: "AXGroup".into(), ..Default::default() }, // not meaningful → dropped
            ],
            ..Default::default()
        };
        let target = ReadyTarget {
            source: "frontmost_app".into(),
            app_name: Some("App".into()),
            bundle_id: None,
            process_identifier: Some(42),
            window: Box::new(window),
            focused: None,
        };
        let snap = build_snapshot(target, 1, "id".into());
        assert_eq!(snap.status, "ready");
        // Window (meaningful) + the labelled static text; the empty AXGroup is filtered out.
        assert_eq!(snap.element_count, 2);
        assert_eq!(snap.elements[0].id, "el_0");
        assert_eq!(snap.elements[1].id, "el_1");
    }

    #[test]
    fn secure_focused_redacts_value_and_fingerprint() {
        let focused = MockNode {
            role: "AXTextField".into(),
            subrole: Some("AXSecureTextField".into()),
            value: Some("hunter2".into()),
            focused: true,
            settable: true,
            selection: Some((0, 0)),
            ..Default::default()
        };
        let target = ReadyTarget {
            source: "frontmost_app".into(),
            app_name: None,
            bundle_id: None,
            process_identifier: None,
            window: Box::new(MockNode { role: "AXWindow".into(), ..Default::default() }),
            focused: Some(Box::new(focused)),
        };
        let snap = build_snapshot(target, 1, "id".into());
        assert!(snap.selected_text.is_none());
        assert!(snap.focused_fingerprint.is_none()); // secure ⇒ no value ⇒ no fingerprint
        assert!(!snap.editable); // secure ⇒ not editable
    }

    #[test]
    fn non_secure_editable_focused_has_fingerprint() {
        let focused = MockNode {
            role: "AXTextArea".into(),
            value: Some("hi".into()),
            focused: true,
            settable: true,
            selection: Some((2, 0)),
            ..Default::default()
        };
        let target = ReadyTarget {
            source: "frontmost_app".into(),
            app_name: None,
            bundle_id: None,
            process_identifier: None,
            window: Box::new(MockNode { role: "AXWindow".into(), ..Default::default() }),
            focused: Some(Box::new(focused)),
        };
        let snap = build_snapshot(target, 1, "id".into());
        assert!(snap.editable);
        assert_eq!(snap.selection_location, Some(2));
        let expected = state_fingerprint("AXTextArea", None, "hi", Some((2, 0)));
        assert_eq!(snap.focused_fingerprint.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn error_snapshot_has_five_privacy_tags() {
        let s = error_snapshot("no_window", "frontmost_app", 1, None, None, None, "x");
        assert_eq!(s.privacy.len(), 5);
        assert!(!s.privacy.iter().any(|t| t == "secure_fields_redacted"));
        assert!(!s.ok);
    }
}
