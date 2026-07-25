//! Process enumeration — `CreateToolhelp32Snapshot` (the Windows analogue of macOS
//! `NSWorkspace.runningApplications`, per PROTOCOL). Used by `meeting-state` to find running meeting
//! apps and by `meeting-tap`/`audio` to resolve a session PID → exe name.
//!
//! REAL Windows sequence below (`#[cfg(windows)]`); a `#[cfg(not(windows))]` fallback returns empty
//! so host builds stay green. The Toolhelp calls are standard and well-documented, so confidence is
//! high — the only on-device caveat is exe-name casing, which we normalise to a lowercase basename.

/// `(pid, exeBasename)` for every process, exe name lowercased (e.g. `"zoom.exe"`).
#[cfg(windows)]
pub fn all_processes() -> Vec<(u32, String)> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut out = Vec::new();
    // SAFETY: standard snapshot walk; handle closed before return.
    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return out,
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let exe = String::from_utf16_lossy(&entry.szExeFile);
                let exe = exe.trim_end_matches('\0').to_lowercase();
                out.push((entry.th32ProcessID, exe));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    out
}

#[cfg(not(windows))]
pub fn all_processes() -> Vec<(u32, String)> {
    crate::degrade("process enumeration (Toolhelp32)");
    Vec::new()
}

/// Resolve one PID → lowercase exe basename.
pub fn exe_for_pid(pid: u32) -> Option<String> {
    all_processes()
        .into_iter()
        .find(|(p, _)| *p == pid)
        .map(|(_, exe)| exe)
}

/// All PIDs whose exe matches `pred`.
pub fn pids_where<F: Fn(&str) -> bool>(pred: F) -> Vec<(u32, String)> {
    all_processes()
        .into_iter()
        .filter(|(_, exe)| pred(exe))
        .collect()
}
