//! Quickstart: assemble one support-chat request, then watch the assembler refuse one.
//!
//!     cargo run --example quickstart
//!
//! A snapshot is everything assembly may look at, frozen: the items producers proposed, the route's policy, the
//! placement profile, the budget, the tokenizer, the renderer and the clock. `assemble` turns it into a payload and a
//! trace, or a refusal and a trace, and reads nothing else (R-23). The README's Quickstart mirrors this file.

use contextwindowarchitecture_assembler::assemble;
use serde_json::{json, Value};

const NOW: &str = "2026-10-21T12:00:00Z";

/// What a retriever returned for the question, with its scores: (id, body, relevance). The route below admits only
/// chunks scoring 0.5 or more.
pub const DOCS: &[(&str, &str, f64)] = &[
    ("kb:refunds", "Pro plans can be refunded in full within 30 days of purchase.", 0.82),
    ("kb:pricing", "The Pro plan costs $20 a month, billed annually.", 0.31),
];

/// An item with the eight fields every item carries; the slot's defaults fill in the rest (R-2, R-3).
fn item(id: &str, slot: &str, authority: &str, trust: &str, body: &str) -> Value {
    json!({"id": id, "slot": slot, "source": format!("src:{id}"), "source_version": "1", "authority": authority,
           "trust": trust, "freshness": NOW, "body": body})
}

/// Freezes one request, as the bytes `assemble` takes. Everything that can change the payload is in here.
pub fn build_snapshot(docs: &[(&str, &str, f64)]) -> Vec<u8> {
    // Which producers may write which slots, and the rules for each slot. Versioned, so a trace names the policy.
    let route_policy = json!({
        "route": "support-chat",
        "version": "support-chat/v1",
        "producers": {
            "policy-registry": {"kind": "policy", "slots": ["governance.instructions"]},
            "docs-search": {"kind": "retrieval", "slots": ["evidence.knowledge"]},
            "conversation": {"kind": "interaction", "slots": ["interaction.query"]}
        },
        "slots": {"evidence.knowledge": {"min_relevance": 0.5}},
        "requires_evidence": true  // refuse rather than let the model answer from nothing (R-12)
    });
    // Where each slot lands in the request. Instructions go to the system channel; the rest is escaped material in
    // the user message, so a retrieved chunk can never speak as the system (R-7, R-10).
    let profile = json!({
        "spec": "cwa/draft",
        "id": "support-chat-default",
        "version": 1,
        "route": "support-chat",
        "model_family": null,
        "route_policy_version": "support-chat/v1",
        "placement": [
            {"slot": "governance.instructions", "wrap": "system"},
            {"slot": "evidence.knowledge", "wrap": "xml:evidence"},
            {"slot": "interaction.query", "wrap": "xml:query"}
        ],
        "evaluation": {"status": "unevaluated", "suite": null, "date": null, "result": null, "artifact": null}
    });
    let chunks: Vec<Value> = docs
        .iter()
        .map(|(id, body, relevance)| {
            let mut chunk = item(id, "evidence.knowledge", "reference_only", "unverified", body);
            chunk["relevance"] = json!(relevance);
            chunk
        })
        .collect();
    let snapshot = json!({
        "assembly_time": NOW,
        "scope": {"tenant": "acme", "user": "u_42", "session": "s_1", "task": "support"},
        "budget": {"input": 8000, "reserved_output": 1024, "margin_percent": 10},  // headroom for an estimate
        "profile": profile,
        "route_policy": route_policy,
        "tokenizer": "estimate-utf8/v1",  // pass your model's own with Options::tokenizer for exact counts
        "renderer": "cwa-messages/v1",    // {system, tools, messages}, ready to map onto a provider's request
        "batches": [
            {"producer": {"id": "policy-registry", "kind": "policy"}, "excluded": [], "items": [
                item("policy:tone", "governance.instructions", "governing", "verified",
                     "Answer from the evidence only, and say so when it does not cover the question.")
            ]},
            {"producer": {"id": "docs-search", "kind": "retrieval"}, "excluded": [], "items": chunks},
            {"producer": {"id": "conversation", "kind": "interaction"}, "excluded": [], "items": [
                item("turn:1", "interaction.query", "user", "unverified", "Can I get a refund on my Pro plan?")
            ]}
        ],
        "conflicts": []
    });
    serde_json::to_vec(&snapshot).expect("a JSON value serializes")
}

/// A tool's text is the body its capability producer wrote, in the provider's tool format.
fn tools(request: &Value) -> Option<Value> {
    let tools = request["tools"].as_array()?;
    (!tools.is_empty()).then(|| {
        tools.iter().map(|t| serde_json::from_str::<Value>(t["text"].as_str().unwrap()).expect("a tool body is JSON")).collect()
    })
}

/// The body of a request with a separate system prompt, as in the Anthropic Messages API, less model and max_tokens.
pub fn to_anthropic(request: &Value) -> Value {
    let system: Vec<&str> = request["system"].as_array().unwrap().iter().map(|e| e["text"].as_str().unwrap()).collect();
    let mut out = json!({"system": system.join("\n\n"), "messages": request["messages"]});
    if let Some(tools) = tools(request) {
        out["tools"] = tools;
    }
    out
}

/// The body of a request with system messages, as in OpenAI Chat Completions, less model.
pub fn to_openai(request: &Value) -> Value {
    let mut messages: Vec<Value> =
        request["system"].as_array().unwrap().iter().map(|e| json!({"role": "system", "content": e["text"]})).collect();
    messages.extend(request["messages"].as_array().unwrap().iter().cloned());
    let mut out = json!({"messages": messages});
    if let Some(tools) = tools(request) {
        out["tools"] = tools;
    }
    out
}

/// What `main` prints.
pub fn run() -> String {
    let assembly = assemble(&build_snapshot(DOCS)).expect("the snapshot is valid");
    let request: Value = serde_json::from_slice(assembly.payload.as_deref().expect("assembled")).unwrap();
    let mut out = serde_json::to_string_pretty(&to_anthropic(&request)).unwrap();
    let tokens = assembly.trace.result.as_ref().map_or(0, |r| r.input_tokens);
    out += &format!("\n\n{tokens} tokens; left out:\n");
    for row in &assembly.trace.excluded {
        out += &format!("  {}  {}\n", row["item_id"].as_str().unwrap(), row["reason"].as_str().unwrap());
    }

    // The retriever found nothing: no payload, so no model call, and the trace says what to do next.
    let refused = assemble(&build_snapshot(&[])).expect("the snapshot is valid");
    let reason = refused.trace.refused.reason.as_deref().unwrap_or("");
    let action = refused.trace.recovery.as_ref().map_or("", |r| r.action);
    out += &format!("\nrefused: {reason}, recovery: {action}\n");
    out
}

fn main() {
    print!("{}", run());
}
