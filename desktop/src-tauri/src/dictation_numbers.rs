use regex::Regex;
use std::sync::OnceLock;

const NUMBER_WORDS: &str = "zero|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|sixty|seventy|eighty|ninety|hundred|thousand|million|billion|lakh|crore|point|minus|oh|शून्य|एक|दो|तीन|चार|पाँच|पांच|छह|सात|आठ|नौ|दस|ग्यारह|बारह|तेरह|चौदह|पंद्रह|सोलह|सत्रह|अठारह|उन्नीस|बीस|तीस|चालीस|पचास|साठ|सत्तर|अस्सी|नब्बे|सौ|हजार|हज़ार|लाख|करोड़|दशमलव|माइनस|ek|do|teen|char|chaar|paanch|panch|chhe|saat|aath|nau|dus|gyarah|barah|bees|tees|chaalis|pachaas|sau|hazaar|hazar|karod";

fn number_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(&format!(
            r"(?iu)\b(?:{NUMBER_WORDS})(?:(?:[\s-]+)(?:{NUMBER_WORDS}))*\b"
        ))
        .expect("number-word regex")
    })
}

fn cardinal(word: &str) -> Option<i64> {
    Some(match word {
        "zero" | "oh" | "शून्य" => 0,
        "one" | "एक" | "ek" => 1,
        "two" | "दो" | "do" => 2,
        "three" | "तीन" | "teen" => 3,
        "four" | "चार" | "char" | "chaar" => 4,
        "five" | "पाँच" | "पांच" | "paanch" | "panch" => 5,
        "six" | "छह" | "chhe" => 6,
        "seven" | "सात" | "saat" => 7,
        "eight" | "आठ" | "aath" => 8,
        "nine" | "नौ" | "nau" => 9,
        "ten" | "दस" | "dus" => 10,
        "eleven" | "ग्यारह" | "gyarah" => 11,
        "twelve" | "बारह" | "barah" => 12,
        "thirteen" | "तेरह" => 13,
        "fourteen" | "चौदह" => 14,
        "fifteen" | "पंद्रह" => 15,
        "sixteen" | "सोलह" => 16,
        "seventeen" | "सत्रह" => 17,
        "eighteen" | "अठारह" => 18,
        "nineteen" | "उन्नीस" => 19,
        "twenty" | "बीस" | "bees" => 20,
        "thirty" | "तीस" | "tees" => 30,
        "forty" | "चालीस" | "chaalis" => 40,
        "fifty" | "पचास" | "pachaas" => 50,
        "sixty" | "साठ" => 60,
        "seventy" | "सत्तर" => 70,
        "eighty" | "अस्सी" => 80,
        "ninety" | "नब्बे" => 90,
        _ => return None,
    })
}

fn scale(word: &str) -> Option<i64> {
    Some(match word {
        "hundred" | "सौ" | "sau" => 100,
        "thousand" | "हजार" | "हज़ार" | "hazaar" | "hazar" => 1_000,
        "lakh" | "लाख" => 100_000,
        "million" => 1_000_000,
        "crore" | "करोड़" | "karod" => 10_000_000,
        "billion" => 1_000_000_000,
        _ => return None,
    })
}

fn parse_integer(words: &[&str]) -> Option<i64> {
    if words.is_empty() {
        return None;
    }
    let mut total = 0i64;
    let mut current = 0i64;
    let mut any = false;
    for word in words {
        if let Some(value) = cardinal(word) {
            current += value;
            any = true;
        } else if let Some(multiplier) = scale(word) {
            any = true;
            if multiplier == 100 {
                current = current.max(1) * multiplier;
            } else {
                total += current.max(1) * multiplier;
                current = 0;
            }
        } else {
            return None;
        }
    }
    any.then_some(total + current)
}

