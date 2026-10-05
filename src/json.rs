//! Reading a snapshot's bytes as I-JSON (RFC 7493; conformance/README.md, Numbers, Snapshot checks).
//!
//! The text must be UTF-8 JSON with no unpaired surrogate escape and no number outside the double range, and
//! every number is then held as the nearest double, so that `9007199254740993` compares and serializes as
//! `9007199254740992` here as in every other language (R-2, R-17).

use serde_json::{Number, Value};

/// Parses JSON text, or says why it is not I-JSON.
pub fn parse(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("the snapshot is not UTF-8: {e}"))?;
    // serde_json refuses an unpaired surrogate escape ("lone leading surrogate", "unexpected end of hex escape")
    // and a number beyond the double range ("number out of range") rather than replacing them silently.
    let value: Value = serde_json::from_str(text).map_err(|e| format!("the snapshot is not I-JSON: {e}"))?;
    Ok(to_doubles(value))
}

/// Every number in a value replaced by the double it reads as.
pub fn to_doubles(value: Value) -> Value {
    match value {
        Value::Number(n) => {
            let x = n.as_f64().expect("serde_json numbers convert to f64");
            Value::Number(Number::from_f64(x).expect("parsed numbers are finite"))
        }
        Value::Array(items) => Value::Array(items.into_iter().map(to_doubles).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, to_doubles(v))).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_round_to_the_nearest_double() {
        let v = parse(b"[9007199254740993, 9007199254740992, 9007199254740991]").unwrap();
        let xs: Vec<f64> = v.as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect();
        assert_eq!(xs[0], xs[1]);
        assert!(xs[2] < xs[1]);
        assert_eq!(crate::canonical::to_string(&v), "[9007199254740992,9007199254740992,9007199254740991]");
    }

    #[test]
    fn rejects_what_has_no_rfc_8785_serialization() {
        let huge = format!("[1{}]", "0".repeat(400));
        for bad in [&br#"["\ud800"]"#[..], br#"["a\udc00"]"#, br#"["\ud800x"]"#, b"[1e400]", huge.as_bytes(), b"[-1e309]", b"\xff"] {
            assert!(parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        assert!(parse(b"[\"\\ud83d\\ude00\", 1e308, 123456789012345678901234567890]").is_ok());
    }
}
