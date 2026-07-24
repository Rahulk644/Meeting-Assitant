use crate::dictation_cleanup::DictationTargetKind;
use serde::Deserialize;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub struct DictationContext {
    pub kind: DictationTargetKind,
    pub starts_sentence: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireContext {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    starts_sentence: bool,
    #[serde(default)]
    live_editable: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    unsafe_fallback: bool,
}

enum TargetCommand {
    Preview(String, Sender<bool>),
    Commit(String, Sender<bool>),
    Cancel(Sender<bool>),
}

/// One bounded Accessibility session tied to the field that was focused when dictation began. The native
/// helper retains the AX element and reconciles only the exact provisional span it inserted; transcript
/// content never returns to Rust from the target field.
#[derive(Clone)]
pub struct DictationTarget {
    context: Arc<Mutex<DictationContext>>,
    commands: Sender<TargetCommand>,
    live_editable: Arc<AtomicBool>,
    preview_active: Arc<AtomicBool>,
    preview_failed: Arc<AtomicBool>,
}

impl DictationTarget {
    pub fn context(&self) -> Arc<Mutex<DictationContext>> {
        self.context.clone()
    }

    pub fn preview(&self, text: String) -> bool {
        let (tx, rx) = mpsc::channel();
        if self
            .commands
            .send(TargetCommand::Preview(text, tx))
            .is_err()
        {
            return false;
        }
        rx.recv_timeout(Duration::from_millis(850)).unwrap_or(false)
    }

    pub fn has_preview(&self) -> bool {
        self.preview_active.load(Ordering::Acquire)
    }

    pub fn requires_copy_fallback(&self) -> bool {
        self.live_editable.load(Ordering::Acquire) && self.preview_failed.load(Ordering::Acquire)
    }

    /// Replace the tracked provisional span with the final polished text. Returns false when no provisional
    /// span exists or when the original target can no longer be reconciled safely.
    pub fn commit(&self, text: String) -> bool {
        if !self.has_preview() {
            return false;
        }
        let (tx, rx) = mpsc::channel();
        if self.commands.send(TargetCommand::Commit(text, tx)).is_err() {
            return false;
        }
        let ok = rx
            .recv_timeout(Duration::from_millis(1_200))
            .unwrap_or(false);
        if ok {
            self.preview_active.store(false, Ordering::Release);
        } else {
            self.preview_failed.store(true, Ordering::Release);
        }
        ok
    }

    /// Remove only the exact provisional span inserted by this session. It is a no-op for final-only models
    /// and unsupported fields.
    pub fn cancel(&self) -> bool {
        if !self.has_preview() {
            return true;
        }
        let (tx, rx) = mpsc::channel();
        if self.commands.send(TargetCommand::Cancel(tx)).is_err() {
            return false;
        }
        let ok = rx
            .recv_timeout(Duration::from_millis(1_200))
            .unwrap_or(false);
        if ok {
            self.preview_active.store(false, Ordering::Release);
        }
        ok
    }
}

fn respond(command: TargetCommand, value: bool) {
    let sender = match command {
        TargetCommand::Preview(_, sender)
        | TargetCommand::Commit(_, sender)
        | TargetCommand::Cancel(sender) => sender,
    };
    let _ = sender.send(value);
}

fn command_payload(command: &TargetCommand) -> serde_json::Value {
    match command {
        TargetCommand::Preview(text, _) => json!({"op":"preview","text":text}),
        TargetCommand::Commit(text, _) => json!({"op":"commit","text":text}),
        TargetCommand::Cancel(_) => json!({"op":"cancel"}),
    }
}

fn command_name(command: &TargetCommand) -> &'static str {
    match command {
        TargetCommand::Preview(_, _) => "preview",
        TargetCommand::Commit(_, _) => "commit",
        TargetCommand::Cancel(_) => "cancel",
    }
}

