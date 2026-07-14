//! Small in-process loopback server for the packaged UI and meeting pipeline.
//!
//! This replaces the bundled Node runtime. The app process owns the listening socket, every
//! request is authenticated with a per-launch secret, and WebViews receive that secret only through
//! a one-time bootstrap URL that is exchanged for a SameSite/HttpOnly cookie.

use objc2::{rc::autoreleasepool, AnyThread};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSImage, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const ADDRESS: &str = "127.0.0.1:8790";
const ORIGIN: &str = "http://127.0.0.1:8790";
const STT_PORT: u16 = 8091;
const LLM_PORT: u16 = 9379;
const MODEL_NAME: &str = "whisper-large-v3-turbo-Q8_0.gguf";
const MODEL_URL: &str = "https://huggingface.co/handy-computer/whisper-large-v3-turbo-gguf/resolve/main/whisper-large-v3-turbo-Q8_0.gguf";
const MODEL_SHA256: &str = "d5e65f2b0828802ae2c231673d31982cebe3a778c95d9494a9f3efee6bd17448";
const MODEL_SIZE: u64 = 886_381_824;
const KEYCHAIN_SERVICE: &str = "com.myassistant.desktop.ai-provider";
const MAX_JSON_BODY: usize = 1_000_000;
const MAX_AUDIO_BODY: usize = 64 * 1024 * 1024;

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
static LLM_HTTP: OnceLock<ureq::Agent> = OnceLock::new();
static APP_ICONS: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();

#[derive(Clone)]
pub struct ServerInfo {
    pub token: String,
}

struct Shared {
    cfg: Config,
    state: Mutex<State>,
    clients: Mutex<Vec<Sender<String>>>,
    stopping: AtomicBool,
}

#[derive(Clone)]
struct Config {
    token: String,
    web_dir: PathBuf,
    release: Value,
    data_dir: PathBuf,
    meetings_dir: PathBuf,
    rec_dir: PathBuf,
    models_dir: PathBuf,
    transcribe_bin: PathBuf,
    meeting_notifier: Arc<dyn Fn(String) + Send + Sync>,
}

