//! The tiny argv convention every helper shares, mirroring the Swift `opt(name, def)` helper and
//! the "first positional unless it starts with `--`" rule for `meeting-tap`.
//!
//! Deliberately dependency-free (no clap) so the helpers stay small and the parsing matches the
//! Swift byte-for-byte, including that unknown flags are ignored rather than rejected.

/// Parsed argv: ordered positionals + `--flag value` / bare `--flag` pairs.
pub struct Args {
    positionals: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    /// Parse from `std::env::args().skip(1)`.
    pub fn from_env() -> Self {
        Self::from_iter(std::env::args().skip(1))
    }

    pub fn from_iter<I: IntoIterator<Item = String>>(iter: I) -> Self {
        let raw: Vec<String> = iter.into_iter().collect();
        let mut positionals = Vec::new();
        let mut flags = Vec::new();
        let mut i = 0;
        while i < raw.len() {
            let tok = &raw[i];
            if let Some(name) = tok.strip_prefix("--") {
                // A flag consumes the next token as its value unless that token is itself a flag.
                let val = raw.get(i + 1).filter(|n| !n.starts_with("--")).cloned();
                if val.is_some() {
                    i += 1;
                }
                flags.push((name.to_string(), val));
            } else {
                positionals.push(tok.clone());
            }
            i += 1;
        }
        Args { positionals, flags }
    }

    /// True if `--name` was present in any form (bare or with a value).
    pub fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }

    /// Value of `--name`, or `default` if absent. Mirrors Swift `opt(name, def)`.
    pub fn opt(&self, name: &str, default: &str) -> String {
        self.flags
            .iter()
            .find(|(k, _)| k == name)
            .and_then(|(_, v)| v.clone())
            .unwrap_or_else(|| default.to_string())
    }

    /// Value of `--name` if present (and given a value).
    pub fn get(&self, name: &str) -> Option<String> {
        self.flags
            .iter()
            .find(|(k, _)| k == name)
            .and_then(|(_, v)| v.clone())
    }

    /// Positional at `index` **only if** it does not start with `--` (the meeting-tap rule).
    pub fn positional(&self, index: usize) -> Option<String> {
        self.positionals.get(index).cloned()
    }

    /// Env var overrides a flag default: `env(ENV) ?? opt(flag, default)`.
    pub fn opt_env(&self, env: &str, flag: &str, default: &str) -> String {
        std::env::var(env).ok().unwrap_or_else(|| self.opt(flag, default))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Args {
        Args::from_iter(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn meeting_tap_positionals_and_flags() {
        let args = a(&["8790", "http://x/inference", "--gate", "on", "--mic", "off", "--rms"]);
        assert_eq!(args.positional(0).as_deref(), Some("8790"));
        assert_eq!(args.positional(1).as_deref(), Some("http://x/inference"));
        assert_eq!(args.opt("gate", "off"), "on");
        assert_eq!(args.opt("mic", "on"), "off");
        assert!(args.has("rms"));
        assert!(!args.has("video"));
    }

    #[test]
    fn flag_before_positional_does_not_swallow_it() {
        // `--list-apps` is bare; the next token is not a value.
        let args = a(&["--list-apps"]);
        assert!(args.has("list-apps"));
        assert_eq!(args.get("list-apps"), None);
    }
}
