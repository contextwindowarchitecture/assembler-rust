//! Strings as the spec compares them: UTF-16 code-unit order, the ECMAScript whitespace set, blank strings and
//! the deduplication key (conformance/README.md, Ordering, Blank strings, Deduplication).

use std::cmp::Ordering;

/// ECMAScript whitespace and line terminators, spelled out: what JavaScript's `\s` matches. Rust's
/// `char::is_whitespace` differs (it includes U+0085 and leaves out U+FEFF), so the set is listed (Blank strings).
pub fn is_whitespace(c: char) -> bool {
    matches!(c,
        '\u{0009}'..='\u{000D}' | '\u{0020}' | '\u{00A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
        | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
}

/// A string is blank when every character in it is whitespace; the empty string is blank (Blank strings).
pub fn is_blank(s: &str) -> bool {
    s.chars().all(is_whitespace)
}

/// Orders strings by UTF-16 code units, shorter first when one is a prefix of the other (Ordering). Rust's
/// default `str` order compares code points, which differs beyond the Basic Multilingual Plane.
pub fn cmp_utf16(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// The deduplication key: every maximal run of whitespace replaced by one U+0020, then any U+0020 at either end
/// removed. No Unicode normalization and no case folding (Deduplication, step 1).
pub fn dedupe_key(body: &str) -> String {
    let mut key = String::with_capacity(body.len());
    let mut in_run = false;
    for c in body.chars() {
        if is_whitespace(c) {
            in_run = true;
        } else {
            if in_run && !key.is_empty() {
                key.push(' ');
            }
            in_run = false;
            key.push(c);
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_is_the_ecmascript_set() {
        for c in ['\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{a0}', '\u{1680}', '\u{2000}', '\u{200a}', '\u{2028}',
                  '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}', '\u{feff}'] {
            assert!(is_whitespace(c), "{:?}", c);
        }
        // Python's \s takes U+001C to U+001F; Rust's is_whitespace takes U+0085. ECMAScript takes neither.
        for c in ['\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', '\u{85}', '\u{200b}', '\u{180e}', 'a'] {
            assert!(!is_whitespace(c), "{:?}", c);
        }
    }

    #[test]
    fn blank_strings() {
        assert!(is_blank(""));
        assert!(is_blank("\u{feff}"));
        assert!(is_blank(" \t\u{3000}"));
        assert!(!is_blank("\u{1c}"));
        assert!(!is_blank(" a "));
    }

    #[test]
    fn utf16_order_differs_from_code_points_beyond_the_bmp() {
        // U+1F600 is the surrogate pair D83D DE00, below U+FF5A in code units and above it in code points.
        assert_eq!(cmp_utf16("\u{1F600}", "\u{FF5A}"), Ordering::Less);
        assert_eq!("\u{1F600}".cmp("\u{FF5A}"), Ordering::Greater);
        assert_eq!(cmp_utf16("ab", "abc"), Ordering::Less);
        assert_eq!(cmp_utf16("turn:10", "turn:9"), Ordering::Less);
        assert_eq!(cmp_utf16("a", "a"), Ordering::Equal);
    }

    #[test]
    fn dedupe_key_collapses_and_trims_without_normalizing() {
        assert_eq!(dedupe_key("  Refund\u{a0}\u{a0}within\n\t30 days  "), "Refund within 30 days");
        assert_eq!(dedupe_key("a\u{feff}b"), "a b");
        assert_ne!(dedupe_key("\u{e9}"), dedupe_key("e\u{301}"));
        assert_ne!(dedupe_key("Refund"), dedupe_key("refund"));
        assert_eq!(dedupe_key("a\u{1c}b"), "a\u{1c}b");
    }
}
