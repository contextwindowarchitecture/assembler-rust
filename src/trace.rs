//! The trace (R-21, R-22; `schema/trace.schema.json`).

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

/// The assembly trace. Serialize it with serde, or call [`Trace::to_json`].
#[derive(Debug, Clone, Serialize)]
pub struct Trace {
    pub trace_id: String,
    pub profile: ProfileRef,
    pub budget: BudgetRecord,
    pub result: Option<ResultRecord>,
    pub included: Vec<Included>,
    pub compressed: Vec<Compressed>,
    /// Producer rows as their producers reported them, then the assembler's rows in pipeline order.
    pub excluded: Vec<Value>,
    pub conflicts: Vec<ConflictRecord>,
    pub refused: Refused,
    pub context: Context,
    pub defaults_filled: Vec<DefaultFilled>,
    /// Measured durations in milliseconds, by stage. They may differ between runs (R-23).
    pub timings: BTreeMap<String, f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Recovery>,
}

impl Trace {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("a trace serializes")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileRef {
    pub id: String,
    #[serde(serialize_with = "number")]
    pub version: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BudgetRecord {
    #[serde(serialize_with = "number")]
    pub input: f64,
    #[serde(serialize_with = "number")]
    pub reserved_output: f64,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "optional_number")]
    pub margin_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResultRecord {
    pub input_tokens: u64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Included {
    pub slot: String,
    pub item_id: String,
    pub tokens: u64,
    pub source_version: String,
    pub eligibility: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Compressed {
    pub slot: String,
    pub item_id: String,
    pub from: u64,
    pub to: u64,
    pub method: String,
    pub variant_id: String,
}

/// An assembler-stage exclusion row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Excluded {
    pub item_id: String,
    pub reason: String,
    pub stage: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

impl Excluded {
    pub fn assembler(item_id: &str, reason: impl Into<String>, slot: Option<String>) -> Excluded {
        Excluded { item_id: item_id.to_string(), reason: reason.into(), stage: "assembler", slot, duplicate_of: None, superseded_by: None }
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("a row serializes")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConflictRecord {
    pub group_id: String,
    pub kind: String,
    pub items: Vec<String>,
    pub resolution: &'static str,
    pub decided_by: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winner: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Refused {
    pub bool: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Context {
    pub spec: String,
    pub assembly_time: String,
    pub route_policy_version: String,
    pub tokenizer: String,
    pub renderer: String,
    pub snapshot_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DefaultFilled {
    pub item_id: String,
    pub field: String,
}

/// A double written as an integer when it is one, as JavaScript writes it, so `1` does not become `1.0`.
fn number<S: serde::Serializer>(x: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if x.fract() == 0.0 && x.abs() < 9_007_199_254_740_992.0 {
        serializer.serialize_i64(*x as i64)
    } else {
        serializer.serialize_f64(*x)
    }
}

fn optional_number<S: serde::Serializer>(x: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
    number(&x.expect("skipped when absent"), serializer)
}

#[derive(Debug, Clone, Serialize)]
pub struct Recovery {
    pub action: &'static str,
    pub detail: String,
}
