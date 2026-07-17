use crate::dictation_cleanup::{ProtectedSpan, VocabularyTerm};
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub accepted: bool,
    pub reason: String,
    pub text: String,
}

fn reject(reason: &str) -> ValidationResult {
    ValidationResult {
        accepted: false,
        reason: reason.to_string(),
        text: String::new(),
    }
}

fn word_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?u)[\p{L}\p{N}]+(?:['’][\p{L}\p{N}]+)*").unwrap())
}

fn words(text: &str) -> Vec<String> {
    word_regex()
        .find_iter(text)
        .map(|found| {
            found
                .as_str()
                .nfc()
                .flat_map(char::to_lowercase)
                .filter(|ch| !matches!(*ch, '\'' | '’'))
                .collect()
        })
        .collect()
}

fn conservative_spelling_change(left: &str, right: &str) -> bool {
    let a = left.chars().collect::<Vec<_>>();
    let b = right.chars().collect::<Vec<_>>();
    if a.len() == b.len() && a.len() >= 3 {
        let changed = a
            .iter()
            .zip(&b)
            .enumerate()
            .filter_map(|(index, (left, right))| (left != right).then_some(index))
            .collect::<Vec<_>>();
        if changed.len() == 2
            && changed[1] == changed[0] + 1
            && a[changed[0]] == b[changed[1]]
            && a[changed[1]] == b[changed[0]]
        {
            return true;
        }
    }
    matches!(
        (left, right),
        ("recieve", "receive")
            | ("adress", "address")
            | ("definately", "definitely")
            | ("seperate", "separate")
            | ("occured", "occurred")
            | ("untill", "until")
            | ("wich", "which")
    )
}

fn approved_filler(word: &str) -> bool {
    matches!(word, "like" | "actually" | "basically" | "well")
}

fn retained_words_are_ordered(mechanical: &str, proposal: &str) -> bool {
    let original = words(mechanical);
    let candidate = words(proposal);
    let (mut i, mut j) = (0, 0);
    while i < original.len() && j < candidate.len() {
        if original[i] == candidate[j] || conservative_spelling_change(&original[i], &candidate[j])
        {
            i += 1;
            j += 1;
        } else if approved_filler(&original[i]) {
            i += 1;
        } else {
            return false;
        }
    }
    j == candidate.len() && original[i..].iter().all(|word| approved_filler(word))
}

fn protected_numbers(text: &str) -> Vec<String> {
    static NUMBER: OnceLock<Regex> = OnceLock::new();
    NUMBER
        .get_or_init(|| Regex::new(r"(?u)(?:[$€₹]\s*)?-?\b\d[\d,.:/\-]*\b%?").unwrap())
        .find_iter(text)
        .map(|found| found.as_str().to_string())
        .collect()
}

fn exact_count(text: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    text.match_indices(needle).count()
}

fn protected_values_unchanged(
    mechanical: &str,
    proposal: &str,
    protected: &[ProtectedSpan],
    vocabulary: &[VocabularyTerm],
) -> bool {
    let mut values = protected
        .iter()
        .map(|span| span.value.clone())
        .collect::<Vec<_>>();
    values.extend(protected_numbers(mechanical));
    values.extend(
        vocabulary
            .iter()
            .map(|term| term.word.trim().to_string())
            .filter(|term| !term.is_empty()),
    );
    values.sort();
    values.dedup();
    values
        .iter()
        .all(|value| exact_count(mechanical, value) == exact_count(proposal, value))
}

fn scripts(text: &str) -> (usize, usize, usize) {
    let mut latin = 0;
    let mut devanagari = 0;
    let mut other = 0;
    for ch in text.chars().filter(|ch| ch.is_alphabetic()) {
        let code = ch as u32;
        if ch.is_ascii_alphabetic() || (0x00c0..=0x024f).contains(&code) {
            latin += 1;
        } else if (0x0900..=0x097f).contains(&code) {
            devanagari += 1;
        } else {
            other += 1;
        }
    }
    (latin, devanagari, other)
}

fn script_is_stable(mechanical: &str, proposal: &str) -> bool {
    let before = scripts(mechanical);
    let after = scripts(proposal);
    (before.0 == 0 || after.0 > 0)
        && (before.1 == 0 || after.1 > 0)
        && (before.2 == 0 || after.2 > 0)
        && (before.1 > 0 || after.1 == 0)
}

fn repeated_output(text: &str) -> bool {
    let normalized = text.split_whitespace().collect::<Vec<_>>();
    normalized.len() >= 8
        && normalized.len() % 2 == 0
        && normalized[..normalized.len() / 2] == normalized[normalized.len() / 2..]
}

pub fn validate(
    mechanical: &str,
    proposal: &str,
    protected: &[ProtectedSpan],
    vocabulary: &[VocabularyTerm],
) -> ValidationResult {
    let candidate = proposal
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .nfc()
        .collect::<String>();
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return reject("empty_output");
    }
    if candidate
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
    {
        return reject("invalid_control_character");
    }
    let lower = candidate.to_lowercase();
    if candidate.contains("```")
        || lower.contains("<transcript>")
        || lower.contains("</transcript>")
        || lower.starts_with("here is")
        || lower.starts_with("edited transcript")
        || ((candidate.starts_with('{') && candidate.ends_with('}'))
            || (candidate.starts_with('[') && candidate.ends_with(']')))
    {
        return reject("model_formatting_or_commentary");
    }
    let base_len = mechanical.chars().count().max(1) as f64;
    let ratio = candidate.chars().count() as f64 / base_len;
    if !(0.65..=1.35).contains(&ratio) {
        return reject("length_out_of_bounds");
    }
    if repeated_output(candidate) {
        return reject("repeated_output");
    }
    if !protected_values_unchanged(mechanical, candidate, protected, vocabulary) {
        return reject("protected_span_changed");
    }
    if !script_is_stable(mechanical, candidate) {
        return reject("language_or_script_changed");
    }
    if !retained_words_are_ordered(mechanical, candidate) {
        return reject("word_added_removed_or_reordered");
    }
    ValidationResult {
        accepted: true,
        reason: "accepted".into(),
        text: candidate.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(base: &str, proposal: &str) -> ValidationResult {
        validate(base, proposal, &[], &[])
    }

    #[test]
    fn accepts_only_lossless_edits() {
        assert!(check("send it tomorrow", "Send it tomorrow.").accepted);
        assert!(check("this is teh draft", "This is the draft.").accepted);
        assert!(check("I was, like, ready", "I was ready.").accepted);
    }

    #[test]
    fn rejects_semantic_or_structural_changes() {
        assert!(!check("send it tomorrow", "Please send it tomorrow").accepted);
        assert!(!check("send it tomorrow", "sand it tomorrow").accepted);
        assert!(!check("send it tomorrow", "tomorrow send it").accepted);
        assert!(!check("do not send", "send").accepted);
        assert!(!check("नमस्ते दुनिया", "hello world").accepted);
        assert!(!check("hello", "Here is the edited transcript: Hello.").accepted);
    }

    #[test]
    fn protects_numbers_urls_and_vocabulary() {
        assert!(
            !check(
                "send $25 to test@example.com",
                "send $20 to test@example.com"
            )
            .accepted
        );
        let vocab = [VocabularyTerm {
            word: "OpenAI".into(),
            alts: String::new(),
        }];
        assert!(!validate("OpenAI ships", "Open AI ships", &[], &vocab).accepted);
    }
}
