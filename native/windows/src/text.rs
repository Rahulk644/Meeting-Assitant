//! Transcript post-processing — a dependency-free port of the Swift `cleanTranscript` /
//! `keepExpressions` in `meeting-tap.swift`. **Fully portable and real.**
//!
//! Whisper hallucinates non-speech as bracketed annotations on quiet/noisy audio (`[BLANK_AUDIO]`,
//! `*sigh*`, `(music)`). `clean` strips those and drops anything left with no letters/digits;
//! `keep_expressions` instead rewrites `*x*`/`(x)` to `[x]` sentiment cues.

/// Remove every non-overlapping `open..close` span (inclusive of the delimiters), replacing each with
/// a single space. Handles `open == close` (the `*...*` case) by pairing delimiters left-to-right.
fn strip_pairs(input: &str, open: char, close: char) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    if open == close {
        // Toggle in/out on each delimiter; text between a pair is dropped.
        let mut inside = false;
        for c in chars.by_ref() {
            if c == open {
                if inside {
                    inside = false;
                    out.push(' ');
                } else {
                    inside = true;
                }
            } else if !inside {
                out.push(c);
            }
        }
        // An unterminated span: keep nothing extra (Swift regex would not match it, but this is safe).
        return out;
    }
    while let Some(c) = chars.next() {
        if c == open {
            // consume until matching close
            for d in chars.by_ref() {
                if d == close {
                    break;
                }
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    // If an open delimiter had no close, drop the remainder (already consumed) — matches "strip it".
    out
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_alnum(s: &str) -> bool {
    s.chars().any(|c| c.is_alphanumeric())
}

/// Strip `[...]`, `*...*`, `(...)`; collapse whitespace; return "" if nothing with a letter/digit
/// remains. Byte-faithful to the Swift `cleanTranscript` intent.
pub fn clean(raw: &str) -> String {
    let mut t = strip_pairs(raw, '[', ']');
    t = strip_pairs(&t, '*', '*');
    t = strip_pairs(&t, '(', ')');
    let t = collapse_ws(&t);
    if has_alnum(&t) {
        t
    } else {
        String::new()
    }
}

/// Keep paralinguistic cues: drop `[BLANK_AUDIO]`, rewrite `*x*`/`(x)` → `[x]`, collapse whitespace.
pub fn keep_expressions(raw: &str) -> String {
    let t = raw.replace("[BLANK_AUDIO]", "");
    let t = rewrite_pairs(&t, '*', '*');
    let t = rewrite_pairs(&t, '(', ')');
    collapse_ws(&t)
}

/// Rewrite each `open..close` span to `[inner]`.
fn rewrite_pairs(input: &str, open: char, close: char) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    if open == close {
        let mut inside = false;
        let mut inner = String::new();
        for c in chars.by_ref() {
            if c == open {
                if inside {
                    out.push('[');
                    out.push_str(&inner);
                    out.push(']');
                    inner.clear();
                    inside = false;
                } else {
                    inside = true;
                }
            } else if inside {
                inner.push(c);
            } else {
                out.push(c);
            }
        }
        if inside {
            // unterminated: emit the raw delimiter + collected text unchanged
            out.push(open);
            out.push_str(&inner);
        }
        return out;
    }
    while let Some(c) = chars.next() {
        if c == open {
            let mut inner = String::new();
            let mut closed = false;
            for d in chars.by_ref() {
                if d == close {
                    closed = true;
                    break;
                }
                inner.push(d);
            }
            if closed {
                out.push('[');
                out.push_str(&inner);
                out.push(']');
            } else {
                out.push(open);
                out.push_str(&inner);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_annotations() {
        assert_eq!(clean("[BLANK_AUDIO] hello (music) *sigh* world"), "hello world");
    }

    #[test]
    fn clean_drops_pure_noise() {
        assert_eq!(clean("[music]"), "");
        assert_eq!(clean(" - - . "), "");
    }

    #[test]
    fn keep_expressions_rewrites_cues() {
        assert_eq!(keep_expressions("well *sigh* okay (laughs)"), "well [sigh] okay [laughs]");
        assert_eq!(keep_expressions("[BLANK_AUDIO]hi"), "hi");
    }
}
