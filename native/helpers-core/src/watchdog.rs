//! Parent-process liveness — the cross-platform port of the Swift `parentIsAlive()` /
//! `kill(PARENT_PID, 0)` contract. All five helpers must exit promptly when the app that spawned
//! them dies, so a crashed parent never leaves an orphaned tap holding the microphone.

/// Reads `ASSISTANT_PARENT_PID` once. `None` means "no parent to watch" (run untethered).
pub fn parent_pid_from_env() -> Option<u32> {
    std::env::var("ASSISTANT_PARENT_PID")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

/// True while the given pid is still alive.
///
/// - Unix: `kill(pid, 0)` — `0` or `EPERM` means alive (EPERM = exists but not ours to signal),
///   `ESRCH` means gone. Identical to the Swift check.
/// - Windows: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` then `WaitForSingleObject(h, 0)` —
///   `WAIT_TIMEOUT` means still running, `WAIT_OBJECT_0` means exited.
#[cfg(unix)]
pub fn is_alive(pid: u32) -> bool {
    // SAFETY: kill with signal 0 performs only an existence/permission check.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    // errno == EPERM → the process exists but we lack permission to signal it → alive.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
pub fn is_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: standard OpenProcess/Wait/CloseHandle sequence; handle checked before use.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // Distinguish "gone" from "exists but not openable", mirroring the Unix EPERM=alive rule:
            // ACCESS_DENIED means the process exists (protected/elevated) → treat as ALIVE so we don't
            // prematurely kill the helper. Any other error (e.g. ERROR_INVALID_PARAMETER) → gone.
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let wait = WaitForSingleObject(handle, 0);
        CloseHandle(handle);
        wait == WAIT_TIMEOUT
    }
}

/// Spawn a background thread that calls `on_death` once the parent exits. Returns immediately.
/// Poll interval matches the Swift 2 s heartbeat. No-op if `ASSISTANT_PARENT_PID` is unset.
pub fn watch_parent<F>(on_death: F)
where
    F: FnOnce() + Send + 'static,
{
    let Some(pid) = parent_pid_from_env() else {
        return;
    };
    std::thread::spawn(move || {
        loop {
            if !is_alive(pid) {
                on_death();
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_is_alive() {
        assert!(is_alive(std::process::id()));
    }

    #[test]
    fn pid_one_or_unlikely_pid() {
        // A very high, almost-certainly-unused pid should read as dead on any OS.
        assert!(!is_alive(4_000_000_000));
    }
}
