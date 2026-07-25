//! Top-level window queries — `EnumWindows` + `GetForegroundWindow` + `SetForegroundWindow`
//! (+`AttachThreadInput`). Windows analogue of the macOS `CGWindowList` / `NSWorkspace.frontmost`
//! and `NSRunningApplication.activate` calls used across the Swift helpers.
//!
//! `#[cfg(windows)]` holds the real, documented Win32 sequences; `#[cfg(not(windows))]` fallbacks
//! return empty/None so host builds compile. The enumeration is standard; the one genuinely
//! device-sensitive spot is foreground **activation** (UIPI / foreground-lock can refuse it) — that
//! path is marked and its failure maps to the `activation_failed` contract status.

/// Visible, titled top-level windows as `(pid, title)`. Powers `meeting-tap --list-apps`
/// (`exeName\twindowTitle`) and window-evidence gathering.
#[cfg(windows)]
pub fn all_windows() -> Vec<(u32, String)> {
    enumerate().into_iter().map(|(pid, _hwnd, title)| (pid, title)).collect()
}

#[cfg(not(windows))]
pub fn all_windows() -> Vec<(u32, String)> {
    crate::degrade("window enumeration (EnumWindows)");
    Vec::new()
}

/// All visible window titles owned by `pid`.
pub fn titles_for_pid(pid: u32) -> Vec<String> {
    all_windows()
        .into_iter()
        .filter(|(p, _)| *p == pid)
        .map(|(_, t)| t)
        .collect()
}

/// PID of the current foreground window, or `None`. Callers skip our own PID.
#[cfg(windows)]
pub fn foreground_pid() -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    // SAFETY: reads only.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            None
        } else {
            Some(pid)
        }
    }
}

#[cfg(not(windows))]
pub fn foreground_pid() -> Option<u32> {
    crate::degrade("foreground window (GetForegroundWindow)");
    None
}

/// Bring a visible top-level window of `pid` to the foreground. Returns whether it became
/// foreground. Used by `agent-action` before mutating; failure ⇒ `activation_failed`.
#[cfg(windows)]
pub fn activate_pid(pid: u32) -> bool {
    // windows 0.62: AttachThreadInput lives in Win32::System::Threading, not KeyboardAndMouse.
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow;

    let target_hwnd = match enumerate().into_iter().find(|(p, _, _)| *p == pid) {
        Some((_, hwnd, _)) => hwnd_from_isize(hwnd),
        None => return false,
    };

    // SAFETY: standard AttachThreadInput dance to defeat the foreground lock. Documented pattern.
    // TODO(on-device): under strict UIPI or a full-screen exclusive foreground, SetForegroundWindow
    // can still be refused; verify against notepad.exe on a real desktop session. Failure returns
    // false, which the caller maps to `activation_failed` (contract-valid).
    unsafe {
        let fg = GetForegroundWindow();
        let our_thread = GetCurrentThreadId();
        let mut fg_pid = 0u32;
        let fg_thread = GetWindowThreadProcessId(fg, Some(&mut fg_pid));

        if fg_thread != our_thread {
            let _ = AttachThreadInput(our_thread, fg_thread, true);
        }
        let _ = SetForegroundWindow(target_hwnd);
        let _ = SetActiveWindow(target_hwnd);
        if fg_thread != our_thread {
            let _ = AttachThreadInput(our_thread, fg_thread, false);
        }

        let now = GetForegroundWindow();
        let mut now_pid = 0u32;
        GetWindowThreadProcessId(now, Some(&mut now_pid));
        now_pid == pid
    }
}

#[cfg(not(windows))]
pub fn activate_pid(_pid: u32) -> bool {
    crate::degrade("window activation (SetForegroundWindow)");
    false
}

// ---------------------------------------------------------------------------
// Windows internals
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn hwnd_from_isize(v: isize) -> windows::Win32::Foundation::HWND {
    windows::Win32::Foundation::HWND(v as *mut core::ffi::c_void)
}

#[cfg(windows)]
struct EnumCtx {
    items: Vec<(u32, isize, String)>,
}

#[cfg(windows)]
unsafe extern "system" fn enum_proc(
    hwnd: windows::Win32::Foundation::HWND,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::core::BOOL {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    if !IsWindowVisible(hwnd).as_bool() {
        return windows::core::BOOL(1);
    }
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return windows::core::BOOL(1);
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(hwnd, &mut buf);
    if n <= 0 {
        return windows::core::BOOL(1);
    }
    let title = String::from_utf16_lossy(&buf[..n as usize]);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    ctx.items.push((pid, hwnd.0 as isize, title));
    windows::core::BOOL(1)
}

#[cfg(windows)]
fn enumerate() -> Vec<(u32, isize, String)> {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;

    let mut ctx = EnumCtx { items: Vec::new() };
    // SAFETY: `ctx` outlives the synchronous EnumWindows call; the callback only touches it.
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.items
}
