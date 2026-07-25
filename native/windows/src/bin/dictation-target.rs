//! dictation-target (Windows) — classify the focused field, then optionally run the live-injection
//! session.
//!
//! REAL / verifiable spine (portable): the always-first-line classification `Result`, arg dispatch
//! (`--session` / `--selftest`), the session command taxonomy + length caps + response framing
//! (`platform::dictation`), and stdin-EOF cleanup.
//!
//! TODO(on-device): the UIA classification (focused ControlType, `IsPassword`, TextPattern) and the
//! `SendInput(KEYEVENTF_UNICODE)` injection with UIPI detection live in `platform::uia` /
//! `platform::inject`. Off-device the classifier reports `unknown`/`liveEditable:false`, so
//! `--session` emits the classification and exits (contract-valid).

use std::io::Write;

use helpers_core::args::Args;

use platform::dictation;
use platform::uia;

fn main() {
    let args = Args::from_env();

    if args.has("selftest") {
        // Pure-logic checks over the portable classifiers.
        assert!(uia::is_code_exe("WindowsTerminal.exe"));
        assert!(uia::is_secure("AXSecureTextField", "", false));
        assert!(!uia::is_secure("AXTextArea", "Message", false));
        assert!(uia::ax_role_for("Edit") == "AXTextField");
        println!("dictation-target selftest ok");
        return;
    }

    let (classification, identity) = uia::classify_focused();
    emit(&classification);

    if args.has("session") {
        if classification.live_editable {
            if let Some(id) = identity {
                dictation::run_session(id);
            }
        }
        // Not live-editable ⇒ nothing more to do; the classification line was already emitted.
    }
}

/// Emit one JSON object + newline to stdout (the classification `Result`).
fn emit<T: serde::Serialize>(value: &T) {
    if let Ok(s) = serde_json::to_string(value) {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        let _ = writeln!(lock, "{s}");
        let _ = lock.flush();
    }
}
