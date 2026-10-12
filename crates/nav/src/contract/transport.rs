//! Transport facts: kind, anchor, landing, takeoff, cost, and distinct
//! held/consumed/returned/worn-any/worn-all requirements. Does not expose
//! `TransportEdge` vectors or packed `QuestGates` layout.

use api::snapshot::WorldTile;

use crate::transport::TransportEdge;

use super::{DoorDir, TransportKind};

/// Borrowed transport facts of one hop.
#[derive(Clone, Copy, Debug)]
pub struct TransportView<'a> {
    edge: &'a TransportEdge,
}

impl<'a> TransportView<'a> {
    pub(super) fn from_edge(edge: &'a TransportEdge) -> Self {
        Self { edge }
    }

    /// Kind semantics (Door, Ladder, Stairs, Boat, Teleport,
    /// AgilityShortcut, Glider, SpiritTree, Npc, EssenceExit).
    pub fn kind(self) -> TransportKind {
        self.edge.kind
    }

    /// Exact tick cost of this hop (never truncated).
    pub fn ticks(self) -> i32 {
        self.edge.ticks
    }

    /// Interaction anchor (`at`).
    pub fn anchor(self) -> WorldTile {
        self.edge.at
    }

    /// Landing (`to`).
    pub fn landing(self) -> WorldTile {
        self.edge.to
    }

    /// Optional exact takeoff; `None` keeps radius/footprint admission.
    pub fn takeoff(self) -> Option<WorldTile> {
        self.edge.takeoff
    }

    /// Effective takeoff: `takeoff.unwrap_or(at)`.
    pub fn effective_takeoff(self) -> WorldTile {
        self.edge.takeoff.unwrap_or(self.edge.at)
    }

    /// Player-relative displacement, when packed.
    pub fn displacement(self) -> Option<WorldTile> {
        self.edge.player_delta
    }

    /// Loc/NPC id the hop interacts with.
    pub fn loc_id(self) -> i32 {
        self.edge.loc_id
    }

    /// Interaction option (`oplocu` 0 / `oploc1` 1).
    pub fn option(self) -> i32 {
        self.edge.option
    }

    /// Door crossing direction, if any.
    pub fn direction(self) -> Option<DoorDir> {
        self.edge.dir
    }

    /// Reusable held-item requirement `(obj id, count)` pairs. Distinct
    /// from consumed supply.
    pub fn held(self) -> &'a [(i32, i32)] {
        &self.edge.item_req
    }

    /// Consumable inventory spent by a successful hop.
    pub fn consumed(self) -> &'a [(i32, i32)] {
        &self.edge.consumed_req
    }

    /// Replacement inventory credited after consumption.
    pub fn returned(self) -> &'a [(i32, i32)] {
        &self.edge.item_returns
    }

    /// Worn-any: any one of these obj ids must be equipped.
    pub fn worn_any(self) -> &'a [i32] {
        &self.edge.worn_req
    }

    /// Worn-all: every listed obj id must be equipped. Cannot be satisfied
    /// by an alternative, a carried item, or a banked item.
    pub fn worn_all(self) -> &'a [i32] {
        &self.edge.worn_all_req
    }

    /// WORLD membership required.
    pub fn members_required(self) -> bool {
        self.edge.members_req
    }

    /// Teleport wilderness cap, if any.
    pub fn wilderness_cap(self) -> Option<i32> {
        self.edge.wildy_cap
    }

    /// Whether a quest-stage gate is present. The packed `QuestGates`
    /// layout is not exposed.
    pub fn has_quest_gate(self) -> bool {
        self.edge.quest_gates.is_some()
    }

    /// Skill `(id, level)` requirements.
    pub fn skill_requirements(self) -> &'a [(i32, i32)] {
        &self.edge.skill_req
    }
}
