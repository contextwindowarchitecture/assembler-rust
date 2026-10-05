//! RFC 8785 JSON canonicalization and SHA-256 hex digests (conformance/README.md, Snapshot digest, Registry,
//! Tokenizers and renderers).
//!
//! Members are sorted by UTF-16 code units, strings escape only what JSON requires, and every number is written
//! as ECMAScript's `Number.prototype.toString` writes the double it reads as.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::strings::cmp_utf16;

/// The RFC 8785 serialization of a JSON value.
pub fn to_string(value: &Value) -> String {
    let mut out = String::new();
    write(value, &mut out);
    out
}

/// Lowercase hex SHA-256 of some bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn write(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(n.as_f64().expect("every number is a finite double"))),
        Value::String(s) => string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| cmp_utf16(a, b));
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                string(key, out);
                out.push(':');
                write(&map[key], out);
            }
            out.push('}');
        }
    }
}

/// A JSON string as RFC 8785 writes it: `"` and `\` escaped, U+0008, U+0009, U+000A, U+000C and U+000D by their
/// short escapes, other controls as `\u00xx` in lowercase hex, and everything else as it is.
pub fn string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A finite double as ECMAScript's `Number.prototype.toString` writes it (ECMA-262, Number::toString), from the
/// shortest digits that round-trip, which Rust's `{:e}` formatting gives.
pub fn number(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    if x < 0.0 {
        return format!("-{}", number(-x));
    }
    let sci = format!("{x:e}");
    let (mantissa, exponent) = sci.split_once('e').expect("{:e} always has an exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exponent.parse::<i32>().expect("{:e} exponent is an integer") + 1;
    if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{sign}{}", e.abs())
        } else {
            format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], e.abs())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_format_as_ecmascript_does() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0"), (-0.0, "0"), (1.0, "1"), (-1.5, "-1.5"), (0.91, "0.91"), (100.0, "100"),
            (1e21, "1e+21"), (1e20, "100000000000000000000"), (123456789012345680000.0, "123456789012345680000"),
            (1e-6, "0.000001"), (1e-7, "1e-7"), (1.5e-7, "1.5e-7"), (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"), (9007199254740993.0, "9007199254740992"),
            (0.1 + 0.2, "0.30000000000000004"), (333333333.3333333, "333333333.3333333"), (4.5e15, "4500000000000000"),
            // RFC 8785, Appendix B.
            (f64::from_bits(0x0000000000000001), "5e-324"), (f64::from_bits(0x7fefffffffffffff), "1.7976931348623157e+308"),
            (f64::from_bits(0x4340000000000000), "9007199254740992"), (f64::from_bits(0x4430000000000000), "295147905179352830000"),
            (f64::from_bits(0x44b52d02c7e14af5), "9.999999999999997e+22"), (f64::from_bits(0x3eb0c6f7a0b5ed8d), "0.000001"),
            (f64::from_bits(0x3eb0c6f7a0b5ed8c), "9.999999999999997e-7"), (f64::from_bits(0x41b3de4355555555), "333333333.3333333"),
        ];
        for (x, expected) in cases {
            assert_eq!(number(*x), *expected, "{x:e}");
        }
    }

    #[test]
    fn members_sort_by_utf16_and_strings_escape_minimally() {
        let value = json!({"\u{FF5A}": 1, "\u{1F600}": 2, "b": [true, null, "\u{1}\u{1f}\"\\\u{2028}é"], "a": {}});
        assert_eq!(to_string(&value), "{\"a\":{},\"b\":[true,null,\"\\u0001\\u001f\\\"\\\\\u{2028}é\"],\"\u{1F600}\":2,\"\u{FF5A}\":1}");
        assert_eq!(to_string(&json!("\u{8}\t\n\u{c}\r\u{7f}")), "\"\\b\\t\\n\\f\\r\u{7f}\"");
    }

    #[test]
    fn sha256_is_lowercase_hex() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
