//! Every JSON structure that crosses the helper ⇄ core boundary.
//!
//! Field names are camelCase and **must not change** — the Rust core parses these by name and the
//! macOS Swift helpers emit the same shapes. See `native/PROTOCOL.md`.
//!
//! Nil handling varies by struct, matching the documented schema: nested `Element` fields and the
//! `unsafeFallback`/`undo`/`error` response fields omit nils (via `skip_serializing_if`), while the
//! top-level `Snapshot` scalar fields are always present (serialized as JSON `null` when absent),
//! matching the PROTOCOL.md sample that shows explicit `"selectedText":null`. The core parses both
//! absent and `null` as `None`, so either is accepted.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// meeting-tap — HTTP control-plane messages (POSTed to /ingest)
// ---------------------------------------------------------------------------

/// A recognized utterance. `self` is a Rust keyword, hence the rename.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Caption {
    #[serde(rename = "type")]
    pub kind: CaptionTag,
    pub speaker: String,
    pub text: String,
    #[serde(rename = "self")]
    pub is_self: bool,
    pub ts: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CaptionTag {
    #[serde(rename = "caption")]
    Caption,
}

/// Screen-recording started notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStarted {
    #[serde(rename = "type")]
    pub kind: String, // "recording"
    pub file: String, // "<epochMs>.mp4" — safe basename only
    #[serde(rename = "startedAt")]
    pub started_at: u64,
}

/// Authoritative, audio-derived meeting lifecycle event. Keep `source:"coreaudio"` on every OS —
/// the core keys on that string to decide this event may start/end a meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStateEvent {
    #[serde(rename = "type")]
    pub kind: String, // "state"
    pub source: String, // "coreaudio"
    pub state: AudioState,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioState {
    #[serde(rename = "inMeeting")]
    pub in_meeting: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(rename = "meetingTitle", skip_serializing_if = "Option::is_none")]
    pub meeting_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Config pulled from `GET /api/state` every ~2 s. All fields optional/defaulted so a partial
/// server response never breaks the helper.
///
/// `manual` is a **string** tri-state on the wire (`"on"|"auto"|"off"|"paused"`), NOT a bool — the
/// server always serializes it as a string (`local_server.rs` `state_response`) and the Swift helper
/// reads it as `o["manual"] as? String`. Typing it as `bool` makes serde fail the *entire* struct
/// on every poll, so the helper never sees any config. Use [`PulledConfig::manual_force_on`] etc.
///
/// `auto_detect` and `browser_meet` default to **true** (matching the Swift helper) so a fresh/partial
/// `/api/state` response — which omits them until the user first toggles a setting — still
/// auto-captures instead of silently sitting idle.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PulledConfig {
    pub sharing: bool,
    #[serde(rename = "videoMode")]
    pub video_mode: Option<String>,
    pub translate: bool,
    pub language: Option<String>,
    pub manual: Option<String>,
    #[serde(rename = "autoDetect")]
    pub auto_detect: bool,
    #[serde(rename = "browserMeet")]
    pub browser_meet: bool,
    pub vocab: Option<String>,
    pub modules: Option<Modules>,
}

impl Default for PulledConfig {
    fn default() -> Self {
        PulledConfig {
            sharing: false,
            video_mode: None,
            translate: false,
            language: None,
            manual: None,
            auto_detect: true,  // Swift default (meeting-tap.swift)
            browser_meet: true, // Swift default
            vocab: None,
            modules: None,
        }
    }
}

