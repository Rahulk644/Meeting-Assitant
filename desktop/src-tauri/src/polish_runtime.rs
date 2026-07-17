use crate::polish_models::{polish_model, PolishModelSpec};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const POLISH_PORT: u16 = 9380;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
// Warm-up starts while the user is still speaking. If a cold start reaches the finish path,
// readiness plus inference remain bounded to twelve seconds before the mechanical fallback wins.
const START_TIMEOUT: Duration = Duration::from_secs(6);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
pub const HARD_TIMEOUT: Duration = Duration::from_secs(12);

struct RuntimeConfig {
    resources: PathBuf,
    models: PathBuf,
    token: String,
}

struct RuntimeState {
    child: Option<Child>,
    model_id: Option<String>,
    last_used: Option<Instant>,
    verified: HashSet<String>,
}

static CONFIG: OnceLock<RuntimeConfig> = OnceLock::new();
static STATE: OnceLock<Mutex<RuntimeState>> = OnceLock::new();
static INFERENCE: Mutex<()> = Mutex::new(());

fn state() -> &'static Mutex<RuntimeState> {
    STATE.get_or_init(|| {
        Mutex::new(RuntimeState {
            child: None,
            model_id: None,
            last_used: None,
            verified: HashSet::new(),
        })
    })
}

pub fn configure(resources: String, models: String, token: String) {
    if CONFIG
        .set(RuntimeConfig {
            resources: resources.into(),
            models: PathBuf::from(models).join("polish"),
            token,
        })
        .is_err()
    {
        return;
    }
    if let Some(config) = CONFIG.get() {
        let _ = fs::create_dir_all(&config.models);
    }
    thread::spawn(|| loop {
        thread::sleep(Duration::from_secs(5));
        let mut runtime = state().lock().unwrap_or_else(|error| error.into_inner());
        let due = runtime
            .last_used
            .map(|last| last.elapsed() >= IDLE_TIMEOUT)
            .unwrap_or(false);
        if due {
            stop_locked(&mut runtime);
        }
    });
}

pub fn model_path(spec: &PolishModelSpec) -> Option<PathBuf> {
    let config = CONFIG.get()?;
    let path = config.models.join(spec.filename);
    fs::metadata(&path)
        .ok()
        .filter(|meta| meta.is_file() && meta.len() == spec.size)
        .map(|_| path)
}

fn runtime_binary(config: &RuntimeConfig) -> Option<PathBuf> {
    let mut candidates = vec![config.resources.join("polish-runtime/llama-server")];
    if cfg!(debug_assertions) {
        candidates.extend([
            PathBuf::from("/opt/homebrew/bin/llama-server"),
            PathBuf::from("/usr/local/bin/llama-server"),
        ]);
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn verify(path: &Path, spec: &PolishModelSpec) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|_| "polish model is unavailable".to_string())?;
    if metadata.len() != spec.size {
        return Err("polish model size mismatch".into());
    }
    let output = Command::new("/usr/bin/shasum")
        .args(["-a", "256", path.to_string_lossy().as_ref()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "polish model verification could not start".to_string())?;
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if !output.status.success() || digest != spec.sha256 {
        return Err("polish model checksum mismatch".into());
    }
    Ok(())
}

fn port_open() -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{POLISH_PORT}")
            .parse()
            .expect("loopback address"),
        Duration::from_millis(150),
    )
    .is_ok()
}

fn healthy(token: &str) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(
        &format!("127.0.0.1:{POLISH_PORT}")
            .parse()
            .expect("loopback address"),
        Duration::from_millis(250),
    ) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let request = format!(
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{POLISH_PORT}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    stream
        .read(&mut head)
        .ok()
        .filter(|count| *count > 0)
        .is_some()
        && head.starts_with(b"HTTP/1.1 200")
}

fn stop_locked(runtime: &mut RuntimeState) {
    if let Some(mut child) = runtime.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    runtime.model_id = None;
    runtime.last_used = None;
}

fn start_model(model_id: &str) -> Result<(), String> {
    let config = CONFIG.get().ok_or("polish runtime is not configured")?;
    let spec = polish_model(model_id).ok_or("unknown polish model")?;
    let path = model_path(spec).ok_or("polish model is not downloaded")?;
    let mut runtime = state().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(child) = runtime.child.as_mut() {
        let running = child
            .try_wait()
            .map_err(|_| "polish runtime status failed")?
            .is_none();
        if running && runtime.model_id.as_deref() == Some(spec.id) {
            runtime.last_used = Some(Instant::now());
            return Ok(());
        }
        stop_locked(&mut runtime);
    }
    if port_open() {
        return Err("polish port is occupied by an untrusted process".into());
    }
    if !runtime.verified.contains(spec.id) {
        verify(&path, spec)?;
        runtime.verified.insert(spec.id.to_string());
    }
    let binary = runtime_binary(config).ok_or("bundled polish runtime is missing")?;
    let child = Command::new(binary)
        .args([
            "--model",
            path.to_string_lossy().as_ref(),
            "--alias",
            spec.id,
            "--host",
            "127.0.0.1",
            "--port",
            &POLISH_PORT.to_string(),
            "--parallel",
            "1",
            "--ctx-size",
            &spec.context_tokens.to_string(),
            "--n-predict",
            "512",
            "--flash-attn",
            "auto",
            "--no-ui",
            "--no-slots",
            "--no-cache-prompt",
            "--offline",
            "--reasoning",
            "off",
            "--log-disable",
        ])
        .env("LLAMA_API_KEY", &config.token)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "polish runtime could not start".to_string())?;
    runtime.child = Some(child);
    runtime.model_id = Some(spec.id.to_string());
    runtime.last_used = Some(Instant::now());
    Ok(())
}

