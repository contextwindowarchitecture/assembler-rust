//! The stages between admission and fitting, in pipeline order: conflicts (R-6, R-11), supersession (R-25),
//! deduplication (R-24) and the source diversity cap (R-26). None of them reads a body except deduplication,
//! and none excludes a protected item (conformance/README.md, Conflicts, Supersession, Deduplication, Source
//! diversity).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::admission::Item;
use crate::model::{Snapshot, Tier};
use crate::strings::{cmp_utf16, dedupe_key};
use crate::trace::{ConflictRecord, Excluded};

/// Orders a slot's items from the highest rank down: its `order_by` keys, by default `-relevance` then
/// `-freshness`, and then `id` (Fitting). Unscored items rank last under `-relevance`.
pub fn rank(s: &Snapshot, slot: &str, a: &Item, b: &Item) -> Ordering {
    const DEFAULT: [&str; 2] = ["-relevance", "-freshness"];
    let keys: Vec<&str> = match &s.route_policy.slot(slot).order_by {
        Some(keys) => keys.iter().map(String::as_str).collect(),
        None => DEFAULT.to_vec(),
    };
    for key in keys {
        let order = match key {
            "-relevance" => match (a.relevance, b.relevance) {
                (Some(x), Some(y)) => y.partial_cmp(&x).expect("relevance is finite"),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
            "-freshness" => b.freshness.cmp(&a.freshness),
            "freshness" => a.freshness.cmp(&b.freshness),
            other => panic!("unknown order_by key {other}"),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    cmp_utf16(&a.id, &b.id)
}

/// What conflict resolution decides.
pub struct Conflicts {
    pub records: Vec<ConflictRecord>,
    /// Rows ordered by item id.
    pub excluded: Vec<Excluded>,
    /// Members of surfaced groups, and their group: the renderer marks them.
    pub surfaced: BTreeMap<String, String>,
    /// The escalation actions of groups that refuse the assembly: `request_context` or `refuse`.
    pub refusing: Vec<&'static str>,
}

/// Resolves the declared conflict groups over the admitted items (Conflicts).
pub fn conflicts(s: &Snapshot, items: &[Item]) -> Conflicts {
    let by_id: BTreeMap<&str, &Item> = items.iter().map(|item| (item.id.as_str(), item)).collect();
    let mut groups: Vec<_> = s.conflicts.iter().collect();
    groups.sort_by(|a, b| cmp_utf16(&a.id, &b.id));
    let mut out = Conflicts { records: Vec::new(), excluded: Vec::new(), surfaced: BTreeMap::new(), refusing: Vec::new() };
    for group in groups {
        let mut named = group.items.clone();
        named.sort_by(|a, b| cmp_utf16(a, b));
        let members: Vec<&Item> = named.iter().filter_map(|id| by_id.get(id.as_str()).copied()).collect();
        let decision = if members.len() < 2 {
            Decision::Moot
        } else if group.kind == "instruction" {
            instruction(&members)
        } else {
            fact(s, group.fact.as_deref().expect("fact groups name a fact"), &members)
        };
        let decision = match decision {
            // A decision that would exclude a protected item escalates instead.
            Decision::Excludes { losers, .. } if losers.iter().any(|item| item.tier == Tier::Protected) => Decision::Escalated,
            other => other,
        };
        let mut record = ConflictRecord {
            group_id: group.id.clone(), kind: group.kind.clone(), items: named.clone(), resolution: "resolved", decided_by: "", winner: None,
        };
        match decision {
            Decision::Moot => {
                record.resolution = "moot";
                record.decided_by = "moot";
            }
            Decision::Authority(winner) => {
                record.decided_by = "authority";
                record.winner = winner.map(|item| item.id.clone());
            }
            Decision::Excludes { winner, losers, decided_by, reason } => {
                record.decided_by = decided_by;
                record.winner = Some(winner.id.clone());
                for loser in losers {
                    out.excluded.push(Excluded::assembler(&loser.id, reason, Some(loser.slot.clone())));
                }
            }
            Decision::Escalated => {
                record.decided_by = "escalated";
                let action = if group.kind == "instruction" {
                    s.route_policy.on_unresolved_instruction.as_deref().unwrap_or("refuse")
                } else {
                    s.route_policy.facts[group.fact.as_deref().unwrap()].on_unresolved.as_str()
                };
                match action {
                    "surface" => {
                        record.resolution = "surfaced";
                        for member in &members {
                            out.surfaced.insert(member.id.clone(), group.id.clone());
                        }
                    }
                    "request_context" => {
                        record.resolution = "context_requested";
                        out.refusing.push("request_context");
                    }
                    _ => {
                        record.resolution = "refused";
                        out.refusing.push("refuse");
                    }
                }
            }
        }
        out.records.push(record);
    }
    out.excluded.sort_by(|a, b| cmp_utf16(&a.item_id, &b.item_id));
    out
}

enum Decision<'a> {
    Moot,
    Authority(Option<&'a Item>),
    Excludes { winner: &'a Item, losers: Vec<&'a Item>, decided_by: &'static str, reason: &'static str },
    Escalated,
}

/// An instruction group: only `governing` and `user` members may instruct, and the peers are those at the highest
/// of the two present; `conflict_policy` decides among two or more peers (R-6, R-11).
fn instruction<'a>(members: &[&'a Item]) -> Decision<'a> {
    let top = if members.iter().any(|m| m.authority == "governing") { "governing" } else { "user" };
    let peers: Vec<&Item> = members.iter().copied().filter(|m| m.authority == top).collect();
    if peers.len() <= 1 {
        return Decision::Authority(peers.first().copied());
    }
    let governing: Vec<&Item> = peers.iter().copied().filter(|p| p.conflict_policy == "governs").collect();
    let deferring: Vec<&Item> = peers.iter().copied().filter(|p| p.conflict_policy == "defers").collect();
    if governing.len() == 1 && deferring.len() == peers.len() - 1 {
        Decision::Excludes { winner: governing[0], losers: deferring, decided_by: "policy", reason: "conflict_deferred" }
    } else {
        Decision::Escalated
    }
}

