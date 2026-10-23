//! Transport facts: kind, anchor, landing, takeoff, cost, and distinct
//! held/consumed/returned/worn-any/worn-all requirements. Does not expose
//! `TransportEdge` vectors or packed `QuestGates` layout.

use std::fmt;

use api::snapshot::WorldTile;

use crate::transport::TransportEdge;

use super::{DoorDir, TransportKind};

/// Borrowed transport facts of one hop.
#[derive(Clone, Copy)]
pub struct TransportView<'a> {
    source: TransportSource<'a>,
}

#[derive(Clone, Copy)]
enum TransportSource<'a> {
    Edge {
        edge: &'a TransportEdge,
        landing: Option<WorldTile>,
    },
    Essence {
        anchor: WorldTile,
        landing: WorldTile,
        loc_id: i32,
        option: i32,
        ticks: i32,
    },
}

impl<'a> TransportView<'a> {
    pub(super) fn from_edge(edge: &'a TransportEdge) -> Self {
        Self {
            source: TransportSource::Edge {
                edge,
                landing: None,
            },
        }
    }

    pub(super) fn from_compact_step(step: crate::router::CompactTransportStep<'a>) -> Self {
        match step.source() {
            Some(edge) => Self {
                source: TransportSource::Edge {
                    edge,
                    landing: Some(step.actual_to()),
                },
            },
            None => Self {
                source: TransportSource::Essence {
                    anchor: step.anchor(),
                    landing: step.actual_to(),
                    loc_id: step.loc_id(),
                    option: step.option(),
                    ticks: step.ticks(),
                },
            },
        }
    }

    fn edge(self) -> Option<&'a TransportEdge> {
        match self.source {
            TransportSource::Edge { edge, .. } => Some(edge),
            TransportSource::Essence { .. } => None,
        }
    }

    /// Kind semantics (Door, Ladder, Stairs, Boat, Teleport,
    /// AgilityShortcut, Glider, SpiritTree, Npc, EssenceExit).
    pub fn kind(self) -> TransportKind {
        self.edge()
            .map_or(TransportKind::EssenceExit, |edge| edge.kind)
    }

    /// Exact tick cost of this hop (never truncated).
    pub fn ticks(self) -> i32 {
        match self.source {
            TransportSource::Edge { edge, .. } => edge.ticks,
            TransportSource::Essence { ticks, .. } => ticks,
        }
    }

    /// Interaction anchor (`at`).
    pub fn anchor(self) -> WorldTile {
        match self.source {
            TransportSource::Edge { edge, .. } => edge.at,
            TransportSource::Essence { anchor, .. } => anchor,
        }
    }

    /// Landing (`to`), including the exact player-relative route landing.
    pub fn landing(self) -> WorldTile {
        match self.source {
            TransportSource::Edge { edge, landing } => landing.unwrap_or(edge.to),
            TransportSource::Essence { landing, .. } => landing,
        }
    }

    /// Optional exact takeoff; `None` keeps radius/footprint admission.
    pub fn takeoff(self) -> Option<WorldTile> {
        self.edge().and_then(|edge| edge.takeoff)
    }

    /// Effective takeoff: `takeoff.unwrap_or(at)`.
    pub fn effective_takeoff(self) -> WorldTile {
        self.edge()
            .map_or_else(|| self.anchor(), |edge| edge.takeoff.unwrap_or(edge.at))
    }

    /// Player-relative displacement, when packed.
    pub fn displacement(self) -> Option<WorldTile> {
        self.edge().and_then(|edge| edge.player_delta)
    }

    /// Loc/NPC id the hop interacts with.
    pub fn loc_id(self) -> i32 {
        match self.source {
            TransportSource::Edge { edge, .. } => edge.loc_id,
            TransportSource::Essence { loc_id, .. } => loc_id,
        }
    }

    /// Interaction option (`oplocu` 0 / `oploc1` 1).
    pub fn option(self) -> i32 {
        match self.source {
            TransportSource::Edge { edge, .. } => edge.option,
            TransportSource::Essence { option, .. } => option,
        }
    }

    /// Door crossing direction, if any.
    pub fn direction(self) -> Option<DoorDir> {
        self.edge().and_then(|edge| edge.dir)
    }

    /// Reusable held-item requirement `(obj id, count)` pairs. Distinct
    /// from consumed supply.
    pub fn held(self) -> &'a [(i32, i32)] {
        self.edge().map_or(&[], |edge| &edge.item_req)
    }

    /// Consumable inventory spent by a successful hop.
    pub fn consumed(self) -> &'a [(i32, i32)] {
        self.edge().map_or(&[], |edge| &edge.consumed_req)
    }

    /// Replacement inventory credited after consumption.
    pub fn returned(self) -> &'a [(i32, i32)] {
        self.edge().map_or(&[], |edge| &edge.item_returns)
    }

    /// Worn-any: any one of these obj ids must be equipped.
    pub fn worn_any(self) -> &'a [i32] {
        self.edge().map_or(&[], |edge| &edge.worn_req)
    }

    /// Worn-all: every listed obj id must be equipped. Cannot be satisfied
    /// by an alternative, a carried item, or a banked item.
    pub fn worn_all(self) -> &'a [i32] {
        self.edge().map_or(&[], |edge| &edge.worn_all_req)
    }

    /// Obj ids that must be absent from an observed held inventory.
    pub fn forbidden_held(self) -> &'a [i32] {
        self.edge().map_or(&[], |edge| &edge.forbidden_held)
    }

    /// Obj ids that must be absent from observed equipment.
    pub fn forbidden_worn(self) -> &'a [i32] {
        self.edge().map_or(&[], |edge| &edge.forbidden_worn)
    }

    /// WORLD membership required.
    pub fn members_required(self) -> bool {
        self.edge().is_some_and(|edge| edge.members_req)
    }

    /// Teleport wilderness cap, if any.
    pub fn wilderness_cap(self) -> Option<i32> {
        self.edge().and_then(|edge| edge.wildy_cap)
    }

    /// Whether a quest-stage gate is present. The packed `QuestGates`
    /// layout is not exposed.
    pub fn has_quest_gate(self) -> bool {
        self.edge().is_some_and(|edge| edge.quest_gates.is_some())
    }

    /// Skill `(id, level)` requirements.
    pub fn skill_requirements(self) -> &'a [(i32, i32)] {
        self.edge().map_or(&[], |edge| &edge.skill_req)
    }
}

impl fmt::Debug for TransportView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransportView")
            .field("kind", &self.kind())
            .field("anchor", &self.anchor())
            .field("landing", &self.landing())
            .finish_non_exhaustive()
    }
}
