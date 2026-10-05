//! Loading a snapshot: I-JSON, the schemas, the snapshot checks and the digest (conformance/README.md, Snapshot
//! checks, Snapshot digest; R-17, R-22, R-23).
//!
//! A snapshot that fails any of them is rejected before assembly, with its problems in words and no trace.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::canonical;
use crate::json;
use crate::model::Snapshot;
use crate::schema;
use crate::strings::{cmp_utf16, is_blank};

/// A snapshot that passed every check, with the digest of its normalized form.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub snapshot: Snapshot,
    pub digest: String,
}

/// Reads and checks a snapshot. The renderer's realizability is checked later, once the renderer is resolved,
/// since a renderer the package does not provide is not a problem with the snapshot (Snapshot checks).
pub fn load(bytes: &[u8]) -> Result<Loaded, Vec<String>> {
    let raw = json::parse(bytes).map_err(|problem| vec![problem])?;
    let errors = schema::validate("snapshot.schema.json", &raw);
    if !errors.is_empty() {
        return Err(errors.iter().map(|e| format!("schema: {e}")).collect());
    }
    let snapshot: Snapshot = serde_json::from_value(raw.clone()).map_err(|e| vec![format!("schema: {e}")])?;
    let problems = checks(&snapshot);
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Loaded { digest: digest(&raw), snapshot })
}

/// The snapshot checks the schemas cannot express, in the README's order.
fn checks(s: &Snapshot) -> Vec<String> {
    let mut problems = Vec::new();

    // One batch per producer (R-15).
    let mut producers = BTreeSet::new();
    for batch in &s.batches {
        if !producers.insert(batch.producer.id.as_str()) {
            problems.push(format!("producer {:?} heads more than one batch", batch.producer.id));
        }
    }

    // Conflict groups (R-11).
    let mut known_ids = BTreeSet::new();
    for batch in &s.batches {
        known_ids.extend(batch.items.iter().filter_map(|item| item.get("id").and_then(Value::as_str)));
        known_ids.extend(batch.excluded.iter().filter_map(|row| row.get("item_id").and_then(Value::as_str)));
    }
    let mut group_ids = BTreeSet::new();
    let mut grouped: BTreeMap<&str, &str> = BTreeMap::new();
    for group in &s.conflicts {
        if !group_ids.insert(group.id.as_str()) {
            problems.push(format!("conflict group id {:?} is used more than once", group.id));
        }
        for item in &group.items {
            if !known_ids.contains(item.as_str()) {
                problems.push(format!("conflict group {:?} names {item:?}, which is neither a candidate nor a producer exclusion", group.id));
            }
            if let Some(other) = grouped.insert(item, &group.id) {
                problems.push(format!("{item:?} belongs to conflict groups {other:?} and {:?}", group.id));
            }
        }
        if group.kind == "fact" {
            let fact = group.fact.as_deref().unwrap_or_default();
            if !s.route_policy.facts.contains_key(fact) {
                problems.push(format!("fact group {:?} names fact {fact:?}, which the route policy's facts do not define", group.id));
            }
        }
    }

    // Producer exclusions (R-9, R-13).
    for batch in &s.batches {
        let candidates: BTreeSet<&str> = batch.items.iter().filter_map(|item| item.get("id").and_then(Value::as_str)).collect();
        for row in &batch.excluded {
            for field in ["duplicate_of", "superseded_by"] {
                if let Some(target) = row.get(field).and_then(Value::as_str) {
                    if !candidates.contains(target) {
                        problems.push(format!("producer {:?} excludes {} with {field} {target:?}, which is not a candidate in its batch",
                                              batch.producer.id, row["item_id"]));
                    }
                }
            }
        }
    }

    // Profile (R-19, R-20).
    let (profile, route) = (&s.profile, &s.route_policy);
    if profile.route != route.route {
        problems.push(format!("the profile is for route {:?}, and the route policy for {:?}", profile.route, route.route));
    }
    if profile.route_policy_version != route.version {
        problems.push(format!("the profile expects route policy {:?}, and the snapshot carries {:?}", profile.route_policy_version, route.version));
    }
    let placed = |slot: &str| profile.placement.iter().any(|p| p.slot == slot);
    for slot in ["governance.instructions", "interaction.query"] {
        if !placed(slot) {
            problems.push(format!("the profile does not place {slot}, which every assembly needs"));
        }
    }
    if route.parser && !placed("governance.output_contract") {
        problems.push("the route sets parser: true, and the profile does not place governance.output_contract".to_string());
    }
    problems
}

