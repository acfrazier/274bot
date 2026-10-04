//! The gating facts a [`crate::router::find`] search checks transport
//! edges against, built from a `GameSnapshot` at find time: inventory
//! stacks, worn items, skill levels, varps, completed quests, and the
//! bound world's `map_members` fact — plus, when the caller holds it, the
//! resolved quest progress evidence stage gates are tested with.
//!
//! Missing facts fail closed — [`WorldState::allows`] is false for any
//! requirement the state cannot prove, so an unpaid toll, an incomplete
//! quest, a missing level, an unbound members world, or a stage gate the
//! evidence leaves `Unknown` never routes.
//! There is no "assume yes". `map_members` is WORLD membership
//! (`Environment.node.members`), never the account or cache flag.

use std::collections::{HashMap, HashSet};

use api::selected::Truth;
use api::snapshot::GameSnapshot;

use crate::quest_gates::QuestEvidence;
use crate::transport::TransportEdge;

/// The quest journal's "completed" green as the **client stores it**: the
/// server sends `if_setcolour` as 15-bit 5-5-5 (`^green_rgb = 0xFF00` →
/// `rgb24to15` = `0x3E0`), and the client decodes it back to
/// `r<<19 | g<<11 | b<<3` — full green reads back `0xF800`, not the raw
/// `0x00FF00`. A quest-tab entry painted this value is done. The
/// not-started red (`0xF80000` as stored) and started yellow are not.
pub const QUEST_COMPLETE_COLOUR: i32 = 0xF800;

/// The facts a route may be gated against. [`WorldState::from_snapshot`]
/// fills it from a live `GameSnapshot`; anything the snapshot does not
/// carry stays empty and an edge needing it fails closed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorldState {
    /// obj id → stacked count carried (the snapshot's `inv` family).
    pub inv: HashMap<i32, i32>,
    /// obj ids currently worn (the snapshot's `equipment` family).
    pub worn: HashSet<i32>,
    /// BankBudget may satisfy the legacy any-of `worn_req` from positive
    /// counts in its available inventory. Never observed equipment, and
    /// never considered by the strict `worn_all_req` gate.
    pub allow_fetchable_worn: bool,
    /// skill id → effective level (the snapshot's `stats` family).
    pub stats: HashMap<i32, i32>,
    /// Base-stat combat level; absent until all seven combat rows are ready.
    /// Unknown levels activate every level-rule danger zone.
    pub combat_level: Option<i32>,
    /// varp index → value (the snapshot's `varps` family).
    pub varps: HashMap<i32, i32>,
    /// Quest names completed (green in the quest journal).
    pub quests: HashSet<String>,
    /// WORLD membership (`Environment.node.members` / `MAP_MEMBERS`).
    /// Default false: [`WorldState::from_snapshot`] cannot honestly fill
    /// this from `GameSnapshot` (account `members` is a different fact).
    /// Host-play / panel / tui / scenario or-in the bound profile fact
    /// via [`WorldState::with_map_members`].
    pub map_members: bool,
    /// Resolved quest progress stage gates are tested with
    /// ([`TransportEdge::quest_gates`]). `None` — every caller without a
    /// pinned progress provider — leaves each gated edge `Unknown`, so it is
    /// never taken. [`WorldState::from_snapshot`] cannot fill it: the
    /// snapshot's varps and quest list are not stage evidence.
    pub quest_evidence: Option<QuestEvidence>,
}

impl WorldState {
    /// The fail-closed empty state: no facts, so no requirement passes.
    pub fn empty() -> Self {
        WorldState::default()
    }

    /// Build from a `GameSnapshot`'s inv, equipment, stats, varps, and
    /// quest-status views. Families the snapshot has not loaded (an empty
    /// inv, an unopened quest tab, …) stay empty — edges needing them
    /// fail closed.
    pub fn from_snapshot(s: &GameSnapshot) -> Self {
        let mut inv = HashMap::new();
        for &(id, n) in s.inv() {
            let held = inv.entry(id).or_insert(0i32);
            *held = held.saturating_add(n);
        }
        let worn: HashSet<i32> = s
            .equipment()
            .iter()
            .filter(|it| it.count > 0)
            .map(|it| it.def.id)
            .collect();
        let stats: HashMap<i32, i32> = s
            .stats()
            .iter()
            .map(|st| (st.index, st.effective))
            .collect();
        let mut base = [0i64; 7];
        for stat in s.stats() {
            if let Ok(index) = usize::try_from(stat.index) {
                if index < base.len() {
                    base[index] = i64::from(stat.base);
                }
            }
        }
        let combat_level = base.iter().all(|&level| level > 0).then(|| {
            let [attack, defence, strength, hitpoints, ranged, prayer, magic] = base;
            let style = (attack + strength)
                .max(ranged / 2 + ranged)
                .max(magic / 2 + magic);
            // Integer arithmetic preserves the engine's floor, without
            // rounding boosted/effective levels into this base-stat fact.
            ((10 * (defence + hitpoints + prayer / 2) + 13 * style) / 40) as i32
        });
        let varps: HashMap<i32, i32> = s.varps().iter().map(|v| (v.index, v.value)).collect();
        let quests: HashSet<String> = s
            .quest_statuses()
            .iter()
            .filter(|q| q.colour == QUEST_COMPLETE_COLOUR)
            .map(|q| q.name.clone())
            .collect();
        WorldState {
            inv,
            worn,
            stats,
            combat_level,
            varps,
            quests,
            allow_fetchable_worn: false,
            map_members: false,
            quest_evidence: None,
        }
    }

