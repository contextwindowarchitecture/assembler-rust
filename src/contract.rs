//! The contract data, read from the vendored files at build time: reason codes in R-21's order, slot defaults
//! (R-3) and the published tokenizers and renderers (conformance/README.md, Tokenizers and renderers).
//!
//! Nothing here is generated or copied by hand; a re-vendor changes what these functions return, and the tests
//! pin each to the code that depends on it.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::Value;

use crate::model::Tier;

pub const README: &str = include_str!("../vendor/cwa/conformance/README.md");
const REASONS_JSON: &str = include_str!("../vendor/cwa/contract/reasons.json");
const SLOT_DEFAULTS_JSON: &str = include_str!("../vendor/cwa/contract/slot-defaults.json");

/// The eleven slots, in the order the item schema lists them.
pub const SLOTS: [&str; 11] = [
    "governance.instructions", "governance.capabilities", "governance.examples", "governance.output_contract",
    "state.user", "state.task", "evidence.knowledge", "evidence.tool_results", "interaction.memory",
    "interaction.history", "interaction.query",
];

/// One code from `contract/reasons.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Reason {
    pub code: String,
    pub kind: String,
}

/// Every exclusion and refusal code, in the order R-21 ranks them.
pub fn reasons() -> &'static [Reason] {
    static REASONS: LazyLock<Vec<Reason>> =
        LazyLock::new(|| serde_json::from_str(REASONS_JSON).expect("vendored contract/reasons.json"));
    &REASONS
}

/// The defaults one slot fills (R-3) and its default tier (R-16).
#[derive(Debug, Clone, Deserialize)]
pub struct SlotDefaults {
    pub plane: String,
    pub authority: String,
    pub tier: Tier,
    pub lineage: String,
    pub injection_risk: String,
    pub conflict_policy: String,
    pub token_budget: Value,
    pub variants: Value,
    pub eligibility: String,
}

impl SlotDefaults {
    /// The default for one of the six fields R-3 fills, as JSON.
    pub fn field(&self, name: &str) -> Value {
        match name {
            "token_budget" => self.token_budget.clone(),
            "variants" => self.variants.clone(),
            "conflict_policy" => Value::String(self.conflict_policy.clone()),
            "lineage" => Value::String(self.lineage.clone()),
            "eligibility" => Value::String(self.eligibility.clone()),
            "injection_risk" => Value::String(self.injection_risk.clone()),
            _ => panic!("{name} is not a defaulted field"),
        }
    }
}

/// The fields R-3 fills, in the order `defaults_filled[]` lists them.
pub const DEFAULTED_FIELDS: [&str; 6] = ["token_budget", "variants", "conflict_policy", "lineage", "eligibility", "injection_risk"];

/// Each slot's defaults, from `contract/slot-defaults.json`.
pub fn slot_defaults(slot: &str) -> &'static SlotDefaults {
    static DEFAULTS: LazyLock<BTreeMap<String, SlotDefaults>> =
        LazyLock::new(|| serde_json::from_str(SLOT_DEFAULTS_JSON).expect("vendored contract/slot-defaults.json"));
    DEFAULTS.get(slot).unwrap_or_else(|| panic!("no slot defaults for {slot}"))
}

/// A tokenizer or renderer the README publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub kind: &'static str,
    pub id: String,
    /// Listed before the Optional heading, so every implementation provides it.
    pub required: bool,
}

/// The published tokenizers and renderers: the bullets under the README's Tokenizers and renderers section, those
/// under its Optional subheading included. A bullet's kind is its verb: a tokenizer counts, a renderer renders.
pub fn published() -> &'static [Published] {
    static PUBLISHED: LazyLock<Vec<Published>> = LazyLock::new(|| {
        let start = README.find("\n## Tokenizers and renderers\n").expect("README has a Tokenizers and renderers section");
        let section = &README[start + 1..];
        let end = section[3..].find("\n## ").map_or(section.len(), |i| i + 3);
        let mut required = true;
        let mut found = Vec::new();
        for line in section[..end].lines() {
            if line.starts_with("### Optional") {
                required = false;
            }
            let Some(rest) = line.strip_prefix("- `") else { continue };
            let (id, text) = rest.split_once('`').expect("a bullet names its component in backticks");
            let kind = if text.starts_with(" counts ") {
                "tokenizer"
            } else if text.starts_with(" renders ") {
                "renderer"
            } else {
                panic!("cannot tell whether the README's {id} is a tokenizer or a renderer")
            };
            found.push(Published { kind, id: id.to_string(), required });
        }
        found
    });
    &PUBLISHED
}

/// Whether an id names a published component of a kind.
pub fn is_published(kind: &str, id: &str) -> bool {
    published().iter().any(|p| p.kind == kind && p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_components_are_read_from_the_readme() {
        let listed: Vec<(&str, &str, bool)> = published().iter().map(|p| (p.kind, p.id.as_str(), p.required)).collect();
        assert_eq!(listed, [
            ("tokenizer", "fixture-whitespace/v1", true),
            ("tokenizer", "estimate-utf8/v1", true),
            ("renderer", "fixture-xml/v1", true),
            ("renderer", "cwa-messages/v1", true),
            ("renderer", "cwa-message-blocks/v1", false),
        ]);
    }

    #[test]
    fn slot_defaults_cover_the_eleven_slots() {
        for slot in SLOTS {
            let d = slot_defaults(slot);
            assert!(d.variants.as_array().is_some_and(Vec::is_empty), "{slot}");
        }
        assert_eq!(slot_defaults("state.task").tier, Tier::Protected);
        assert_eq!(slot_defaults("governance.examples").tier, Tier::Droppable);
    }

    #[test]
    fn reasons_are_exclusions_then_refusals() {
        let kinds: Vec<&str> = reasons().iter().map(|r| r.kind.as_str()).collect();
        let first_refusal = kinds.iter().position(|k| *k == "refusal").unwrap();
        assert!(kinds[..first_refusal].iter().all(|k| *k == "exclusion"));
        assert!(kinds[first_refusal..].iter().all(|k| *k == "refusal"));
    }
}
