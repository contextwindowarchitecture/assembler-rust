//! The adapter protocol of `scripts/conformance.py` (PORTING.md, step 5): one snapshot's bytes on stdin, and
//! the answer by exit code. 0: assembled, refusals included, with `{"payload": <base64 or null>, "trace": ...}` on
//! stdout. 2: rejected before assembly, the problems on stderr. 3: a component this package does not provide,
//! one `tokenizer <id> is not provided` or `renderer <id> is not provided` line each on stderr.

use std::io::{Read, Write};
use std::process::ExitCode;

use contextwindowarchitecture_assembler::{assemble, Error};
use serde_json::{json, Value};

fn main() -> ExitCode {
    let mut snapshot = Vec::new();
    std::io::stdin().read_to_end(&mut snapshot).expect("stdin is readable");
    match assemble(&snapshot) {
        Ok(assembly) => {
            let payload = assembly.payload.as_deref().map_or(Value::Null, |bytes| Value::String(base64(bytes)));
            let answer = json!({"payload": payload, "trace": assembly.trace.to_json()});
            std::io::stdout().write_all(answer.to_string().as_bytes()).expect("stdout is writable");
            ExitCode::SUCCESS
        }
        Err(Error::Rejected(problems)) => {
            eprintln!("{}", problems.join("\n"));
            ExitCode::from(2)
        }
        Err(Error::Unsupported(lines)) => {
            eprintln!("{}", lines.join("\n"));
            ExitCode::from(3)
        }
        Err(other) => {
            eprintln!("{other}");
            ExitCode::from(1)
        }
    }
}

/// Standard base64 with padding (RFC 4648, section 4).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc_4648() {
        for (input, output) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(super::base64(input.as_bytes()), output);
        }
    }
}
