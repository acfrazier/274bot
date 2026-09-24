//! The gating facts a [`crate::router::find`] search checks transport
//! edges against, built from a `GameSnapshot` at find time: inventory
//! stacks, worn items, skill levels, varps, completed quests, and the
//! bound world's `map_members` fact.
//!
//! Missing facts fail closed — [`WorldState::allows`] is false for any
//! requirement the state cannot prove, so an unpaid toll, an incomplete
//! quest, a missing level, or an unbound members world never routes.
//! There is no "assume yes". `map_members` is WORLD membership
//! (`Environment.node.members`), never the account or cache flag.

use std::collections::{HashMap, HashSet};

use api::snapshot::GameSnapshot;

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
    /// skill id → effective level (the snapshot's `stats` family).
    pub stats: HashMap<i32, i32>,
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
            *inv.entry(id).or_insert(0) += n;
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
            varps,
            quests,
            map_members: false,
        }
    }

    /// Bind the WORLD `map_members` fact from the selected server profile.
    /// `from_snapshot` always leaves this false; routing callers that can
    /// reach a members-required edge must apply the profile fact here.
    pub fn with_map_members(mut self, map_members: bool) -> Self {
        self.map_members = map_members;
        self
    }

    /// Whether the edge's requirements are all satisfied: every
    /// `skill_req` level met, every `item_req` count carried, every
    /// `quest_req` completed, every `varp_req` value reached, **any**
    /// `worn_req` obj worn (empty is no worn gate — a Dramen staff is a
    /// one-id list; a slash-weapon web lists every slash blade), and a
    /// `members_req` edge only when [`Self::map_members`] is true. Any
    /// requirement the state cannot prove fails the edge.
    pub fn allows(&self, e: &TransportEdge) -> bool {
        self.members_ok(e)
            && e.skill_req
                .iter()
                .all(|&(skill, level)| self.stats.get(&skill).is_some_and(|&l| l >= level))
            && e.item_req
                .iter()
                .all(|&(id, n)| self.inv.get(&id).is_some_and(|&c| c >= n))
            && e.quest_req.iter().all(|q| self.quests.contains(q))
            && e.varp_req
                .iter()
                .all(|&(varp, min)| self.varps.get(&varp).is_some_and(|&v| v >= min))
            && (e.worn_req.is_empty() || e.worn_req.iter().any(|id| self.worn.contains(id)))
    }

    /// Like [`WorldState::allows`] but ignoring the `item_req`/`worn_req`
    /// gates: every other requirement still fails closed. This is the
    /// BankBudget diagnosis arm only ([`crate::router::find_missing_item_reqs`]
    /// feeds it the search's relaxed gate) — [`find`] and [`find_with`]
    /// never skip a carry/wear gate.
    pub fn allows_without_carry_worn(&self, e: &TransportEdge) -> bool {
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