struct State {
    settings: HashMap<String, String>,
    last_state: Map<String, Value>,
    gate: Gate,
    current: Option<Meeting>,
    modules: Modules,
    dictation: DictationConfig,
    recent_dictations: Vec<Value>,
    dict_stop: u64,
    unload_idle: bool,
    whisper_idle_minutes: f64,
    vocab: String,
    model_downloading: bool,
    model_verified: bool,
    stt_child: Option<Child>,
    stt_last_used: Option<Instant>,
    llm_child: Option<Child>,
    llm_last_used: Option<Instant>,
    finalize_generation: u64,
    meeting_last_checkpoint: Option<Instant>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    #[serde(default)]
    id: String,
    #[serde(default = "unknown_speaker")]
    speaker: String,
    #[serde(default)]
    speaker_source: String,
    #[serde(default)]
    speaker_context: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    ts: u64,
    #[serde(default, rename = "self")]
    is_self: bool,
    #[serde(default)]
    excluded: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meeting {
    id: String,
    #[serde(default = "untitled")]
    title: String,
    #[serde(default)]
    app: String,
    #[serde(default)]
    started_at: u64,
    #[serde(default)]
    ended_at: Option<u64>,
    #[serde(default)]
    participants: Vec<String>,
    #[serde(default)]
    participant_evidence: Vec<Value>,
    #[serde(default)]
    transcript_revision: u64,
    #[serde(default)]
    lines: Vec<Line>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    video: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    summary_state: String,
    #[serde(default)]
    summary_error: String,
    #[serde(default)]
    summary_stale: bool,
    #[serde(default)]
    summary_revision: u64,
    #[serde(default)]
    title_source: String,
    #[serde(default)]
    calendar_title: String,
    #[serde(default)]
    last_correction_at: u64,
    #[serde(default)]
    last_correction_reason: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Modules {
    dictation: bool,
    meetings: bool,
    writing: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct DictationConfig {
    enabled: bool,
    language: String,
    punctuation: String,
}

#[derive(Default)]
struct Gate {
    detected: bool,
    manual: Option<String>,
    auto_detect: bool,
    manual_detected: bool,
    manual_hard: bool,
}

impl Gate {
    fn new() -> Self {
        Self {
            auto_detect: true,
            ..Self::default()
        }
    }
    fn recording(&self) -> bool {
        match self.manual.as_deref() {
            Some("on") => true,
            Some("paused") | Some("off") => false,
            _ => self.auto_detect && self.detected,
        }
    }
    fn paused(&self) -> bool {
        self.manual.as_deref() == Some("paused")
    }
    fn set_detected(&mut self, detected: bool) -> bool {
        if self.detected == detected {
            return false;
        }
        self.detected = detected;
        if self.manual.is_some() && !self.manual_hard && self.detected != self.manual_detected {
            self.manual = None;
        }
        true
    }
    fn set_manual(&mut self, action: &str) -> bool {
        match action {
            "start" => {
                self.manual = Some("on".into());
                self.manual_hard = !(self.auto_detect && self.detected);
            }
            "resume" => {
                self.manual = Some("on".into());
                self.manual_hard = false;
            }
            "pause" => {
                self.manual = Some("paused".into());
                self.manual_hard = false;
            }
            "stop" => {
                self.manual = Some("off".into());
                self.manual_hard = false;
            }
            "auto" => {
                self.manual = None;
                self.manual_hard = false;
            }
            _ => return false,
        }
        self.manual_detected = self.detected;
        true
    }
}

fn unknown_speaker() -> String {
    "Unknown speaker".into()
}
fn untitled() -> String {
    "Impromptu meeting".into()
}
fn meeting_start_edge(
    authoritative: bool,
    previously_detected: bool,
    detected: Option<bool>,
) -> bool {
    authoritative && !previously_detected && detected == Some(true)
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn validate_release_contract(value: &Value) -> Result<(), String> {
    let capabilities = value
        .get("capabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| "release contract is missing capabilities".to_string())?;
    let routes = value
        .get("routes")
        .and_then(Value::as_object)
        .ok_or_else(|| "release contract is missing routes".to_string())?;
    for (capability, route) in [
        ("dictation", "dictation"),
        ("meetings", "meetings"),
        ("settings", "settings"),
    ] {
        if capabilities.get(capability).and_then(Value::as_bool) != Some(true) {
            return Err(format!(
                "release contract must enable the {capability} core capability"
            ));
        }
        if routes.get(route).and_then(Value::as_str) != Some(capability) {
            return Err(format!(
                "release contract has an invalid {route} route mapping"
            ));
        }
    }
    Ok(())
}

pub fn start(
    web_dir: String,
    data_dir: String,
    models_dir: String,
    res_base: String,
    legacy_model_dirs: Vec<String>,
    meeting_notifier: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<ServerInfo, String> {
    if SHARED.get().is_some() {
        return Err("local server already started".into());
    }
    let listener = TcpListener::bind(ADDRESS).map_err(|e| {
        format!("My Assistant cannot safely start because {ADDRESS} is already in use: {e}")
    })?;
    let token = random_token()?;
    let web_dir = PathBuf::from(web_dir);
    if !web_dir.join("index.html").is_file() {
        return Err("packaged web UI is missing index.html".into());
    }
    let release_path = web_dir.join("release-payload.json");
    let release: Value = serde_json::from_slice(
        &fs::read(&release_path)
            .map_err(|error| format!("packaged release contract is missing: {error}"))?,
    )
    .map_err(|error| format!("packaged release contract is invalid JSON: {error}"))?;
    validate_release_contract(&release)?;
    let data_dir = PathBuf::from(data_dir);
    let meetings_dir = data_dir.join("meetings");
    let rec_dir = meetings_dir.join("rec");
    let temp_dir = data_dir.join("tmp");
    let models_dir = PathBuf::from(models_dir);
    for dir in [&data_dir, &meetings_dir, &rec_dir, &temp_dir, &models_dir] {
        private_dir(dir)?;
    }
    clean_private_temp(&temp_dir);
    if let Err(error) = migrate_legacy_model(&models_dir, &legacy_model_dirs) {
        // Migration is an upgrade convenience, never a startup dependency. If it fails, the verified normal
        // download remains available and the setup prompt explains what the user needs.
        log::warn!("legacy speech-model migration skipped: {error}");
    }

    let settings_path = data_dir.join("settings.json");
    let mut settings = fs::read(&settings_path)
        .ok()
        .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
        .unwrap_or_default();
    if let Some(raw) = settings.get("set.aiModel").cloned() {
        if let Some(sanitized) = sanitize_model_setting(&raw, true) {
            settings.insert("set.aiModel".into(), sanitized);
            let _ = write_private(
                &settings_path,
                &serde_json::to_vec(&settings).unwrap_or_default(),
            );
        }
    }
    let modules = setting_json(&settings, "modules")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or(Modules {
            dictation: true,
            meetings: true,
            writing: false,
        });
    let state = State {
        settings,
        last_state: Map::new(),
        gate: Gate::new(),
        current: None,
        modules: modules.clone(),
        dictation: DictationConfig {
            enabled: modules.dictation,
            language: "auto".into(),
            punctuation: "auto".into(),
        },
        recent_dictations: Vec::new(),
        dict_stop: 0,
        unload_idle: true,
        whisper_idle_minutes: 10.0,
        vocab: String::new(),
        model_downloading: false,
        model_verified: false,
        stt_child: None,
        stt_last_used: None,
        llm_child: None,
        llm_last_used: None,
        finalize_generation: 0,
        meeting_last_checkpoint: None,
    };
    let shared = Arc::new(Shared {
        cfg: Config {
            token: token.clone(),
            web_dir,
            release,
            data_dir,
            meetings_dir,
            rec_dir,
            models_dir,
            transcribe_bin: PathBuf::from(res_base).join("stt/transcribe-server/transcribe-server"),
            meeting_notifier,
        },
        state: Mutex::new(state),
        clients: Mutex::new(Vec::new()),
        stopping: AtomicBool::new(false),
    });
    finalize_orphans(&shared.cfg.meetings_dir);
    SHARED
        .set(shared.clone())
        .map_err(|_| "local server state already set".to_string())?;

    let server = Server::from_listener(listener, None).map_err(|e| e.to_string())?;
    thread::spawn(move || {
        for request in server.incoming_requests() {
            if shared.stopping.load(Ordering::Relaxed) {
                break;
            }
            let s = shared.clone();
            thread::spawn(move || handle(request, s));
        }
    });
    start_maintenance(SHARED.get().unwrap().clone());
    Ok(ServerInfo { token })
}

// Native app events do not need to loop back through HTTP. Delivering them directly keeps dictation
// lifecycle updates ordered (important for a very fast start -> stop) and avoids spawning a curl process
// for every UI state change. External helpers still use the authenticated /ingest endpoint below.
pub fn ingest_local(value: Value) -> bool {
    let Some(shared) = SHARED.get() else {
        return false;
    };
    ingest_event(shared.clone(), value);
    true
}

pub fn shutdown() {
    let Some(shared) = SHARED.get() else { return };
    shared.stopping.store(true, Ordering::Relaxed);
    if let Ok(mut state) = shared.state.lock() {
        if let Some(mut child) = state.stt_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(mut child) = state.llm_child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn start_maintenance(shared: Arc<Shared>) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(10));
        if shared.stopping.load(Ordering::Relaxed) {
            return;
        }
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(meeting) = state.current.as_mut() {
            if !meeting.lines.is_empty() && save_meeting(&shared.cfg, meeting).is_ok() {
                state.meeting_last_checkpoint = Some(Instant::now());
            }
        }
        let no_meeting = state.current.is_none();
        let stt_due = state.unload_idle
            && no_meeting
            && state
                .stt_last_used
                .map(|t| {
                    t.elapsed()
                        >= Duration::from_secs_f64(state.whisper_idle_minutes.max(0.0) * 60.0)
                })
                .unwrap_or(false);
        if stt_due {
            if let Some(mut child) = state.stt_child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            state.stt_last_used = None;
        }
        let llm_due = state
            .llm_last_used
            .map(|t| t.elapsed() >= Duration::from_secs(300))
            .unwrap_or(false);
        if llm_due {
            if let Some(mut child) = state.llm_child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            state.llm_last_used = None;
        }
    });
}

fn clean_private_temp(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() {
            let _ = fs::remove_file(p);
        }
    }
}

fn header(req: &Request, name: &str) -> String {
    req.headers()
        .iter()
        .find(|h| h.field.to_string().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
        .unwrap_or_default()
}

fn h(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("valid header")
}

fn respond(
    request: Request,
    status: u16,
    content_type: &str,
    body: impl Into<Vec<u8>>,
    extra: Vec<Header>,
) {
    let mut response = Response::from_data(body.into())
        .with_status_code(StatusCode(status))
        .with_header(h("Content-Type", content_type))
        .with_header(h(
            "Cache-Control",
            "no-store, no-cache, must-revalidate, max-age=0",
        ))
        .with_header(h("Pragma", "no-cache"))
        .with_header(h("Expires", "0"))
        .with_header(h("X-Content-Type-Options", "nosniff"))
        .with_header(h("Referrer-Policy", "no-referrer"));
    for header in extra {
        response.add_header(header);
    }
    let _ = request.respond(response);
}

fn json_response(request: Request, status: u16, value: Value) {
    respond(
        request,
        status,
        "application/json",
        serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
        vec![],
    )
}

fn authenticated(req: &Request, token: &str) -> bool {
    let bearer = header(req, "Authorization");
    if bearer == format!("Bearer {token}") {
        return true;
    }
    header(req, "Cookie")
        .split(';')
        .map(str::trim)
        .any(|c| c == format!("assistant_session={token}"))
}

fn valid_origin(req: &Request) -> bool {
    let origin = header(req, "Origin");
    if !origin.is_empty() && origin != ORIGIN {
        return false;
    }
    let site = header(req, "Sec-Fetch-Site");
    site.is_empty() || matches!(site.as_str(), "same-origin" | "none")
}

fn bootstrap_token(url: &str) -> Option<&str> {
    url.split_once('?')?
        .1
        .split('&')
        .find_map(|pair| pair.strip_prefix("bootstrap="))
}

fn fresh_ui_location() -> String {
    // WKWebView can restore a previous launch's DOM even when the HTML response is no-store. Keep the
    // authenticated page URL unique per launch so an old hidden/blank release surface is never eligible.
    format!("/?run={}-{}", std::process::id(), now_ms())
}

fn handle(mut request: Request, shared: Arc<Shared>) {
    let host = header(&request, "Host");
    if host != ADDRESS && host != "localhost:8790" {
        return respond(request, 403, "text/plain", b"forbidden".to_vec(), vec![]);
    }
    let raw_url = request.url().to_string();
    let path = raw_url.split('?').next().unwrap_or("/").to_string();
    if request.method() == &Method::Get
        && path == "/"
        && bootstrap_token(&raw_url) == Some(shared.cfg.token.as_str())
    {
        let location = fresh_ui_location();
        return respond(
            request,
            302,
            "text/plain",
            Vec::new(),
            vec![
                h("Location", &location),
                h(
                    "Set-Cookie",
                    &format!(
                        "assistant_session={}; HttpOnly; SameSite=Strict; Path=/",
                        shared.cfg.token
                    ),
                ),
            ],
        );
    }
    if !authenticated(&request, &shared.cfg.token) || !valid_origin(&request) {
        return respond(
            request,
            401,
            "application/json",
            br#"{"error":"unauthorized"}"#.to_vec(),
            vec![],
        );
    }

    match (request.method().clone(), path.as_str()) {
        (Method::Get, "/") | (Method::Get, "/index.html") => serve_asset(
            request,
            &shared.cfg.web_dir.join("index.html"),
            "text/html; charset=utf-8",
            true,
        ),
        (Method::Get, "/logo.png") => serve_asset(
            request,
            &shared.cfg.web_dir.join("logo.png"),
            "image/png",
            false,
        ),
        (Method::Get, "/favicon.ico") => respond(request, 204, "image/x-icon", Vec::new(), vec![]),
        (Method::Get, "/events") => serve_events(request, shared),
        (Method::Get, "/api/release") => json_response(request, 200, shared.cfg.release.clone()),
        (Method::Post, "/ingest") => {
            let body = match read_body(&mut request, MAX_JSON_BODY) {
                Ok(b) => b,
                Err(e) => return json_response(request, 413, json!({"error":e})),
            };
            let value = match serde_json::from_slice::<Value>(&body) {
                Ok(v) => v,
                Err(_) => return json_response(request, 400, json!({"error":"invalid JSON"})),
            };
            ingest_event(shared, value);
            respond(request, 204, "text/plain", Vec::new(), vec![])
        }
        (Method::Get, "/api/state") => state_response(request, &shared),
        (Method::Post, "/api/meeting/control") => meeting_control(request, shared),
        (Method::Post, "/api/meeting/notes") => meeting_notes(request, shared),
        (Method::Get, "/api/settings") => settings_get(request, &shared),
        (Method::Post, "/api/settings") => settings_post(request, shared),
        (Method::Post, "/api/config") => config_post(request, shared),
        (Method::Post, "/api/transcribe") => transcribe_proxy(request, shared),
        (Method::Get, "/api/dictations") => {
            let values = shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .recent_dictations
                .clone();
            json_response(request, 200, Value::Array(values))
        }
        (Method::Post, "/api/dictation/stop") => {
            shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .dict_stop += 1;
            respond(request, 204, "text/plain", Vec::new(), vec![])
        }
        (Method::Get, "/api/model/status") => {
            let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            json_response(
                request,
                200,
                json!({"present": model_path(&shared.cfg).is_some(), "downloading": state.model_downloading}),
            )
        }
        (Method::Post, "/api/model/download") => {
            start_model_download(shared);
            respond(request, 202, "text/plain", Vec::new(), vec![])
        }
        (Method::Post, "/api/open-settings") => open_settings(request),
        (Method::Get, "/api/meetings") => meetings_list(request, &shared),
        _ => dynamic_route(request, shared, path),
    }
}

fn serve_asset(request: Request, path: &Path, content_type: &str, html: bool) {
    match fs::read(path) {
        Ok(bytes) => {
            let mut extra = Vec::new();
            if html {
                extra.push(h("Content-Security-Policy", "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; img-src 'self' data:; media-src 'self'; connect-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'"));
            }
            respond(request, 200, content_type, bytes, extra)
        }
        Err(_) => respond(request, 404, "text/plain", b"not found".to_vec(), vec![]),
    }
}

struct EventReader {
    rx: Receiver<String>,
    pending: Vec<u8>,
    pos: usize,
}
impl Read for EventReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.pending.len() {
            self.pending = match self.rx.recv_timeout(Duration::from_secs(15)) {
                Ok(value) => format!("data: {value}\n\n").into_bytes(),
                Err(RecvTimeoutError::Timeout) => b": keepalive\n\n".to_vec(),
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            };
            self.pos = 0;
        }
        let n = buf.len().min(self.pending.len() - self.pos);
        buf[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn serve_events(request: Request, shared: Arc<Shared>) {
    let (tx, rx) = mpsc::channel();
    if let Ok(state) = shared.state.lock() {
        if !state.last_state.is_empty() {
            let _ = tx.send(json!({"type":"state","state":state.last_state}).to_string());
        }
    }
    shared
        .clients
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(tx);
    let response = Response::new(
        StatusCode(200),
        vec![
            h("Content-Type", "text/event-stream"),
            h("Cache-Control", "no-cache"),
            h("Connection", "keep-alive"),
            h("X-Content-Type-Options", "nosniff"),
        ],
        EventReader {
            rx,
            pending: Vec::new(),
            pos: 0,
        },
        None,
        None,
    );
    let _ = request.respond(response);
}

fn read_body(request: &mut Request, max: usize) -> Result<Vec<u8>, String> {
    if request.body_length().unwrap_or(0) > max {
        return Err("request body too large".into());
    }
    let mut body = Vec::new();
    request
        .as_reader()
        .take((max + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    if body.len() > max {
        return Err("request body too large".into());
    }
    Ok(body)
}

fn read_json(request: &mut Request) -> Result<Value, String> {
    if !header(request, "Content-Type")
        .to_ascii_lowercase()
        .starts_with("application/json")
    {
        return Err("Content-Type must be application/json".into());
    }
    serde_json::from_slice(&read_body(request, MAX_JSON_BODY)?).map_err(|_| "invalid JSON".into())
}

fn broadcast(shared: &Shared, value: &Value) {
    let text = value.to_string();
    shared
        .clients
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|tx| tx.send(text.clone()).is_ok());
}

fn state_response(request: Request, shared: &Shared) {
    let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = state.last_state.clone();
    out.insert(
        "manual".into(),
        Value::String(state.gate.manual.clone().unwrap_or_else(|| "auto".into())),
    );
    out.insert(
        "dictation".into(),
        serde_json::to_value(&state.dictation).unwrap_or(Value::Null),
    );
    out.insert(
        "modules".into(),
        serde_json::to_value(&state.modules).unwrap_or(Value::Null),
    );
    out.insert("dictStop".into(), Value::Number(state.dict_stop.into()));
    json_response(request, 200, Value::Object(out));
}

fn ingest_event(shared: Arc<Shared>, mut value: Value) {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if kind == "dictation" {
        if let Some(text) = value
            .get("text")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            let item = json!({"text": clean(text, 4000), "ts": value.get("ts").and_then(Value::as_u64).unwrap_or_else(now_ms), "ms": value.get("ms").and_then(Value::as_u64).unwrap_or(0)});
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.recent_dictations.insert(0, item);
            state.recent_dictations.truncate(50);
        }
        broadcast(&shared, &value);
        return;
    }
    if kind == "dictation-state" {
        broadcast(&shared, &value);
        return;
    }

    let authoritative = kind == "state"
        && value.get("source").and_then(Value::as_str) == Some("coreaudio")
        && value
            .pointer("/state/inMeeting")
            .and_then(Value::as_bool)
            .is_some();
    let detected = value.pointer("/state/inMeeting").and_then(Value::as_bool);
    if kind == "state" {
        if let Some(obj) = value.get_mut("state").and_then(Value::as_object_mut) {
            obj.remove("inMeeting");
            if obj.is_empty() {
                value = Value::Null;
            }
        }
    }
    let mut notify_app = None;
    {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if !value.is_null() {
            record_event(&shared.cfg, &mut state, &value);
        }
        if authoritative {
            let started = meeting_start_edge(authoritative, state.gate.detected, detected);
            state.gate.set_detected(detected.unwrap_or(false));
            if started {
                notify_app = state
                    .last_state
                    .get("app")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| Some("your call".into()));
            }
        }
    }
    if let Some(app) = notify_app {
        (shared.cfg.meeting_notifier)(app);
    }
    if !value.is_null() {
        broadcast(&shared, &value);
    }
    if authoritative {
        recompute_lifecycle(shared, None);
    }
}

fn record_event(cfg: &Config, state: &mut State, value: &Value) {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    if kind == "state" {
        if let Some(incoming) = value.get("state").and_then(Value::as_object) {
            for (key, val) in incoming {
                state.last_state.insert(key.clone(), val.clone());
            }
            if let Some(meeting) = state.current.as_mut() {
                update_meta(meeting, incoming);
            }
        }
    } else if kind == "caption" && state.gate.recording() {
        if state.current.is_none() {
            state.current = Some(new_meeting(&state.last_state));
        }
        let checkpoint_due = state
            .meeting_last_checkpoint
            .map(|at| at.elapsed() >= Duration::from_secs(2))
            .unwrap_or(true);
        let mut checkpoint_saved = false;
        if let Some(meeting) = state.current.as_mut() {
            let ts = value
                .get("ts")
                .and_then(Value::as_u64)
                .unwrap_or_else(now_ms);
            let index = meeting.lines.len();
            meeting.lines.push(Line {
                id: format!("line-{ts}-{index}"),
                speaker: clean(
                    value
                        .get("speaker")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown speaker"),
                    120,
                ),
                speaker_source: clean(
                    value
                        .get("speakerSource")
                        .and_then(Value::as_str)
                        .unwrap_or(
                            if value.get("self").and_then(Value::as_bool).unwrap_or(false) {
                                "user"
                            } else {
                                "unknown"
                            },
                        ),
                    80,
                ),
                speaker_context: clean(
                    value
                        .get("speakerContext")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    240,
                ),
                text: clean(
                    value.get("text").and_then(Value::as_str).unwrap_or(""),
                    4000,
                ),
                ts,
                is_self: value.get("self").and_then(Value::as_bool).unwrap_or(false),
                excluded: false,
            });
            if checkpoint_due {
                checkpoint_saved = save_meeting(cfg, meeting).is_ok();
            }
        }
        if checkpoint_saved {
            state.meeting_last_checkpoint = Some(Instant::now());
        }
    } else if kind == "recording" {
        if state.current.is_none() {
            state.current = Some(new_meeting(&state.last_state));
        }
        let file = value.get("file").and_then(Value::as_str).unwrap_or("");
        if safe_basename(file, Some("mp4")) {
            if let Some(meeting) = state.current.as_mut() {
                meeting.video = file.into();
                let _ = save_meeting(cfg, meeting);
            }
        }
    }
}

fn update_meta(meeting: &mut Meeting, state: &Map<String, Value>) {
    if let Some(title) =
        first_state_title(state, &["calendarTitle", "scheduledTitle", "eventTitle"])
            .filter(|title| valid_title(title))
    {
        meeting.calendar_title = title.clone();
        if meeting.title_source != "user" {
            meeting.title = title;
            meeting.title_source = "calendar".into();
        }
    } else if meeting.title_source != "user" && meeting.title_source != "calendar" {
        if let Some(title) =
            first_state_title(state, &["meetingTitle", "title"]).filter(|title| valid_title(title))
        {
            meeting.title = title;
            meeting.title_source = "detected".into();
        }
    }
    if let Some(app) = state.get("app").and_then(Value::as_str) {
        meeting.app = normalize_app(app);
    }
    if let Some(values) = state.get("participants").and_then(Value::as_array) {
        for name in values
            .iter()
            .filter_map(Value::as_str)
            .map(|s| clean(s, 120))
            .filter(|s| !s.is_empty())
        {
            if !meeting.participants.contains(&name) {
                meeting.participants.push(name);
            }
        }
    }
    if let Some(values) = state.get("participantEvidence").and_then(Value::as_array) {
        meeting.participant_evidence = values.clone();
    }
}

fn new_meeting(state: &Map<String, Value>) -> Meeting {
    let now = now_ms();
    let calendar_title =
        first_state_title(state, &["calendarTitle", "scheduledTitle", "eventTitle"])
            .filter(|title| valid_title(title))
            .unwrap_or_default();
    let detected_title = first_state_title(state, &["meetingTitle", "title"])
        .filter(|title| valid_title(title))
        .unwrap_or_default();
    let (title, title_source) = if !calendar_title.is_empty() {
        (calendar_title.clone(), "calendar".to_string())
    } else if !detected_title.is_empty() {
        (detected_title, "detected".to_string())
    } else {
        (untitled(), "fallback".to_string())
    };
    Meeting {
        id: now.to_string(),
        title,
        app: state
            .get("app")
            .and_then(Value::as_str)
            .map(normalize_app)
            .unwrap_or_default(),
        started_at: now,
        ended_at: None,
        participants: state
            .get("participants")
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        participant_evidence: state
            .get("participantEvidence")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        transcript_revision: 0,
        lines: Vec::new(),
        notes: String::new(),
        video: String::new(),
        summary: String::new(),
        summary_state: String::new(),
        summary_error: String::new(),
        summary_stale: false,
        summary_revision: 0,
        title_source,
        calendar_title,
        last_correction_at: 0,
        last_correction_reason: String::new(),
    }
}

fn recompute_lifecycle(shared: Arc<Shared>, action: Option<&str>) {
    let mut finalized = None;
    let mut schedule = None;
    {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.gate.recording() {
            state.finalize_generation += 1;
            if state.current.is_none() {
                state.current = Some(new_meeting(&state.last_state));
            }
            let _ = ensure_stt(&shared.cfg, &mut state);
        } else if state.current.is_some() {
            if state.gate.paused() {
                state.finalize_generation += 1;
            } else if action == Some("stop") {
                finalized = finalize_locked(&shared.cfg, &mut state);
            } else {
                state.finalize_generation += 1;
                schedule = Some(state.finalize_generation);
            }
        }
    }
    if let Some(id) = finalized {
        after_finalize(shared.clone(), id);
    }
    if let Some(generation) = schedule {
        let s = shared.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(15));
            let id = {
                let mut state = s.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.finalize_generation == generation
                    && !state.gate.recording()
                    && !state.gate.paused()
                {
                    finalize_locked(&s.cfg, &mut state)
                } else {
                    None
                }
            };
            if let Some(id) = id {
                after_finalize(s, id);
            }
        });
    }
    broadcast_call(&shared);
}

fn broadcast_call(shared: &Shared) {
    let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    let current = state.current.as_ref();
    broadcast(
        shared,
        &json!({"type":"state","state":{"inMeeting": state.gate.recording() || state.gate.paused(), "paused":state.gate.paused(), "startedAt":current.map(|m|m.started_at), "title":current.map(resolved_title), "app":current.map(|m|m.app.clone())}}),
    );
}

fn finalize_locked(cfg: &Config, state: &mut State) -> Option<String> {
    let mut meeting = state.current.take()?;
    meeting.ended_at = Some(now_ms());
    if meeting.lines.is_empty() {
        return None;
    }
    ensure_line_ids(&mut meeting);
    if meeting.title_source != "user" {
        meeting.title = resolved_title(&meeting);
        if !valid_title(&meeting.title) {
            meeting.title_source = "fallback".into();
        }
    }
    match can_generate_notes(&meeting) {
        Ok(()) => {
            meeting.summary_state = "queued".into();
            meeting.summary_error.clear();
        }
        Err(reason) => {
            meeting.summary_state = "skipped".into();
            meeting.summary_error = reason;
        }
    }
    let id = meeting.id.clone();
    let _ = save_meeting(cfg, &meeting);
    state.meeting_last_checkpoint = None;
    state.last_state.remove("startedAt");
    state.stt_last_used = Some(Instant::now());
    Some(id)
}

fn after_finalize(shared: Arc<Shared>, id: String) {
    broadcast(
        &shared,
        &json!({"type":"meeting-updated","id":id,"summaryState":"queued"}),
    );
    thread::spawn(move || {
        if let Err(error) = generate_summary(&shared, &id, Value::Null) {
            if let Some(mut meeting) = read_meeting(&shared.cfg, &id, None) {
                if meeting.summary_state != "stale" {
                    meeting.summary_state = "failed".into();
                    meeting.summary_error = clean(&error, 240);
                    let _ = save_meeting(&shared.cfg, &meeting);
                    broadcast(
                        &shared,
                        &json!({"type":"meeting-updated","id":id,"summaryState":"failed"}),
                    );
                }
            }
        }
    });
}

fn meeting_control(mut request: Request, shared: Arc<Shared>) {
    let body = match read_json(&mut request) {
        Ok(v) => v,
        Err(e) => return json_response(request, 400, json!({"error":e})),
    };
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let ok = shared
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .gate
        .set_manual(&action);
    if ok {
        recompute_lifecycle(shared.clone(), Some(&action));
    }
    let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    json_response(
        request,
        if ok { 200 } else { 400 },
        json!({"manual":state.gate.manual.clone().unwrap_or_else(||"auto".into()),"inMeeting":state.gate.recording()||state.gate.paused(),"paused":state.gate.paused()}),
    )
}

fn meeting_notes(mut request: Request, shared: Arc<Shared>) {
    let body = match read_json(&mut request) {
        Ok(v) => v,
        Err(e) => return json_response(request, 400, json!({"error":e})),
    };
    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    let Some(meeting) = state.current.as_mut() else {
        return respond(request, 409, "text/plain", Vec::new(), vec![]);
    };
    meeting.notes = clean(
        body.get("notes").and_then(Value::as_str).unwrap_or(""),
        8000,
    );
    let _ = save_meeting(&shared.cfg, meeting);
    respond(request, 204, "text/plain", Vec::new(), vec![])
}

fn settings_get(request: Request, shared: &Shared) {
    let mut settings = shared
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .settings
        .clone();
    if let Some(raw) = settings.get("set.aiModel").cloned() {
        settings.insert(
            "set.aiModel".into(),
            sanitize_model_setting(&raw, false).unwrap_or(raw),
        );
    }
    json_response(
        request,
        200,
        serde_json::to_value(settings).unwrap_or_else(|_| json!({})),
    )
}

fn settings_post(mut request: Request, shared: Arc<Shared>) {
    let body = match read_json(&mut request) {
        Ok(v) => v,
        Err(e) => return json_response(request, 400, json!({"error":e})),
    };
    let patch = body
        .get("set")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        for (key, value) in patch {
            if value.is_null() {
                state.settings.remove(&key);
                continue;
            }
            let mut value = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            if key == "set.aiModel" {
                value = sanitize_model_setting(&value, true).unwrap_or(value);
            }
            state.settings.insert(key, value);
        }
        if let Some(modules) = setting_json(&state.settings, "modules")
            .and_then(|v| serde_json::from_value::<Modules>(v).ok())
        {
            state.modules = Modules {
                dictation: modules.dictation,
                meetings: modules.meetings,
                writing: false,
            };
            state.dictation.enabled = state.modules.dictation;
        }
        let _ = save_settings(&shared.cfg, &state.settings);
    }
    respond(request, 204, "text/plain", Vec::new(), vec![])
}

