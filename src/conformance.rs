//! Running the vendored conformance cases and writing the report (conformance/README.md, Running a case,
//! Reporting results). The `conformance` example and the test suite both use it.

use std::fs;
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::strings::cmp_utf16;
use crate::{assemble, contract, schema, Error, IMPLEMENTATION_LANGUAGE, IMPLEMENTATION_NAME, IMPLEMENTATION_VERSION};

/// One case's or rejection's outcome, with what differed when it did not pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub outcome: &'static str,
    pub detail: Option<String>,
}

impl Outcome {
    fn new(outcome: &'static str) -> Outcome {
        Outcome { outcome, detail: None }
    }

    fn with(outcome: &'static str, detail: impl Into<String>) -> Outcome {
        Outcome { outcome, detail: Some(detail.into()) }
    }
}

/// The case or rejection ids under a directory, ordered by id.
pub fn ids(dir: &Path) -> Vec<String> {
    let mut ids: Vec<String> = fs::read_dir(dir).map(|entries| {
        entries.filter_map(Result::ok).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()
    }).unwrap_or_default();
    ids.sort_by(|a, b| cmp_utf16(a, b));
    ids
}

/// Runs one case: assemble, validate the trace, compare the payload bytes and the trace without `trace_id`,
/// `timings` and `recovery.detail`.
pub fn run_case(dir: &Path) -> Outcome {
    let snapshot = fs::read(dir.join("snapshot.json")).expect("a case has a snapshot");
    let assembly = match assemble(&snapshot) {
        Ok(assembly) => assembly,
        Err(Error::Unsupported(lines)) => return unsupported(&snapshot, &lines, &["tokenizer", "renderer"]),
        Err(Error::Rejected(problems)) => {
            return Outcome::with("failed", format!("rejected the snapshot where the case expects an assembly: {}", problems.join("; ")))
        }
        Err(other) => return Outcome::with("failed", other.to_string()),
    };
    let trace = assembly.trace.to_json();
    if let Some(error) = schema::validate("trace.schema.json", &trace).first() {
        return Outcome::with("failed", format!("the trace fails trace.schema.json at {error}"));
    }
    let expected_payload = fs::read(dir.join("expected.payload.txt")).ok();
    match (&expected_payload, &assembly.payload) {
        (None, Some(_)) => return Outcome::with("failed", "rendered a payload where the case expects a refusal"),
        (Some(_), None) => {
            let reason = trace["refused"]["reason"].as_str().unwrap_or("unknown");
            return Outcome::with("failed", format!("refused with {reason} where the case expects a payload"));
        }
        (Some(expected), Some(actual)) if expected != actual => {
            return Outcome::with("failed", "the payload bytes differ from expected.payload.txt")
        }
        _ => {}
    }
    let expected: Value = serde_json::from_slice(&fs::read(dir.join("expected.trace.json")).expect("a case has a trace"))
        .expect("an expected trace is JSON");
    match first_difference(&comparable(&expected), &comparable(&trace), "") {
        Some(difference) => Outcome::with("failed", format!("the trace differs at {difference}")),
        None => Outcome::new("passed"),
    }
}

/// Runs one rejection case: the snapshot must be rejected before assembly, with no payload and no trace.
pub fn run_rejection(dir: &Path) -> Outcome {
    let snapshot = fs::read(dir.join("snapshot.json")).expect("a rejection has a snapshot");
    match assemble(&snapshot) {
        Err(Error::Rejected(_)) => Outcome::new("rejected"),
        Err(Error::Unsupported(lines)) => unsupported(&snapshot, &lines, &["renderer"]),
        Err(other) => Outcome::with("failed", other.to_string()),
        Ok(assembly) if assembly.payload.is_none() => {
            let reason = assembly.trace.refused.reason.unwrap_or_default();
            Outcome::with("failed", format!("refused with {reason} instead of rejecting"))
        }
        Ok(_) => Outcome::with("failed", "assembled a payload instead of rejecting"),
    }
}

