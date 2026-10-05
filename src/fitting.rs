//! Fitting (conformance/README.md, Fitting; R-16, R-17, R-18).
//!
//! Every reduction under budget pressure is its own fit test over the whole rendered payload, and a slot floor
//! is checked against the slot's size as the reduction would leave it. Nothing here estimates: each decision is
//! the one the README's steps make. Token counts of rendered bodies are cached per item, wrap and body, since a
//! tokenizer reads nothing but its text (R-23).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::admission::Item;
use crate::model::{Snapshot, Tier};
use crate::render::{rendered_body, Occurrence, Rendered, Renderer};
use crate::resolve::rank;
use crate::strings::cmp_utf16;
use crate::tokenize::Tokenizer;
use crate::trace::{Compressed, Excluded, Included};

/// Which body an item renders: its own, or one of its variants by index.
type Body = Option<usize>;

/// How fitting ended.
pub enum Outcome {
    Fitted { rendered: Rendered, included: Vec<Included>, compressed: Vec<Compressed> },
    Refused(&'static str),
}

pub struct Fitting {
    pub outcome: Outcome,
    /// `over_budget` rows, in the order items were omitted.
    pub excluded: Vec<Excluded>,
    /// The slot of each item omitted for budget, and whether it had variants, for R-12's recovery action.
    pub omitted: Vec<(String, bool)>,
}

pub struct Fitter<'a> {
    s: &'a Snapshot,
    tokenizer: &'a dyn Tokenizer,
    renderer: Renderer,
    items: Vec<&'a Item>,
    surfaced: &'a BTreeMap<String, String>,
    /// Per placement, in profile order: its wrap, slot, and the indices of its slot's items in the renderer's order.
    placements: Vec<(&'a str, &'a str, Vec<usize>)>,
    included: Vec<bool>,
    body: Vec<Body>,
    frozen: BTreeSet<String>,
    excluded: Vec<Excluded>,
    omitted: Vec<(String, bool)>,
    counts: RefCell<HashMap<(usize, usize, Body), u64>>,
}

impl<'a> Fitter<'a> {
    /// Sets up fitting over the items left for it: admitted, placed, and not excluded by an earlier stage.
    pub fn new(s: &'a Snapshot, tokenizer: &'a dyn Tokenizer, renderer: Renderer, items: Vec<&'a Item>,
               surfaced: &'a BTreeMap<String, String>) -> Fitter<'a> {
        let placements = s.profile.placement.iter().map(|p| {
            let mut members: Vec<usize> = (0..items.len()).filter(|&i| items[i].slot == p.slot).collect();
            members.sort_by(|&a, &b| {
                let (a, b) = (items[a], items[b]);
                if p.slot == "interaction.history" {
                    a.freshness.cmp(&b.freshness).then_with(|| cmp_utf16(&a.id, &b.id))
                } else {
                    cmp_utf16(&a.id, &b.id)
                }
            });
            (p.wrap.as_str(), p.slot.as_str(), members)
        }).collect();
        let n = items.len();
        Fitter {
            s, tokenizer, renderer, items, surfaced, placements, included: vec![true; n], body: vec![None; n],
            frozen: BTreeSet::new(), excluded: Vec::new(), omitted: Vec::new(), counts: RefCell::new(HashMap::new()),
        }
    }

    pub fn run(mut self) -> Fitting {
        let outcome = self.fit();
        Fitting { outcome, excluded: self.excluded, omitted: self.omitted }
    }

    fn fit(&mut self) -> Outcome {
        // 1. Protected content must fit as it is.
        let protected: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].tier == Tier::Protected).collect();
        for &i in &protected {
            if let Some(cap) = self.items[i].token_budget {
                if self.size(i, None) as f64 > cap {
                    return Outcome::Refused("protected_content_over_budget");
                }
            }
        }
        for slot in self.slots() {
            if let Some(cap) = self.s.route_policy.slot(&slot).max_tokens {
                if self.slot_size_of(&slot, |i| self.items[i].tier == Tier::Protected) as f64 > cap {
                    return Outcome::Refused("protected_content_over_budget");
                }
            }
        }
        let all = self.included.clone();
        self.included = (0..self.items.len()).map(|i| self.items[i].tier == Tier::Protected).collect();
        let protected_fit = self.fits();
        self.included = all;
        if !protected_fit {
            return Outcome::Refused("protected_content_over_budget");
        }

