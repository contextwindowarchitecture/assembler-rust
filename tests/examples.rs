//! The examples run as the README shows them. The quickstart the README mirrors admits relevant evidence and leaves
//! out the rest (R-13), renders governance apart from untrusted material (R-7, R-10), and refuses when the evidence
//! is missing (R-12).

#[allow(dead_code)]
#[path = "../examples/quickstart.rs"]
mod quickstart;

use contextwindowarchitecture_assembler::assemble;
use serde_json::{json, Value};

fn request(assembly: &contextwindowarchitecture_assembler::Assembly) -> Value {
    serde_json::from_slice(assembly.payload.as_deref().expect("assembled, not refused")).unwrap()
}

#[test]
fn the_quickstart_sends_the_relevant_chunk_and_traces_the_weak_one() {
    let assembly = assemble(&quickstart::build_snapshot(quickstart::DOCS)).unwrap();
    let request = request(&assembly);
    let system: Vec<&str> = request["system"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(system, ["policy:tone"]);
    let messages = request["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");
    let content = messages[0]["content"].as_str().unwrap();
    assert!(content.contains("kb:refunds") && !content.contains("kb:pricing"));
    let excluded: Vec<(&str, &str)> =
        assembly.trace.excluded.iter().map(|row| (row["item_id"].as_str().unwrap(), row["reason"].as_str().unwrap())).collect();
    assert_eq!(excluded, [("kb:pricing", "below_threshold")]);
}

#[test]
fn the_quickstart_refuses_before_any_model_call_without_evidence() {
    let assembly = assemble(&quickstart::build_snapshot(&[])).unwrap();
    assert!(assembly.payload.is_none());
    assert_eq!(assembly.trace.refused.reason.as_deref(), Some("evidence_required"));
    assert_eq!(assembly.trace.recovery.as_ref().map(|r| r.action), Some("request_context"));
}

#[test]
fn the_quickstart_maps_the_payload_to_provider_requests() {
    let request = request(&assemble(&quickstart::build_snapshot(quickstart::DOCS)).unwrap());
    let (anthropic, openai) = (quickstart::to_anthropic(&request), quickstart::to_openai(&request));
    let system = &request["system"][0]["text"];
    assert_eq!(&anthropic["system"], system);
    assert_eq!(anthropic["messages"], request["messages"]);
    let mut messages = vec![json!({"role": "system", "content": system})];
    messages.extend(request["messages"].as_array().unwrap().iter().cloned());
    assert_eq!(openai["messages"], Value::Array(messages));
    assert!(anthropic.get("tools").is_none() && openai.get("tools").is_none());
}

#[test]
fn the_quickstart_runs() {
    let out = quickstart::run();
    assert!(out.contains("kb:pricing  below_threshold") && out.contains("refused: evidence_required"));
}

#[test]
fn the_readme_shows_what_the_quickstart_prints() {
    let readme = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md")).unwrap();
    let start = "cargo run --example quickstart\n```\n\n```text\n";
    let shown = readme.split_once(start).and_then(|(_, rest)| rest.split_once("```")).map(|(block, _)| block);
    assert_eq!(shown, Some(quickstart::run().as_str()));
}