fn parse_number(raw: &str) -> Option<String> {
    let normalized = raw.to_lowercase().replace('-', " ");
    let mut words = normalized.split_whitespace().collect::<Vec<_>>();
    let negative = matches!(words.first(), Some(&"minus" | &"माइनस"));
    if negative {
        words.remove(0);
    }
    let point = words
        .iter()
        .position(|word| matches!(*word, "point" | "दशमलव"));
    let value = if let Some(index) = point {
        let whole = parse_integer(&words[..index])?;
        let decimals = words[index + 1..]
            .iter()
            .map(|word| cardinal(word).filter(|value| *value < 10))
            .collect::<Option<Vec<_>>>()?;
        if decimals.is_empty() {
            return None;
        }
        format!(
            "{whole}.{}",
            decimals.iter().map(i64::to_string).collect::<String>()
        )
    } else if words.len() >= 3 && words[0] == "twenty" && words[1] == "twenty" {
        format!("{}", 2_000 + parse_integer(&words[1..])?)
    } else {
        parse_integer(&words)?.to_string()
    };
    Some(if negative { format!("-{value}") } else { value })
}

fn context_is_numeric(text: &str, start: usize, end: usize, raw: &str) -> bool {
    let prefix = text[..start].to_lowercase();
    let suffix = text[end..].trim_start().to_lowercase();
    let words = raw.replace('-', " ").split_whitespace().count();
    let explicit_before = ["version ", "phone number ", "mobile number "]
        .iter()
        .any(|value| prefix.ends_with(value));
    let explicit_after = [
        "percent",
        "प्रतिशत",
        "%",
        "dollar",
        "dollars",
        "rupee",
        "rupees",
        "रुपया",
        "रुपये",
        "euro",
        "euros",
        "kilogram",
        "kilograms",
        "kg",
        "meter",
        "meters",
        "metre",
        "metres",
        "pm",
        "am",
    ]
    .iter()
    .any(|value| suffix.starts_with(value));
    if raw.to_lowercase().contains("-on-")
        || (raw.eq_ignore_ascii_case("hundred") && prefix.ends_with("a "))
    {
        return false;
    }
    explicit_before
        || explicit_after
        || words > 1
        || raw.to_lowercase().starts_with("minus ")
        || raw.to_lowercase().contains(" point ")
        || raw.contains("दशमलव")
}

fn normalize_ordinals(text: &str) -> String {
    static ORDINAL: OnceLock<Regex> = OnceLock::new();
    ORDINAL
        .get_or_init(|| {
            Regex::new(r"(?i)\b(January|February|March|April|May|June|July|August|September|October|November|December)\s+(first|second|third|fourth|fifth|sixth|seventh|eighth|ninth|tenth|eleventh|twelfth|thirteenth|fourteenth|fifteenth|sixteenth|seventeenth|eighteenth|nineteenth|twentieth|twenty[- ]first|twenty[- ]second|twenty[- ]third|twenty[- ]fourth|twenty[- ]fifth|twenty[- ]sixth|twenty[- ]seventh|twenty[- ]eighth|twenty[- ]ninth|thirtieth|thirty[- ]first)\b").unwrap()
        })
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let ordinal = captures[2].to_lowercase().replace('-', " ");
            let value = match ordinal.as_str() {
                "first" => 1, "second" => 2, "third" => 3, "fourth" => 4, "fifth" => 5,
                "sixth" => 6, "seventh" => 7, "eighth" => 8, "ninth" => 9, "tenth" => 10,
                "eleventh" => 11, "twelfth" => 12, "thirteenth" => 13, "fourteenth" => 14,
                "fifteenth" => 15, "sixteenth" => 16, "seventeenth" => 17, "eighteenth" => 18,
                "nineteenth" => 19, "twentieth" => 20, "twenty first" => 21, "twenty second" => 22,
                "twenty third" => 23, "twenty fourth" => 24, "twenty fifth" => 25,
                "twenty sixth" => 26, "twenty seventh" => 27, "twenty eighth" => 28,
                "twenty ninth" => 29, "thirtieth" => 30, "thirty first" => 31, _ => 0,
            };
            format!("{} {value}", &captures[1])
        })
        .into_owned()
}

fn normalize_phone_numbers(text: &str) -> String {
    static PHONE: OnceLock<Regex> = OnceLock::new();
    PHONE
        .get_or_init(|| {
            Regex::new(
                r"(?iu)\b(phone number|mobile number)\s+((?:zero|oh|one|two|three|four|five|six|seven|eight|nine|शून्य|एक|दो|तीन|चार|पाँच|पांच|छह|सात|आठ|नौ|ek|do|teen|char|chaar|paanch|panch|chhe|saat|aath|nau)(?:\s+(?:zero|oh|one|two|three|four|five|six|seven|eight|nine|शून्य|एक|दो|तीन|चार|पाँच|पांच|छह|सात|आठ|नौ|ek|do|teen|char|chaar|paanch|panch|chhe|saat|aath|nau)){5,})",
            )
            .unwrap()
        })
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let digits = captures[2]
                .split_whitespace()
                .filter_map(cardinal)
                .map(|value| value.to_string())
                .collect::<String>();
            format!("{} {digits}", &captures[1])
        })
        .into_owned()
}

