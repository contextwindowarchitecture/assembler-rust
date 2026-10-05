//! The pipeline: admission, conflicts, supersession, deduplication, source diversity, refusal checks, fitting,
//! render and trace (conformance/README.md, Running a case, Refusals).
//!
//! It reads nothing but the snapshot and the resolved components. The clock is read only for `timings`, and
//! randomness only for a `trace_id` the caller did not supply: the two fields R-23 lets differ between runs.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{BuildHasher, Hasher};
use std::time::Instant as Clock;

use serde_json::Value;

use crate::admission::{self, Item};
use crate::canonical;
use crate::fitting::{Fitter, Outcome};
use crate::model::Tier;
use crate::resolve::{self, ids};
use crate::strings::cmp_utf16;
use crate::trace::{BudgetRecord, Context, ProfileRef, Recovery, Refused, ResultRecord, Trace};
use crate::{Assembly, Prepared};

const EVIDENCE: [&str; 2] = ["evidence.knowledge", "evidence.tool_results"];

pub(crate) fn run(prepared: Prepared, trace_id: Option<&str>) -> Assembly {
    let started = Clock::now();
    let mut timings = BTreeMap::new();
    let lap = |timings: &mut BTreeMap<String, f64>, name: &str, since: Clock| {
        timings.insert(name.to_string(), since.elapsed().as_secs_f64() * 1000.0);
    };
    let s = &prepared.loaded.snapshot;

    let stage = Clock::now();
    let admitted = admission::admit(s);
    lap(&mut timings, "admission_ms", stage);

    let stage = Clock::now();
    let conflicts = resolve::conflicts(s, &admitted.items);
    let mut gone = ids(&conflicts.excluded);
    let left = |gone: &BTreeSet<String>| -> Vec<&Item> { admitted.items.iter().filter(|item| !gone.contains(&item.id)).collect() };
    let superseded = resolve::supersede(s, &left(&gone));
    gone.extend(ids(&superseded));
    let duplicates = resolve::dedupe(s, &left(&gone));
    gone.extend(ids(&duplicates));
    let capped = resolve::diversity(s, &left(&gone));
    gone.extend(ids(&capped));
    lap(&mut timings, "resolution_ms", stage);

    let mut excluded = producer_rows(s);
    for rows in [&admitted.excluded, &conflicts.excluded, &superseded, &duplicates, &capped] {
        excluded.extend(rows.iter().map(|row| row.to_json()));
    }

    let mut trace = Trace {
        trace_id: trace_id.map_or_else(random_trace_id, str::to_string),
        profile: ProfileRef { id: s.profile.id.clone(), version: s.profile.version },
        budget: BudgetRecord { input: s.budget.input, reserved_output: s.budget.reserved_output, margin_percent: s.budget.margin_percent },
        result: None,
        included: Vec::new(),
        compressed: Vec::new(),
        excluded,
        conflicts: conflicts.records.clone(),
        refused: Refused { bool: false, reason: None },
        context: Context {
            spec: s.profile.spec.clone(),
            assembly_time: s.assembly_time.clone(),
            route_policy_version: s.route_policy.version.clone(),
            tokenizer: s.tokenizer.clone(),
            renderer: s.renderer.clone(),
            snapshot_digest: prepared.loaded.digest.clone(),
        },
        defaults_filled: admitted.defaults_filled.clone(),
        timings: BTreeMap::new(),
        recovery: None,
    };
    let finish = |mut trace: Trace, mut timings: BTreeMap<String, f64>, payload: Option<Vec<u8>>| {
        lap(&mut timings, "total_ms", started);
        trace.timings = timings;
        Assembly { payload, trace }
    };

    // Refusals before fitting, in contract/reasons.json order.
    let placed = |slot: &str| s.profile.placement.iter().any(|p| p.slot == slot);
    let has = |slot: &str| admitted.items.iter().any(|item| item.slot == slot);
    let mut required = vec!["governance.instructions", "interaction.query"];
    if s.route_policy.parser {
        required.push("governance.output_contract");
    }
    if let Some(slot) = required.into_iter().find(|slot| !has(slot)) {
        return finish(refuse(trace, "required_slot_missing", None, format!("no admitted item in {slot}")), timings, None);
    }
    if let Some(item) = admitted.items.iter().find(|item| item.tier == Tier::Protected && !placed(&item.slot)) {
        let detail = format!("protected item {} is in {}, which the profile does not place", item.id, item.slot);
        return finish(refuse(trace, "protected_slot_unplaced", None, detail), timings, None);
    }
    if !conflicts.refusing.is_empty() {
        let recovery = conflicts.refusing.iter().all(|a| *a == "request_context").then_some("request_context");
        return finish(refuse(trace, "conflict_unresolved", recovery, "a declared conflict group escalated".into()), timings, None);
    }

    let stage = Clock::now();
    let candidates: Vec<&Item> = left(&gone).into_iter().filter(|item| placed(&item.slot)).collect();
    let fitting = Fitter::new(s, prepared.tokenizer, prepared.renderer, candidates, &conflicts.surfaced).run();
    lap(&mut timings, "fitting_ms", stage);
    trace.excluded.extend(fitting.excluded.iter().map(|row| row.to_json()));

    let (rendered, included, compressed) = match fitting.outcome {
        Outcome::Refused(reason) => {
            let detail = if reason == "slot_floor_over_budget" {
                "the payload does not fit without shedding a slot below its min_tokens"
            } else {
                "protected content alone does not fit"
            };
            return finish(refuse(trace, reason, None, detail.into()), timings, None);
        }
        Outcome::Fitted { rendered, included, compressed } => (rendered, included, compressed),
    };

    // R-12: never answer from nothing.
    if s.route_policy.requires_evidence {
        let count = |slot: &str| included.iter().filter(|row| row.slot == slot).map(|row| row.item_id.as_str()).collect::<BTreeSet<_>>().len();
        let short_slot = EVIDENCE.iter().find(|slot| {
            s.route_policy.slot(slot).min_included.is_some_and(|min| (count(slot) as f64) < min)
        });
        if count(EVIDENCE[0]) + count(EVIDENCE[1]) == 0 || short_slot.is_some() {
            let omitted: Vec<bool> = fitting.omitted.iter().filter(|(slot, _)| EVIDENCE.contains(&slot.as_str())).map(|(_, v)| *v).collect();
            let action = if omitted.is_empty() {
                "request_context"
            } else if omitted.iter().any(|had_variants| !had_variants) {
                "precompute_summary"
            } else {
                "retrieve_narrower"
            };
            let detail = format!("the route requires evidence and {} survived admission and fitting",
                                 short_slot.map_or("none".to_string(), |slot| format!("too few {slot} items")));
            return finish(refuse(trace, "evidence_required", Some(action), detail), timings, None);
        }
    }

    trace.result = Some(ResultRecord { input_tokens: rendered.input_tokens, hash: canonical::sha256_hex(rendered.payload.as_bytes()) });
    trace.included = included;
    trace.compressed = compressed;
    finish(trace, timings, Some(rendered.payload.into_bytes()))
}