fn sanitize_model_setting(raw: &str, persist_secret: bool) -> Option<String> {
    let mut value: Value = serde_json::from_str(raw).ok()?;
    let obj = value.as_object_mut()?;
    let provider = obj
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("local")
        .to_string();
    if persist_secret {
        if let Some(key) = obj
            .get("key")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            let _ = security_framework::passwords::set_generic_password(
                KEYCHAIN_SERVICE,
                &provider,
                key.as_bytes(),
            );
            obj.insert("keySaved".into(), Value::Bool(true));
        } else if keychain_key(&provider).is_some() {
            obj.insert("keySaved".into(), Value::Bool(true));
        }
    } else if keychain_key(&provider).is_some() {
        obj.insert("keySaved".into(), Value::Bool(true));
    }
    obj.insert("key".into(), Value::String(String::new()));
    Some(value.to_string())
}

fn keychain_key(account: &str) -> Option<String> {
    security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, account)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|s| !s.is_empty())
}

fn save_settings(cfg: &Config, settings: &HashMap<String, String>) -> Result<(), String> {
    write_private(
        &cfg.data_dir.join("settings.json"),
        &serde_json::to_vec(settings).map_err(|e| e.to_string())?,
    )
}

fn setting_json(settings: &HashMap<String, String>, key: &str) -> Option<Value> {
    serde_json::from_str(settings.get(&format!("set.{key}"))?).ok()
}

