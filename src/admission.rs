//! Admission (conformance/README.md, Running a case; R-1 to R-3, R-8 to R-10, R-13 to R-16, R-18, R-20).
//!
//! Each candidate is checked in `contract/reasons.json` order and excluded with the first code that applies to
//! it (R-21). Defaults are filled, and traced, for every schema-valid item from a producer the route admits,
//! before the checks that read them (R-3).

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::canonical;
use crate::contract::{slot_defaults, DEFAULTED_FIELDS, SLOTS};
use crate::instant::Instant;
use crate::model::{Snapshot, Tier};
use crate::schema::{self, SchemaError};
use crate::snapshot::non_blank_id;
use crate::strings::cmp_utf16;
use crate::trace::{DefaultFilled, Excluded};


/// An admitted item, with its defaults filled and its effective tier.
#[derive(Debug, Clone)]
pub struct Item {
    pub id: String,
    pub slot: String,
    pub source: String,
    pub source_version: String,
    pub authority: String,
    pub freshness: Instant,
    pub scope: BTreeMap<String, String>,
    pub eligibility: String,
    pub token_budget: Option<f64>,
    pub tier: Tier,
    pub conflict_policy: String,
    pub lineage: String,
    pub body: String,
    pub variants: Vec<Variant>,
    pub relevance: Option<f64>,
    /// The authenticated producer of the item's batch.
    pub producer: String,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub id: String,
    pub body: String,
    pub method: String,
}

/// What admission decides.
pub struct Admission {
    /// Admitted items, in no particular order.
    pub items: Vec<Item>,
    /// Admission rows, ordered by producer id, recorded item id, then the candidate's serialization.
    pub excluded: Vec<Excluded>,
    /// Filled defaults, ordered by item id and then field.
    pub defaults_filled: Vec<DefaultFilled>,
}

/// A slot's tier: its default raised by the route's `tier_upgrades`, never lowered (R-16).
pub fn slot_tier(s: &Snapshot, slot: &str) -> Tier {
    let default = slot_defaults(slot).tier;
    s.route_policy.tier_upgrades.get(slot).map_or(default, |upgrade| default.max(*upgrade))
}

pub fn admit(s: &Snapshot) -> Admission {
    let assembly_time = Instant::parse(&s.assembly_time).expect("assembly_time passed its schema");
    let mut id_uses: BTreeMap<&str, usize> = BTreeMap::new();
    for batch in &s.batches {
        for item in &batch.items {
            if let Some(id) = non_blank_id(item) {
                *id_uses.entry(id).or_default() += 1;
            }
        }
        for row in &batch.excluded {
            if let Some(id) = row.get("item_id").and_then(Value::as_str) {
                *id_uses.entry(id).or_default() += 1;
            }
        }
    }
    let shared = |id: &str| id_uses.get(id).copied().unwrap_or(0) > 1;

    let mut items = Vec::new();
    let mut rows: Vec<(String, String, String, Excluded)> = Vec::new();
    let mut defaults_filled = Vec::new();
    for batch in &s.batches {
        let producer = &batch.producer;
        let rule = s.route_policy.producers.get(&producer.id).filter(|rule| rule.kind == producer.kind);
        let mut invalid = 0;
        for raw in &batch.items {
            let recorded = match non_blank_id(raw) {
                Some(id) => id.to_string(),
                None => {
                    invalid += 1;
                    format!("{}#invalid-{}", producer.id, invalid - 1)
                }
            };
            let named_slot = raw.get("slot").and_then(Value::as_str).filter(|slot| SLOTS.contains(slot)).map(str::to_string);
            let mut exclude = |reason: String| {
                rows.push((producer.id.clone(), recorded.clone(), canonical::to_string(raw), Excluded::assembler(&recorded, reason, named_slot.clone())));
            };
            let Some(rule) = rule else {
                exclude("producer_not_authenticated".into());
                continue;
            };
            let errors = schema::validate("context_item.schema.json", raw);
            if !errors.is_empty() {
                exclude(schema_code(&errors));
                continue;
            }
            let mut filled = raw.as_object().expect("a valid item is an object").clone();
            let slot = filled["slot"].as_str().expect("a valid item has a slot").to_string();
            for field in DEFAULTED_FIELDS {
                if !filled.contains_key(field) {
                    let value = s.route_policy.default_overrides.get(&slot).and_then(|o| o.get(field)).cloned()
                        .unwrap_or_else(|| slot_defaults(&slot).field(field));
                    filled.insert(field.to_string(), value);
                    defaults_filled.push(DefaultFilled { item_id: recorded.clone(), field: field.to_string() });
                }
            }
            let item = item_from(&filled, &producer.id, slot_tier(s, &slot));
            match check(s, &assembly_time, &filled, &item, &rule.kind, rule.verified, &rule.slots, shared(&item.id)) {
                Some(reason) => exclude(reason.to_string()),
                None => items.push(item),
            }
        }
    }
    rows.sort_by(|a, b| cmp_utf16(&a.0, &b.0).then_with(|| cmp_utf16(&a.1, &b.1)).then_with(|| a.2.cmp(&b.2)));
    defaults_filled.sort_by(|a, b| cmp_utf16(&a.item_id, &b.item_id).then_with(|| field_rank(&a.field).cmp(&field_rank(&b.field))));
    Admission { items, excluded: rows.into_iter().map(|row| row.3).collect(), defaults_filled }
}

