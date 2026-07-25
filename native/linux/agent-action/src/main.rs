//! agent-action (Linux) — one guarded UI mutation over AT-SPI. PROTOCOL.md §4.
//!
//! REAL / verifiable: arg dispatch + exit codes, stdin size guard + request decode, the whole
//! mutation brain in `exec.rs` (UTF-16 splice, MutationDisposition state machine, verify/rollback,
//! undo payload, fingerprints — all unit-tested), sorted-key JSON emission, `--selftest`, and the
//! permission + single-app allowlist gates.
//!
//! TODO(on-device): the concrete atspi control resolution in `atspi_target.rs` (needs a live control;
//! the Wayland activation gate has no equivalent at all). Until wired, resolution fails closed with
//! a contract-valid `stale_target`.

mod atspi_target;
mod exec;

use helpers_core::protocol::{ActionKind, ActionOperation, ActionRequest, ActionResponse};
use std::io::{Read, Write};

const STDIN_LIMIT: usize = 256 * 1024;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--selftest") {
        if exec::selftest() {
            println!("agent-action self-test passed");
            std::process::exit(0);
        } else {
            eprintln!("agent-action self-test failed");
            std::process::exit(1);
        }
    }
    if !args.iter().any(|a| a == "--request") {
        eprintln!("usage: agent-action --request | --selftest");
        std::process::exit(2);
    }

    let response = run_request();
    emit(&response);
    // Implicit exit 0 for every --request outcome.
}

fn run_request() -> ActionResponse {
    // Read stdin with the hard cap.
    let mut buf = Vec::new();
    if std::io::stdin().read_to_end(&mut buf).is_err() {
        return ActionResponse::failure("invalid_request", "The action request was invalid.");
    }
    if buf.len() > STDIN_LIMIT {
        return ActionResponse::failure(
            "invalid_request",
            "The action request exceeded the local limit.",
        );
    }
    let request: ActionRequest = match serde_json::from_slice(&buf) {
        Ok(r) => r,
        Err(_) => {
            return ActionResponse::failure("invalid_request", "The action request was invalid.")
        }
    };

    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(_) => return ActionResponse::failure("invalid_request", "Internal runtime error."),
    };

    match request.operation {
        ActionOperation::Apply => rt.block_on(do_apply(request)),
        ActionOperation::Undo => rt.block_on(do_undo(request)),
    }
}

async fn do_apply(request: ActionRequest) -> ActionResponse {
    // Request validation (pure).
    let action = match request.action {
        Some(a) => a,
        None => {
            return ActionResponse::failure(
                "invalid_request",
                "Only a non-empty bounded draft or selection replacement can be applied.",
            )
        }
    };
    let text = request.text.clone().unwrap_or_default();
    if text.is_empty() || text.encode_utf16().count() > exec::MAX_TEXT_LEN {
        return ActionResponse::failure(
            "invalid_request",
            "Only a non-empty bounded draft or selection replacement can be applied.",
        );
    }

    match atspi_target::resolve_target(&request.expected, true).await {
        Err(resp) => resp,
        Ok(mut target) => exec::apply_mutation(
            matches!(action, ActionKind::ReplaceSelection),
            &text,
            target.as_mut(),
            request.expected.clone(),
        ),
    }
}

async fn do_undo(request: ActionRequest) -> ActionResponse {
    let undo = match &request.undo {
        Some(u) if u.original_value.encode_utf16().count() <= exec::MAX_TEXT_LEN => u.clone(),
        _ => return ActionResponse::failure("invalid_request", "The undo record is unavailable."),
    };
    match atspi_target::resolve_target(&request.expected, false).await {
        Err(resp) => resp,
        Ok(mut target) => exec::perform_undo(&undo, target.as_mut()),
    }
}

/// Emit one sorted-key JSON line + newline (Swift `.sortedKeys`).
fn emit(resp: &ActionResponse) {
    let line = match serde_json::to_value(resp).and_then(|v| serde_json::to_string(&v)) {
        Ok(s) => s,
        Err(_) => "{\"ok\":false,\"status\":\"encoding_error\",\"verified\":false}".to_string(),
    };
    let stdout = std::io::stdout();
    let mut h = stdout.lock();
    let _ = h.write_all(line.as_bytes());
    let _ = h.write_all(b"\n");
    let _ = h.flush();
}
