use regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
struct Command {
    phrase: &'static str,
    output: &'static str,
}

const PUNCTUATION: &[Command] = &[
    Command {
        phrase: "exclamation point",
        output: "!",
    },
    Command {
        phrase: "exclamation mark",
        output: "!",
    },
    Command {
        phrase: "close parenthesis",
        output: ")",
    },
    Command {
        phrase: "open parenthesis",
        output: "(",
    },
    Command {
        phrase: "question mark",
        output: "?",
    },
    Command {
        phrase: "close bracket",
        output: "]",
    },
    Command {
        phrase: "open bracket",
        output: "[",
    },
    Command {
        phrase: "close brace",
        output: "}",
    },
    Command {
        phrase: "open brace",
        output: "{",
    },
    Command {
        phrase: "forward slash",
        output: "/",
    },
    Command {
        phrase: "close quote",
        output: "”",
    },
    Command {
        phrase: "open quote",
        output: "“",
    },
    Command {
        phrase: "dot dot dot",
        output: "…",
    },
    Command {
        phrase: "full stop",
        output: ".",
    },
    Command {
        phrase: "hash sign",
        output: "#",
    },
    Command {
        phrase: "at sign",
        output: "@",
    },
    Command {
        phrase: "plus sign",
        output: "+",
    },
    Command {
        phrase: "equals sign",
        output: "=",
    },
    Command {
        phrase: "percent sign",
        output: "%",
    },
    Command {
        phrase: "bullet point",
        output: "\n• ",
    },
    Command {
        phrase: "new bullet",
        output: "\n• ",
    },
    Command {
        phrase: "new paragraph",
        output: "\n\n",
    },
    Command {
        phrase: "next line",
        output: "\n",
    },
    Command {
        phrase: "new line",
        output: "\n",
    },
    Command {
        phrase: "ampersand",
        output: "&",
    },
    Command {
        phrase: "backslash",
        output: "\\",
    },
    Command {
        phrase: "apostrophe",
        output: "'",
    },
    Command {
        phrase: "semicolon",
        output: ";",
    },
    Command {
        phrase: "ellipsis",
        output: "…",
    },
    Command {
        phrase: "period",
        output: ".",
    },
    Command {
        phrase: "comma",
        output: ",",
    },
    Command {
        phrase: "colon",
        output: ":",
    },
    Command {
        phrase: "hyphen",
        output: "-",
    },
    Command {
        phrase: "hashtag",
        output: "#",
    },
    Command {
        phrase: "slash",
        output: "/",
    },
    Command {
        phrase: "dash",
        output: "—",
    },
    Command {
        phrase: "tab",
        output: "\t",
    },
];

const HINDI: &[Command] = &[
    Command {
        phrase: "विस्मयादिबोधक चिन्ह",
        output: "!",
    },
    Command {
        phrase: "प्रश्न चिन्ह",
        output: "?",
    },
    Command {
        phrase: "पूर्ण विराम",
        output: ".",
    },
    Command {
        phrase: "अल्प विराम",
        output: ",",
    },
    Command {
        phrase: "नया पैराग्राफ",
        output: "\n\n",
    },
    Command {
        phrase: "नई पंक्ति",
        output: "\n",
    },
];

const HINGLISH: &[Command] = &[
    Command {
        phrase: "naya paragraph",
        output: "\n\n",
    },
    Command {
        phrase: "nayi line",
        output: "\n",
    },
    Command {
        phrase: "sawal ka nishan",
        output: "?",
    },
];

fn command_regex(command: &'static Command) -> &'static Regex {
    static CACHE: OnceLock<Vec<Regex>> = OnceLock::new();
    let all = CACHE.get_or_init(|| {
        PUNCTUATION
            .iter()
            .chain(HINDI)
            .chain(HINGLISH)
            .map(|item| {
                Regex::new(&format!(r"(?iu)\b{}\b", regex::escape(item.phrase)))
                    .expect("command regex")
            })
            .collect()
    });
    let index = PUNCTUATION
        .iter()
        .chain(HINDI)
        .chain(HINGLISH)
        .position(|candidate| std::ptr::eq(candidate, command))
        .expect("registered command");
    &all[index]
}

fn denied_literal_context(before: &str, phrase: &str) -> bool {
    let tail = before
        .chars()
        .rev()
        .take(36)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>()
        .to_lowercase();
    let generic = ["literal ", "word ", "term ", "say ", "write ", "spell "];
    if generic.iter().any(|prefix| tail.ends_with(prefix)) {
        return true;
    }
    match phrase {
        "period" => tail.ends_with("trial ") || tail.ends_with("time "),
        "comma" => tail.ends_with("oxford ") || tail.ends_with("a ") || tail.ends_with("the "),
        "question mark" | "exclamation mark" | "exclamation point" => {
            tail.ends_with("the ") || tail.ends_with("a ")
        }
        _ => false,
    }
}

fn plausible(text: &str, start: usize, phrase: &str) -> bool {
    let before = &text[..start];
    let can_start = matches!(phrase, "bullet point" | "new bullet");
    if (before.trim().is_empty() && !can_start) || denied_literal_context(before, phrase) {
        return false;
    }
    true
}

fn replace_one(text: &str, command: &'static Command) -> String {
    let regex = command_regex(command);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for found in regex.find_iter(text) {
        if !plausible(text, found.start(), command.phrase) {
            continue;
        }
        out.push_str(&text[cursor..found.start()]);
        let attaches_left = !command.output.is_empty()
            && !command.output.chars().next().unwrap().is_whitespace()
            && !matches!(command.output, "(" | "[" | "{" | "“");
        if attaches_left {
            while out.ends_with(char::is_whitespace) {
                out.pop();
            }
        }
        out.push_str(command.output);
        cursor = found.end();
    }
    if cursor == 0 {
        text.to_string()
    } else {
        out.push_str(&text[cursor..]);
        out
    }
}

fn protect_literals(text: &str) -> (String, Vec<(String, String)>) {
    static LITERAL: OnceLock<Regex> = OnceLock::new();
    let regex = LITERAL.get_or_init(|| {
        let phrases = PUNCTUATION
            .iter()
            .map(|command| regex::escape(command.phrase))
            .chain(
                [
                    "numbered item",
                    "caps on",
                    "caps off",
                    "all caps",
                    "capitalize",
                    "no space",
                ]
                .into_iter()
                .map(regex::escape),
            )
            .collect::<Vec<_>>()
            .join("|");
        Regex::new(&format!(
            r"(?iu)\bliteral\s+({phrases}|[\p{{L}}\p{{N}}_\-]+)\b"
        ))
        .expect("literal command regex")
    });
    let mut values = Vec::new();
    let result = regex
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let token = format!("MAZXQLITERAL{}TOKEN", values.len());
            values.push((token.clone(), captures[1].to_string()));
            token
        })
        .into_owned();
    (result, values)
}

