//! Platform selection for the bundled native helper binaries.
//!
//! macOS ships Swift helpers; Windows and Linux ship the Rust helpers under `native/{windows,linux}`
//! (see `native/PROTOCOL.md`). Whichever OS we run on, the core spawns a helper by the same logical
//! name and this module resolves it to the correct on-disk path:
//!
//! - the executable stem gains a `.exe` suffix on Windows (`meeting-tap` → `meeting-tap.exe`);
//! - the resource sub-directory layout is otherwise identical across platforms.
//!
//! Routing every helper-path construction through here means Windows `.exe` suffixing (and any
//! future per-OS path divergence) lives in exactly one place instead of being sprinkled across
//! `lib.rs`, `local_server.rs`, `dictation_context.rs`, and `polish_runtime.rs`.

use std::path::{Path, PathBuf};

/// The five spawned-helper + two model-server stems and their resource sub-directories, so call
/// sites can refer to a helper by identity rather than repeating string literals.
pub mod helpers {
    /// `(subdir, stem)` pairs. The stem is the bare name; `.exe` is added per-OS at resolve time.
    pub const MEETING_TAP: (&str, &str) = ("stt", "meeting-tap");
    pub const MEETING_STATE: (&str, &str) = ("meeting-state", "meeting-state");
    pub const DICTATION_TARGET: (&str, &str) = ("dictation-target", "dictation-target");
    pub const AGENT_CONTEXT: (&str, &str) = ("agent-context", "agent-context");
    pub const AGENT_ACTION: (&str, &str) = ("agent-action", "agent-action");
    pub const TRANSCRIBE_SERVER: (&str, &str) = ("stt/transcribe-server", "transcribe-server");
    pub const LLAMA_SERVER: (&str, &str) = ("polish-runtime", "llama-server");
}

/// Platform-correct executable file name — adds `.exe` on Windows, leaves it bare elsewhere.
pub fn exe_name(stem: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// Absolute path to a bundled native helper: `<res_base>/<subdir>/<stem>[.exe]`.
pub fn helper_path(res_base: &Path, subdir: &str, stem: &str) -> PathBuf {
    res_base.join(subdir).join(exe_name(stem))
}

/// Convenience for the `format!("{}/...", res_base())` call sites that pass a `String` base.
pub fn helper_path_str(res_base: &str, subdir: &str, stem: &str) -> PathBuf {
    helper_path(Path::new(res_base), subdir, stem)
}

/// Resolve one of the [`helpers`] `(subdir, stem)` tuples against a string base.
pub fn resolve(res_base: &str, helper: (&str, &str)) -> PathBuf {
    helper_path_str(res_base, helper.0, helper.1)
}

/// Resolve one of the [`helpers`] `(subdir, stem)` tuples against a `Path` base.
pub fn resolve_path(res_base: &Path, helper: (&str, &str)) -> PathBuf {
    helper_path(res_base, helper.0, helper.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_matches_platform() {
        let p = helper_path_str("/res", "stt", "meeting-tap");
        if cfg!(target_os = "windows") {
            assert!(p.ends_with("meeting-tap.exe"));
        } else {
            assert!(p.ends_with("meeting-tap"));
        }
        assert!(p.to_string_lossy().contains("stt"));
    }

    #[test]
    fn resolve_uses_helper_tuple() {
        let p = resolve("/res", helpers::AGENT_CONTEXT);
        assert!(p.to_string_lossy().contains("agent-context"));
    }
}