        // 2. Item caps, in shedding order, whether or not the payload fits.
        for i in self.shedding_order() {
            let item = self.items[i];
            let Some(cap) = item.token_budget else { continue };
            if item.tier == Tier::Protected || self.size(i, None) as f64 <= cap {
                continue;
            }
            if item.tier == Tier::Compressible {
                let within = (0..item.variants.len()).filter(|&v| self.size(i, Some(v)) as f64 <= cap);
                if let Some(v) = self.longest(i, within) {
                    self.body[i] = Some(v);
                    continue;
                }
            }
            self.omit(i);
        }

        // 3. Slot caps, in shedding order, whether or not the payload fits; no floors.
        for slot in self.slots() {
            let Some(cap) = self.s.route_policy.slot(&slot).max_tokens else { continue };
            let within = |f: &Fitter| f.slot_size(&slot) as f64 <= cap;
            for i in self.slot_shedding(&slot, Tier::Droppable) {
                if within(self) {
                    break;
                }
                self.omit(i);
            }
            for (step_slot, action) in self.steps_for(Some(&slot)) {
                if within(self) {
                    break;
                }
                self.step(&step_slot, action, &within, false);
            }
        }

        // 4. While the payload does not fit, omit droppable items in shedding order, under slot floors. The fit
        // test runs only before a reduction that could be made, which changes no decision.
        for i in self.shedding_order() {
            if !self.included[i] || self.items[i].tier != Tier::Droppable || self.frozen.contains(&self.items[i].slot) {
                continue;
            }
            if self.fits() {
                break;
            }
            if self.floor_holds(i, None) {
                self.omit(i);
            }
        }

        // 5. While it still does not fit, reduce compressible items step by step, under slot floors.
        for (slot, action) in self.steps_for(None) {
            if self.fits() {
                break;
            }
            self.step(&slot, action, &|f: &Fitter| f.fits(), true);
        }