fn uppercase_first(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_uppercase().chain(chars).collect()
}

fn apply_formatting_commands(text: &str) -> String {
    static CAPS_BLOCK: OnceLock<Regex> = OnceLock::new();
    static ALL_CAPS: OnceLock<Regex> = OnceLock::new();
    static CAPITALIZE: OnceLock<Regex> = OnceLock::new();
    static NO_SPACE: OnceLock<Regex> = OnceLock::new();
    static NUMBERED: OnceLock<Regex> = OnceLock::new();

    let value = CAPS_BLOCK
        .get_or_init(|| Regex::new(r"(?isu)\bcaps\s+on\b(.*?)\bcaps\s+off\b").unwrap())
        .replace_all(text, |captures: &regex::Captures<'_>| {
            captures[1].to_uppercase()
        })
        .into_owned();
    let value = ALL_CAPS
        .get_or_init(|| Regex::new(r"(?iu)\ball\s+caps\s+([\p{L}\p{N}_\-]+)\b").unwrap())
        .replace_all(&value, |captures: &regex::Captures<'_>| {
            captures[1].to_uppercase()
        })
        .into_owned();
    let value = CAPITALIZE
        .get_or_init(|| Regex::new(r"(?iu)\bcapitalize\s+([\p{L}\p{N}_\-]+)\b").unwrap())
        .replace_all(&value, |captures: &regex::Captures<'_>| {
            uppercase_first(&captures[1])
        })
        .into_owned();
    let value = NO_SPACE
        .get_or_init(|| Regex::new(r"(?iu)\s+\bno\s+space\b\s+").unwrap())
        .replace_all(&value, "")
        .into_owned();
    let mut index = 0usize;
    NUMBERED
        .get_or_init(|| Regex::new(r"(?iu)\b(?:new\s+)?numbered\s+item\b").unwrap())
        .replace_all(&value, |_captures: &regex::Captures<'_>| {
            index += 1;
            format!("\n{index}. ")
        })
        .into_owned()
}

pub fn apply_commands(text: &str, language: &str) -> String {
    let (mut result, literals) = protect_literals(text);
    let lang = language.to_ascii_lowercase();
    let english =
        lang.is_empty() || lang == "auto" || lang.starts_with("en") || lang.contains("hinglish");
    let hindi = lang == "auto" || lang.starts_with("hi") || lang.contains("hinglish");
    if english {
        for command in PUNCTUATION {
            result = replace_one(&result, command);
        }
    }
    if hindi {
        for command in HINDI.iter().chain(HINGLISH) {
            result = replace_one(&result, command);
        }
    }
    if english {
        result = apply_formatting_commands(&result);
    }
    for (token, literal) in literals {
        result = result.replace(&token, &literal);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_plausible_commands_and_protects_literal_words() {
        assert_eq!(
            apply_commands("Hello comma how are you question mark", "en"),
            "Hello, how are you?"
        );
        assert_eq!(
            apply_commands("The trial period is short", "en"),
            "The trial period is short"
        );
        assert_eq!(
            apply_commands("Use an Oxford comma", "en"),
            "Use an Oxford comma"
        );
        assert_eq!(apply_commands("Say literal period", "en"), "Say period");
    }

    #[test]
    fn supports_hindi_and_hinglish_formatting() {
        assert_eq!(apply_commands("नमस्ते पूर्ण विराम", "hi"), "नमस्ते.");
        assert_eq!(
            apply_commands("पहली लाइन nayi line दूसरी लाइन", "hinglish"),
            "पहली लाइन \n दूसरी लाइन"
        );
    }

    #[test]
    fn applies_explicit_case_spacing_and_list_commands() {
        assert_eq!(
            apply_commands(
                "capitalize hello all caps api caps on very important caps off",
                "en"
            ),
            "Hello API  VERY IMPORTANT "
        );
        assert_eq!(apply_commands("file no space name", "en"), "filename");
        assert_eq!(
            apply_commands("numbered item first numbered item second", "en"),
            "\n1.  first \n2.  second"
        );
        assert_eq!(apply_commands("bullet point first", "en"), "\n•  first");
        assert_eq!(
            apply_commands("say literal capitalize hello", "en"),
            "say capitalize hello"
        );
    }
}
