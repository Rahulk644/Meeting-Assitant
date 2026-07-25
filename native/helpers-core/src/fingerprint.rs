//! SHA-256 content/state fingerprints — a verbatim port of the Swift `digest` /
//! `contentFingerprint` / `stateFingerprint` in `agent-context.swift:151` and
//! `agent-action.swift:139`. These identify a focused control across reads so `agent-action` can
//! refuse to mutate a target that has changed underneath it. The field separator is the ASCII Unit
//! Separator `0x1F`; the version tag is `v1`. Both must match byte-for-byte so a snapshot taken by
//! one helper is honoured by the other.

use sha2::{Digest, Sha256};

/// The Unit Separator that joins fingerprint fields (`\u{1f}` in the Swift string literals).
const US: char = '\u{1f}';

/// Lowercase hex SHA-256 of the UTF-8 bytes of `value`.
pub fn digest(value: &str) -> String {
    let mut h = Sha256::new();
    h.update(value.as_bytes());
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

/// Identifies control *content*: `v1␟role␟label␟value`.
pub fn content_fingerprint(role: &str, label: Option<&str>, value: &str) -> String {
    digest(&format!("v1{US}{role}{US}{}{US}{value}", label.unwrap_or("")))
}

/// Identifies control *state* including selection: `v1␟role␟label␟value␟loc:len`.
/// A missing range encodes as `-1:-1`, exactly as the Swift `range?.location ?? -1` does.
pub fn state_fingerprint(
    role: &str,
    label: Option<&str>,
    value: &str,
    range: Option<(i64, i64)>,
) -> String {
    let (loc, len) = range.unwrap_or((-1, -1));
    digest(&format!(
        "v1{US}{role}{US}{}{US}{value}{US}{loc}:{len}",
        label.unwrap_or("")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_lowercase_hex_sha256() {
        // Known SHA-256("") vector.
        assert_eq!(
            digest(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn state_fingerprint_encodes_missing_range_as_neg1() {
        let a = state_fingerprint("AXTextArea", Some("Body"), "hello", None);
        let b = digest("v1\u{1f}AXTextArea\u{1f}Body\u{1f}hello\u{1f}-1:-1");
        assert_eq!(a, b);
    }

    #[test]
    fn content_fingerprint_omits_range() {
        let a = content_fingerprint("AXTextField", None, "x");
        let b = digest("v1\u{1f}AXTextField\u{1f}\u{1f}x");
        assert_eq!(a, b);
    }
}