/// A case is skipped only for an optional component this package lacks and the case uses; anything else it
/// lacks fails the case (Reporting results). No snapshot check needs a tokenizer, so a rejection looks at its
/// renderer alone.
fn unsupported(snapshot: &[u8], lines: &[String], fields: &[&str]) -> Outcome {
    let named: Value = serde_json::from_slice(snapshot).unwrap_or(Value::Null);
    let optional: Vec<&String> = lines.iter().filter(|line| {
        let mut words = line.split(' ');
        let (Some(kind), Some(id)) = (words.next(), words.next()) else { return false };
        fields.contains(&kind) && named[kind] == id && contract::published().iter().any(|p| p.kind == kind && p.id == id && !p.required)
    }).collect();
    if optional.is_empty() {
        Outcome::with("failed", format!("a required component is not provided: {}", lines.join("; ")))
    } else {
        Outcome::with("skipped", optional.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"))
    }
}

/// A trace without the fields that may differ between runs, with every number as a double.
pub fn comparable(trace: &Value) -> Value {
    let mut trace = crate::json::to_doubles(trace.clone());
    if let Value::Object(map) = &mut trace {
        map.remove("trace_id");
        map.remove("timings");
        if let Some(Value::Object(recovery)) = map.get_mut("recovery") {
            recovery.remove("detail");
        }
    }
    trace
}

/// The JSON Pointer of the first place two values differ, with both values, or `None` when they are equal.
pub fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    let show = |v: Option<&Value>| v.map_or("nothing".to_string(), Value::to_string);
    match (expected, actual) {
        (Value::Array(e), Value::Array(a)) => (0..e.len().max(a.len())).find_map(|i| match (e.get(i), a.get(i)) {
            (Some(x), Some(y)) => first_difference(x, y, &format!("{path}/{i}")),
            (x, y) => Some(format!("{path}/{i}: expected {}, got {}", show(x), show(y))),
        }),
        (Value::Object(e), Value::Object(a)) => {
            let mut keys: Vec<&String> = e.keys().chain(a.keys()).collect();
            keys.sort_by(|x, y| cmp_utf16(x, y));
            keys.dedup();
            keys.into_iter().find_map(|key| {
                let child = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                match (e.get(key), a.get(key)) {
                    (Some(x), Some(y)) => first_difference(x, y, &child),
                    (x, y) => Some(format!("{child}: expected {}, got {}", show(x), show(y))),
                }
            })
        }
        _ if expected == actual => None,
        _ => Some(format!("{}: expected {expected}, got {actual}", if path.is_empty() { "/" } else { path })),
    }
}

/// The repository the vendored cases come from, as `owner/name` on GitHub. The lock does not record it.
pub const CONTRACT_REPOSITORY: &str = "contextwindowarchitecture/website";

/// Runs every case and rejection under a conformance directory and returns the report, in the shape of
/// `schema/conformance_report.schema.json`.
pub fn report(conformance: &Path, lock: &Value) -> Value {
    let entries = |kind: &str, run: fn(&Path) -> Outcome| -> Vec<Value> {
        let dir = conformance.join(kind);
        ids(&dir).into_iter().map(|id| {
            let case: Value = serde_json::from_slice(&fs::read(dir.join(&id).join("case.json")).expect("case.json")).expect("case.json is JSON");
            let outcome = run(&dir.join(&id));
            let mut entry = Map::new();
            entry.insert("id".into(), json!(id));
            entry.insert("rules".into(), case["rules"].clone());
            entry.insert("outcome".into(), json!(outcome.outcome));
            if let Some(detail) = outcome.detail {
                entry.insert("detail".into(), json!(detail));
            }
            Value::Object(entry)
        }).collect()
    };
    json!({
        "implementation": {"name": IMPLEMENTATION_NAME, "version": IMPLEMENTATION_VERSION, "language": IMPLEMENTATION_LANGUAGE},
        "contract": {"repository": CONTRACT_REPOSITORY, "commit": lock["website_commit"], "dirty": lock["dirty"]},
        "cases": entries("cases", run_case),
        "rejections": entries("rejections", run_rejection),
    })
}

/// The report as `scripts/conformance.py` writes it: two-space indentation and a final newline.
pub fn report_text(report: &Value) -> String {
    let mut text = serde_json::to_string_pretty(report).expect("a report serializes");
    text.push('\n');
    text
}
