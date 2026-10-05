//! Tokenizers (conformance/README.md, Tokenizers and renderers; R-16).
//!
//! A tokenizer counts a text as a non-negative integer, reading nothing but that text (R-23). The two published
//! ones are built in; an application may pass its own under an id no published tokenizer uses.

use crate::strings::is_whitespace;

/// Counts the tokens of a text.
pub trait Tokenizer: Send + Sync {
    fn count(&self, text: &str) -> u64;
}

impl<F: Fn(&str) -> u64 + Send + Sync> Tokenizer for F {
    fn count(&self, text: &str) -> u64 {
        self(text)
    }
}

/// `fixture-whitespace/v1`: maximal runs of characters outside the ECMAScript whitespace set, what JavaScript's
/// `/\S+/gu` matches. A test fixture, not a model tokenizer.
pub struct FixtureWhitespace;

impl Tokenizer for FixtureWhitespace {
    fn count(&self, text: &str) -> u64 {
        let mut count = 0;
        let mut in_token = false;
        for c in text.chars() {
            let token_char = !is_whitespace(c);
            if token_char && !in_token {
                count += 1;
            }
            in_token = token_char;
        }
        count
    }
}

/// `estimate-utf8/v1`: the text's UTF-8 bytes divided by 4, rounded up, so an empty text counts 0.
pub struct EstimateUtf8;

impl Tokenizer for EstimateUtf8 {
    fn count(&self, text: &str) -> u64 {
        (text.len() as u64).div_ceil(4)
    }
}

/// The tokenizers this package provides, by id: every published one.
pub fn builtin(id: &str) -> Option<&'static dyn Tokenizer> {
    match id {
        "fixture-whitespace/v1" => Some(&FixtureWhitespace),
        "estimate-utf8/v1" => Some(&EstimateUtf8),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_whitespace_counts_ecmascript_non_whitespace_runs() {
        assert_eq!(FixtureWhitespace.count(""), 0);
        assert_eq!(FixtureWhitespace.count("  a  bc\td\n"), 3);
        assert_eq!(FixtureWhitespace.count("a\u{feff}b\u{a0}c"), 3);
        assert_eq!(FixtureWhitespace.count("a\u{1c}b"), 1);
        assert_eq!(FixtureWhitespace.count("a\u{85}b"), 1);
        assert_eq!(FixtureWhitespace.count("<x id=\"1\">\nbody text\n</x>\n"), 5);
    }

    #[test]
    fn estimate_utf8_rounds_bytes_over_four_up() {
        assert_eq!(EstimateUtf8.count(""), 0);
        assert_eq!(EstimateUtf8.count("abcd"), 1);
        assert_eq!(EstimateUtf8.count("abcde"), 2);
        assert_eq!(EstimateUtf8.count("é"), 1);
        assert_eq!(EstimateUtf8.count("日本語"), 3);
        assert_eq!(EstimateUtf8.count("\u{1F600}\u{1F600}"), 2);
    }
}