fn config_post(mut request: Request, shared: Arc<Shared>) {
    let body = match read_json(&mut request) {
        Ok(v) => v,
        Err(e) => return json_response(request, 400, json!({"error":e})),
    };
    let mut broadcasts = Vec::new();
    {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        for key in ["videoMode", "translate", "language", "browserMeet"] {
            if let Some(value) = body.get(key) {
                state.last_state.insert(key.into(), value.clone());
                broadcasts.push((key, value.clone()));
            }
        }
        if let Some(auto) = body.get("autoDetect").and_then(Value::as_bool) {
            state.gate.auto_detect = auto;
            state
                .last_state
                .insert("autoDetect".into(), Value::Bool(auto));
            broadcasts.push(("autoDetect", Value::Bool(auto)));
        }
        if let Some(unload) = body.get("unloadIdle").and_then(Value::as_bool) {
            state.unload_idle = unload;
        }
        if let Some(minutes) = body.get("whisperIdleMinutes").and_then(Value::as_f64) {
            state.whisper_idle_minutes = minutes.clamp(0.0, 60.0);
        }
        if let Some(vocab) = body.get("vocab").and_then(Value::as_str) {
            let cleaned = clean(vocab, 800);
            state.vocab = cleaned.clone();
            state
                .last_state
                .insert("vocab".into(), Value::String(cleaned));
        }
        if let Some(dictation) = body.get("dictation").and_then(Value::as_object) {
            if let Some(v) = dictation.get("enabled").and_then(Value::as_bool) {
                state.dictation.enabled = v;
            }
            if let Some(v) = dictation.get("language").and_then(Value::as_str) {
                state.dictation.language = clean(v, 24);
            }
            if let Some(v) = dictation.get("punctuation").and_then(Value::as_str) {
                state.dictation.punctuation = clean(v, 24);
            }
        }
        if let Some(modules) = body.get("modules") {
            if let Ok(m) = serde_json::from_value::<Modules>(modules.clone()) {
                state.modules = Modules {
                    dictation: m.dictation,
                    meetings: m.meetings,
                    writing: false,
                };
                state.dictation.enabled = state.modules.dictation;
                let modules_json = serde_json::to_string(&state.modules).unwrap_or_default();
                state.settings.insert("set.modules".into(), modules_json);
                let _ = save_settings(&shared.cfg, &state.settings);
            }
        }
    }
    for (key, value) in broadcasts {
        broadcast(&shared, &json!({"type":"state","state":{key:value}}));
    }
    recompute_lifecycle(shared, None);
    respond(request, 204, "text/plain", Vec::new(), vec![])
}

fn transcribe_proxy(mut request: Request, shared: Arc<Shared>) {
    let content_type = header(&request, "Content-Type");
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return json_response(
            request,
            400,
            json!({"error":"multipart/form-data required"}),
        );
    }
    let body = match read_body(&mut request, MAX_AUDIO_BODY) {
        Ok(b) => b,
        Err(e) => return json_response(request, 413, json!({"error":e})),
    };
    let (language, punctuation, vocab) = {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = ensure_stt(&shared.cfg, &mut state) {
            return json_response(request, 503, json!({"error":error}));
        }
        state.stt_last_used = Some(Instant::now());
        (
            if state.dictation.language == "auto" {
                "en".into()
            } else {
                state.dictation.language.clone()
            },
            state.dictation.punctuation.clone(),
            state.vocab.clone(),
        )
    };
    let body = inject_multipart_fields(
        body,
        &content_type,
        &[
            ("language", language),
            ("translate", "false".into()),
            (
                "prompt",
                if vocab.is_empty() {
                    String::new()
                } else {
                    format!("Vocabulary: {vocab}")
                },
            ),
        ],
    );
    match forward_stt(&shared.cfg.token, &content_type, &body) {
        Ok(bytes) => {
            let mut value: Value =
                serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({"text":""}));
            if punctuation == "off" {
                if let Some(text) = value.get("text").and_then(Value::as_str) {
                    let stripped: String =
                        text.chars().filter(|c| !".,!?;:…".contains(*c)).collect();
                    value["text"] =
                        Value::String(stripped.split_whitespace().collect::<Vec<_>>().join(" "));
                }
            }
            json_response(request, 200, value)
        }
        Err(error) => json_response(request, 502, json!({"error":error})),
    }
}

fn inject_multipart_fields(
    mut body: Vec<u8>,
    content_type: &str,
    fields: &[(&str, String)],
) -> Vec<u8> {
    let Some(boundary) = content_type
        .split(';')
        .find_map(|part| part.trim().strip_prefix("boundary="))
        .map(|s| s.trim_matches('"').to_string())
    else {
        return body;
    };
    let closing = format!("--{boundary}--").into_bytes();
    let cut = body
        .windows(closing.len())
        .rposition(|w| w == closing.as_slice())
        .unwrap_or(body.len());
    body.truncate(cut);
    for (name, value) in fields.iter().filter(|(_, v)| !v.is_empty()) {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

fn ensure_stt(cfg: &Config, state: &mut State) -> Result<(), String> {
    if let Some(child) = state.stt_child.as_mut() {
        if child.try_wait().map_err(|e| e.to_string())?.is_none() {
            state.stt_last_used = Some(Instant::now());
            return Ok(());
        }
        state.stt_child = None;
    }
    let model = model_path(cfg).ok_or_else(|| "speech model is not installed".to_string())?;
    if !state.model_verified {
        verify_model(&model)?;
        state.model_verified = true;
    }
    if !cfg.transcribe_bin.is_file() {
        return Err("bundled transcription engine is missing".into());
    }
    if port_open(STT_PORT) {
        return Err(
            "transcription port is occupied by an untrusted process; relaunch after closing it"
                .into(),
        );
    }
    let child = Command::new(&cfg.transcribe_bin)
        .args([
            "-m",
            model.to_string_lossy().as_ref(),
            "--host",
            "127.0.0.1",
            "--port",
            &STT_PORT.to_string(),
        ])
        .env("ASSISTANT_STT_TOKEN", &cfg.token)
        .env("ASSISTANT_PARENT_PID", std::process::id().to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    state.stt_child = Some(child);
    for _ in 0..150 {
        if authenticated_health(STT_PORT, &cfg.token) {
            state.stt_last_used = Some(Instant::now());
            return Ok(());
        }
        if state
            .stt_child
            .as_mut()
            .and_then(|c| c.try_wait().ok())
            .flatten()
            .is_some()
        {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    if let Some(mut child) = state.stt_child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    Err("transcription engine did not start securely".into())
}

fn authenticated_health(port: u16, token: &str) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(300),
    ) else {
        return false;
    };
    let request = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).is_ok() && {
        let mut head = [0u8; 12];
        stream.read(&mut head).ok().filter(|n| *n > 0).is_some()
            && head.starts_with(b"HTTP/1.1 200")
    }
}

fn forward_stt(token: &str, content_type: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{STT_PORT}").parse().unwrap(),
        Duration::from_secs(2),
    )
    .map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(120))).ok();
    let head = format!("POST /inference HTTP/1.1\r\nHost: 127.0.0.1:{STT_PORT}\r\nAuthorization: Bearer {token}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| e.to_string())?;
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("invalid transcription response")?;
    if !response.starts_with(b"HTTP/1.1 200") {
        return Err(String::from_utf8_lossy(&response[..split]).to_string());
    }
    Ok(response[split + 4..].to_vec())
}

fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_millis(200),
    )
    .is_ok()
}

fn model_path(cfg: &Config) -> Option<PathBuf> {
    let path = cfg.models_dir.join(MODEL_NAME);
    if fs::metadata(&path)
        .ok()
        .map(|m| m.is_file() && m.len() == MODEL_SIZE)
        .unwrap_or(false)
    {
        Some(path)
    } else {
        None
    }
}

