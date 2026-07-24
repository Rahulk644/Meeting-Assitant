//! `clean()` + label composition — REAL, pure, unit-tested. Verbatim port of the Swift helpers
//! shared by agent-context and agent-action (`clean` / `label`).
//!
//! Caveat (documented, not a bug): Swift `String.count` / `prefix` operate on **grapheme
//! clusters**; Rust `chars()` counts **scalar values**. For plain text they agree; they diverge only
//! on combining sequences / emoji ZWJ. We use `chars()` (no `unicode-segmentation` dependency); the
//! truncation boundary can differ by a few code points on such inputs. Field *content* and
//! fingerprints are unaffected (fingerprints hash the raw untruncated value).

/// Collapse internal whitespace to single spaces, trim, drop-if-empty, and truncate to `limit`
/// scalar values with a trailing `…` (U+2026). `None` in ⇒ `None` out.
pub fn clean(raw: Option<&str>, limit: usize) -> Option<String> {
    let raw = raw?;
    // `split_whitespace` collapses every run of Unicode whitespace AND trims the ends — exactly the
    // Swift "regex \s+ → ' '" followed by "trim whitespacesAndNewlines".
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= limit {
        Some(collapsed)
    } else {
        let mut out: String = collapsed.chars().take(limit).collect();
        out.push('…');
        Some(out)
    }
}

/// Compose a label from candidate attribute strings: clean each to `per_limit`, drop nils/empties,
/// dedupe preserving first-seen order, and join with `sep`. `None` if nothing survives.
///
/// agent-context / agent-action use `sep = " · "` (space, U+00B7, space) and `per_limit = 160`.
pub fn compose_label(raw_parts: &[Option<String>], sep: &str, per_limit: usize) -> Option<String> {
    let mut seen: Vec<String> = Vec::new();
    for part in raw_parts {
        if let Some(cleaned) = clean(part.as_deref(), per_limit) {
            if !seen.iter().any(|s| s == &cleaned) {
                seen.push(cleaned);
            }
        }
    }
    if seen.is_empty() {
        None
    } else {
        Some(seen.join(sep))
    }
}

/// The exact secure-field heuristic shared by all three accessibility helpers (`isSecure`).
/// `structural` = `"<role> <subrole>"`; `semantic` = the label. Substring, case-insensitive.
pub fn is_secure(role: &str, subrole: Option<&str>, label: Option<&str>) -> bool {
    let structural = format!("{} {}", role, subrole.unwrap_or("")).to_ascii_lowercase();
    if structural.contains("securetextfield") || structural.contains("password") {
        return true;
    }
    let semantic = label.unwrap_or("").to_ascii_lowercase();
    const TERMS: [&str; 8] = [
        "password", "passcode", "pin", "secret", "api key", "credit card", "cvv", "ssn",
    ];
    TERMS.iter().any(|t| semantic.contains(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_collapses_and_trims() {
        assert_eq!(clean(Some("  hello\n  world  "), 280).as_deref(), Some("hello world"));
        assert_eq!(clean(Some("   "), 280), None);
        assert_eq!(clean(None, 280), None);
    }

    #[test]
    fn clean_truncates_with_ellipsis() {
        let long = "a".repeat(300);
        let out = clean(Some(&long), 280).unwrap();
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), 281);
    }

    #[test]
    fn compose_dedupes_and_joins() {
        let parts = vec![Some("Body".to_string()), Some("Body".to_string()), Some("Compose".to_string())];
        assert_eq!(compose_label(&parts, " · ", 160).as_deref(), Some("Body · Compose"));
        assert_eq!(compose_label(&[None, Some("   ".to_string())], " · ", 160), None);
    }

    #[test]
    fn secure_heuristic() {
        assert!(is_secure("AXTextField", Some("AXSecureTextField"), None));
        assert!(is_secure("AXTextField", None, Some("API key")));
        assert!(!is_secure("AXTextArea", None, Some("Email body")));
    }
}
