//! The snapshot as types, read from JSON that has already passed `snapshot.schema.json` (Validate at the
//! boundary, once). Batch items stay raw JSON: admission validates each one and records an invalid item as an
//! exclusion rather than rejecting the snapshot (R-2). Every number is a double (R-2).

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

/// An item's tier, lowest first, so that the derived order is the order of protection (R-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Droppable,
    Compressible,
    Protected,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    pub assembly_time: String,
    #[serde(default)]
    pub scope: BTreeMap<String, String>,
    pub budget: Budget,
    pub profile: Profile,
    pub route_policy: RoutePolicy,
    pub tokenizer: String,
    pub renderer: String,
    pub batches: Vec<Batch>,
    pub capabilities: Option<Capabilities>,
    pub conflicts: Vec<ConflictGroup>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Budget {
    pub input: f64,
    pub reserved_output: f64,
    pub margin_percent: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub spec: String,
    pub id: String,
    pub version: f64,
    pub route: String,
    pub route_policy_version: String,
    pub placement: Vec<Placement>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Placement {
    pub slot: String,
    pub wrap: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoutePolicy {
    pub route: String,
    pub version: String,
    pub clock_skew_seconds: Option<f64>,
    #[serde(default)]
    pub parser: bool,
    #[serde(default)]
    pub requires_evidence: bool,
    pub on_unresolved_instruction: Option<String>,
    pub producers: BTreeMap<String, ProducerRule>,
    #[serde(default)]
    pub slots: BTreeMap<String, SlotRules>,
    #[serde(default)]
    pub default_overrides: BTreeMap<String, Map<String, Value>>,
    #[serde(default)]
    pub tier_upgrades: BTreeMap<String, Tier>,
    #[serde(default)]
    pub fitting_order: Vec<FittingStep>,
    #[serde(default)]
    pub facts: BTreeMap<String, FactPolicy>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProducerRule {
    pub kind: String,
    pub slots: Vec<String>,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SlotRules {
    pub min_relevance: Option<f64>,
    pub max_age_seconds: Option<f64>,
    #[serde(default)]
    pub required_scope: Vec<String>,
    pub source_prefix: Option<String>,
    pub priority: Option<f64>,
    pub order_by: Option<Vec<String>>,
    pub min_included: Option<f64>,
    pub max_tokens: Option<f64>,
    pub min_tokens: Option<f64>,
    pub supersede: Option<String>,
    pub dedupe: Option<String>,
    pub max_per_source: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FittingStep {
    pub slot: String,
    pub action: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FactPolicy {
    pub precedence: Vec<String>,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub freshness_tiebreak: bool,
    pub on_unresolved: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Batch {
    pub producer: Producer,
    pub items: Vec<Value>,
    pub excluded: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Producer {
    pub id: String,
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Capabilities {
    pub policy_producer: String,
    pub allowed_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConflictGroup {
    pub id: String,
    pub kind: String,
    pub fact: Option<String>,
    pub items: Vec<String>,
}

impl RoutePolicy {
    /// The route's rules for a slot; a slot it does not name has none.
    pub fn slot(&self, slot: &str) -> &SlotRules {
        static NONE: SlotRules = SlotRules {
            min_relevance: None, max_age_seconds: None, required_scope: Vec::new(), source_prefix: None, priority: None,
            order_by: None, min_included: None, max_tokens: None, min_tokens: None, supersede: None, dedupe: None,
            max_per_source: None,
        };
        self.slots.get(slot).unwrap_or(&NONE)
    }
}
