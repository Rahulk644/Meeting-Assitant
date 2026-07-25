//! Keyboard injection — the Windows analogue of the Swift Unicode `CGEvent` keystrokes. Two real
//! entry points:
//! - `type_text` / `backspace` via **enigo** (`KEYEVENTF_UNICODE`), used by `agent-action`.
//! - `send_unicode` / `send_backspace` via a **raw `SendInput`** call, used by `dictation-target`
//!   because its return count gives the UIPI signal PROTOCOL §2 asks for: `SendInput` inserting
//!   fewer events than requested (0 into an elevated window) ⇒ `unsafeFallback:true`.
//!
//! `#[cfg(windows)]` holds the real calls; `#[cfg(not(windows))]` fallbacks return `Failed`/`false`
//! so host builds compile. The enigo generics are marked TODO(on-device) for exact 0.6 API.

/// Outcome of a raw injection, carrying the UIPI signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectOutcome {
    /// All synthesized events were accepted by the system.
    Ok,
    /// The call failed outright (no source, bad args).
    Failed,
    /// `SendInput` inserted fewer events than requested — the classic UIPI block (injecting into a
    /// higher-integrity/elevated window from a normal process). Callers surface `unsafeFallback`.
    UipiBlocked,
}

// ---------------------------------------------------------------------------
// enigo path (agent-action)
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub fn type_text(text: &str) -> bool {
    use enigo::{Enigo, Keyboard, Settings};
    // TODO(on-device): confirm enigo 0.6 constructor/trait. `Enigo::new(&Settings::default())` →
    // `Keyboard::text(&mut e, text)`.
    match Enigo::new(&Settings::default()) {
        Ok(mut e) => e.text(text).is_ok(),
        Err(_) => false,
    }
}

#[cfg(windows)]
pub fn backspace(n: usize) -> bool {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let Ok(mut e) = Enigo::new(&Settings::default()) else {
        return false;
    };
    for _ in 0..n {
        if e.key(Key::Backspace, Direction::Click).is_err() {
            return false;
        }
    }
    true
}

#[cfg(not(windows))]
pub fn type_text(text: &str) -> bool {
    crate::degrade("enigo text injection");
    let _ = text;
    false
}

#[cfg(not(windows))]
pub fn backspace(n: usize) -> bool {
    crate::degrade("enigo backspace");
    let _ = n;
    false
}

// ---------------------------------------------------------------------------
// raw SendInput path (dictation-target — needs the UIPI return count)
// ---------------------------------------------------------------------------

/// Synthesize `text` as Unicode key events via a single `SendInput` batch. The return count vs the
/// requested count yields the UIPI signal. Surrogate pairs are sent as two units, matching the Swift
/// `postUnicodeText` chunking.
#[cfg(windows)]
pub fn send_unicode(text: &str) -> InjectOutcome {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };

    if text.is_empty() {
        return InjectOutcome::Ok;
    }
    let mut inputs: Vec<INPUT> = Vec::new();
    for unit in text.encode_utf16() {
        for keyup in [false, true] {
            let mut flags = KEYEVENTF_UNICODE;
            if keyup {
                flags |= KEYEVENTF_KEYUP;
            }
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY(0),
                        wScan: unit,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    // SAFETY: `inputs` is a valid contiguous slice; cbsize is the element size.
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    classify(sent as usize, inputs.len())
}

/// Synthesize `n` Backspace presses via `SendInput`.
#[cfg(windows)]
pub fn send_backspace(n: usize) -> InjectOutcome {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_BACK,
    };
    if n == 0 {
        return InjectOutcome::Ok;
    }
    let mut inputs: Vec<INPUT> = Vec::new();
    for _ in 0..n {
        for keyup in [false, true] {
            let flags = if keyup { KEYEVENTF_KEYUP } else { Default::default() };
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VK_BACK,
                        wScan: 0,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    classify(sent as usize, inputs.len())
}

#[cfg(windows)]
fn classify(sent: usize, requested: usize) -> InjectOutcome {
    if sent == requested {
        InjectOutcome::Ok
    } else if sent == 0 {
        InjectOutcome::UipiBlocked
    } else {
        // Partial insertion also indicates a block partway through.
        InjectOutcome::UipiBlocked
    }
}

#[cfg(not(windows))]
pub fn send_unicode(text: &str) -> InjectOutcome {
    crate::degrade("SendInput unicode");
    let _ = text;
    InjectOutcome::Failed
}

#[cfg(not(windows))]
pub fn send_backspace(n: usize) -> InjectOutcome {
    crate::degrade("SendInput backspace");
    let _ = n;
    InjectOutcome::Failed
}
