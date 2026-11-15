//! Spoken symbol phrases. Applied only after Tier 0, and only in Terminal / Code mode.
//! Bare words such as "dash", "slash", and "one" are intentionally absent. They are ordinary English.

use regex::Regex;

struct Phrase {
    pattern: &'static str,
    replacement: &'static str,
}

const PHRASES: &[Phrase] = &[
    Phrase { pattern: r"(?i)\bforward\s+slash\b", replacement: "/" },
    Phrase { pattern: r"(?i)\bback\s*slash\b", replacement: "\\" },
    Phrase { pattern: r"(?i)\bunderscore\b", replacement: "_" },
    Phrase { pattern: r"(?i)\bopen\s+square\s+bracket\b", replacement: "[" },
    Phrase { pattern: r"(?i)\bclose\s+square\s+bracket\b", replacement: "]" },
    Phrase { pattern: r"(?i)\bopen\s+bracket\b", replacement: "[" },
    Phrase { pattern: r"(?i)\bclose\s+bracket\b", replacement: "]" },
];

pub fn apply_spoken_symbols(input: &str) -> String {
    let mut text = input.to_string();
    for phrase in PHRASES {
        let re = Regex::new(phrase.pattern).expect("symbol phrase regex");
        text = re.replace_all(&text, phrase.replacement).to_string();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_explicit_symbol_phrases() {
        let out = apply_spoken_symbols("cd forward slash var backslash tmp underscore name open bracket close bracket");
        assert_eq!(out, "cd / var \\ tmp _ name [ ]");
    }

    #[test]
    fn does_not_rewrite_ordinary_words() {
        let src = "run the one test then dash home and slash the price";
        assert_eq!(apply_spoken_symbols(src), src);
    }
}