fn field_rank(field: &str) -> usize {
    DEFAULTED_FIELDS.iter().position(|f| *f == field).expect("a defaulted field")
}

/// The code for a schema-invalid item: the alphabetically first missing field of the item itself, else an
/// unknown slot or authority, whatever its JSON type, else invalid structure (R-1, R-2, R-21). A variant's missing
/// field is not the item's.
fn schema_code(errors: &[SchemaError]) -> String {
    let mut missing: Vec<&str> = errors.iter().filter(|e| e.path.is_empty()).filter_map(|e| e.missing.as_deref()).collect();
    missing.sort_by(|a, b| cmp_utf16(a, b));
    if let Some(first) = missing.first() {
        format!("missing_field:{first}")
    } else if errors.iter().any(|e| e.path == "/slot") {
        "unknown_slot".into()
    } else if errors.iter().any(|e| e.path == "/authority") {
        "unknown_authority".into()
    } else {
        "invalid_structure".into()
    }
}

fn item_from(filled: &Map<String, Value>, producer: &str, slot_tier: Tier) -> Item {
    let text = |field: &str| filled[field].as_str().expect("a valid item's field is a string").to_string();
    let variants = filled["variants"].as_array().expect("variants are an array").iter()
        .map(|v| Variant { id: v["id"].as_str().unwrap().into(), body: v["body"].as_str().unwrap().into(), method: v["method"].as_str().unwrap().into() })
        .collect();
    let scope = filled.get("scope").and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default();
    let own_tier: Option<Tier> = filled.get("tier").map(|t| serde_json::from_value(t.clone()).expect("a valid tier"));
    Item {
        id: text("id"),
        slot: text("slot"),
        source: text("source"),
        source_version: text("source_version"),
        authority: text("authority"),
        freshness: Instant::parse(filled["freshness"].as_str().unwrap()).expect("freshness passed its schema"),
        scope,
        eligibility: text("eligibility"),
        token_budget: filled["token_budget"].as_f64(),
        tier: own_tier.unwrap_or(slot_tier),
        conflict_policy: text("conflict_policy"),
        lineage: text("lineage"),
        body: text("body"),
        variants,
        relevance: filled.get("relevance").and_then(Value::as_f64),
        producer: producer.to_string(),
    }
}

