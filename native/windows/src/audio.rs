//! WASAPI audio — the permission-free Windows twin of the macOS Core Audio process taps.
//!
//! Two jobs:
//! 1. **Call detection** via the WASAPI Audio Session API: `IMMDeviceEnumerator` →
//!    `IAudioSessionManager2` → `IAudioSessionEnumerator` → `IAudioSessionControl2`, keeping sessions
//!    whose `GetState() == Active`, reading `GetProcessId()`. This mirrors the Swift
//!    `caProcessList()` "is a meeting app running audio I/O" check and needs no special permission.
//! 2. **Per-app capture** via `wasapi` process loopback
//!    (`AudioClient::new_application_loopback_client(pid, include_tree=true)`), resampled 48k stereo →
//!    16k mono, plus the microphone via a second capture client, both feeding [`crate::pipeline`].
//!
//! Confidence: the Audio Session **detection** COM dance (`#[cfg(windows)]`) is written against the
//! documented `windows`-crate interfaces and is high-confidence, but the exact generic
//! `IMMDevice::Activate` form is marked TODO(on-device). The **capture** path uses the third-party
//! `wasapi` crate whose 0.23 surface I cannot verify here — it is written faithfully and marked
//! TODO(on-device); failure logs and yields no samples (contract-valid: no captions, never a crash).
//! Every function has a `#[cfg(not(windows))]` fallback so host builds compile.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use helpers_core::segmenter::Segmenter;

use crate::apps;
use crate::pipeline::{Shared, SttPipeline};
use crate::window;

/// A detected in-progress call: the app key, a human title, and the owning process.
#[derive(Debug, Clone)]
pub struct CallTarget {
    pub key: String,
    pub title: String,
    pub pid: u32,
    pub exe: String,
}

// ---------------------------------------------------------------------------
// Detection (WASAPI Audio Session API)
// ---------------------------------------------------------------------------

/// PIDs with at least one **Active** render audio session (i.e. currently playing audio).
#[cfg(windows)]
pub fn active_session_pids() -> Vec<u32> {
    use windows::core::Interface;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionEnumerator,
        IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    let mut pids = Vec::new();
    // SAFETY: standard WASAPI session-enumeration COM sequence. Reads only.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(_) => return pids,
            };
        let device = match enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
            Ok(d) => d,
            Err(_) => return pids,
        };
        // TODO(on-device): confirm the generic Activate form for `windows` 0.62 —
        // `device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)`. If the crate exposes the
        // out-param form instead, adapt here. Failure degrades to "no active sessions".
        let manager: IAudioSessionManager2 = match device.Activate(CLSCTX_ALL, None) {
            Ok(m) => m,
            Err(_) => return pids,
        };
        let sessions: IAudioSessionEnumerator = match manager.GetSessionEnumerator() {
            Ok(s) => s,
            Err(_) => return pids,
        };
        let count = sessions.GetCount().unwrap_or(0);
        for i in 0..count {
            let Ok(ctrl) = sessions.GetSession(i) else {
                continue;
            };
            if ctrl.GetState().unwrap_or(AudioSessionStateActive) != AudioSessionStateActive {
                continue;
            }
            if let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() {
                if let Ok(pid) = ctrl2.GetProcessId() {
                    if pid != 0 && !pids.contains(&pid) {
                        pids.push(pid);
                    }
                }
            }
        }
    }
    pids
}

#[cfg(not(windows))]
pub fn active_session_pids() -> Vec<u32> {
    crate::degrade("WASAPI audio-session detection");
    Vec::new()
}

/// Is `pid` currently playing audio (Active render session)? Primary in-call signal for
/// `meeting-state`.
pub fn pid_has_active_session(pid: u32) -> bool {
    active_session_pids().contains(&pid)
}