/// A fact group: the eligible members whose producer comes first in the route's precedence lead, and a single
/// leader wins, or the single newest leader when the policy breaks ties by freshness (R-11).
fn fact<'a>(s: &Snapshot, fact: &str, members: &[&'a Item]) -> Decision<'a> {
    let policy = &s.route_policy.facts[fact];
    let place = |item: &Item| policy.precedence.iter().position(|p| *p == item.producer);
    let eligible: Vec<(usize, &Item)> = members.iter()
        .filter(|m| policy.scope.iter().all(|key| m.scope.contains_key(key)))
        .filter_map(|m| place(m).map(|p| (p, *m)))
        .collect();
    let Some(first) = eligible.iter().map(|(p, _)| *p).min() else { return Decision::Escalated };
    let leaders: Vec<&Item> = eligible.iter().filter(|(p, _)| *p == first).map(|(_, m)| *m).collect();
    let (winner, decided_by) = if leaders.len() == 1 {
        (leaders[0], "policy")
    } else if policy.freshness_tiebreak {
        let newest = leaders.iter().map(|l| &l.freshness).max().unwrap();
        let newest: Vec<&Item> = leaders.iter().copied().filter(|l| &l.freshness == newest).collect();
        if newest.len() != 1 {
            return Decision::Escalated;
        }
        (newest[0], "freshness")
    } else {
        return Decision::Escalated;
    };
    let losers = members.iter().copied().filter(|m| m.id != winner.id).collect();
    Decision::Excludes { winner, losers, decided_by, reason: "conflict_lost" }
}

/// Whether an item is exempt from supersession, deduplication and the diversity cap: protected, or named by a
/// conflict group whatever its resolution.
fn exempt(s: &Snapshot, item: &Item) -> bool {
    item.tier == Tier::Protected || s.conflicts.iter().any(|g| g.items.contains(&item.id))
}