impl PulledConfig {
    /// Manual gate forced ON — always capture (`manual == "on"`).
    pub fn manual_force_on(&self) -> bool {
        self.manual.as_deref() == Some("on")
    }
    /// Manual gate forced OFF — never capture (`manual` is `"off"` or `"paused"`).
    pub fn manual_force_off(&self) -> bool {
        matches!(self.manual.as_deref(), Some("off") | Some("paused"))
    }
    /// Auto mode — defer to `autoDetect` + live call detection (`"auto"` or absent).
    pub fn manual_is_auto(&self) -> bool {
        !self.manual_force_on() && !self.manual_force_off()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Modules {
    pub meetings: bool,
}

// ---------------------------------------------------------------------------
// dictation-target
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationClassification {
    pub kind: FieldKind,
    pub secure: bool,
    #[serde(rename = "startsSentence")]
    pub starts_sentence: bool,
    #[serde(rename = "liveEditable")]
    pub live_editable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    #[serde(rename = "secure")]
    Secure,
    #[serde(rename = "unknown")]
    Unknown,
    #[serde(rename = "codeOrTerminal")]
    CodeOrTerminal,
    #[serde(rename = "naturalLanguage")]
    NaturalLanguage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionCommand {
    pub op: SessionOp,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionOp {
    Preview,
    Commit,
    Cancel,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionResponse {
    pub ok: bool,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(rename = "unsafeFallback", skip_serializing_if = "is_false")]
    pub unsafe_fallback: bool,
}

/// The full error taxonomy from `native/PROTOCOL.md` §2. Kept as a helper so every platform emits
/// the exact same tokens the core matches on.
pub mod session_error {
    pub const INVALID_COMMAND: &str = "invalidCommand";
    pub const TARGET_CHANGED: &str = "targetChanged";
    pub const PROVISIONAL_TEXT_CHANGED: &str = "provisionalTextChanged";
    pub const CURSOR_CHANGED: &str = "cursorChanged";
    pub const PREVIEW_EDIT_REJECTED: &str = "previewEditRejected";
    pub const PREVIEW_EDIT_NOT_OBSERVED: &str = "previewEditNotObserved";
    pub const PROVISIONAL_TEXT_UNAVAILABLE: &str = "provisionalTextUnavailable";
    pub const FINAL_EDIT_REJECTED: &str = "finalEditRejected";
    pub const FINAL_EDIT_NOT_OBSERVED: &str = "finalEditNotObserved";
    pub const PREVIEW_REMOVAL_REJECTED: &str = "previewRemovalRejected";
    pub const UNKNOWN_COMMAND: &str = "unknownCommand";
}

pub const PREVIEW_TEXT_MAX: usize = 4000;
pub const COMMIT_TEXT_MAX: usize = 20000;

// ---------------------------------------------------------------------------
// agent-context
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub ok: bool,
    pub status: String, // ready | permission_required | no_target | no_window
    #[serde(rename = "snapshotId")]
    pub snapshot_id: String,
    #[serde(rename = "capturedAt")]
    pub captured_at: u64,
    pub source: String,
    #[serde(rename = "appName")]
    pub app_name: Option<String>,
    #[serde(rename = "bundleId")]
    pub bundle_id: Option<String>,
    #[serde(rename = "processIdentifier")]
    pub process_identifier: Option<i64>,
    #[serde(rename = "windowTitle")]
    pub window_title: Option<String>,
    #[serde(rename = "focusedRole")]
    pub focused_role: Option<String>,
    #[serde(rename = "focusedLabel")]
    pub focused_label: Option<String>,
    #[serde(rename = "focusedFingerprint")]
    pub focused_fingerprint: Option<String>,
    #[serde(rename = "selectionLocation")]
    pub selection_location: Option<i64>,
    #[serde(rename = "selectionLength")]
    pub selection_length: Option<i64>,
    #[serde(rename = "selectedText")]
    pub selected_text: Option<String>,
    pub editable: bool,
    #[serde(rename = "elementCount")]
    pub element_count: usize,
    pub truncated: bool,
    pub elements: Vec<Element>,
    pub privacy: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Element {
    pub id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subrole: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub frame: Frame,
    pub enabled: bool,
    pub focused: bool,
    pub editable: bool,
    pub secure: bool,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The exact privacy tags the macOS snapshot advertises. Same guarantees hold on every OS
/// (read-only, on-demand, no screenshot/network/persistence, secure fields redacted).
pub const PRIVACY_TAGS: [&str; 6] = [
    "read_only",
    "on_demand",
    "secure_fields_redacted",
    "no_screenshot",
    "no_network",
    "no_persistence",
];

pub const MAX_ELEMENTS: usize = 180;
pub const MAX_DEPTH: usize = 12;
pub const MAX_CHILDREN_PER_ELEMENT: usize = 80;
pub const CAPTURE_BUDGET_MS: u64 = 750;

// ---------------------------------------------------------------------------
// agent-action
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ActionRequest {
    pub operation: ActionOperation,
    #[serde(default)]
    pub action: Option<ActionKind>,
    #[serde(default)]
    pub text: Option<String>,
    pub expected: ExpectedTarget,
    #[serde(default)]
    pub undo: Option<UndoPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionOperation {
    Apply,
    Undo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    DraftText,
    ReplaceSelection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedTarget {
    #[serde(rename = "bundleId")]
    pub bundle_id: String,
    #[serde(rename = "processIdentifier")]
    pub process_identifier: i64,
    #[serde(rename = "windowTitle", skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    #[serde(rename = "focusedRole")]
    pub focused_role: String,
    #[serde(rename = "focusedLabel", skip_serializing_if = "Option::is_none")]
    pub focused_label: Option<String>,
    #[serde(rename = "focusedFingerprint")]
    pub focused_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoPayload {
    pub expected: ExpectedTarget,
    #[serde(rename = "originalValue")]
    pub original_value: String,
    #[serde(rename = "originalSelectionLocation", skip_serializing_if = "Option::is_none")]
    pub original_selection_location: Option<i64>,
    #[serde(rename = "originalSelectionLength", skip_serializing_if = "Option::is_none")]
    pub original_selection_length: Option<i64>,
    #[serde(rename = "appliedContentFingerprint")]
    pub applied_content_fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionResponse {
    pub ok: bool,
    pub status: String,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo: Option<UndoPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ActionResponse {
    pub fn failure(status: &str, error: &str) -> Self {
        ActionResponse {
            ok: false,
            status: status.to_string(),
            verified: false,
            undo: None,
            error: Some(error.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// meeting-state
// ---------------------------------------------------------------------------

/// One NDJSON line. `type` is `state` or `status`.
#[derive(Debug, Clone, Serialize)]
pub struct StateLine {
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String, // "accessibility"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<u64>,
    #[serde(rename = "axTrusted", skip_serializing_if = "Option::is_none")]
    pub ax_trusted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<MeetingState>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MeetingState {
    pub app: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "inMeeting")]
    pub in_meeting: bool,
    #[serde(rename = "selfMuted", skip_serializing_if = "Option::is_none")]
    pub self_muted: Option<bool>,
    #[serde(rename = "videoOn", skip_serializing_if = "Option::is_none")]
    pub video_on: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharing: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<String>,
    #[serde(rename = "activeSpeaker", skip_serializing_if = "Option::is_none")]
    pub active_speaker: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_uses_type_and_self_keys() {
        let c = Caption {
            kind: CaptionTag::Caption,
            speaker: "Speaker".into(),
            text: "hi".into(),
            is_self: false,
            ts: 1,
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["type"], "caption");
        assert_eq!(v["self"], false);
        assert!(v.get("is_self").is_none());
    }

    #[test]
    fn classification_camelcase() {
        let r = DictationClassification {
            kind: FieldKind::NaturalLanguage,
            secure: false,
            starts_sentence: true,
            live_editable: true,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"kind\":\"naturalLanguage\""));
        assert!(s.contains("\"startsSentence\":true"));
        assert!(s.contains("\"liveEditable\":true"));
    }

    #[test]
    fn pulled_config_parses_server_manual_string() {
        // Exactly what local_server.rs `state_response` emits: manual as a STRING, plus modules.
        let json = r#"{"manual":"auto","sharing":false,"videoMode":"share","translate":true,
            "language":"en","autoDetect":true,"browserMeet":true,"vocab":"foo","modules":{"meetings":true}}"#;
        let cfg: PulledConfig = serde_json::from_str(json).expect("must not fail on a string manual");
        assert!(cfg.manual_is_auto());
        assert!(!cfg.manual_force_on());
        assert_eq!(cfg.modules.as_ref().map(|m| m.meetings), Some(true));
        assert_eq!(cfg.video_mode.as_deref(), Some("share"));
    }

    #[test]
    fn pulled_config_manual_tristate() {
        let on: PulledConfig = serde_json::from_str(r#"{"manual":"on"}"#).unwrap();
        assert!(on.manual_force_on() && !on.manual_force_off() && !on.manual_is_auto());
        for off in ["off", "paused"] {
            let c: PulledConfig = serde_json::from_str(&format!(r#"{{"manual":"{off}"}}"#)).unwrap();
            assert!(c.manual_force_off() && !c.manual_force_on() && !c.manual_is_auto());
        }
    }

    #[test]
    fn pulled_config_defaults_auto_detect_and_browser_meet_true() {
        // A partial response (fields omitted until the user toggles a setting) must still auto-capture.
        let cfg: PulledConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.auto_detect, "autoDetect must default true, matching Swift");
        assert!(cfg.browser_meet, "browserMeet must default true, matching Swift");
        assert!(cfg.manual_is_auto());
    }

    #[test]
    fn action_request_parses_expected() {
        let json = r#"{"operation":"apply","action":"draft_text","text":"x",
            "expected":{"bundleId":"b","processIdentifier":42,"focusedRole":"AXTextArea",
            "focusedFingerprint":"abc"}}"#;
        let req: ActionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.operation, ActionOperation::Apply);
        assert_eq!(req.expected.process_identifier, 42);
    }
}