fn refuse(mut trace: Trace, reason: &str, recovery: Option<&'static str>, detail: String) -> Trace {
    trace.refused = Refused { bool: true, reason: Some(reason.to_string()) };
    trace.result = None;
    trace.included.clear();
    trace.compressed.clear();
    trace.recovery = recovery.map(|action| Recovery { action, detail });
    trace
}

/// Producer rows from every batch, as reported: by producer id, then item id, then the row's serialization (R-9).
fn producer_rows(s: &crate::model::Snapshot) -> Vec<Value> {
    let mut rows: Vec<(&str, &Value)> = s.batches.iter().flat_map(|b| b.excluded.iter().map(move |row| (b.producer.id.as_str(), row))).collect();
    rows.sort_by(|(pa, a), (pb, b)| {
        cmp_utf16(pa, pb)
            .then_with(|| cmp_utf16(a["item_id"].as_str().unwrap_or_default(), b["item_id"].as_str().unwrap_or_default()))
            .then_with(|| canonical::to_string(a).cmp(&canonical::to_string(b)))
    });
    rows.into_iter().map(|(_, row)| row.clone()).collect()
}

fn random_trace_id() -> String {
    let word = || RandomState::new().build_hasher().finish();
    format!("trace-{:016x}{:016x}", word(), word())
}