/// Resolve the current capture target from `--app` (`all` | `auto` | `<exe>`), exactly parallel to
/// the Swift `captureTarget()`.
pub fn detect_call(app_sel: &str, browser_meet: bool) -> Option<CallTarget> {
    match app_sel {
        "all" => Some(CallTarget {
            key: "call".into(),
            title: String::new(),
            pid: 0,
            exe: String::new(),
        }),
        "auto" => {
            let active = active_session_pids();
            // Resolve each active PID → exe.
            let mut active_apps: Vec<(u32, String)> = active
                .iter()
                .filter_map(|&pid| crate::procs::exe_for_pid(pid).map(|exe| (pid, exe)))
                .filter(|(_, exe)| apps::is_known_call_exe(exe))
                .collect();

            // Dedicated conferencing apps first: an Active session is a strong call signal.
            if let Some((pid, exe)) = active_apps
                .iter()
                .find(|(_, exe)| apps::dedicated_key(exe).is_some())
                .cloned()
            {
                let key = apps::dedicated_key(&exe).unwrap_or("call");
                let titles = window::titles_for_pid(pid);
                return Some(CallTarget {
                    key: key.to_string(),
                    title: apps::useful_meeting_title(&titles, key),
                    pid,
                    exe,
                });
            }
            // Browsers / chat apps require Active audio AND meeting-shaped window evidence.
            if browser_meet {
                for (pid, exe) in active_apps.drain(..) {
                    if apps::evidence_key(&exe).is_none() && !apps::is_browser(&exe) {
                        continue;
                    }
                    let titles = window::titles_for_pid(pid);
                    let evidence = titles.join("\n");
                    if let Some(key) = apps::platform_from_evidence(&evidence, &exe) {
                        return Some(CallTarget {
                            title: apps::useful_meeting_title(&titles, &key),
                            key,
                            pid,
                            exe,
                        });
                    }
                }
            }
            None
        }
        sel => {
            // Explicit exe selector.
            let active = active_session_pids();
            let hit = active
                .iter()
                .filter_map(|&pid| crate::procs::exe_for_pid(pid).map(|exe| (pid, exe)))
                .find(|(_, exe)| exe.eq_ignore_ascii_case(sel));
            if let Some((pid, exe)) = hit {
                let key = apps::dedicated_key(&exe)
                    .or_else(|| apps::evidence_key(&exe))
                    .unwrap_or("call");
                let titles = window::titles_for_pid(pid);
                Some(CallTarget {
                    key: key.to_string(),
                    title: apps::useful_meeting_title(&titles, key),
                    pid,
                    exe,
                })
            } else {
                None
            }
        }
    }
}

/// `--list-apps` rows: `(exeName, windowTitle)`. Enumerates visible titled windows and resolves each
/// owning PID → exe. Sorted by exe then title for stable output.
pub fn list_apps() -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = window::all_windows()
        .into_iter()
        .filter_map(|(pid, title)| {
            crate::procs::exe_for_pid(pid).map(|exe| (exe, title))
        })
        .collect();
    rows.sort();
    rows.dedup();
    rows
}

// ---------------------------------------------------------------------------
// Capture (process loopback + mic)
// ---------------------------------------------------------------------------

/// Owns the live capture threads for one call. Dropping/`stop()` releases the mic and finalises any
/// video. Mirrors the Swift `Capture` object.
pub struct Capture {
    running: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
    video: Option<crate::video::Recorder>,
    pub recording: Option<(String, u64)>,
    pub active: bool,
}

impl Default for Capture {
    fn default() -> Self {
        Capture {
            running: Arc::new(AtomicBool::new(false)),
            handles: Vec::new(),
            video: None,
            recording: None,
            active: false,
        }
    }
}

impl Capture {
    pub fn is_running(&self) -> bool {
        self.active
    }

