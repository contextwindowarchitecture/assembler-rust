//! Budget pressure that sheds nearly every chunk (conformance/README.md, Fitting, Cost): every reduction is its
//! own fit test over the whole payload, and the chunks that stay are the highest-ranked ones.

use std::path::Path;

use contextwindowarchitecture_assembler::assemble;
use serde_json::{json, Value};

#[test]
fn shedding_most_of_500_chunks_keeps_the_highest_ranked() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/cwa/conformance/cases/fixture-three-slot/snapshot.json");
    let mut snapshot: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let template = snapshot["batches"][1]["items"][0].clone();
    let chunks: Vec<Value> = (0..500).map(|i| {
        let mut chunk = template.clone();
        chunk["id"] = json!(format!("chunk:{i:03}"));
        chunk["relevance"] = json!(0.5 + f64::from(i) / 1000.0);
        chunk["body"] = json!((0..20).map(|j| format!("w{i}-{j}")).collect::<Vec<_>>().join(" "));
        chunk
    }).collect();
    snapshot["batches"][1]["items"] = Value::Array(chunks);
    snapshot["budget"]["input"] = json!(100);
    let assembly = assemble(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let kept: Vec<&str> = assembly.trace.included.iter().filter(|r| r.slot == "evidence.knowledge").map(|r| r.item_id.as_str()).collect();
    assert_eq!(kept, ["chunk:497", "chunk:498", "chunk:499"]);
    assert_eq!(assembly.trace.excluded.len(), 1 + 497);
    assert!(assembly.trace.result.unwrap().input_tokens <= 100);
}
