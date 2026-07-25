//! agent-context (Windows) — one bounded, read-only UI Automation snapshot.
//!
//! REAL / verifiable spine (portable): arg dispatch (`--once` / `--selftest`), the sorted-key JSON
//! emission (Swift used `.sortedKeys`; serde_json's default BTreeMap ordering reproduces it), the
//! `AX*` role mapping and secure redaction, `helpers_core` fingerprints, the PROTOCOL privacy tags,
//! and the well-formed degraded snapshots (`permission_required` / `no_target` / `no_window`).
//!
//! TODO(on-device): the UIA BFS itself (tree walk, `IsPassword`, bounding rects) and — critically for
//! the 0.75 s budget — a `CacheRequest` batched read (see `platform::uia`). Off-device it degrades to
//! a `no_window` snapshot with empty elements.

use helpers_core::args::Args;

use platform::uia;

fn main() {
    let args = Args::from_env();

    if args.has("selftest") {
        let ok = uia::ax_role_for("Document") == "AXTextArea"
            && uia::ax_role_for("Edit") == "AXTextField"
            && uia::is_secure("AXTextField", "API key", false)
            && !uia::is_secure("AXTextArea", "Email body", false)
            && helpers_core::fingerprint::state_fingerprint("AXTextArea", None, "x", Some((0, 0)))
                != helpers_core::fingerprint::state_fingerprint("AXTextArea", None, "x", Some((1, 0)));
        if ok {
            println!("agent-context self-test passed");
            std::process::exit(0);
        }
        eprintln!("agent-context self-test failed");
        std::process::exit(1);
    }

    if !args.has("once") {
        eprintln!("usage: agent-context --once | --selftest");
        std::process::exit(2);
    }

    let snapshot = uia::snapshot();
    emit_sorted(&snapshot);
}

/// Emit with sorted keys (serde_json Value ⇒ BTreeMap when `preserve_order` is off) + newline.
fn emit_sorted<T: serde::Serialize>(value: &T) {
    use std::io::Write;
    let json = serde_json::to_value(value)
        .and_then(|v| serde_json::to_string(&v))
        .unwrap_or_else(|_| r#"{"ok":false,"status":"encoding_error"}"#.into());
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{json}");
    let _ = lock.flush();
}
