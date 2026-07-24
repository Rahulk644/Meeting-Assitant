//! agent-action (Windows) — perform one guarded UI mutation (apply/undo) on the disposable editor.
//!
//! REAL / verifiable spine (portable): stdin read + 256 KB cap, `ActionRequest` parse, the
//! validation / `replacing` splice / `mutation_disposition` / fingerprint / undo-record logic in
//! `platform::action`, sorted-key JSON output, and `--selftest`.
//!
//! TODO(on-device): resolving the live focused control, `SetForegroundWindow` activation,
//! `ValuePattern.SetValue` / select-all+inject, and the ~30 ms verify read (see `platform::action`).
//! Gated to `notepad.exe`. Off-device it returns `permission_required`.

use std::io::Read;

use helpers_core::args::Args;
use helpers_core::protocol::{ActionRequest, ActionResponse};

use platform::action;

fn main() {
    let args = Args::from_env();

    if args.has("selftest") {
        // Delegate to the tested pure helpers (they panic on failure via #[test]); re-check here.
        let ok = action::replacing("A🙂B", (1, 2), "ok").as_deref() == Some("AokB")
            && action::replacing("hello", (6, 0), "!").is_none()
            && action::action_gate_ok(42, 7, "notepad.exe")
            && !action::action_gate_ok(42, 7, "chrome.exe")
            && action::mutation_disposition("before", "after", Some("after"))
                == action::Disposition::Verified;
        if ok {
            println!("agent-action self-test passed");
            std::process::exit(0);
        }
        eprintln!("agent-action self-test failed");
        std::process::exit(1);
    }

    if !args.has("request") {
        eprintln!("usage: agent-action --request | --selftest");
        std::process::exit(2);
    }

    // Read the request from stdin (draft/original text never appears in argv).
    let mut buf = Vec::new();
    if std::io::stdin().read_to_end(&mut buf).is_err() || buf.len() > 256 * 1024 {
        emit_sorted(&ActionResponse::failure("invalid_request", "The action request exceeded the local limit."));
        return;
    }
    let response = match serde_json::from_slice::<ActionRequest>(&buf) {
        Ok(req) => action::handle(&req),
        Err(_) => ActionResponse::failure("invalid_request", "The action request was invalid."),
    };
    emit_sorted(&response);
}

fn emit_sorted<T: serde::Serialize>(value: &T) {
    use std::io::Write;
    let json = serde_json::to_value(value)
        .and_then(|v| serde_json::to_string(&v))
        .unwrap_or_else(|_| r#"{"ok":false,"status":"encoding_error","verified":false}"#.into());
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{json}");
    let _ = lock.flush();
}