/// The lowercase SHA-256 of the RFC 8785 serialization of the normalized snapshot (Snapshot digest).
pub fn digest(raw: &Value) -> String {
    canonical::sha256_hex(canonical::to_string(&normalize(raw)).as_bytes())
}

/// Reorders only the arrays whose order producers do not control.
fn normalize(raw: &Value) -> Value {
    let mut snapshot = raw.clone();
    if let Some(batches) = snapshot.get_mut("batches").and_then(Value::as_array_mut) {
        for batch in batches.iter_mut() {
            if let Some(items) = batch.get_mut("items").and_then(Value::as_array_mut) {
                let (mut named, unnamed): (Vec<Value>, Vec<Value>) = items.drain(..).partition(|item| non_blank_id(item).is_some());
                named.sort_by(|a, b| {
                    cmp_utf16(non_blank_id(a).unwrap(), non_blank_id(b).unwrap())
                        .then_with(|| canonical::to_string(a).cmp(&canonical::to_string(b)))
                });
                items.extend(named);
                items.extend(unnamed);
            }
            if let Some(rows) = batch.get_mut("excluded").and_then(Value::as_array_mut) {
                rows.sort_by(|a, b| by_string_then_bytes(a, b, "item_id"));
            }
        }
        batches.sort_by(|a, b| cmp_utf16(producer_id(a), producer_id(b)));
    }
    if let Some(groups) = snapshot.get_mut("conflicts").and_then(Value::as_array_mut) {
        for group in groups.iter_mut() {
            if let Some(items) = group.get_mut("items").and_then(Value::as_array_mut) {
                items.sort_by(|a, b| cmp_utf16(a.as_str().unwrap_or_default(), b.as_str().unwrap_or_default()));
            }
        }
        groups.sort_by(|a, b| by_string_then_bytes(a, b, "id"));
    }
    snapshot
}

/// A candidate's id when it is a non-blank string (R-2).
pub fn non_blank_id(item: &Value) -> Option<&str> {
    item.get("id").and_then(Value::as_str).filter(|id| !is_blank(id))
}

fn producer_id(batch: &Value) -> &str {
    batch.pointer("/producer/id").and_then(Value::as_str).unwrap_or_default()
}

fn by_string_then_bytes(a: &Value, b: &Value, field: &str) -> Ordering {
    let key = |v: &Value| v.get(field).and_then(Value::as_str).unwrap_or_default().to_string();
    cmp_utf16(&key(a), &key(b)).then_with(|| canonical::to_string(a).cmp(&canonical::to_string(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_ignores_the_order_producers_do_not_control() {
        let a = json::parse(br#"{"batches": [
            {"producer": {"id": "b"}, "items": [{"id": "y"}, {"id": " "}, {"id": "x"}, {"body": 1}], "excluded": [{"item_id": "q"}, {"item_id": "p"}]},
            {"producer": {"id": "a"}, "items": [], "excluded": []}],
            "conflicts": [{"id": "g2", "items": ["n", "m"]}, {"id": "g1", "items": ["k", "j"]}]}"#).unwrap();
        let b = json::parse(br#"{"batches": [
            {"producer": {"id": "a"}, "items": [], "excluded": []},
            {"producer": {"id": "b"}, "items": [{"id": "x"}, {"id": "y"}, {"id": " "}, {"body": 1}], "excluded": [{"item_id": "p"}, {"item_id": "q"}]}],
            "conflicts": [{"id": "g1", "items": ["j", "k"]}, {"id": "g2", "items": ["m", "n"]}]}"#).unwrap();
        assert_eq!(digest(&a), digest(&b));
        // Candidates without a non-blank id keep the order the batch supplied, since R-2 numbers them in it.
        let c = json::parse(br#"{"batches": [
            {"producer": {"id": "a"}, "items": [], "excluded": []},
            {"producer": {"id": "b"}, "items": [{"id": "x"}, {"id": "y"}, {"body": 1}, {"id": " "}], "excluded": [{"item_id": "p"}, {"item_id": "q"}]}],
            "conflicts": [{"id": "g1", "items": ["j", "k"]}, {"id": "g2", "items": ["m", "n"]}]}"#).unwrap();
        assert_ne!(digest(&a), digest(&c));
    }
}