fn run_target_worker(
    resources: String,
    context: Arc<Mutex<DictationContext>>,
    live_editable: Arc<AtomicBool>,
    preview_active: Arc<AtomicBool>,
    preview_failed: Arc<AtomicBool>,
    commands: Receiver<TargetCommand>,
) {
    let binary = crate::native_host::resolve(&resources, crate::native_host::helpers::DICTATION_TARGET);
    if !binary.is_file() {
        while let Ok(command) = commands.recv() {
            respond(command, false);
        }
        return;
    }
    let Ok(mut child) = Command::new(binary)
        .arg("--session")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        while let Ok(command) = commands.recv() {
            respond(command, false);
        }
        return;
    };
    let Some(mut input) = child.stdin.take() else {
        let _ = child.kill();
        return;
    };
    let Some(output) = child.stdout.take() else {
        let _ = child.kill();
        return;
    };
    let mut output = BufReader::new(output);
    let mut line = String::new();
    let wire = if output
        .read_line(&mut line)
        .ok()
        .filter(|size| *size <= 4_096)
        .is_some()
    {
        serde_json::from_str::<WireContext>(&line).ok()
    } else {
        None
    };
    let Some(wire) = wire else {
        let _ = child.kill();
        while let Ok(command) = commands.recv() {
            respond(command, false);
        }
        return;
    };
    let kind = if wire.secure {
        DictationTargetKind::Secure
    } else {
        DictationTargetKind::from_wire(&wire.kind)
    };
    *context.lock().unwrap_or_else(|error| error.into_inner()) = DictationContext {
        kind,
        starts_sentence: wire.starts_sentence,
    };
    live_editable.store(wire.live_editable, Ordering::Release);

    if !wire.live_editable {
        let _ = child.wait();
        while let Ok(command) = commands.recv() {
            respond(command, false);
        }
        return;
    }

    while let Ok(command) = commands.recv() {
        let operation = command_name(&command);
        let payload = command_payload(&command);
        let mut bytes = match serde_json::to_vec(&payload) {
            Ok(bytes) => bytes,
            Err(_) => {
                respond(command, false);
                continue;
            }
        };
        bytes.push(b'\n');
        let wrote = input.write_all(&bytes).and_then(|_| input.flush()).is_ok();
        line.clear();
        let response = if wrote && output.read_line(&mut line).is_ok() && line.len() <= 4_096 {
            serde_json::from_str::<WireResponse>(&line).ok()
        } else {
            None
        };
        let ok = response.as_ref().map(|value| value.ok).unwrap_or(false);
        preview_active.store(
            response.as_ref().map(|value| value.active).unwrap_or(false),
            Ordering::Release,
        );
        if let Some(value) = response.as_ref() {
            if !value.ok {
                let reason = value.error.as_deref().unwrap_or("unspecified");
                log::warn!(
                    "dictation target {operation} rejected active={} unsafeFallback={} reason={reason}",
                    value.active,
                    value.unsafe_fallback
                );
                if value.unsafe_fallback {
                    preview_failed.store(true, Ordering::Release);
                }
            }
        } else if live_editable.load(Ordering::Acquire) {
            log::warn!("dictation target {operation} session ended without a response");
            preview_failed.store(true, Ordering::Release);
        }
        let terminal = matches!(
            &command,
            TargetCommand::Commit(_, _) | TargetCommand::Cancel(_)
        );
        respond(command, ok);
        if terminal || response.is_none() {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub fn classify_async(resources: String) -> DictationTarget {
    // Missing, denied, slow, or malformed Accessibility evidence fails closed: no model request and no
    // transcript-bearing history. A successful helper result replaces this before normal dictations finish.
    let context = Arc::new(Mutex::new(DictationContext {
        kind: DictationTargetKind::Secure,
        starts_sentence: false,
    }));
    let preview_active = Arc::new(AtomicBool::new(false));
    let live_editable = Arc::new(AtomicBool::new(false));
    let preview_failed = Arc::new(AtomicBool::new(false));
    let (commands, receiver) = mpsc::channel();
    let target = DictationTarget {
        context: context.clone(),
        commands,
        live_editable: live_editable.clone(),
        preview_active: preview_active.clone(),
        preview_failed: preview_failed.clone(),
    };
    std::thread::spawn(move || {
        run_target_worker(
            resources,
            context,
            live_editable,
            preview_active,
            preview_failed,
            receiver,
        )
    });
    target
}