/// Groups the remaining items of each slot that sets a rule, by a key, in a deterministic order.
fn groups_by<'a, K: Ord>(items: &[&'a Item], applies: impl Fn(&str) -> bool, key: impl Fn(&Item) -> K) -> Vec<Vec<&'a Item>> {
    let mut groups: BTreeMap<(String, K), Vec<&Item>> = BTreeMap::new();
    for item in items.iter().copied().filter(|item| applies(&item.slot)) {
        groups.entry((item.slot.clone(), key(item))).or_default().push(item);
    }
    groups.into_values().collect()
}

/// Supersession: within a slot that asks for it, only the latest observations of each producer and source stay
/// (R-25). Rows ordered by item id.
pub fn supersede(s: &Snapshot, items: &[&Item]) -> Vec<Excluded> {
    let mut rows = Vec::new();
    let applies = |slot: &str| s.route_policy.slot(slot).supersede.as_deref() == Some("source");
    for call in groups_by(items, applies, |item| (item.producer.clone(), item.source.clone())) {
        let latest = call.iter().map(|item| &item.freshness).max().expect("a call has items");
        let mut kept: Vec<&Item> = call.iter().copied().filter(|item| &item.freshness == latest).collect();
        kept.sort_by(|a, b| rank(s, &a.slot, a, b));
        for item in call.iter().filter(|item| &item.freshness != latest && !exempt(s, item)) {
            let mut row = Excluded::assembler(&item.id, "superseded", Some(item.slot.clone()));
            row.superseded_by = Some(kept[0].id.clone());
            rows.push(row);
        }
    }
    rows.sort_by(|a, b| cmp_utf16(&a.item_id, &b.item_id));
    rows
}

/// Deduplication: within a slot that asks for it, equal bodies once whitespace is collapsed keep their exempt
/// members, or else their highest-ranked one (R-24). Rows ordered by item id.
pub fn dedupe(s: &Snapshot, items: &[&Item]) -> Vec<Excluded> {
    let mut rows = Vec::new();
    let applies = |slot: &str| s.route_policy.slot(slot).dedupe.as_deref() == Some("exact");
    for mut set in groups_by(items, applies, |item| dedupe_key(&item.body).encode_utf16().collect::<Vec<u16>>()) {
        if set.len() < 2 {
            continue;
        }
        set.sort_by(|a, b| rank(s, &a.slot, a, b));
        let exempt_members: Vec<&Item> = set.iter().copied().filter(|item| exempt(s, item)).collect();
        let kept = exempt_members.first().copied().unwrap_or(set[0]);
        for item in set.iter().filter(|item| item.id != kept.id && !exempt(s, item)) {
            let mut row = Excluded::assembler(&item.id, "duplicate_content", Some(item.slot.clone()));
            row.duplicate_of = Some(kept.id.clone());
            rows.push(row);
        }
    }
    rows.sort_by(|a, b| cmp_utf16(&a.item_id, &b.item_id));
    rows
}

/// The source diversity cap: within a slot that sets `max_per_source`, each producer and source keeps its exempt
/// items and then its highest-ranked others up to the cap (R-26). Rows ordered by item id.
pub fn diversity(s: &Snapshot, items: &[&Item]) -> Vec<Excluded> {
    let mut rows = Vec::new();
    let applies = |slot: &str| s.route_policy.slot(slot).max_per_source.is_some();
    for mut source in groups_by(items, applies, |item| (item.producer.clone(), item.source.clone())) {
        let cap = s.route_policy.slot(&source[0].slot).max_per_source.unwrap();
        let exempt_count = source.iter().filter(|item| exempt(s, item)).count() as f64;
        source.sort_by(|a, b| rank(s, &a.slot, a, b));
        let mut places = (cap - exempt_count).max(0.0);
        for item in source.iter().filter(|item| !exempt(s, item)) {
            if places >= 1.0 {
                places -= 1.0;
            } else {
                rows.push(Excluded::assembler(&item.id, "source_diversity_cap", Some(item.slot.clone())));
            }
        }
    }
    rows.sort_by(|a, b| cmp_utf16(&a.item_id, &b.item_id));
    rows
}

/// The ids in a list of rows.
pub fn ids(rows: &[Excluded]) -> BTreeSet<String> {
    rows.iter().map(|row| row.item_id.clone()).collect()
}