// Adopt a model used by the Node beta into the canonical Application Support directory. `cp -c` makes an
// APFS clone (independent inode, shared blocks) so migration is instant and does not consume another 845 MB;
// a regular copy is the portable fallback. The existing ensure_stt path verifies the pinned SHA-256 before
// the model is ever loaded, so startup only needs the exact expected size and remains fast.
fn migrate_legacy_model(models_dir: &Path, legacy_dirs: &[String]) -> Result<bool, String> {
    migrate_legacy_file(models_dir, legacy_dirs, MODEL_NAME, MODEL_SIZE)
}

fn migrate_legacy_file(
    models_dir: &Path,
    legacy_dirs: &[String],
    name: &str,
    expected_size: u64,
) -> Result<bool, String> {
    let dest = models_dir.join(name);
    if fs::metadata(&dest)
        .ok()
        .map(|m| m.is_file() && m.len() == expected_size)
        .unwrap_or(false)
    {
        return Ok(false);
    }

    for dir in legacy_dirs {
        let source = Path::new(dir).join(name);
        if !fs::metadata(&source)
            .ok()
            .map(|m| m.is_file() && m.len() == expected_size)
            .unwrap_or(false)
        {
            continue;
        }

        let temp = models_dir.join(format!("{name}.migrating"));
        let _ = fs::remove_file(&temp);
        let cloned = Command::new("/bin/cp")
            .args([
                "-c",
                source.to_string_lossy().as_ref(),
                temp.to_string_lossy().as_ref(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !cloned {
            fs::copy(&source, &temp).map_err(|e| format!("copy {}: {e}", source.display()))?;
        }
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        if fs::metadata(&temp).ok().map(|m| m.len()) != Some(expected_size) {
            let _ = fs::remove_file(&temp);
            return Err("migrated speech model has the wrong size".into());
        }
        fs::rename(&temp, &dest).map_err(|e| e.to_string())?;
        log::info!("adopted the existing speech model into Application Support");
        return Ok(true);
    }
    Ok(false)
}

fn start_model_download(shared: Arc<Shared>) {
    {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.model_downloading || model_path(&shared.cfg).is_some() {
            return;
        }
        state.model_downloading = true;
    }
    broadcast(&shared, &json!({"type":"model-progress","pct":0}));
    thread::spawn(move || {
        let dest = shared.cfg.models_dir.join(MODEL_NAME);
        let tmp = shared.cfg.models_dir.join(format!("{MODEL_NAME}.part"));
        let mut child = Command::new("/usr/bin/curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                MODEL_URL,
                "--output",
                tmp.to_string_lossy().as_ref(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let result = match child.as_mut() {
            Ok(child) => loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        break if status.success() {
                            verify_model(&tmp)
                        } else {
                            Err("model download failed".into())
                        }
                    }
                    Ok(None) => {
                        let got = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
                        let pct = ((got.saturating_mul(100) / MODEL_SIZE).min(99)) as u64;
                        broadcast(&shared, &json!({"type":"model-progress","pct":pct}));
                        thread::sleep(Duration::from_millis(500));
                    }
                    Err(e) => break Err(e.to_string()),
                }
            },
            Err(e) => Err(e.to_string()),
        };
        match result.and_then(|_| {
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
            fs::rename(&tmp, &dest).map_err(|e| e.to_string())
        }) {
            Ok(()) => {
                shared
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .model_verified = true;
                broadcast(
                    &shared,
                    &json!({"type":"model-progress","pct":100,"done":true}),
                );
            }
            Err(error) => {
                let _ = fs::remove_file(&tmp);
                broadcast(&shared, &json!({"type":"model-progress","error":error}));
            }
        }
        shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .model_downloading = false;
    });
}

fn verify_model(path: &Path) -> Result<(), String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() != MODEL_SIZE {
        return Err(format!(
            "speech model size mismatch: expected {MODEL_SIZE}, got {}",
            meta.len()
        ));
    }
    let output = Command::new("/usr/bin/shasum")
        .args(["-a", "256", path.to_string_lossy().as_ref()])
        .output()
        .map_err(|e| e.to_string())?;
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if !output.status.success() || digest != MODEL_SHA256 {
        return Err("speech model checksum mismatch".into());
    }
    Ok(())
}

fn open_settings(mut request: Request) {
    let body = match read_json(&mut request) {
        Ok(v) => v,
        Err(e) => return json_response(request, 400, json!({"error":e})),
    };
    let pane = body.get("pane").and_then(Value::as_str).unwrap_or("");
    let allowed = matches!(
        pane,
        "Privacy_Microphone"
            | "Privacy_AudioCapture"
            | "Privacy_ScreenCapture"
            | "Privacy_Accessibility"
    );
    if allowed {
        let _ = Command::new("/usr/bin/open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{pane}"
            ))
            .spawn();
    }
    respond(
        request,
        if allowed { 204 } else { 400 },
        "text/plain",
        Vec::new(),
        vec![],
    )
}

fn meetings_list(request: Request, shared: &Shared) {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&shared.cfg.meetings_dir) {
        for path in entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        {
            if let Some(meeting) = read_meeting_path(&path) {
                out.push(meeting_summary(&shared.cfg, &meeting));
            }
        }
    }
    if let Some(current) = shared
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .current
        .as_ref()
    {
        if !current.lines.is_empty()
            && !out
                .iter()
                .any(|v| v.get("id") == Some(&Value::String(current.id.clone())))
        {
            out.push(meeting_summary(&shared.cfg, current));
        }
    }
    out.sort_by_key(|v| std::cmp::Reverse(v.get("startedAt").and_then(Value::as_u64).unwrap_or(0)));
    json_response(request, 200, Value::Array(out))
}

fn dynamic_route(mut request: Request, shared: Arc<Shared>, path: String) {
    if let Some(file) = path.strip_prefix("/api/rec/") {
        if request.method() == &Method::Get {
            return serve_recording(request, &shared.cfg, file);
        }
    }
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    if parts.len() >= 3 && parts[0] == "api" && parts[1] == "meetings" && valid_meeting_id(parts[2])
    {
        let id = parts[2];
        match (request.method().clone(), parts.get(3).copied()) {
            (Method::Get, None) => {
                let current = shared
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .current
                    .clone();
                return match read_meeting(&shared.cfg, id, current.as_ref()) {
                    Some(mut m) => {
                        m.title = resolved_title(&m);
                        json_response(request, 200, serde_json::to_value(m).unwrap())
                    }
                    None => json_response(request, 404, json!({"error":"not found"})),
                };
            }
            (Method::Post, Some("title")) => return meeting_title(request, &shared, id),
            (Method::Post, Some("corrections")) => return meeting_correction(request, &shared, id),
            (Method::Get, Some("thumb")) => return meeting_thumb(request, &shared.cfg, id),
            (Method::Post, Some("summary")) => {
                let body = read_json(&mut request).unwrap_or(Value::Null);
                let result = generate_summary(&shared, id, body);
                return match result {
                    Ok(m) => json_response(
                        request,
                        200,
                        json!({"summary":m.summary,"summaryRevision":m.summary_revision}),
                    ),
                    Err(e) => json_response(request, 502, json!({"error":e})),
                };
            }
            (Method::Post, Some("ask")) => return meeting_ask(request, shared, id),
            _ => {}
        }
    }
    respond(request, 404, "text/plain", b"not found".to_vec(), vec![])
}

fn meeting_title(mut request: Request, shared: &Shared, id: &str) {
    let body = match read_json(&mut request) {
        Ok(v) => v,
        Err(e) => return json_response(request, 400, json!({"error":e})),
    };
    let title = clean(body.get("title").and_then(Value::as_str).unwrap_or(""), 120);
    if title.is_empty() {
        return json_response(request, 400, json!({"error":"title is required"}));
    }
    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    let mut meeting = if state.current.as_ref().map(|m| m.id.as_str()) == Some(id) {
        state.current.take().unwrap()
    } else if let Some(m) = read_meeting(&shared.cfg, id, None) {
        m
    } else {
        return json_response(request, 404, json!({"error":"not found"}));
    };
    meeting.title = title.clone();
    meeting.title_source = "user".into();
    let _ = save_meeting(&shared.cfg, &meeting);
    if meeting.ended_at.is_none() {
        state.current = Some(meeting);
    }
    json_response(request, 200, json!({"title":title}))
}

fn meeting_correction(mut request: Request, shared: &Shared, id: &str) {
    let correction = match read_json(&mut request) {
        Ok(v) => v,
        Err(e) => return json_response(request, 400, json!({"error":e})),
    };
    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    let is_current = state.current.as_ref().map(|m| m.id.as_str()) == Some(id);
    let mut meeting = if is_current {
        state.current.take().unwrap()
    } else if let Some(m) = read_meeting(&shared.cfg, id, None) {
        m
    } else {
        return json_response(request, 404, json!({"error":"not found"}));
    };
    match apply_correction(&mut meeting, &correction) {
        Ok(()) => {
            let _ = save_meeting(&shared.cfg, &meeting);
            let response = serde_json::to_value(&meeting).unwrap();
            if is_current {
                state.current = Some(meeting);
            }
            drop(state);
            broadcast(
                shared,
                &json!({"type":"meeting-updated","id":id,"summaryState":"stale"}),
            );
            json_response(request, 200, response)
        }
        Err(error) => {
            if is_current {
                state.current = Some(meeting);
            }
            json_response(request, 400, json!({"error":error}))
        }
    }
}

fn meeting_thumb(request: Request, cfg: &Config, id: &str) {
    let Some(meeting) = read_meeting(cfg, id, None) else {
        return respond(request, 404, "text/plain", Vec::new(), vec![]);
    };
    if safe_basename(&meeting.video, Some("mp4")) {
        let video = cfg.rec_dir.join(&meeting.video);
        let thumb = cfg
            .rec_dir
            .join(format!("{}.jpg", meeting.video.trim_end_matches(".mp4")));
        if video.is_file() {
            // Current builds write the first complete screen frame beside the MP4. Keep this
            // best-effort fallback for recordings made by an older build on developer machines.
            if !thumb.is_file() {
                let ffmpeg = [
                    "/opt/homebrew/bin/ffmpeg",
                    "/usr/local/bin/ffmpeg",
                    "/usr/bin/ffmpeg",
                ]
                .iter()
                .map(Path::new)
                .find(|p| p.is_file());
                if let Some(ffmpeg) = ffmpeg {
                    let _ = Command::new(ffmpeg)
                        .args([
                            "-loglevel",
                            "error",
                            "-y",
                            "-ss",
                            "1",
                            "-i",
                            video.to_string_lossy().as_ref(),
                            "-frames:v",
                            "1",
                            "-vf",
                            "scale=480:-1",
                            thumb.to_string_lossy().as_ref(),
                        ])
                        .status();
                }
            }
            if let Ok(bytes) = fs::read(&thumb) {
                return respond(
                    request,
                    200,
                    "image/jpeg",
                    bytes,
                    vec![h("Cache-Control", "private, max-age=3600")],
                );
            }
        }
    }
    if let Some(bytes) = installed_app_icon_png(&meeting.app) {
        return respond(
            request,
            200,
            "image/png",
            bytes,
            vec![h("Cache-Control", "private, max-age=86400")],
        );
    }
    respond(
        request,
        200,
        "image/svg+xml; charset=utf-8",
        app_thumbnail_svg(&meeting.app).into_bytes(),
        vec![h("Cache-Control", "private, max-age=86400")],
    )
}

