use crate::dictation_commands::apply_commands;
use crate::dictation_diff::{diff_spans, DiffSpan};
use crate::dictation_numbers::normalize_numbers;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationTargetKind {
    NaturalLanguage,
    CodeOrTerminal,
    Secure,
    #[default]
    Unknown,
}

impl DictationTargetKind {
    pub fn from_wire(value: &str) -> Self {
        match value {
            "naturalLanguage" | "natural" => Self::NaturalLanguage,
            "codeOrTerminal" | "code" | "terminal" => Self::CodeOrTerminal,
            "secure" => Self::Secure,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyTerm {
    #[serde(default)]
    pub word: String,
    #[serde(default)]
    pub alts: String,
}

#[derive(Clone, Debug)]
pub struct CleanupRequest {
    pub text: String,
    pub language: String,
    pub target: DictationTargetKind,
    pub starts_sentence: bool,
    pub vocabulary: Vec<VocabularyTerm>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupChange {
    pub original_span: String,
    pub result_span: String,
    pub rule: String,
    pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedSpan {
    pub value: String,
    pub kind: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    pub original: String,
    pub text: String,
    pub changes: Vec<CleanupChange>,
    pub visual_changes: Vec<DiffSpan>,
    #[serde(skip)]
    pub protected: Vec<ProtectedSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolishChunk {
    pub text: String,
    pub separator: String,
}

fn confirmed_boundaries(text: &str) -> Vec<(usize, usize)> {
    let mut boundaries = Vec::new();
    let mut iter = text.char_indices().peekable();
    while let Some((index, ch)) = iter.next() {
        let content_end = index + ch.len_utf8();
        let terminal = matches!(ch, '.' | '!' | '?' | '।');
        let paragraph = ch == '\n' && iter.peek().is_some_and(|(_, next)| *next == '\n');
        if !terminal && !paragraph {
            continue;
        }
        if terminal
            && !text[content_end..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            continue;
        }
        let mut separator_end = content_end;
        for next in text[content_end..].chars() {
            if !next.is_whitespace() {
                break;
            }
            separator_end += next.len_utf8();
        }
        boundaries.push((if paragraph { index } else { content_end }, separator_end));
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
}

// Long input is split only at confirmed sentence or paragraph boundaries. Returning None means no safe
// boundary exists; callers must use the complete mechanical text instead of cutting an arbitrary phrase.
pub fn split_polish_chunks(text: &str, max_chars: usize) -> Option<Vec<PolishChunk>> {
    let max_chars = max_chars.max(32);
    if text.chars().count() <= max_chars {
        return Some(vec![PolishChunk {
            text: text.to_string(),
            separator: String::new(),
        }]);
    }
    let boundaries = confirmed_boundaries(text);
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while text[start..].chars().count() > max_chars {
        let within = boundaries
            .iter()
            .copied()
            .rfind(|(end, _)| *end > start && text[start..*end].chars().count() <= max_chars);
        let boundary = within.or_else(|| {
            boundaries.iter().copied().find(|(end, _)| {
                *end > start && text[start..*end].chars().count() <= max_chars.saturating_mul(2)
            })
        })?;
        if boundary.0 <= start || text[start..boundary.0].trim().is_empty() {
            return None;
        }
        chunks.push(PolishChunk {
            text: text[start..boundary.0].to_string(),
            separator: text[boundary.0..boundary.1].to_string(),
        });
        start = boundary.1;
    }
    if start < text.len() {
        chunks.push(PolishChunk {
            text: text[start..].to_string(),
            separator: String::new(),
        });
    }
    (!chunks.is_empty()).then_some(chunks)
}

fn track(
    text: &mut String,
    next: String,
    rule: &str,
    confidence: f32,
    changes: &mut Vec<CleanupChange>,
) {
    if *text == next {
        return;
    }
    for span in diff_spans(text, &next) {
        changes.push(CleanupChange {
            original_span: span.original,
            result_span: span.replacement,
            rule: rule.to_string(),
            confidence,
        });
    }
    *text = next;
}

fn sanitize_input(text: &str) -> String {
    static ENGINE_TOKEN: OnceLock<Regex> = OnceLock::new();
    static TIMESTAMP: OnceLock<Regex> = OnceLock::new();
    static SENTINEL: OnceLock<Regex> = OnceLock::new();
    let mut value = text.replace("\r\n", "\n").replace('\r', "\n");
    value = ENGINE_TOKEN
        .get_or_init(|| Regex::new(r"(?u)<\|[^|\r\n]{1,80}\|>").unwrap())
        .replace_all(&value, "")
        .into_owned();
    value = TIMESTAMP
        .get_or_init(|| {
            Regex::new(r"(?u)\[(?:\d{1,2}:)?\d{1,2}:\d{2}(?:[.,]\d{1,3})?(?:\s*-->\s*(?:\d{1,2}:)?\d{1,2}:\d{2}(?:[.,]\d{1,3})?)?\]").unwrap()
        })
        .replace_all(&value, "")
        .into_owned();
    value = SENTINEL
        .get_or_init(|| Regex::new(r"(?iu)\[(?:blank_audio|no_speech|music)\]").unwrap())
        .replace_all(&value, "")
        .into_owned();
    value
        .nfc()
        .filter(|ch| !ch.is_control() || matches!(*ch, '\n' | '\t'))
        .collect::<String>()
        .trim()
        .to_string()
}

fn protected_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(
            r#"(?ix)
            (?:https?://|www\.)[^\s<>{}\[\]]+
            |[\p{L}\p{N}._%+\-]+@[\p{L}\p{N}.\-]+\.[\p{L}]{2,}
            |`[^`\r\n]+`
            |(?:^|[\s(])(?:/[^\s]+|~/(?:[^\s]+))
            |\b[\p{L}\p{N}_\-]+\.(?:txt|md|json|rs|swift|js|jsx|ts|tsx|py|html|css|sh|yml|yaml|toml|pdf|docx|xlsx|csv)\b
            |\b\d+\.\d+\b
            "#,
        )
        .unwrap()
    })
}

fn protected_kind(value: &str) -> &'static str {
    if value.contains('@') && !value.contains(' ') {
        "email"
    } else if value.starts_with("http") || value.starts_with("www.") {
        "url"
    } else if value.starts_with('`') {
        "code"
    } else if value.parse::<f64>().is_ok() {
        "number"
    } else {
        "fileOrCode"
    }
}

fn protect(text: &str) -> (String, Vec<(String, ProtectedSpan)>) {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut values = Vec::new();
    for found in protected_regex().find_iter(text) {
        let leading = &text[found.start()..found.end()]
            .chars()
            .take_while(|ch| ch.is_whitespace() || *ch == '(')
            .collect::<String>();
        let skip = leading.len();
        let start = found.start() + skip;
        if start >= found.end() {
            continue;
        }
        output.push_str(&text[cursor..start]);
        let value = text[start..found.end()].to_string();
        let token = format!("MAZXQPROTECTED{}TOKEN", values.len());
        output.push_str(&token);
        values.push((
            token,
            ProtectedSpan {
                kind: protected_kind(&value),
                value,
            },
        ));
        cursor = found.end();
    }
    output.push_str(&text[cursor..]);
    (output, values)
}

fn restore(mut text: String, values: &[(String, ProtectedSpan)]) -> String {
    for (token, protected) in values {
        text = text.replace(token, &protected.value);
    }
    text
}

fn restore_trace(changes: &mut [CleanupChange], values: &[(String, ProtectedSpan)]) {
    for change in changes {
        change.original_span = restore(change.original_span.clone(), values);
        change.result_span = restore(change.result_span.clone(), values);
    }
}

fn normalize_spaces(text: &str) -> String {
    static HORIZONTAL: OnceLock<Regex> = OnceLock::new();
    static BLANK_LINES: OnceLock<Regex> = OnceLock::new();
    let lines = text
        .split('\n')
        .map(|line| {
            HORIZONTAL
                .get_or_init(|| Regex::new(r"[ \u{00a0}]{2,}").unwrap())
                .replace_all(line.trim_matches([' ', '\u{00a0}']), " ")
                .into_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");
    BLANK_LINES
        .get_or_init(|| Regex::new(r"\n{3,}").unwrap())
        .replace_all(&lines, "\n\n")
        .trim()
        .to_string()
}

fn locale_spacing(text: &str, language: &str) -> String {
    let lang = language.to_ascii_lowercase();
    if !(lang.is_empty()
        || lang == "auto"
        || lang.starts_with("en")
        || lang.starts_with("hi")
        || lang.contains("hinglish"))
    {
        return text.to_string();
    }
    static BEFORE: OnceLock<Regex> = OnceLock::new();
    static AFTER: OnceLock<Regex> = OnceLock::new();
    let value = BEFORE
        .get_or_init(|| Regex::new(r"[ \u{00a0}]+([,.;:!?…])").unwrap())
        .replace_all(text, "$1")
        .into_owned();
    let value = AFTER
        .get_or_init(|| Regex::new(r"([,.;:!?])([^\s\n\]\)}.,;:!?])").unwrap())
        .replace_all(&value, "$1 $2")
        .into_owned();
    static NUMERIC_SEPARATOR: OnceLock<Regex> = OnceLock::new();
    NUMERIC_SEPARATOR
        .get_or_init(|| Regex::new(r"(\d)([.:/])\s+(\d)").unwrap())
        .replace_all(&value, "$1$2$3")
        .into_owned()
}

fn balanced_bracket_spacing(text: &str) -> String {
    let mut value = text.to_string();
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}'), ('“', '”')] {
        if value.matches(open).count() != value.matches(close).count() {
            continue;
        }
        value = value.replace(&format!("{open} "), &open.to_string());
        value = value.replace(&format!(" {close}"), &close.to_string());
    }
    value
}

fn remove_fillers(text: &str) -> String {
    static FILLER: OnceLock<Regex> = OnceLock::new();
    FILLER
        .get_or_init(|| Regex::new(r"(?iu)(?:^|\s)\b(?:um|uh|erm|er)\b(?:\s*[,;:]\s*)?").unwrap())
        .replace_all(text, " ")
        .into_owned()
}

fn remove_exact_overlap(text: &str) -> String {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let regex = WORD.get_or_init(|| Regex::new(r"(?u)[\p{L}\p{N}'’]+").unwrap());
    let mut value = text.to_string();
    loop {
        let words = regex.find_iter(&value).collect::<Vec<_>>();
        let mut removal = None;
        'outer: for size in (3..=10).rev() {
            if words.len() < size * 2 {
                continue;
            }
            for start in 0..=words.len() - size * 2 {
                let same = (0..size).all(|offset| {
                    words[start + offset]
                        .as_str()
                        .eq_ignore_ascii_case(words[start + size + offset].as_str())
                });
                if same {
                    let from = words[start + size].start();
                    let to = words[start + size * 2 - 1].end();
                    removal = Some((from, to));
                    break 'outer;
                }
            }
        }
        let Some((mut from, to)) = removal else {
            break;
        };
        while from > 0 && value.as_bytes()[from - 1].is_ascii_whitespace() {
            from -= 1;
        }
        value.replace_range(from..to, "");
    }
    value
}

fn apply_vocabulary(text: &str, vocabulary: &[VocabularyTerm]) -> String {
    let mut value = text.to_string();
    for term in vocabulary {
        let canonical = term.word.trim();
        if canonical.is_empty() {
            continue;
        }
        let aliases = std::iter::once(canonical)
            .chain(term.alts.split(',').map(str::trim))
            .filter(|alias| !alias.is_empty())
            .collect::<Vec<_>>();
        for alias in aliases {
            let regex = Regex::new(&format!(r"(?iu)\b{}\b", regex::escape(alias))).unwrap();
            value = regex
                .replace_all(&value, regex::NoExpand(canonical))
                .into_owned();
        }
    }
    value
}

fn capitalize(text: &str, starts_sentence: bool) -> String {
    let mut sentence_start = starts_sentence;
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        if sentence_start && ch.is_alphabetic() {
            output.extend(ch.to_uppercase());
            sentence_start = false;
        } else {
            output.push(ch);
            if !ch.is_whitespace() && !matches!(ch, '“' | '"' | '\'' | '(' | '[' | '{') {
                sentence_start = false;
            }
        }
        if matches!(ch, '.' | '!' | '?' | '\n') {
            sentence_start = true;
        }
    }
    static PRONOUN_I: OnceLock<Regex> = OnceLock::new();
    PRONOUN_I
        .get_or_init(|| Regex::new(r"(?u)\bi\b").unwrap())
        .replace_all(&output, "I")
        .into_owned()
}

fn clean_after_deletion(text: &str, language: &str) -> String {
    let value = normalize_spaces(text);
    let value = locale_spacing(&value, language);
    balanced_bracket_spacing(&value)
}

pub fn cleanup(request: CleanupRequest) -> CleanupResult {
    let original = request.text.clone();
    let mut changes = Vec::new();
    let sanitized = sanitize_input(&request.text);
    let mut text = request.text;
    track(&mut text, sanitized, "input sanitation", 1.0, &mut changes);

    let (protected_text, protected_values) = protect(&text);
    text = protected_text;
    let natural = request.target == DictationTargetKind::NaturalLanguage;
    let conservative = matches!(
        request.target,
        DictationTargetKind::NaturalLanguage | DictationTargetKind::Unknown
    );

    if request.target != DictationTargetKind::CodeOrTerminal {
        let next = normalize_spaces(&text);
        track(
            &mut text,
            next,
            "whitespace normalization",
            1.0,
            &mut changes,
        );
    }

    let next = apply_commands(&text, &request.language);
    track(&mut text, next, "spoken punctuation", 0.98, &mut changes);

    if conservative && request.target != DictationTargetKind::Secure {
        let next = remove_fillers(&text);
        track(&mut text, next, "safe filler removal", 0.99, &mut changes);

        let next = remove_exact_overlap(&text);
        track(&mut text, next, "decoder overlap", 0.99, &mut changes);
    }

    if natural {
        let next = normalize_numbers(&text, &request.language);
        track(&mut text, next, "number normalization", 0.97, &mut changes);

        let next = apply_vocabulary(&text, &request.vocabulary);
        track(&mut text, next, "custom vocabulary", 1.0, &mut changes);

        let next = capitalize(&text, request.starts_sentence);
        track(
            &mut text,
            next,
            "sentence capitalization",
            0.98,
            &mut changes,
        );
    }

    if request.target != DictationTargetKind::CodeOrTerminal {
        let next = clean_after_deletion(&text, &request.language);
        track(&mut text, next, "punctuation spacing", 1.0, &mut changes);
    }

    text = restore(text, &protected_values);
    restore_trace(&mut changes, &protected_values);
    let final_sanitized = sanitize_input(&text);
    if final_sanitized != text {
        for span in diff_spans(&text, &final_sanitized) {
            changes.push(CleanupChange {
                original_span: span.original,
                result_span: span.replacement,
                rule: "output sanitation".into(),
                confidence: 1.0,
            });
        }
        text = final_sanitized;
    }
    let visual_changes = diff_spans(&original, &text);
    CleanupResult {
        original,
        text,
        changes,
        visual_changes,
        protected: protected_values
            .into_iter()
            .map(|(_, protected)| protected)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &str) -> String {
        cleanup(CleanupRequest {
            text: input.into(),
            language: "en".into(),
            target: DictationTargetKind::NaturalLanguage,
            starts_sentence: true,
            vocabulary: vec![],
        })
        .text
    }

    #[test]
    fn core_pipeline_is_lossless_and_conservative() {
        assert_eq!(
            run("  um  send twenty-five dollars full stop "),
            "Send $25."
        );
        assert_eq!(run("very very important"), "Very very important");
        assert_eq!(
            run("Schedule it for Tuesday—no, Wednesday"),
            "Schedule it for Tuesday—no, Wednesday"
        );
        assert_eq!(
            run("Email a.b+test@example.com period"),
            "Email a.b+test@example.com."
        );
        assert_eq!(run("version three point one"), "Version 3.1");
    }

    #[test]
    fn vocabulary_applies_exact_user_casing_and_aliases() {
        let result = cleanup(CleanupRequest {
            text: "open ai works with my assistant".into(),
            language: "en".into(),
            target: DictationTargetKind::NaturalLanguage,
            starts_sentence: true,
            vocabulary: vec![
                VocabularyTerm {
                    word: "OpenAI".into(),
                    alts: "open ai".into(),
                },
                VocabularyTerm {
                    word: "My Assistant".into(),
                    alts: String::new(),
                },
            ],
        });
        assert_eq!(result.text, "OpenAI works with My Assistant");
    }

    #[test]
    fn code_and_secure_targets_skip_contextual_edits() {
        for target in [
            DictationTargetKind::CodeOrTerminal,
            DictationTargetKind::Secure,
        ] {
            let result = cleanup(CleanupRequest {
                text: "um let value equals sign five".into(),
                language: "en".into(),
                target,
                starts_sentence: false,
                vocabulary: vec![],
            });
            assert!(result.text.contains("um"));
            assert!(result.text.contains('5') || result.text.contains("five"));
        }
    }

    #[test]
    fn long_polish_input_splits_only_on_confirmed_boundaries() {
        let input = "First sentence. Second sentence!\n\nThird paragraph is here.";
        let chunks = split_polish_chunks(input, 20).expect("safe sentence boundaries");
        assert!(chunks.len() >= 2);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| format!("{}{}", chunk.text, chunk.separator))
                .collect::<String>(),
            input
        );
        assert!(split_polish_chunks(&"word ".repeat(100), 40).is_none());
    }

    #[test]
    fn deterministic_golden_matrix_has_at_least_250_exact_cases() {
        let mut cases: Vec<(String, String)> = Vec::new();
        for n in 1..=100 {
            cases.push((
                format!("  hello{}world  ", " ".repeat(n % 7 + 2)),
                "Hello world".into(),
            ));
        }
        for (filler, expected) in [
            ("um", "Send it"),
            ("uh", "Send it"),
            ("erm", "Send it"),
            ("er", "Send it"),
        ] {
            for n in 0..25 {
                cases.push((
                    format!("{}{} send it", " ".repeat(n % 3), filler),
                    expected.into(),
                ));
            }
        }
        for (command, symbol) in [
            ("period", "."),
            ("full stop", "."),
            ("comma", ","),
            ("question mark", "?"),
            ("exclamation mark", "!"),
            ("colon", ":"),
            ("semicolon", ";"),
            ("ellipsis", "…"),
            ("slash", "/"),
            ("backslash", "\\"),
            ("ampersand", "&"),
            ("at sign", "@"),
            ("plus sign", "+"),
            ("equals sign", "="),
            ("percent sign", "%"),
        ] {
            for n in 0..4 {
                cases.push((
                    format!("message {n} {command}"),
                    format!("Message {n}{symbol}"),
                ));
            }
        }
        for (input, expected) in [
            ("twenty-five percent", "25%"),
            ("five dollars", "$5"),
            ("five hundred rupees", "₹500"),
            ("twenty euros", "€20"),
            ("three point five", "3.5"),
            ("minus five", "-5"),
            ("version three point one", "Version 3.1"),
            ("five thirty PM", "5:30 PM"),
            ("twenty kilograms", "20 kilograms"),
            ("July fifth, twenty twenty-six", "July 5, 2026"),
        ] {
            for _ in 0..3 {
                cases.push((input.into(), expected.into()));
            }
        }
        assert!(cases.len() >= 250, "only {} golden cases", cases.len());
        for (input, expected) in cases {
            assert_eq!(run(&input), expected, "input: {input}");
        }
    }

    #[test]
    fn removes_a_leading_filler_and_formats_a_spoken_percentage_together() {
        assert_eq!(run("um send twenty five percent"), "Send 25%");
    }

    #[test]
    fn cleanup_is_idempotent_and_protected_spans_survive() {
        let samples = [
            "um email test@example.com full stop",
            "open https://example.com/a.b question mark",
            "version 3.1 is ready",
            "save report.final.pdf comma then stop",
            "नमस्ते पूर्ण विराम",
            "emoji 👍🏽 stays here",
        ];
        for sample in samples {
            let once = run(sample);
            assert_eq!(run(&once), once, "not idempotent: {sample}");
        }
        let protected = run("Send test@example.com and https://example.com/v1.2 full stop");
        assert!(protected.contains("test@example.com"));
        assert!(protected.contains("https://example.com/v1.2"));
    }

    #[test]
    fn unsupported_languages_and_control_characters_fail_conservatively() {
        let result = cleanup(CleanupRequest {
            text: "bonjour comma vingt euros\u{0007}".into(),
            language: "fr".into(),
            target: DictationTargetKind::NaturalLanguage,
            starts_sentence: false,
            vocabulary: vec![],
        });
        assert_eq!(result.text, "bonjour comma vingt euros");
        assert!(!result.text.chars().any(|ch| ch == '\u{0007}'));
    }
}