fn normalize_time(text: &str) -> String {
    static TIME: OnceLock<Regex> = OnceLock::new();
    TIME
        .get_or_init(|| {
            Regex::new(r"(?iu)\b(zero|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)\s+(twenty|thirty|forty|fifty)(?:[- ](one|two|three|four|five|six|seven|eight|nine))?\s+(AM|PM)\b").unwrap()
        })
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let hour = cardinal(&captures[1]).unwrap_or(0);
            let minute = cardinal(&captures[2]).unwrap_or(0)
                + captures.get(3).and_then(|value| cardinal(value.as_str())).unwrap_or(0);
            format!("{hour}:{minute:02} {}", captures[4].to_uppercase())
        })
        .into_owned()
}

pub fn normalize_numbers(text: &str, language: &str) -> String {
    let language = language.to_ascii_lowercase();
    if !(language.is_empty()
        || language == "auto"
        || language.starts_with("en")
        || language.starts_with("hi")
        || language.contains("hinglish"))
    {
        return text.to_string();
    }
    let text = normalize_time(&normalize_phone_numbers(&normalize_ordinals(text)));
    let regex = number_regex();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for found in regex.find_iter(&text) {
        out.push_str(&text[cursor..found.start()]);
        let raw = found.as_str();
        if context_is_numeric(&text, found.start(), found.end(), raw) {
            out.push_str(parse_number(raw).as_deref().unwrap_or(raw));
        } else {
            out.push_str(raw);
        }
        cursor = found.end();
    }
    out.push_str(&text[cursor..]);

    static MONEY_PERCENT: OnceLock<Regex> = OnceLock::new();
    MONEY_PERCENT
        .get_or_init(|| {
            Regex::new(
                r"(?iu)\b(-?\d+(?:\.\d+)?)\s+(percent|प्रतिशत|dollars?|rupees?|रुपया|रुपये|euros?)\b",
            )
            .unwrap()
        })
        .replace_all(&out, |captures: &regex::Captures<'_>| {
            let value = &captures[1];
            match captures[2].to_lowercase().as_str() {
                "percent" | "प्रतिशत" => format!("{value}%"),
                "dollar" | "dollars" => format!("${value}"),
                "rupee" | "rupees" | "रुपया" | "रुपये" => format!("₹{value}"),
                "euro" | "euros" => format!("€{value}"),
                _ => captures[0].to_string(),
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_only_high_confidence_numbers() {
        assert_eq!(normalize_numbers("twenty-five percent", "en"), "25%");
        assert_eq!(normalize_numbers("five hundred rupees", "en"), "₹500");
        assert_eq!(normalize_numbers("three point five", "en"), "3.5");
        assert_eq!(normalize_numbers("minus five", "en"), "-5");
        assert_eq!(
            normalize_numbers("version three point one", "en"),
            "version 3.1"
        );
        assert_eq!(normalize_numbers("five thirty PM", "en"), "5:30 PM");
        assert_eq!(
            normalize_numbers("July fifth, twenty twenty-six", "en"),
            "July 5, 2026"
        );
    }

    #[test]
    fn preserves_ambiguous_language() {
        assert_eq!(
            normalize_numbers("You are the one", "en"),
            "You are the one"
        );
        assert_eq!(
            normalize_numbers("A hundred reasons", "en"),
            "A hundred reasons"
        );
        assert_eq!(
            normalize_numbers("One-on-one meeting", "en"),
            "One-on-one meeting"
        );
        assert_eq!(
            normalize_numbers("five to ten people", "en"),
            "five to ten people"
        );
    }

    #[test]
    fn supports_hindi_and_hinglish_numeric_contexts() {
        assert_eq!(normalize_numbers("पाँच सौ रुपये", "hi"), "₹500");
        assert_eq!(normalize_numbers("paanch sau rupees", "hinglish"), "₹500");
    }
}
