//! The public call: components the application supplies (R-16), unsupported components, and determinism
//! (R-23). No conformance case can cover these, so this package tests them itself (conformance/README.md,
//! Tokenizers and renderers).

use std::fs;
use std::path::{Path, PathBuf};

use contextwindowarchitecture_assembler::{assemble, assemble_with, contract, Error, Options};
use serde_json::Value;

fn case(id: &str) -> Vec<u8> {
    fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/cwa/conformance/cases").join(id).join("snapshot.json")).unwrap()
}

fn with_tokenizer(bytes: &[u8], tokenizer: &str) -> Vec<u8> {
    let mut snapshot: Value = serde_json::from_slice(bytes).unwrap();
    snapshot["tokenizer"] = Value::String(tokenizer.into());
    serde_json::to_vec(&snapshot).unwrap()
}

#[test]
fn a_tokenizer_under_any_published_id_stops_the_call() {
    let tokenizers: Vec<&str> = contract::published().iter().filter(|p| p.kind == "tokenizer").map(|p| p.id.as_str()).collect();
    assert!(tokenizers.len() >= 2);
    for id in tokenizers {
        let options = Options::new().tokenizer(id, |text: &str| text.len() as u64);
        // Even when the snapshot does not name it: no payload and no trace.
        match assemble_with(&case("fixture-three-slot"), &options) {
            Err(Error::PublishedId(message)) => assert!(message.contains(id)),
            other => panic!("{id}: {other:?}"),
        }
    }
}

#[test]
fn an_application_tokenizer_counts_under_its_own_id() {
    let snapshot = with_tokenizer(&case("fixture-three-slot"), "chars/v1");
    let options = Options::new().tokenizer("chars/v1", |text: &str| text.chars().count() as u64);
    let assembly = assemble_with(&snapshot, &options).unwrap();
    let payload = String::from_utf8(assembly.payload.unwrap()).unwrap();
    assert_eq!(assembly.trace.result.unwrap().input_tokens, payload.chars().count() as u64);
    assert_eq!(assembly.trace.context.tokenizer, "chars/v1");
}

#[test]
fn an_unknown_tokenizer_or_renderer_is_unsupported_not_invalid() {
    for name in ["toString", "__proto__", "constructor", "gpt-tokenizer/v9"] {
        let snapshot = with_tokenizer(&case("fixture-three-slot"), name);
        assert_eq!(assemble(&snapshot).unwrap_err(), Error::Unsupported(vec![format!("tokenizer {name} is not provided")]));
    }
    let mut snapshot: Value = serde_json::from_slice(&case("fixture-three-slot")).unwrap();
    snapshot["renderer"] = Value::String("acme-chat/v2".into());
    snapshot["tokenizer"] = Value::String("acme-tok/v2".into());
    assert_eq!(assemble(&serde_json::to_vec(&snapshot).unwrap()).unwrap_err(),
               Error::Unsupported(vec!["tokenizer acme-tok/v2 is not provided".into(), "renderer acme-chat/v2 is not provided".into()]));
}

#[test]
fn only_trace_id_and_timings_differ_between_runs() {
    let cases = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/cwa/conformance/cases");
    let mut dirs: Vec<PathBuf> = fs::read_dir(cases).unwrap().map(|e| e.unwrap().path()).collect();
    dirs.sort();
    for dir in dirs {
        let bytes = fs::read(dir.join("snapshot.json")).unwrap();
        let (a, b) = (assemble(&bytes).unwrap(), assemble(&bytes).unwrap());
        assert_eq!(a.payload, b.payload, "{}", dir.display());
        let strip = |assembly: &contextwindowarchitecture_assembler::Assembly| {
            let mut trace = assembly.trace.to_json();
            trace.as_object_mut().unwrap().remove("trace_id");
            trace.as_object_mut().unwrap().remove("timings");
            trace
        };
        assert_eq!(strip(&a), strip(&b), "{}", dir.display());
        assert!(a.trace.timings.values().all(|ms| *ms >= 0.0));
    }
}

#[test]
fn a_supplied_trace_id_is_used() {
    let assembly = assemble_with(&case("fixture-three-slot"), &Options::new().trace_id("call-42")).unwrap();
    assert_eq!(assembly.trace.trace_id, "call-42");
    assert_ne!(assemble(&case("fixture-three-slot")).unwrap().trace.trace_id, assemble(&case("fixture-three-slot")).unwrap().trace.trace_id);
}

#[test]
fn a_rejection_lists_its_problems() {
    let mut snapshot: Value = serde_json::from_slice(&case("fixture-three-slot")).unwrap();
    snapshot.as_object_mut().unwrap().remove("budget");
    snapshot["profile"]["route"] = Value::String("elsewhere".into());
    match assemble(&serde_json::to_vec(&snapshot).unwrap()) {
        Err(Error::Rejected(problems)) => assert!(problems.iter().any(|p| p.contains("budget")), "{problems:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_respelled_number_is_the_same_double() {
    // The published case with each number beyond 2^53 written with a zero fraction: the same doubles, so the same
    // digest, payload and trace (conformance/README.md, Numbers).
    let id = "threshold-beyond-2-53";
    let original = case(id);
    let text = String::from_utf8(original.clone()).unwrap();
    let respelled = ["9007199254740991", "9007199254740992", "9007199254740993"]
        .iter()
        .fold(text, |t, n| t.replace(&format!(": {n}\n"), &format!(": {n}.0\n")));
    assert_eq!(respelled.matches(".0\n").count(), 4);
    let (a, b) = (assemble(&original).unwrap(), assemble(respelled.as_bytes()).unwrap());
    assert_eq!(a.trace.context.snapshot_digest, b.trace.context.snapshot_digest);
    let expected = fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/cwa/conformance/cases").join(id).join("expected.payload.txt")).unwrap();
    assert_eq!(b.payload.as_deref(), Some(&expected[..]));
    let strip = |assembly: &contextwindowarchitecture_assembler::Assembly| {
        let mut trace = assembly.trace.to_json();
        trace.as_object_mut().unwrap().remove("trace_id");
        trace.as_object_mut().unwrap().remove("timings");
        trace
    };
    assert_eq!(strip(&a), strip(&b));
}
