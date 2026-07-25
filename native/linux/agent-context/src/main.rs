//! agent-context (Linux) — read-only AT-SPI accessibility snapshot. PROTOCOL.md §3.
//!
//! REAL / verifiable: arg dispatch + exit codes, the whole snapshot brain in `ax.rs` (BFS budgets,
//! secure redaction, label composition, fingerprint, the four status outcomes — all unit-tested),
//! sorted-key + newline JSON emission, and `--selftest`.
//!
//! TODO(on-device): the concrete atspi tree walk in `atspi_tree.rs` (needs a live AT-SPI registry).
//! Until then the helper connects to the a11y bus for real and degrades to `permission_required`
//! (bus down) or `no_window` (bus up, tree not yet wired) — both contract-valid.

mod atspi_tree;
mod ax;

use helpers_core::epoch_ms;
use helpers_core::protocol::Snapshot;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Dispatch order matches the Swift: --selftest wins even if --once is also present.
    if args.iter().any(|a| a == "--selftest") {
        if ax::selftest() {
            println!("agent-context self-test passed");
            std::process::exit(0);
        } else {
            eprintln!("agent-context self-test failed");
            std::process::exit(1);
        }
    }
    if !args.iter().any(|a| a == "--once") {
        eprintln!("usage: agent-context --once | --selftest");
        std::process::exit(2);
    }

    let snapshot = capture();
    emit(&snapshot);
    // Implicit exit 0.
}

/// Drive the one-shot capture on a current-thread tokio runtime (AT-SPI is async).
fn capture() -> Snapshot {
    let captured_at = epoch_ms();
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!(">> failed to start async runtime: {e}");
            return ax::error_snapshot(
                "permission_required",
                "none",
                captured_at,
                None,
                None,
                None,
                "Internal runtime error.",
            );
        }
    };

    rt.block_on(async move {
        match atspi_tree::discover(captured_at).await {
            atspi_tree::Discovery::Error(s) => s,
            atspi_tree::Discovery::Ready(target) => {
                let id = uuid::Uuid::new_v4().to_string(); // lowercased hyphenated v4
                ax::build_snapshot(target, captured_at, id)
            }
        }
    })
}

/// Emit the snapshot as one line: sorted keys (serde_json `Value` is a sorted `BTreeMap` by
/// default), then a single `\n`. Matches the Swift `.sortedKeys` + trailing newline.
fn emit(snapshot: &Snapshot) {
    let line = match serde_json::to_value(snapshot).and_then(|v| serde_json::to_string(&v)) {
        Ok(s) => s,
        Err(_) => "{\"ok\":false,\"status\":\"encoding_error\"}".to_string(),
    };
    let stdout = std::io::stdout();
    let mut h = stdout.lock();
    let _ = h.write_all(line.as_bytes());
    let _ = h.write_all(b"\n");
    let _ = h.flush();
}
