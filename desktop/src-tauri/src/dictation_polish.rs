use crate::dictation_cleanup::{split_polish_chunks, ProtectedSpan, VocabularyTerm};
use crate::{polish_runtime, polish_validator};
use std::time::Instant;

const MAX_CHUNK_CHARS: usize = 1_200;

pub struct PolishOutcome {
    pub text: String,
    pub accepted: bool,
    pub fallback: bool,
    pub decision: String,
}

pub fn run(
    mechanical: &str,
    model_id: &str,
    protected: &[ProtectedSpan],
    vocabulary: &[VocabularyTerm],
) -> PolishOutcome {
    if mechanical.trim().is_empty() {
        return PolishOutcome {
            text: mechanical.to_string(),
            accepted: false,
            fallback: true,
            decision: "empty_mechanical_result".into(),
        };
    }
    let Some(chunks) = split_polish_chunks(mechanical, MAX_CHUNK_CHARS) else {
        return PolishOutcome {
            text: mechanical.to_string(),
            accepted: false,
            fallback: true,
            decision: "long_input_without_safe_boundary".into(),
        };
    };
    let deadline = Instant::now() + polish_runtime::HARD_TIMEOUT;
    let total = chunks.len();
    let mut accepted = 0usize;
    let mut fallback = 0usize;
    let mut last_reason = String::new();
    let mut output = String::with_capacity(mechanical.len());
    let mut deadline_reached = false;

    for chunk in chunks {
        let chosen = if deadline_reached {
            fallback += 1;
            chunk.text.clone()
        } else {
            match polish_runtime::polish(&chunk.text, model_id, deadline) {
                Ok(proposal) => {
                    let validated =
                        polish_validator::validate(&chunk.text, &proposal, protected, vocabulary);
                    last_reason = validated.reason.clone();
                    if validated.accepted {
                        accepted += 1;
                        validated.text
                    } else {
                        fallback += 1;
                        chunk.text.clone()
                    }
                }
                Err(error) => {
                    deadline_reached = Instant::now() >= deadline;
                    last_reason = error;
                    fallback += 1;
                    chunk.text.clone()
                }
            }
        };
        output.push_str(&chosen);
        output.push_str(&chunk.separator);
    }

    PolishOutcome {
        text: output,
        accepted: accepted == total,
        fallback: fallback > 0,
        decision: if fallback == 0 {
            "accepted".into()
        } else if accepted > 0 {
            "partial_mechanical_fallback".into()
        } else if last_reason.is_empty() {
            "mechanical_fallback".into()
        } else {
            last_reason
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_runtime_preserves_the_complete_mechanical_result() {
        let input = "Keep every protected word in this mechanical result.";
        let outcome = run(input, "gemma-3-270m-it-qat-q4", &[], &[]);
        assert_eq!(outcome.text, input);
        assert!(outcome.fallback);
        assert!(!outcome.accepted);
    }
}