    /// Bind the WORLD `map_members` fact from the selected server profile.
    /// `from_snapshot` always leaves this false; routing callers that can
    /// reach a members-required edge must apply the profile fact here.
    pub fn with_map_members(mut self, map_members: bool) -> Self {
        self.map_members = map_members;
        self
    }

    /// Attach the caller's resolved quest progress evidence (its immutable
    /// provider snapshot, source quest family and freshness floor). A
    /// refreshed provider replaces the previous evidence wholesale.
    pub fn with_quest_evidence(mut self, evidence: QuestEvidence) -> Self {
        self.quest_evidence = Some(evidence);
        self
    }

    /// Whether the edge's requirements are all satisfied: every
    /// `skill_req` level met, every held `item_req` and per-hop
    /// `consumed_req` count carried, every `quest_req` completed, every
    /// `varp_req` value reached, **any** `worn_req` alternative equipped (or
    /// available to the BankBudget probe), every `worn_all_req` item equipped
    /// in the observed state, a `members_req` edge only when
    /// [`Self::map_members`] is true, and every quest-stage gate `True` under
    /// [`Self::quest_evidence`]. Any requirement the state cannot prove fails
    /// the edge.
    pub fn allows(&self, e: &TransportEdge) -> bool {
        self.snapshot_allows(e) && self.quest_gates(e) == Truth::True
    }

    /// The edge's quest-stage gates under [`Self::quest_evidence`]: `True`
    /// for an ungated edge or proven gates, `False` when evidence disproves
    /// one, otherwise `Unknown` (no evidence, stale evidence, another quest
    /// family, possible values straddling a window bound). Only `True`
    /// authorizes a crossing, here and when it is rechecked.
    pub fn quest_gates(&self, e: &TransportEdge) -> Truth {
        e.quest_gates.as_ref().map_or(Truth::True, |gates| {
            gates.test(self.quest_evidence.as_ref())
        })
    }

    /// Every requirement except the quest-stage gates.
    pub(crate) fn snapshot_allows(&self, e: &TransportEdge) -> bool {
        self.fixed_reqs_allow(e)
            && e.item_req
                .iter()
                .all(|&(id, n)| self.inv.get(&id).is_some_and(|&c| c >= n))
            && e.consumed_req.iter().all(|&(id, packed_count)| {
                let carried = self.inv.get(&id).copied().unwrap_or(0);
                carried >= e.consumption_count(id, packed_count, carried)
            })
            && self.worn_req_allows(e)
            && self.worn_all_req_allows(e)
    }

    /// The legacy any-of gate also accepts items explicitly made available
    /// to a BankBudget route probe. `worn_all_req` deliberately does not.
    pub(crate) fn worn_req_allows(&self, e: &TransportEdge) -> bool {
        e.worn_req.is_empty()
            || e.worn_req.iter().any(|id| {
                self.worn.contains(id)
                    || (self.allow_fetchable_worn
                        && self.inv.get(id).is_some_and(|&count| count > 0))
            })
    }

    /// Every item in the conjunctive equipment gate must be currently worn.
    #[inline(always)]
    pub(crate) fn worn_all_req_allows(&self, e: &TransportEdge) -> bool {
        e.worn_all_req.is_empty() || self.worn_all_items_allow(&e.worn_all_req)
    }

    // Keep the equipment lookup loop out of the common ungated edge predicate.
    #[inline(never)]
    fn worn_all_items_allow(&self, ids: &[i32]) -> bool {
        ids.iter().all(|id| self.worn.contains(id))
    }

    /// Like [`WorldState::allows`] but ignoring the `item_req`/`worn_req`
    /// gates: every other requirement, including `worn_all_req` and
    /// quest-stage gates, still fails closed. This is the BankBudget
    /// diagnosis arm only ([`crate::router::find_missing_item_reqs`]
    /// feeds it the search's relaxed gate) — [`find`] and [`find_with`]
    /// never skip a carry/wear gate.
    pub fn allows_without_carry_worn(&self, e: &TransportEdge) -> bool {
        self.fixed_reqs_allow(e)
            && self.worn_all_req_allows(e)
            && self.quest_gates(e) == Truth::True
    }

    /// The requirements neither a bank trip nor a journal read can supply:
    /// membership, skill levels, completed quests and varp thresholds.
    /// Everything but `item_req`/`worn_req` and the quest-stage gates.
    // Keep one gate implementation without a nested call in the hot edge
    // predicate: an ordinary inline hint left it outlined in the bank-target
    // search, and forcing inlining measured about 1–2% faster there.
    #[inline(always)]
    pub(crate) fn fixed_reqs_allow(&self, e: &TransportEdge) -> bool {
        self.members_ok(e)
            && e.skill_req
                .iter()
                .all(|&(skill, level)| self.stats.get(&skill).is_some_and(|&l| l >= level))
            && e.quest_req.iter().all(|q| self.quests.contains(q))
            && e.varp_req
                .iter()
                .all(|&(varp, min)| self.varps.get(&varp).is_some_and(|&v| v >= min))
    }

    fn members_ok(&self, e: &TransportEdge) -> bool {
        !e.members_req || self.map_members
    }
}

#[cfg(test)]
#[path = "world_state_tests.rs"]
mod tests;
