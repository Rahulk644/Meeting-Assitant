use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSpan {
    pub original: String,
    pub replacement: String,
}

fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut word = None;
    for ch in text.chars() {
        let next_word = ch.is_alphanumeric() || ch == '\'' || ch == '’';
        if word == Some(next_word) {
            current.push(ch);
        } else {
            if !current.is_empty() && !current.chars().all(char::is_whitespace) {
                out.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            current.push(ch);
            word = Some(next_word);
        }
    }
    if !current.is_empty() && !current.chars().all(char::is_whitespace) {
        out.push(current);
    }
    out
}

fn join(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts {
        let punctuation = part
            .chars()
            .all(|c| !c.is_alphanumeric() && !c.is_whitespace())
            && !matches!(part.as_str(), "(" | "[" | "{" | "“" | "\"");
        if !out.is_empty() && !punctuation && !out.ends_with(['(', '[', '{', '“', '\'', '’']) {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

// A small LCS is enough for the pill and trace. Inputs are bounded before reaching this layer; cap the
// matrix so a pathological long dictation can never turn UI diffing into quadratic work.
pub fn diff_spans(original: &str, replacement: &str) -> Vec<DiffSpan> {
    if original == replacement {
        return Vec::new();
    }
    let left = tokens(original);
    let right = tokens(replacement);
    if left.len() > 256 || right.len() > 256 {
        return vec![DiffSpan {
            original: original.chars().take(96).collect(),
            replacement: replacement.chars().take(96).collect(),
        }];
    }
    let mut lcs = vec![vec![0u16; right.len() + 1]; left.len() + 1];
    for i in (0..left.len()).rev() {
        for j in (0..right.len()).rev() {
            lcs[i][j] = if left[i] == right[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut changes = Vec::new();
    let flush =
        |removed: &mut Vec<String>, added: &mut Vec<String>, changes: &mut Vec<DiffSpan>| {
            if removed.is_empty() && added.is_empty() {
                return;
            }
            changes.push(DiffSpan {
                original: join(removed),
                replacement: join(added),
            });
            removed.clear();
            added.clear();
        };
    while i < left.len() || j < right.len() {
        if i < left.len() && j < right.len() && left[i] == right[j] {
            flush(&mut removed, &mut added, &mut changes);
            i += 1;
            j += 1;
        } else if j < right.len() && (i == left.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            added.push(right[j].clone());
            j += 1;
        } else if i < left.len() {
            removed.push(left[i].clone());
            i += 1;
        }
    }
    flush(&mut removed, &mut added, &mut changes);
    changes
}

pub fn most_meaningful(changes: &[DiffSpan]) -> Option<&DiffSpan> {
    changes.iter().max_by_key(|change| {
        change.original.chars().count() + change.replacement.chars().count() * 2
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_finds_separate_changes() {
        let changes = diff_spans("um send twenty five dollars", "Send $25.");
        assert!(!changes.is_empty());
        assert!(changes.iter().any(|change| change.original.contains("um")));
    }

    #[test]
    fn equal_text_has_no_fabricated_change() {
        assert!(diff_spans("already right", "already right").is_empty());
    }

    #[test]
    fn additions_and_removals_keep_an_explicit_empty_side_for_the_overlay() {
        let addition = diff_spans("send it", "please send it");
        assert!(addition
            .iter()
            .any(|change| change.original.is_empty() && change.replacement == "please"));

        let removal = diff_spans("um send it", "send it");
        assert!(removal
            .iter()
            .any(|change| change.original == "um" && change.replacement.is_empty()));
    }
}