fn platform_bundle_ids(app: &str) -> &'static [&'static str] {
    match normalize_app(app).to_ascii_lowercase().as_str() {
        "zoom" => &["us.zoom.xos"],
        "teams" => &["com.microsoft.teams2", "com.microsoft.teams"],
        "webex" => &["com.cisco.webex2", "com.cisco.webexmeetingsapp"],
        "facetime" => &["com.apple.FaceTime"],
        "slack" => &["com.tinyspeck.slackmacgap"],
        "discord" => &["com.hnc.Discord"],
        "ringcentral" => &["com.ringcentral.RingCentral", "com.ringcentral.glip"],
        "goto" => &["com.goto.GoTo", "com.logmein.GoToMeeting"],
        "around" => &["com.getaround.Around"],
        "skype" => &["com.skype.skype"],
        "whatsapp" => &["net.whatsapp.WhatsApp"],
        "telegram" => &["ru.keepcoder.Telegram"],
        "signal" => &["org.whispersystems.signal-desktop"],
        _ => &[],
    }
}

fn installed_app_icon_png(app: &str) -> Option<Vec<u8>> {
    let key = normalize_app(app).to_ascii_lowercase();
    if key.is_empty() || key == "call" {
        return None;
    }
    let cache = APP_ICONS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .cloned()
    {
        return (!cached.is_empty()).then_some(cached);
    }
    let bytes = autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        for bundle in platform_bundle_ids(&key) {
            let identifier = NSString::from_str(bundle);
            let Some(url) = workspace.URLForApplicationWithBundleIdentifier(&identifier) else {
                continue;
            };
            let Some(path) = url.path() else { continue };
            let image = workspace.iconForFile(&path);
            if let Some(data) = downscaled_icon_png(&image) {
                return data;
            }
        }
        Vec::new()
    });
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, bytes.clone());
    (!bytes.is_empty()).then_some(bytes)
}

#[allow(deprecated)]
fn downscaled_icon_png(image: &NSImage) -> Option<Vec<u8>> {
    const SIDE: f64 = 128.0;
    let size = NSSize::new(SIDE, SIDE);
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), size);
    let resized = NSImage::initWithSize(NSImage::alloc(), size);
    resized.lockFocus();
    image.drawInRect_fromRect_operation_fraction(
        rect,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        NSCompositingOperation::SourceOver,
        1.0,
    );
    resized.unlockFocus();
    let tiff = resized.TIFFRepresentation()?;
    let bitmap = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)?;
    let properties = NSDictionary::new();
    // SAFETY: an empty properties dictionary is valid for PNG representation.
    unsafe {
        bitmap
            .representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
            .map(|data| data.to_vec())
    }
}

fn platform_identity(app: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    match normalize_app(app).to_ascii_lowercase().as_str() {
        "zoom" => ("Zoom", "Z", "#2D8CFF", "#EAF3FF"),
        "teams" => ("Microsoft Teams", "T", "#5B5FC7", "#EEEEFF"),
        "meet" => ("Google Meet", "M", "#0F9D58", "#E7F7EE"),
        "webex" => ("Webex", "W", "#087E8B", "#E5F6F7"),
        "facetime" => ("FaceTime", "F", "#34C759", "#E8F8EC"),
        "slack" => ("Slack Huddle", "S", "#611F69", "#F5EAF6"),
        "discord" => ("Discord", "D", "#5865F2", "#ECEEFF"),
        "jitsi" => ("Jitsi Meet", "J", "#205081", "#E8F0F7"),
        "whereby" => ("Whereby", "W", "#FF5A5F", "#FFF0F0"),
        "chime" => ("Amazon Chime", "C", "#1F73B7", "#E9F3FA"),
        "ringcentral" => ("RingCentral", "R", "#066FAC", "#E8F4FA"),
        "goto" => ("GoTo Meeting", "G", "#6D57FF", "#F0EDFF"),
        "riverside" => ("Riverside", "R", "#7C3AED", "#F1EAFF"),
        "around" => ("Around", "A", "#FF6B35", "#FFF0EA"),
        "gather" => ("Gather", "G", "#6B4EFF", "#F0EDFF"),
        "butter" => ("Butter", "B", "#FFB000", "#FFF7DF"),
        "skype" => ("Skype", "S", "#00AFF0", "#E5F7FD"),
        "whatsapp" => ("WhatsApp", "W", "#25D366", "#E8F9EE"),
        "telegram" => ("Telegram", "T", "#229ED9", "#E8F6FC"),
        "signal" => ("Signal", "S", "#3A76F0", "#EAF0FE"),
        _ => ("Meeting", "M", "#4F64C8", "#EEF0FA"),
    }
}

fn app_thumbnail_svg(app: &str) -> String {
    let (label, mark, accent, background) = platform_identity(app);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="240" viewBox="0 0 240 240" role="img" aria-label="{label}"><rect width="240" height="240" rx="42" fill="{background}"/><rect x="58" y="58" width="124" height="124" rx="36" fill="{accent}"/><text x="120" y="142" text-anchor="middle" font-family="-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif" font-size="66" font-weight="700" fill="white">{mark}</text></svg>"#
    )
}

fn serve_recording(request: Request, cfg: &Config, encoded: &str) {
    let name = percent_decode(encoded);
    if !safe_basename(&name, Some("mp4")) {
        return respond(request, 404, "text/plain", Vec::new(), vec![]);
    }
    let Ok(mut file) = File::open(cfg.rec_dir.join(name)) else {
        return respond(request, 404, "text/plain", Vec::new(), vec![]);
    };
    let Ok(meta) = file.metadata() else {
        return respond(request, 404, "text/plain", Vec::new(), vec![]);
    };
    let len = meta.len();
    let range_header = header(&request, "Range");
    let range = match parse_byte_range(&range_header, len) {
        Ok(value) => value,
        Err(()) => {
            return respond(
                request,
                416,
                "text/plain",
                Vec::new(),
                vec![
                    h("Content-Range", &format!("bytes */{len}")),
                    h("Accept-Ranges", "bytes"),
                ],
            );
        }
    };

    let common = vec![
        h("Content-Type", "video/mp4"),
        h("Cache-Control", "private, no-store"),
        h("X-Content-Type-Options", "nosniff"),
        h("Referrer-Policy", "no-referrer"),
        h("Accept-Ranges", "bytes"),
    ];
    if let Some((start, end)) = range {
        if file.seek(SeekFrom::Start(start)).is_err() {
            return respond(request, 500, "text/plain", Vec::new(), vec![]);
        }
        let count = end - start + 1;
        let mut headers = common;
        headers.push(h("Content-Range", &format!("bytes {start}-{end}/{len}")));
        let response = Response::new(
            StatusCode(206),
            headers,
            file.take(count),
            Some(count as usize),
            None,
        );
        let _ = request.respond(response);
    } else {
        let response = Response::new(StatusCode(200), common, file, Some(len as usize), None);
        let _ = request.respond(response);
    }
}

fn meeting_ask(mut request: Request, shared: Arc<Shared>, id: &str) {
    let body = match read_json(&mut request) {
        Ok(v) => v,
        Err(e) => return json_response(request, 400, json!({"error":e})),
    };
    let Some(meeting) = read_meeting(
        &shared.cfg,
        id,
        shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .current
            .as_ref(),
    ) else {
        return json_response(request, 404, json!({"error":"not found"}));
    };
    let question = clean(
        body.get("question").and_then(Value::as_str).unwrap_or(""),
        2000,
    );
    let messages = json!([
        {"role":"system","content":"Answer questions about this meeting using ONLY its transcript. If it isn't covered, say so briefly."},
        {"role":"user","content":format!("Transcript:\n{}\n\nQuestion: {question}", transcript_text(&meeting,16000))}
    ]);
    match chat(&shared, messages, 500, 0.3) {
        Ok(answer) => json_response(request, 200, json!({"answer":answer})),
        Err(error) => json_response(request, 502, json!({"error":error})),
    }
}

fn generate_summary(shared: &Shared, id: &str, opts: Value) -> Result<Meeting, String> {
    let current = shared
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .current
        .clone();
    let mut meeting = read_meeting(&shared.cfg, id, current.as_ref()).ok_or("meeting not found")?;
    can_generate_notes(&meeting)?;
    let revision = meeting.transcript_revision;
    meeting.summary_state = "generating".into();
    meeting.summary_error.clear();
    save_meeting(&shared.cfg, &meeting)?;
    let deeper = opts
        .get("deeper")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            setting_json(
                &shared
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .settings,
                "deeper",
            )
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        });
    let template = opts
        .get("template")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            stored_template(
                &shared
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .settings,
            )
        });
    let instruction = if template.trim().is_empty() {
        "Summarise this meeting as 3–5 short bullet points, then a line \"Action items:\" with each as \"- <owner>: <task>\" (or \"None\").".to_string()
    } else {
        template
    };
    let mut system = "You summarise meeting transcripts for a busy professional. Be concise, specific, neutral — no preamble.".to_string();
    if deeper {
        system.push_str(" Reason carefully through the discussion — capture decisions, owners, and nuance you might otherwise miss.");
    }
    let notes = if meeting.notes.is_empty() {
        String::new()
    } else {
        format!(
            "The note-taker's own scratchpad from the call (weight these points):\n{}\n\n",
            meeting.notes
        )
    };
    let messages = json!([{"role":"system","content":system},{"role":"user","content":format!("{instruction}\n\n{notes}Transcript:\n\n{}",transcript_text(&meeting,16000))}]);
    let text = chat(
        shared,
        messages,
        if deeper { 1100 } else { 650 },
        if deeper { 0.2 } else { 0.3 },
    )?;
    let mut latest = read_meeting(&shared.cfg, id, None)
        .ok_or("meeting disappeared while notes were generating")?;
    if latest.transcript_revision != revision {
        latest.summary_state = "stale".into();
        latest.summary_stale = true;
        latest.summary_error =
            "The transcript changed while notes were generating. Regenerate notes.".into();
        save_meeting(&shared.cfg, &latest)?;
        return Err(latest.summary_error);
    }
    latest.summary = text;
    latest.summary_revision = revision;
    latest.summary_stale = false;
    latest.summary_state = "ready".into();
    latest.summary_error.clear();
    save_meeting(&shared.cfg, &latest)?;
    broadcast(
        shared,
        &json!({"type":"meeting-updated","id":id,"summaryState":"ready"}),
    );
    Ok(latest)
}

#[derive(Clone)]
struct LlmConfig {
    base: String,
    key: String,
    model: String,
    local: bool,
}