/// The admission checks after the schema, in `contract/reasons.json` order; the first that applies, or none.
#[allow(clippy::too_many_arguments)]
fn check(s: &Snapshot, now: &Instant, filled: &Map<String, Value>, item: &Item, kind: &str, verified: bool,
         route_slots: &[String], shared_id: bool) -> Option<&'static str> {
    let slot = item.slot.as_str();
    let defaults = slot_defaults(slot);
    let rules = s.route_policy.slot(slot);
    let is_state = slot.starts_with("state.");
    let is_evidence = matches!(slot, "evidence.knowledge" | "evidence.tool_results");
    let injection_risk = filled["injection_risk"].as_str().unwrap();
    let own_tier: Option<Tier> = filled.get("tier").map(|t| serde_json::from_value(t.clone()).unwrap());

    if shared_id {
        return Some("duplicate_item_id");
    }
    let kind_allows = match kind {
        _ if is_state && kind != "state" => false,
        "memory" => slot == "interaction.memory",
        "retrieval" => is_evidence,
        "mcp" => is_evidence || slot == "governance.capabilities",
        _ => true,
    };
    if !route_slots.iter().any(|s| s == slot) || !kind_allows {
        return Some("producer_slot_not_allowed");
    }
    if !authority_allowed(slot, &item.authority, &item.lineage) {
        return Some("authority_not_allowed");
    }
    if slot == "governance.capabilities" {
        let granted = s.capabilities.as_ref().is_some_and(|grant| {
            item.producer == grant.policy_producer && kind == "capability_policy" && grant.allowed_ids.contains(&item.id)
        });
        if !granted {
            return Some("capability_not_allowed");
        }
    }
    if defaults.plane == "gov" && (filled["trust"] != "verified" || injection_risk == "untrusted_content") {
        return Some("untrusted_in_governance");
    }
    if defaults.injection_risk == "untrusted_content" && injection_risk != "untrusted_content" && !(kind == "mcp" && verified) {
        return Some("untrusted_content_unmarked");
    }
    if let Some(own) = own_tier {
        if defaults.tier == Tier::Protected && own != Tier::Protected {
            return Some("protected_tier_changed");
        }
        if own > slot_tier(s, slot) {
            return Some("tier_upgrade_not_allowed");
        }
    }
    let mut variant_ids = vec![item.id.as_str()];
    for variant in &item.variants {
        if variant_ids.contains(&variant.id.as_str()) {
            return Some("duplicate_variant_id");
        }
        variant_ids.push(&variant.id);
    }
    if filled.contains_key("revoked_by") {
        return Some("revoked");
    }
    if let Some(expires) = filled.get("expires").and_then(Value::as_str) {
        if Instant::parse(expires).expect("expires passed its schema") <= *now {
            return Some("expired");
        }
    }
    if item.freshness > now.plus_seconds(s.route_policy.clock_skew_seconds.unwrap_or(0.0)) {
        return Some("future_freshness");
    }
    let too_old = rules.max_age_seconds.is_some_and(|age| item.freshness < now.plus_seconds(-age));
    if is_state && too_old {
        return Some("stale_state");
    }
    if rules.source_prefix.as_ref().is_some_and(|prefix| !item.source.starts_with(prefix.as_str())) {
        return Some("source_invalid");
    }
    let foreign_key = item.scope.iter().any(|(key, value)| s.scope.get(key) != Some(value));
    let missing_key = rules.required_scope.iter().any(|key| !item.scope.contains_key(key));
    if foreign_key || missing_key {
        return Some("out_of_scope");
    }
    if let Some(threshold) = rules.min_relevance {
        if item.relevance.map_or(true, |r| r < threshold) {
            return Some("below_threshold");
        }
    }
    if !is_state && too_old {
        return Some("not_eligible");
    }
    if !s.profile.placement.iter().any(|p| p.slot == slot) && item.tier != Tier::Protected {
        return Some("slot_unplaced");
    }
    None
}

/// The authority values R-1 permits in a slot: its own role, `untrusted` for a prior model turn in history, and
/// `untrusted` instead for any other item in tool results, memory and history.
fn authority_allowed(slot: &str, authority: &str, lineage: &str) -> bool {
    let own = slot_defaults(slot).authority.as_str();
    match slot {
        "interaction.history" if lineage == "generated" => authority == "untrusted",
        "evidence.tool_results" | "interaction.memory" | "interaction.history" => authority == own || authority == "untrusted",
        _ => authority == own,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract;

    /// The admission codes in the order this module checks them, which a test holds to `contract/reasons.json`.
    pub const CHECK_ORDER: &[&str] = &[
        "producer_not_authenticated", "missing_field:<name>", "unknown_slot", "unknown_authority", "invalid_structure",
        "duplicate_item_id", "producer_slot_not_allowed", "authority_not_allowed", "capability_not_allowed",
        "untrusted_in_governance", "untrusted_content_unmarked", "protected_tier_changed", "tier_upgrade_not_allowed",
        "duplicate_variant_id", "revoked", "expired", "future_freshness", "stale_state", "source_invalid", "out_of_scope",
        "below_threshold", "not_eligible", "slot_unplaced",
    ];

    #[test]
    fn checks_run_in_the_order_reasons_json_ranks_them() {
        let exclusions: Vec<&str> = contract::reasons().iter().filter(|r| r.kind == "exclusion").map(|r| r.code.as_str()).collect();
        let admission: Vec<&str> = exclusions.iter().copied().filter(|code| CHECK_ORDER.contains(code)).collect();
        assert_eq!(admission, CHECK_ORDER);
        // The codes after admission belong to the later stages, in pipeline order.
        let later: Vec<&str> = exclusions.into_iter().filter(|code| !CHECK_ORDER.contains(code)).collect();
        assert_eq!(later, ["conflict_deferred", "conflict_lost", "superseded", "duplicate_content", "source_diversity_cap", "over_budget"]);
    }

    #[test]
    fn untrusted_is_allowed_only_where_r1_allows_it() {
        assert!(authority_allowed("interaction.history", "untrusted", "generated"));
        assert!(!authority_allowed("interaction.history", "user", "generated"));
        assert!(authority_allowed("interaction.history", "untrusted", "verbatim"));
        assert!(authority_allowed("interaction.memory", "untrusted", "summarised"));
        assert!(!authority_allowed("interaction.query", "untrusted", "verbatim"));
        assert!(!authority_allowed("evidence.knowledge", "untrusted", "verbatim"));
        assert!(!authority_allowed("state.task", "untrusted", "extracted"));
        assert!(authority_allowed("evidence.tool_results", "observation", "verbatim"));
    }
}