        // 7. Only a slot floor can leave the payload over budget.
        let rendered = self.render();
        if !self.charged_fits(rendered.input_tokens) {
            return Outcome::Refused("slot_floor_over_budget");
        }
        let (included, compressed) = self.rows();
        Outcome::Fitted { rendered, included, compressed }
    }

    /// The steps of step 5, or for one slot those of step 3: the route's `fitting_order` steps, then a `compress`
    /// and an `omit` step for each slot in shedding order, skipping any step the route listed.
    fn steps_for(&self, only: Option<&str>) -> Vec<(String, &'static str)> {
        let listed: Vec<(String, &'static str)> = self.s.route_policy.fitting_order.iter()
            .map(|step| (step.slot.clone(), if step.action == "compress" { "compress" } else { "omit" }))
            .collect();
        let mut steps: Vec<(String, &'static str)> = listed.clone();
        for action in ["compress", "omit"] {
            for slot in self.slots() {
                if !listed.iter().any(|(s, a)| *s == slot && *a == action) {
                    steps.push((slot, action));
                }
            }
        }
        steps.retain(|(slot, _)| only.map_or(true, |o| o == slot));
        steps
    }

    /// One `compress` or `omit` step: visit the slot's compressible items from the lowest rank up, and stop as
    /// soon as `done` holds. Under budget pressure a slot floor can withhold a reduction and freeze the slot.
    fn step(&mut self, slot: &str, action: &str, done: &dyn Fn(&Fitter) -> bool, floors: bool) {
        for i in self.slot_shedding(slot, Tier::Compressible) {
            if floors && self.frozen.contains(slot) {
                return;
            }
            if action == "omit" {
                if done(self) {
                    return;
                }
                if !floors || self.floor_holds(i, None) {
                    self.omit(i);
                }
                continue;
            }
            let current = self.size(i, self.body[i]);
            let shorter: Vec<usize> = (0..self.items[i].variants.len()).filter(|&v| self.size(i, Some(v)) < current).collect();
            if shorter.is_empty() {
                continue;
            }
            if done(self) {
                return;
            }
            let before = self.body[i];
            let fitting: Vec<usize> = shorter.iter().copied().filter(|&v| {
                self.body[i] = Some(v);
                let ok = done(self);
                self.body[i] = before;
                ok
            }).collect();
            let choice = if fitting.is_empty() { self.shortest(i, shorter.into_iter()) } else { self.longest(i, fitting.into_iter()) };
            let Some(v) = choice else { continue };
            if !floors || self.floor_holds(i, Some(v)) {
                self.body[i] = Some(v);
            }
        }
    }

    /// Whether a reduction keeps its slot at or above the slot's `min_tokens`; when it would not, the slot freezes.
    fn floor_holds(&mut self, i: usize, to: Option<usize>) -> bool {
        let slot = self.items[i].slot.clone();
        let Some(floor) = self.s.route_policy.slot(&slot).min_tokens else { return true };
        let (was_included, was_body) = (self.included[i], self.body[i]);
        match to {
            None => self.included[i] = false,
            Some(v) => self.body[i] = Some(v),
        }
        let after = self.slot_size(&slot);
        self.included[i] = was_included;
        self.body[i] = was_body;
        if (after as f64) < floor {
            self.frozen.insert(slot);
            false
        } else {
            true
        }
    }

    fn omit(&mut self, i: usize) {
        self.included[i] = false;
        let item = self.items[i];
        self.omitted.push((item.slot.clone(), !item.variants.is_empty()));
        self.excluded.push(Excluded::assembler(&item.id, "over_budget", Some(item.slot.clone())));
    }

    /// Among variants, the one with the most tokens, the earlier on ties.
    fn longest(&self, i: usize, variants: impl Iterator<Item = usize>) -> Option<usize> {
        variants.fold(None, |best: Option<usize>, v| match best {
            Some(b) if self.size(i, Some(b)) >= self.size(i, Some(v)) => Some(b),
            _ => Some(v),
        })
    }

    /// Among variants, the one with the fewest tokens, the earlier on ties.
    fn shortest(&self, i: usize, variants: impl Iterator<Item = usize>) -> Option<usize> {
        variants.fold(None, |best: Option<usize>, v| match best {
            Some(b) if self.size(i, Some(b)) <= self.size(i, Some(v)) => Some(b),
            _ => Some(v),
        })
    }

    /// The placed slots in shedding order: ascending `priority` (default 0), then slot name.
    fn slots(&self) -> Vec<String> {
        let mut slots: Vec<String> = self.placements.iter().map(|(_, slot, _)| slot.to_string()).collect::<BTreeSet<_>>().into_iter().collect();
        let priority = |slot: &str| self.s.route_policy.slot(slot).priority.unwrap_or(0.0);
        slots.sort_by(|a, b| priority(a).partial_cmp(&priority(b)).unwrap().then_with(|| cmp_utf16(a, b)));
        slots
    }

    /// Every item in shedding order: slots as `slots` orders them, and each slot's items from the lowest rank up.
    fn shedding_order(&self) -> Vec<usize> {
        self.slots().iter().flat_map(|slot| self.slot_shedding_all(slot)).collect()
    }

    fn slot_shedding_all(&self, slot: &str) -> Vec<usize> {
        let mut members: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].slot == slot).collect();
        members.sort_by(|&a, &b| rank(self.s, slot, self.items[b], self.items[a]));
        members
    }

    /// A slot's included items of one tier, from the lowest rank up.
    fn slot_shedding(&self, slot: &str, tier: Tier) -> Vec<usize> {
        self.slot_shedding_all(slot).into_iter().filter(|&i| self.included[i] && self.items[i].tier == tier).collect()
    }

    /// The tokens of one occurrence's rendered body: placement `p`, item `i`, body `b`.
    fn tokens(&self, p: usize, i: usize, b: Body) -> u64 {
        if let Some(&count) = self.counts.borrow().get(&(p, i, b)) {
            return count;
        }
        let count = self.tokenizer.count(&rendered_body(self.placements[p].0, self.text(i, b)));
        self.counts.borrow_mut().insert((p, i, b), count);
        count
    }

    fn text(&self, i: usize, b: Body) -> &'a str {
        let item = self.items[i];
        match b {
            None => &item.body,
            Some(v) => &item.variants[v].body,
        }
    }

    /// The size of a body: the largest of its occurrences' renderings, since a cap bounds a body however it renders.
    fn size(&self, i: usize, b: Body) -> u64 {
        self.placements.iter().enumerate().filter(|(_, (_, slot, _))| *slot == self.items[i].slot)
            .map(|(p, _)| self.tokens(p, i, b)).max().unwrap_or(0)
    }

    /// A slot's size: the tokens of its included items' rendered bodies, every occurrence counted.
    fn slot_size(&self, slot: &str) -> u64 {
        self.slot_size_of(slot, |i| self.included[i])
    }

    fn slot_size_of(&self, slot: &str, counted: impl Fn(usize) -> bool) -> u64 {
        self.placements.iter().enumerate().filter(|(_, (_, s, _))| *s == slot)
            .flat_map(|(p, (_, _, members))| members.iter().filter(|&&i| counted(i)).map(move |&i| (p, i)))
            .map(|(p, i)| self.tokens(p, i, self.body[i]))
            .sum()
    }

    fn occurrences(&self) -> Vec<Occurrence<'a>> {
        let mut out = Vec::new();
        for (wrap, slot, members) in &self.placements {
            for &i in members.iter().filter(|&&i| self.included[i]) {
                let item = self.items[i];
                out.push(Occurrence {
                    wrap, slot, id: &item.id, body: self.text(i, self.body[i]),
                    conflict: self.surfaced.get(&item.id).map(String::as_str), generated: item.lineage == "generated",
                });
            }
        }
        out
    }

    fn render(&self) -> Rendered {
        self.renderer.render(&self.occurrences(), self.tokenizer)
    }

    /// The fit test: the whole payload, rendered and counted, charged with the margin, against `budget.input`.
    fn fits(&self) -> bool {
        self.charged_fits(self.render().input_tokens)
    }

    fn charged_fits(&self, count: u64) -> bool {
        let margin = self.s.budget.margin_percent.unwrap_or(0.0) as u128;
        // The README's integer arithmetic, as it writes it: (n × (100 + m) + 99) / 100.
        #[allow(clippy::manual_div_ceil)]
        let charged = (u128::from(count) * (100 + margin) + 99) / 100;
        (charged as f64) <= self.s.budget.input
    }

    fn rows(&self) -> (Vec<Included>, Vec<Compressed>) {
        let mut included = Vec::new();
        let mut compressed = Vec::new();
        for (p, (_, slot, members)) in self.placements.iter().enumerate() {
            for &i in members.iter().filter(|&&i| self.included[i]) {
                let item = self.items[i];
                included.push(Included {
                    slot: slot.to_string(), item_id: item.id.clone(), tokens: self.tokens(p, i, self.body[i]),
                    source_version: item.source_version.clone(), eligibility: item.eligibility.clone(),
                });
                if let Some(v) = self.body[i] {
                    let variant = &item.variants[v];
                    compressed.push(Compressed {
                        slot: slot.to_string(), item_id: item.id.clone(), from: self.tokens(p, i, None), to: self.tokens(p, i, Some(v)),
                        method: variant.method.clone(), variant_id: variant.id.clone(),
                    });
                }
            }
        }
        (included, compressed)
    }
}
