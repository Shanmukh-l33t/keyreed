//! Spoken symbol phrases. Applied only after Tier 0, and only in Terminal / Code mode.
//! Bare words such as "dash", "slash", and "one" are intentionally absent. They are ordinary English.

use once_cell::sync::Lazy;
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
    Phrase { pattern: r"(?i)\bopen\s+paren(?:thesis)?\b", replacement: "(" },
    Phrase { pattern: r"(?i)\bclose\s+paren(?:thesis)?\b", replacement: ")" },
    Phrase { pattern: r"(?i)\bopen\s+curly\s+brace\b", replacement: "{" },
    Phrase { pattern: r"(?i)\bclose\s+curly\s+brace\b", replacement: "}" },
];

/// Compiled once; the patterns are constants so the unwrap cannot fail at run time.
static COMPILED: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    PHRASES
        .iter()
        .map(|p| (Regex::new(p.pattern).expect("symbol phrase regex"), p.replacement))
        .collect()
});

pub fn apply_spoken_symbols(input: &str) -> String {
    let mut text = input.to_string();
    for (re, replacement) in COMPILED.iter() {
        text = re.replace_all(&text, *replacement).to_string();
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
