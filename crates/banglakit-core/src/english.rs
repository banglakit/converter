//! Embedded top English wordlist used by the classifier's dictionary
//! coverage feature.
//!
//! Source: first20hours/google-10000-english (MIT). We embed the top ~3,000
//! entries which is plenty to distinguish English prose from Bijoy-encoded
//! text, where tokenized "words" like `evsjvq`, `Avwg`, `†k‡l` have zero
//! coverage in English.

use once_cell::sync::OnceCell;
use std::collections::HashSet;

const RAW: &str = include_str!("../data/english_words.txt");

static SET: OnceCell<HashSet<&'static str>> = OnceCell::new();

fn set() -> &'static HashSet<&'static str> {
    SET.get_or_init(|| RAW.lines().filter(|l| !l.is_empty()).collect())
}

/// Lower-case `token` (ASCII fast path) and check membership in the embedded
/// English wordlist.
pub fn contains(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    // ASCII fast path: most English tokens are pure ASCII letters.
    if token.bytes().all(|b| b.is_ascii_alphabetic()) {
        let lower = token.to_ascii_lowercase();
        set().contains(lower.as_str())
    } else {
        let lower = token.to_lowercase();
        set().contains(lower.as_str())
    }
}

/// Tokenize on ASCII whitespace + a small punctuation set; returns owned
/// strings stripped of leading/trailing ASCII punctuation. Tokens of length
/// < 3 are filtered out (they're too small to distinguish English from
/// random Latin noise).
pub fn tokenize(text: &str) -> Vec<&str> {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '.' | ',' | ';' | ':' | '!' | '?' | '(' | ')' | '[' | ']'
                    | '{' | '}' | '<' | '>' | '"' | '\'' | '/' | '\\'
                    | '|' | '`'
            )
    })
    .filter(|t| t.len() >= 3)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_top_words() {
        assert!(contains("the"));
        assert!(contains("price"));
        assert!(contains("Today"));
        assert!(!contains("evsjvq"));
    }

    #[test]
    fn tokenize_filters_short_and_punct() {
        let toks = tokenize("Gas price is $5 today!");
        // "is" is length 2 → filtered. "$5" trims to "$5", length 2 → filtered.
        // Remaining: Gas, price, today
        assert_eq!(toks, vec!["Gas", "price", "today"]);
    }
}