fn stored_llm(settings: &HashMap<String, String>) -> Result<LlmConfig, String> {
    let value = setting_json(settings, "aiModel").unwrap_or_else(|| json!({"provider":"local"}));
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("local");
    let model_override = value.get("model").and_then(Value::as_str).unwrap_or("");
    let custom_base = value.get("base").and_then(Value::as_str).unwrap_or("");
    let (base, default_model, local, needs_key) = match provider {
        "groq" => (
            "https://api.groq.com/openai/v1",
            "llama-3.3-70b-versatile",
            false,
            true,
        ),
        "openai" => ("https://api.openai.com/v1", "gpt-4o-mini", false, true),
        "cerebras" => ("https://api.cerebras.ai/v1", "llama-3.3-70b", false, true),
        "custom" => (custom_base, "", false, true),
        _ => ("http://127.0.0.1:9379/v1", "gemma4-e2b", true, false),
    };
    if base.is_empty() {
        return Err(
            "AI provider endpoint is missing. Choose a provider in Settings → Meetings.".into(),
        );
    }
    if !(base.starts_with("https://")
        || base.starts_with("http://127.0.0.1:")
        || base.starts_with("http://localhost:"))
    {
        return Err("Custom providers must use HTTPS or a loopback address".into());
    }
    let key = if needs_key {
        keychain_key(provider).ok_or("API key is not saved in Keychain")?
    } else {
        String::new()
    };
    Ok(LlmConfig {
        base: base.trim_end_matches('/').into(),
        key,
        model: if model_override.is_empty() {
            default_model.into()
        } else {
            clean(model_override, 120)
        },
        local,
    })
}

fn chat(
    shared: &Shared,
    messages: Value,
    max_tokens: u64,
    temperature: f64,
) -> Result<String, String> {
    let llm = {
        let state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        stored_llm(&state.settings)?
    };
    if llm.local {
        ensure_local_llm(shared)?;
    }
    let body = json!({"model":llm.model,"messages":messages,"temperature":temperature,"max_tokens":max_tokens,"stream":false});
    let agent = LLM_HTTP.get_or_init(|| {
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .tls_config(tls)
            .build()
            .new_agent()
    });
    let url = format!("{}/chat/completions", llm.base);
    let mut request = agent.post(&url).header("Content-Type", "application/json");
    if !llm.key.is_empty() {
        if llm.key.contains(['\r', '\n']) {
            return Err("invalid API key characters".into());
        }
        request = request.header("Authorization", &format!("Bearer {}", llm.key));
    }
    let result = request.send_json(&body);
    if llm.local {
        shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .llm_last_used = Some(Instant::now());
    }
    let mut response = result.map_err(|e| format!("AI provider request failed: {e}"))?;
    let status = response.status();
    let response_body = response
        .body_mut()
        .read_to_string()
        .map_err(|_| "AI provider response could not be read".to_string())?;
    if !status.is_success() {
        return Err(clean(&response_body, 240));
    }
    let value: Value = serde_json::from_str(&response_body)
        .map_err(|_| "AI provider returned invalid JSON".to_string())?;
    Ok(value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string())
}