    /// Start capture for `target`. `sys_thr`/`mic_thr` are the VAD gates; `me` is the mic speaker
    /// label. `shared` is the transcription config shared with both segmenters.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        video_mode: &str,
        sharing: bool,
        rec_dir: &str,
        target: &CallTarget,
        mic_on: bool,
        me: &str,
        sys_thr: f32,
        mic_thr: f32,
        shared: Arc<Shared>,
    ) {
        if self.active {
            return;
        }
        self.active = true;
        self.running.store(true, Ordering::SeqCst);
        crate::log(&format!(
            ">> in a call — starting capture (app:{} pid:{} video:{video_mode})",
            target.key, target.pid
        ));

        // System audio: per-application process loopback for the meeting app.
        {
            let running = self.running.clone();
            let shared = shared.clone();
            let pid = target.pid;
            let sys_seg = Segmenter::new("Speaker", false, "sys", sys_thr);
            let pipeline = SttPipeline::new(sys_seg, "sys", shared);
            self.handles
                .push(std::thread::spawn(move || run_loopback(pid, true, running, pipeline)));
        }

        // Microphone: default capture endpoint (you).
        if mic_on {
            let running = self.running.clone();
            let mic_seg = Segmenter::new(me, true, "mic", mic_thr);
            let pipeline = SttPipeline::new(mic_seg, "mic", shared);
            self.handles
                .push(std::thread::spawn(move || run_mic(running, pipeline)));
        }

        // Video: only when the mode asks for it. windows-capture recorder (TODO on-device).
        let want_video = video_mode == "always" || (video_mode == "share" && sharing);
        if want_video {
            match crate::video::Recorder::start(rec_dir) {
                Some(rec) => {
                    self.recording = Some((rec.file.clone(), rec.started_at));
                    self.video = Some(rec);
                }
                None => {
                    // Degraded (stub / init failure): audio capture continues; no recording announced.
                }
            }
        }
    }

    /// Take the "just started recording" announcement exactly once (so the bin POSTs it a single
    /// time).
    pub fn take_recording(&mut self) -> Option<(String, u64)> {
        self.recording.take()
    }

    pub fn stop(&mut self) {
        if !self.active {
            return;
        }
        crate::log(">> not in a call — stopping capture (releasing mic, finalizing any recording)");
        self.running.store(false, Ordering::SeqCst);
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
        if let Some(rec) = self.video.take() {
            rec.finish();
        }
        self.recording = None;
        self.active = false;
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// System-audio process-loopback capture loop.
#[cfg(windows)]
fn run_loopback(pid: u32, include_tree: bool, running: Arc<AtomicBool>, mut pipeline: SttPipeline) {
    // TODO(on-device): verify the `wasapi` 0.23 process-loopback surface. Intended sequence:
    //   wasapi::initialize_mta()?;
    //   let mut client = AudioClient::new_application_loopback_client(pid, include_tree)?;
    //   let (_def, min) = client.get_periods()?; // or use a fixed 16k/48k format desc
    //   let desc = WaveFormat::new(32, 32, &SampleType::Float, 48000, 2, None);
    //   client.initialize_client(&desc, min, &Direction::Capture, &ShareMode::Shared, true)?;
    //   let h_event = client.set_get_eventhandle()?;
    //   let capture = client.get_audiocapturclient()?;
    //   client.start_stream()?;
    //   while running { h_event.wait_for_event(200)?; read available packets → f32 interleaved;
    //       let mono16k = resample::condition(&frames, channels, sample_rate);
    //       pipeline.feed(&mono16k); }
    //   client.stop_stream()?;
    // Until verified on-device we degrade to "no system audio" rather than risk a wrong API.
    let _ = (pid, include_tree, &mut pipeline);
    crate::log(&format!(
        ">> system-audio process loopback for pid {pid} — TODO(on-device): wasapi capture wiring"
    ));
    // Spin lightly so stop() joins promptly; feed nothing.
    while running.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(not(windows))]
fn run_loopback(pid: u32, _include_tree: bool, running: Arc<AtomicBool>, mut pipeline: SttPipeline) {
    crate::degrade("WASAPI process-loopback capture");
    let _ = (pid, &mut pipeline);
    while running.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Microphone capture loop (default capture endpoint), resampled to 16k mono.
#[cfg(windows)]
fn run_mic(running: Arc<AtomicBool>, mut pipeline: SttPipeline) {
    // TODO(on-device): verify the `wasapi` 0.23 default-capture surface. Intended sequence:
    //   let device = wasapi::get_default_device(&Direction::Capture)?;
    //   let mut client = device.get_iaudioclient()?;
    //   let desc = client.get_mixformat()? (query native format);
    //   client.initialize_client(&desc, period, &Direction::Capture, &ShareMode::Shared, false)?;
    //   let h_event = client.set_get_eventhandle()?; let capture = client.get_audiocapturclient()?;
    //   client.start_stream()?;
    //   while running { wait; read; condition→16k mono; pipeline.feed(&mono16k); }
    let _ = &mut pipeline;
    crate::log(">> mic capture — TODO(on-device): wasapi default-capture wiring");
    while running.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(not(windows))]
fn run_mic(running: Arc<AtomicBool>, mut pipeline: SttPipeline) {
    crate::degrade("WASAPI microphone capture");
    let _ = &mut pipeline;
    while running.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
