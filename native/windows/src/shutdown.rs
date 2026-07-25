//! Graceful-stop plumbing. **Windows has no SIGTERM** (PROTOCOL §Shutdown): the core closes the
//! child's stdin and/or sends `CTRL_BREAK` to a `CREATE_NEW_PROCESS_GROUP` child. `meeting-tap` must
//! treat **stdin-EOF** and **console-ctrl** as "finalize (write the mp4 moov atom) and exit".
//!
//! - The stdin-EOF watcher is portable and real.
//! - The `SetConsoleCtrlHandler` install is `#[cfg(windows)]` real; a no-op elsewhere.
//!
//! Both simply flip a global `STOP` flag; the helper's main loop polls it, tears down capture
//! (finalising any in-progress recording), and exits after a short grace.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);

/// Request a graceful stop (idempotent).
pub fn request_stop() {
    STOP.store(true, Ordering::SeqCst);
}

/// Has a stop been requested (stdin-EOF, console-ctrl, or dead parent)?
pub fn stopping() -> bool {
    STOP.load(Ordering::SeqCst)
}

/// Spawn a thread that flips `STOP` when stdin reaches EOF. Use ONLY in helpers that do not read
/// data commands from stdin (i.e. `meeting-tap`). `dictation-target --session` reads commands from
/// stdin and must NOT install this.
pub fn watch_stdin_eof() {
    std::thread::spawn(|| {
        let mut buf = [0u8; 256];
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        loop {
            match lock.read(&mut buf) {
                Ok(0) => {
                    request_stop();
                    return;
                }
                Ok(_) => {} // ignore any bytes; meeting-tap has no stdin protocol
                Err(_) => {
                    request_stop();
                    return;
                }
            }
        }
    });
}

/// Install the Windows console-ctrl handler (CTRL_C / CTRL_BREAK / CTRL_CLOSE → request stop).
#[cfg(windows)]
pub fn install_console_handler() {
    use windows::core::BOOL; // windows 0.62 moved BOOL out of Win32::Foundation into windows::core
    use windows::Win32::System::Console::SetConsoleCtrlHandler;

    unsafe extern "system" fn handler(_ctrl_type: u32) -> BOOL {
        // Any control event means "finalize and exit". Returning TRUE marks it handled; the main
        // loop is responsible for the actual teardown + timely exit.
        super::shutdown::request_stop();
        BOOL(1)
    }

    // SAFETY: registering a static handler pointer; documented, thread-safe call.
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(handler), true);
    }
}

/// No-op on non-Windows hosts (this helper only ships on Windows; the mac/Linux twins use signals).
#[cfg(not(windows))]
pub fn install_console_handler() {
    // Nothing to do; the portable stdin-EOF watcher covers the graceful-stop contract in dev.
}
