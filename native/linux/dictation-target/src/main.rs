//! dictation-target (Linux) — classify the focused field, optionally run a live edit session.
//! PROTOCOL.md §2.
//!
//! REAL / verifiable: arg dispatch, the classification brain (`classify.rs`) and the session state
//! machine (`session.rs`) — both fully unit-tested — the Result-first output contract, and
//! `--selftest`. Injection is REAL on X11 (XTEST via enigo) and the documented clipboard-paste
//! degrade on Wayland (`platform::inject`).
//!
//! TODO(on-device): the concrete atspi focus read + live text ops (`atspi_focus.rs`). Until wired,
//! the helper classifies as `unknown` (bus up) or `secure` (bus down) and starts no session — the
//! `--session` loop only runs for a naturalLanguage + liveEditable target, which requires the focus
//! read.

mod atspi_focus;
mod classify;
mod session;

use helpers_core::protocol::{DictationClassification, FieldKind};
use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--selftest") {
        if classify::selftest() {
            println!("dictation-target selftest ok");
            std::process::exit(0);
        } else {
            eprintln!("dictation-target selftest failed");
            std::process::exit(1);
        }
    }

    let session_mode = args.iter().any(|a| a == "--session");
    let server = platform::display::detect();

    // Resolve focus + classify on a current-thread async runtime.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let ctx = rt.block_on(atspi_focus::resolve_context());
    let classification = classify::classify(&ctx, server);

    // The Result line is ALWAYS emitted first (both one-shot and --session).
    emit_result(&classification);

    if !session_mode {
        return;
    }

    // A session only runs for a live-editable natural-language target.
    let eligible = classification.kind == FieldKind::NaturalLanguage
        && classification.live_editable
        && ctx.focus.is_some();
    if !eligible {
        return;
    }

    let insertion_location = ctx
        .focus
        .as_ref()
        .and_then(|f| f.selection)
        .map(|(l, _)| l)
        .unwrap_or(0);

    let Some(mut ops) = atspi_focus::AtspiSessionOps::new(server) else {
        return; // no injection backend → cannot run a session
    };

    let stdin = std::io::stdin();
    let lines = stdin.lock().lines().map_while(Result::ok);
    session::run_session(insertion_location, &mut ops, lines, |r| emit_session(r));
}

/// Emit the classification `Result` in Swift field order (kind, secure, startsSentence,
/// liveEditable) + newline. Not sorted — the Swift default encoder preserves declaration order.
fn emit_result(c: &DictationClassification) {
    let line = serde_json::to_string(c).unwrap_or_else(|_| {
        // Fail-closed shape.
        "{\"kind\":\"secure\",\"secure\":true,\"startsSentence\":false,\"liveEditable\":false}".into()
    });
    write_line(&line);
}

fn emit_session(r: &helpers_core::protocol::SessionResponse) {
    if let Ok(line) = serde_json::to_string(r) {
        write_line(&line);
    }
}

fn write_line(line: &str) {
    let stdout = std::io::stdout();
    let mut h = stdout.lock();
    let _ = h.write_all(line.as_bytes());
    let _ = h.write_all(b"\n");
    let _ = h.flush();
}