// Warm-up never blocks capture. It verifies and starts the selected model on a detached thread while the
// user speaks; the finish path still has a hard readiness deadline and falls back mechanically.
pub fn warm(model_id: String) {
    thread::spawn(move || {
        let _ = start_model(&model_id);
    });
}

fn wait_ready(deadline: Instant) -> Result<(), String> {
    let config = CONFIG.get().ok_or("polish runtime is not configured")?;
    let started = Instant::now();
    while started.elapsed() < START_TIMEOUT && Instant::now() < deadline {
        if healthy(&config.token) {
            return Ok(());
        }
        {
            let mut runtime = state().lock().unwrap_or_else(|error| error.into_inner());
            if runtime
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok())
                .flatten()
                .is_some()
            {
                stop_locked(&mut runtime);
                return Err("polish runtime exited during startup".into());
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("polish runtime startup timed out".into())
}

pub fn polish(text: &str, model_id: &str, deadline: Instant) -> Result<String, String> {
    let _slot = INFERENCE.lock().unwrap_or_else(|error| error.into_inner());
    if Instant::now() >= deadline {
        return Err("polish deadline reached".into());
    }
    start_model(model_id)?;
    wait_ready(deadline)?;
    let config = CONFIG.get().ok_or("polish runtime is not configured")?;
    let spec = polish_model(model_id).ok_or("unknown polish model")?;
    let max_tokens = ((text.chars().count() as u64 * 3 / 4) + 32).clamp(48, 512);
    let body = json!({
        "model": model_id,
        "messages": [
            {"role":"system","content":spec.system_prompt},
            {"role":"user","content":format!("<transcript>\n{text}\n</transcript>")}
        ],
        "temperature": 0,
        "max_tokens": max_tokens,
        "stream": false,
        "cache_prompt": false,
        "seed": 0
    });
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("polish deadline reached".into());
    }
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(remaining.min(REQUEST_TIMEOUT)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut response = agent
        .post(&format!(
            "http://127.0.0.1:{POLISH_PORT}/v1/chat/completions"
        ))
        .header("Authorization", &format!("Bearer {}", config.token))
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|_| "polish request failed or timed out".to_string())?;
    if !response.status().is_success() {
        return Err("polish runtime rejected the request".into());
    }
    let value: Value = response
        .body_mut()
        .read_json()
        .map_err(|_| "polish runtime returned malformed output".to_string())?;
    let output = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let output = exact_transcript_payload(output).trim().to_string();
    state()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .last_used = Some(Instant::now());
    if output.is_empty() {
        Err("polish runtime returned empty output".into())
    } else {
        Ok(output)
    }
}

// Gemma 3 270M sometimes mirrors the input envelope despite the output instruction. Accept only
// one exact outer envelope; code fences, commentary, extra tags, or partial envelopes remain intact
// so the strict validator rejects them.
fn exact_transcript_payload(output: &str) -> &str {
    output
        .strip_prefix("<transcript>")
        .and_then(|value| value.strip_suffix("</transcript>"))
        .unwrap_or(output)
}

pub fn shutdown() {
    let mut runtime = state().lock().unwrap_or_else(|error| error.into_inner());
    stop_locked(&mut runtime);
}

#[cfg(test)]
mod tests {
    use super::exact_transcript_payload;

    #[test]
    fn output_budget_is_bounded_by_design() {
        for length in [0u64, 12, 10_000] {
            let budget = ((length * 3 / 4) + 32).clamp(48, 512);
            assert!((48..=512).contains(&budget));
        }
    }

    #[test]
    fn unwraps_only_one_exact_transcript_envelope() {
        assert_eq!(
            exact_transcript_payload("<transcript>\nEdited text.\n</transcript>").trim(),
            "Edited text."
        );
        assert_eq!(
            exact_transcript_payload("Here is <transcript>Edited</transcript>"),
            "Here is <transcript>Edited</transcript>"
        );
        assert_eq!(
            exact_transcript_payload("```\n<transcript>Edited</transcript>\n```"),
            "```\n<transcript>Edited</transcript>\n```"
        );
    }
}