fn ensure_local_llm(shared: &Shared) -> Result<(), String> {
    let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(child) = state.llm_child.as_mut() {
        if child.try_wait().map_err(|e| e.to_string())?.is_none() {
            state.llm_last_used = Some(Instant::now());
            return Ok(());
        }
        state.llm_child = None;
    }
    if port_open(LLM_PORT) {
        return Err(
            "local AI port is occupied by an untrusted process; relaunch after closing it".into(),
        );
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let bin = [
        PathBuf::from(format!("{home}/.local/bin/litert-lm")),
        PathBuf::from("/opt/homebrew/bin/litert-lm"),
        PathBuf::from("/usr/local/bin/litert-lm"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .ok_or("on-device summary model is not installed — choose a cloud provider in Settings")?;
    let child = Command::new(bin)
        .args([
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            &LLM_PORT.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    state.llm_child = Some(child);
    drop(state);
    for _ in 0..900 {
        if port_open(LLM_PORT) {
            shared
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .llm_last_used = Some(Instant::now());
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("on-device summary model did not start".into())
}

fn stored_template(settings: &HashMap<String, String>) -> String {
    let selected = setting_json(settings, "template_sel")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "standard".into());
    setting_json(settings, "templates")
        .and_then(|v| v.as_array().cloned())
        .and_then(|items| {
            items
                .into_iter()
                .find(|v| v.get("id").and_then(Value::as_str) == Some(&selected))
        })
        .and_then(|v| v.get("prompt").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

fn save_meeting(cfg: &Config, meeting: &Meeting) -> Result<(), String> {
    write_private(
        &cfg.meetings_dir.join(format!("{}.json", meeting.id)),
        &serde_json::to_vec(meeting).map_err(|e| e.to_string())?,
    )
}
fn read_meeting_path(path: &Path) -> Option<Meeting> {
    let mut m: Meeting = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    ensure_line_ids(&mut m);
    Some(m)
}
fn read_meeting(cfg: &Config, id: &str, current: Option<&Meeting>) -> Option<Meeting> {
    if let Some(m) = current.filter(|m| m.id == id) {
        return Some(m.clone());
    }
    if !valid_meeting_id(id) {
        return None;
    }
    read_meeting_path(&cfg.meetings_dir.join(format!("{id}.json")))
}
fn ensure_line_ids(m: &mut Meeting) {
    for (i, line) in m.lines.iter_mut().enumerate() {
        if line.id.is_empty() {
            line.id = format!(
                "line-{}-{i}",
                if line.ts > 0 { line.ts } else { m.started_at }
            );
        }
        if line.speaker.is_empty() {
            line.speaker = unknown_speaker();
        }
        if line.speaker_source.is_empty() {
            line.speaker_source = if line.is_self {
                "user".into()
            } else {
                "unknown".into()
            };
        }
    }
}
fn finalize_orphans(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for path in entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
    {
        if let Some(mut m) = read_meeting_path(&path) {
            if m.ended_at.is_none() {
                m.ended_at = Some(
                    m.lines
                        .last()
                        .map(|l| l.ts)
                        .filter(|n| *n > 0)
                        .unwrap_or(m.started_at.max(now_ms())),
                );
                let _ = write_private(&path, &serde_json::to_vec(&m).unwrap_or_default());
            }
        }
    }
}

fn meeting_summary(cfg: &Config, m: &Meeting) -> Value {
    let lines: Vec<&Line> = m.lines.iter().filter(|l| !l.excluded).collect();
    json!({"id":m.id,"title":resolved_title(m),"app":m.app,"startedAt":m.started_at,"endedAt":m.ended_at,"participants":m.participants,"lineCount":lines.len(),"snippet":lines.iter().find(|l|l.text.len()>1).map(|l|clean(&l.text,140)).unwrap_or_default(),"summarized":!m.summary.is_empty(),"hasVideo":safe_basename(&m.video,Some("mp4"))&&cfg.rec_dir.join(&m.video).is_file(),"summaryState":if m.summary_state.is_empty(){if m.summary.is_empty(){"idle"}else{"ready"}}else{&m.summary_state},"summaryError":m.summary_error,"summaryStale":m.summary_stale,"live":m.ended_at.is_none()})
}
fn clean(s: &str, max: usize) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}
fn first_state_title(state: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| state.get(*key).and_then(Value::as_str))
        .map(|value| clean(value, 120))
        .find(|value| valid_title(value))
}
fn normalize_app(value: &str) -> String {
    let raw = clean(value, 40);
    let key = raw.to_ascii_lowercase();
    if key.contains("zoom") {
        "zoom".into()
    } else if key.contains("team") {
        "teams".into()
    } else if key.contains("google") && key.contains("meet") || key == "meet" {
        "meet".into()
    } else if key.contains("webex") {
        "webex".into()
    } else if key.contains("facetime") {
        "facetime".into()
    } else if key.contains("slack") || key.contains("huddle") {
        "slack".into()
    } else if key.contains("discord") {
        "discord".into()
    } else if key.contains("jitsi") {
        "jitsi".into()
    } else if key.contains("whereby") {
        "whereby".into()
    } else if key.contains("chime") {
        "chime".into()
    } else if key.contains("ringcentral") {
        "ringcentral".into()
    } else if key.contains("goto") {
        "goto".into()
    } else if key.contains("riverside") {
        "riverside".into()
    } else if key.contains("around") {
        "around".into()
    } else if key.contains("gather") {
        "gather".into()
    } else if key.contains("butter") {
        "butter".into()
    } else if key.contains("skype") {
        "skype".into()
    } else if key.contains("whatsapp") {
        "whatsapp".into()
    } else if key.contains("telegram") {
        "telegram".into()
    } else if key.contains("signal") {
        "signal".into()
    } else if key == "call" || key.is_empty() {
        key
    } else {
        raw
    }
}
fn substantive(line: &Line) -> bool {
    if line.excluded {
        return false;
    }
    let t = clean(&line.text, 4000);
    t.chars().count() >= 8
        && t.chars().any(|c| c.is_alphabetic())
        && !matches!(
            t.to_ascii_lowercase().as_str(),
            "[music]" | "[silence]" | "[inaudible]"
        )
}
fn transcript_text(m: &Meeting, cap: usize) -> String {
    m.lines
        .iter()
        .filter(|l| !l.excluded && !clean(&l.text, 4000).is_empty())
        .map(|l| format!("{}: {}", clean(&l.speaker, 120), clean(&l.text, 4000)))
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(cap)
        .collect()
}
fn can_generate_notes(m: &Meeting) -> Result<(), String> {
    let lines: Vec<&Line> = m.lines.iter().filter(|l| substantive(l)).collect();
    if lines.len() < 3 {
        return Err("Not enough substantive transcript lines.".into());
    }
    let words: usize = lines
        .iter()
        .map(|l| clean(&l.text, 4000).split_whitespace().count())
        .sum();
    if words < 30 {
        return Err("The transcript is too short to generate useful notes.".into());
    }
    if let Some(end) = m.ended_at {
        if end.saturating_sub(m.started_at) < 30_000 {
            return Err("The meeting was too short to generate useful notes.".into());
        }
    }
    Ok(())
}
fn valid_title(s: &str) -> bool {
    let t = clean(s, 120).to_ascii_lowercase();
    !t.is_empty()
        && t.len() >= 3
        && !matches!(
            t.as_str(),
            "untitled meeting"
                | "impromptu meeting"
                | "meeting"
                | "zoom meeting"
                | "teams meeting"
                | "webex meeting"
                | "new meeting"
                | "personal meeting room"
        )
        && !t.contains("zoom floating video window")
        && !t.contains("audio settings")
        && !t.contains("meeting controls")
}
fn resolved_title(m: &Meeting) -> String {
    if m.title_source == "user" && !clean(&m.title, 120).is_empty() {
        return clean(&m.title, 120);
    }
    if valid_title(&m.calendar_title) {
        return clean(&m.calendar_title, 120);
    }
    // Preserve valid titles already stored by earlier releases, and titles detected from the
    // meeting window. Do not invent a title from somebody's first spoken sentence: a scheduled
    // calendar title is truth; an unscheduled call is explicitly an impromptu meeting.
    if valid_title(&m.title) {
        return clean(&m.title, 120);
    }
    untitled()
}

fn apply_correction(m: &mut Meeting, c: &Value) -> Result<(), String> {
    ensure_line_ids(m);
    let op = clean(c.get("operation").and_then(Value::as_str).unwrap_or(""), 60);
    let name = |k: &str| -> Result<String, String> {
        let v = clean(c.get(k).and_then(Value::as_str).unwrap_or(""), 120);
        if v.is_empty() {
            Err(format!("{k} is required"))
        } else {
            Ok(v)
        }
    };
    match op.as_str() {
        "rename_participant" => {
            let from = name("from")?;
            let to = name("to")?;
            for p in &mut m.participants {
                if *p == from {
                    *p = to.clone();
                }
            }
            if !m.participants.contains(&to) {
                m.participants.push(to.clone());
            }
            for l in &mut m.lines {
                if l.speaker == from {
                    l.speaker = to.clone();
                    l.speaker_source = "user_correction".into();
                }
            }
        }
        "remove_participant" => {
            let n = name("name")?;
            m.participants.retain(|p| p != &n);
            for l in &mut m.lines {
                if l.speaker == n {
                    l.speaker = unknown_speaker();
                    l.speaker_source = "user_correction".into();
                }
            }
        }
        "merge_speakers" => {
            let from = name("from")?;
            let into = name("into")?;
            if from == into {
                return Err("speaker identities must be different".into());
            }
            m.participants.retain(|p| p != &from);
            if !m.participants.contains(&into) {
                m.participants.push(into.clone());
            }
            for l in &mut m.lines {
                if l.speaker == from {
                    l.speaker = into.clone();
                    l.speaker_source = "user_correction".into();
                }
            }
        }
        "edit_transcript" | "reassign_line" | "exclude_line" => {
            let idx = if let Some(id) = c.get("lineId").and_then(Value::as_str) {
                m.lines.iter().position(|l| l.id == id)
            } else {
                c.get("lineIndex")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize)
            }
            .filter(|i| *i < m.lines.len())
            .ok_or("transcript line not found")?;
            let line = &mut m.lines[idx];
            if op == "edit_transcript" {
                let text = clean(c.get("text").and_then(Value::as_str).unwrap_or(""), 4000);
                if text.is_empty() {
                    return Err("text is required".into());
                }
                line.text = text;
            } else if op == "reassign_line" {
                let speaker = name("speaker")?;
                line.speaker = speaker.clone();
                line.is_self = speaker == "You";
                line.speaker_source = "user_correction".into();
                if !line.is_self
                    && speaker != "Unknown speaker"
                    && !m.participants.contains(&speaker)
                {
                    m.participants.push(speaker);
                }
            } else {
                line.excluded = c.get("excluded").and_then(Value::as_bool).unwrap_or(true);
            }
        }
        _ => return Err("unknown correction operation".into()),
    }
    m.transcript_revision += 1;
    m.last_correction_at = now_ms();
    m.last_correction_reason = op;
    if !m.summary.is_empty() {
        m.summary_stale = true;
        m.summary_state = "stale".into();
    }
    Ok(())
}

fn valid_meeting_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}
fn safe_basename(name: &str, extension: Option<&str>) -> bool {
    if name.is_empty()
        || name.len() > 240
        || Path::new(name).file_name().and_then(|s| s.to_str()) != Some(name)
    {
        return false;
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return false;
    }
    extension
        .map(|e| Path::new(name).extension().and_then(|s| s.to_str()) == Some(e))
        .unwrap_or(true)
}
fn percent_decode(s: &str) -> String {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}
fn parse_byte_range(value: &str, len: u64) -> Result<Option<(u64, u64)>, ()> {
    if value.is_empty() {
        return Ok(None);
    }
    let raw = value.strip_prefix("bytes=").ok_or(())?;
    if raw.contains(',') || len == 0 {
        return Err(());
    }
    let (start_s, end_s) = raw.split_once('-').ok_or(())?;
    if start_s.is_empty() {
        let suffix = end_s.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let count = suffix.min(len);
        return Ok(Some((len - count, len - 1)));
    }
    let start = start_s.parse::<u64>().map_err(|_| ())?;
    if start >= len {
        return Err(());
    }
    let end = if end_s.is_empty() {
        len - 1
    } else {
        end_s.parse::<u64>().map_err(|_| ())?.min(len - 1)
    };
    if end < start {
        return Err(());
    }
    Ok(Some((start, end)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gate_matches_product_contract() {
        let mut g = Gate::new();
        assert!(!g.recording());
        g.set_detected(true);
        assert!(g.recording());
        g.set_manual("pause");
        assert!(g.paused());
        g.set_detected(false);
        assert!(g.manual.is_none());
    }
    #[test]
    fn meeting_notification_requires_a_real_rising_edge() {
        assert!(meeting_start_edge(true, false, Some(true)));
        assert!(!meeting_start_edge(false, false, Some(true)));
        assert!(!meeting_start_edge(true, true, Some(true)));
        assert!(!meeting_start_edge(true, true, Some(false)));
    }
    #[test]
    fn meeting_titles_preserve_calendar_truth_and_mark_unscheduled_calls() {
        let mut state = Map::new();
        state.insert("title".into(), Value::String("Zoom Meeting".into()));
        state.insert(
            "calendarTitle".into(),
            Value::String("Enterprise launch review".into()),
        );
        let scheduled = new_meeting(&state);
        assert_eq!(scheduled.title, "Enterprise launch review");
        assert_eq!(scheduled.title_source, "calendar");
        assert_eq!(resolved_title(&scheduled), "Enterprise launch review");

        let impromptu = new_meeting(&Map::new());
        assert_eq!(impromptu.title, "Impromptu meeting");
        assert_eq!(impromptu.title_source, "fallback");
        assert_eq!(resolved_title(&impromptu), "Impromptu meeting");
    }
    #[test]
    fn app_thumbnail_is_always_available_without_a_recording() {
        let svg = app_thumbnail_svg("discord");
        assert!(svg.contains("Discord"));
        assert!(svg.contains("#5865F2"));
        assert!(svg.starts_with("<svg"));
        let facetime = installed_app_icon_png("facetime").expect("FaceTime icon is available");
        assert_eq!(&facetime[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes(facetime[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(facetime[20..24].try_into().unwrap());
        assert!(width <= 512 && height <= 512, "icon is {width}x{height}");
        assert!(
            facetime.len() <= 1_000_000,
            "icon is {} bytes",
            facetime.len()
        );
    }
    #[test]
    fn recording_names_cannot_escape_directory() {
        assert!(safe_basename("meeting-1.mp4", Some("mp4")));
        assert!(!safe_basename("../../tmp/x.mp4", Some("mp4")));
        assert!(!safe_basename("x.mov", Some("mp4")));
    }
    #[test]
    fn ids_are_timestamp_only() {
        assert!(valid_meeting_id("1700000000000"));
        assert!(!valid_meeting_id("../settings"));
    }
    #[test]
    fn multipart_fields_are_appended() {
        let ct = "multipart/form-data; boundary=x";
        let body =
            b"--x\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\na\r\n--x--\r\n".to_vec();
        let out = inject_multipart_fields(body, ct, &[("language", "en".into())]);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("name=\"language\""));
        assert!(text.ends_with("--x--\r\n"));
    }
    #[test]
    fn recording_ranges_are_bounded() {
        assert_eq!(parse_byte_range("", 100), Ok(None));
        assert_eq!(parse_byte_range("bytes=10-19", 100), Ok(Some((10, 19))));
        assert_eq!(parse_byte_range("bytes=90-", 100), Ok(Some((90, 99))));
        assert_eq!(parse_byte_range("bytes=-10", 100), Ok(Some((90, 99))));
        assert!(parse_byte_range("bytes=100-", 100).is_err());
        assert!(parse_byte_range("bytes=1-2,4-5", 100).is_err());
    }
    #[test]
    fn legacy_model_is_adopted_privately() {
        let root = std::env::temp_dir().join(format!(
            "my-assistant-migrate-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let old = root.join("old");
        let new = root.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("model.gguf"), b"model").unwrap();
        assert_eq!(
            migrate_legacy_file(&new, &[old.to_string_lossy().into_owned()], "model.gguf", 5),
            Ok(true)
        );
        assert_eq!(fs::read(new.join("model.gguf")).unwrap(), b"model");
        assert_eq!(
            fs::metadata(new.join("model.gguf"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn interrupted_meeting_checkpoint_is_private_and_recoverable() {
        let root = std::env::temp_dir().join(format!(
            "my-assistant-meeting-{}-{}",
            std::process::id(),
            now_ms()
        ));
        private_dir(&root).unwrap();
        let mut m = new_meeting(&Map::new());
        m.lines.push(Line {
            id: "line-test".into(),
            speaker: "You".into(),
            speaker_source: "user".into(),
            speaker_context: String::new(),
            text: "A recoverable transcript line".into(),
            ts: now_ms(),
            is_self: true,
            excluded: false,
        });
        let cfg = Config {
            token: String::new(),
            web_dir: root.clone(),
            release: json!({"capabilities":{},"routes":{}}),
            data_dir: root.clone(),
            meetings_dir: root.clone(),
            rec_dir: root.clone(),
            models_dir: root.clone(),
            transcribe_bin: root.join("none"),
            meeting_notifier: Arc::new(|_| {}),
        };
        save_meeting(&cfg, &m).unwrap();
        let path = root.join(format!("{}.json", m.id));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        finalize_orphans(&root);
        let recovered = read_meeting_path(&path).unwrap();
        assert!(recovered.ended_at.is_some());
        assert_eq!(recovered.lines[0].text, "A recoverable transcript line");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn release_contract_requires_all_core_routes() {
        assert!(validate_release_contract(&json!({
            "capabilities": {"dictation":true,"meetings":true,"settings":true},
            "routes": {"dictation":"dictation","meetings":"meetings","settings":"settings"}
        }))
        .is_ok());
        assert!(validate_release_contract(&json!({
            "capabilities": {},
            "routes": {}
        }))
        .is_err());
    }

    #[test]
    fn authenticated_ui_redirect_uses_a_non_secret_fresh_url() {
        let first = fresh_ui_location();
        std::thread::sleep(Duration::from_millis(1));
        let second = fresh_ui_location();
        assert!(first.starts_with("/?run="));
        assert!(!first.contains("bootstrap"));
        assert_ne!(first, second);
    }
}
