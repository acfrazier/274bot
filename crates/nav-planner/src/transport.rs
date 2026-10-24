//! Runtime transport graph types and policies.
//!
//! Content producers live in `nav-bake`; the planner keeps the graph schema,
//! runtime edge admission, supply accounting, and shared transport rules.

use std::collections::HashMap;

use api::snapshot::WorldTile;

use crate::collision::WorldCollision;
use crate::quest_gates::{QuestFamilyId, QuestGates};

mod static_routes;
mod webs;
mod wilderness;

pub(crate) use webs::select_web_action;
pub use wilderness::{WildernessRules, WildernessZone};
/// The kinds of transport edge this graph derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// A closed/open directional loc crossing (wall doors and slashable webs).
    Door,
    /// A ladder placement (climb up/down per the script's op).
    Ladder,
    /// A staircase placement.
    Stairs,
    /// A ship/boat journey (origin is the dock NPC's tile).
    Boat,
    /// A magic teleport spell (destination is the spell's landing).
    Teleport,
    /// An agility shortcut (stile, wall climb, …).
    AgilityShortcut,
    /// A gnome-glider flight between two fixed platforms.
    Glider,
    /// A spirit-tree journey between a tree loc tile and a sibling tree's
    /// tile (the `^…_tree` destination constant).
    SpiritTree,
    /// An NPC-triggered transport hop (carts, essence-mine wizards,
    /// Elkoy's maze escorts).
    Npc,
    /// The Rune Essence mine exit portal (`blankrunestone_exit_portal`):
    /// never packed — the pack carries wizard → mine-pad entry edges
    /// only. The router synthesizes one per live [`crate::essence::EssenceSession`],
    /// returning to the entry wizard's overworld anchor.
    EssenceExit,
}

/// The crossing direction of a door edge (step 2 derives it from the door
/// angle); `None` for every other edge kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DoorDir {
    N,
    E,
    S,
    W,
}

/// One directed transport hop. `at` remains the loc/NPC interaction anchor
/// and `to` the resolved landing; optional `takeoff` pins conditional or
/// multi-step content to its exact required game-plane starting tile. A
/// pinned takeoff is the only admissible stand, rather than a nearby approach.
/// Requirement vectors are `(skill id, level)` /
/// `(item id, count)` pairs and spell/quest names and `(varp, value)` pairs,
/// filled from what the source scripts/defs declare. `item_req` is a
/// reusable held-item gate: satisfying it does not consume the item.
/// `consumed_req` is the `(obj id, count)` supply spent by this hop.
/// `item_returns` is the `(obj id, count)` replacement credited after that
/// spend, derived only when the source proves an inventory transformation.
/// `worn_req` is the obj ids of which **any one** must be equipped (a Dramen
/// staff is a one-id list; slashable webs list every blade whose
/// `slashattack_anim` is not unarmed). `worn_all_req` is a conjunctive
/// equipment gate: every listed obj id must currently be equipped. `option` 0
/// on a loc hop means use the first `item_req` obj on the loc (`oplocu`);
/// option 1 is `oploc1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportEdge {
    pub kind: TransportKind,
    pub at: WorldTile,
    pub to: WorldTile,
    /// Exact required starting tile; `None` preserves radius/footprint
    /// admission for existing transports.
    pub takeoff: Option<WorldTile>,
    /// Content `movecoord(coord, ...)` displacement from the actual operable
    /// takeoff stand. `None` is an absolute landing. Packed in the kind byte's
    /// high bit; the canonical graph's `to - at` supplies the displacement.
    pub player_delta: Option<WorldTile>,
    pub loc_id: i32,
    pub option: i32,
    pub ticks: i32,
    pub dir: Option<DoorDir>,
    pub open_loc_id: Option<i32>,
    pub skill_req: Vec<(i32, i32)>,
    /// Reusable held-item requirement `(obj id, count)` pairs.
    pub item_req: Vec<(i32, i32)>,
    /// Consumable inventory supply spent by a successful hop.
    pub consumed_req: Vec<(i32, i32)>,
    /// Content-derived replacement inventory credited after consumption.
    pub item_returns: Vec<(i32, i32)>,
    pub quest_req: Vec<String>,
    pub varp_req: Vec<(i32, i32)>,
    pub worn_req: Vec<i32>,
    /// Conjunctive equipment prerequisites: every obj id must be equipped.
    /// Unlike `worn_req`, this cannot be satisfied by an alternative, a
    /// carried item, or a banked item.
    pub worn_all_req: Vec<i32>,
    /// Each listed obj must be absent from an observed held inventory.
    pub forbidden_held: Vec<i32>,
    /// Each listed obj must be absent from observed worn equipment.
    /// Carried or banked copies do not violate this gate.
    pub forbidden_worn: Vec<i32>,
    /// WORLD membership required (`MAP_MEMBERS`). Set from the edge's source
    /// handler (see `members_guard`): an F2P refusal on its leading path
    /// gates every edge through it; gliders, Zanaris, spirit trees, stage
    /// doors and the spell table read their own `map_members` shape. The
    /// bake refuses a graph whose free edge's source reads `map_members`,
    /// or whose members edge has no refusal or pinned members arm. Packed
    /// as a `u8` on the v9 wire.
    pub members_req: bool,
    /// Content-derived max wilderness level this teleport may be used from
    /// (`~wilderness_level(coord) > cap` refuses). `None` = no wilderness cap.
    /// Packed as i32le on the v10 wire after `members_req` (`-1` = None).
    pub wildy_cap: Option<i32>,
    /// Quest-stage gates ([`QuestGates`]: a completed quest or a closed
    /// stage window over a selected progress signal), bound to the quest
    /// family the pack was baked against. `None` = no stage gate. Only
    /// resolved progress evidence from that family can open a gated edge;
    /// snapshot varps and the quest list never do. Packed on the v11 wire
    /// after `wildy_cap`.
    pub quest_gates: Option<QuestGates>,
}

impl TransportEdge {
    /// Supply consumed at this hop's carried balance. The Shilo carts
    /// pack a minimum; their content charges five percent, clamped to
    /// 10–200. Other edges use the packed fixed consumption.
    pub fn consumption_count(&self, id: i32, packed_count: i32, carried: i32) -> i32 {
        static_routes::cart_fare(self, id, carried).unwrap_or(packed_count)
    }

    /// Minimum pre-hop stack that leaves `needed_after` after consumption.
    /// Backward route budgeting avoids underfetching when a larger coin
    /// float itself raises the cart fare.
    pub(crate) fn required_supply_before(
        &self,
        id: i32,
        packed_count: i32,
        needed_after: i32,
    ) -> i32 {
        if static_routes::cart_fare(self, id, 0).is_none() {
            return needed_after.saturating_add(packed_count);
        }
        let mut low = needed_after.saturating_add(10);
        let mut high = needed_after.saturating_add(200);
        while low < high {
            let mid = low + (high - low) / 2;
            let fare = self.consumption_count(id, packed_count, mid);
            if mid.saturating_sub(fare) >= needed_after {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        low
    }

    /// Resolve a landing from the actual takeoff, not the loc's anchor.
    /// Reconstructed route edges keep their exact planned `to` while retaining
    /// this content delta for live settlement from the stand used at send time.
    /// Returns `None` when applying the relative displacement overflows.
    pub fn landing_from(&self, from: WorldTile) -> Option<WorldTile> {
        match self.player_delta {
            Some(delta) => Some(WorldTile {
                x: from.x.checked_add(delta.x)?,
                z: from.z.checked_add(delta.z)?,
                level: from.level.checked_add(delta.level)?,
            }),
            None => Some(self.to),
        }
    }

    /// Whether this is one of the packed `bigweb_slashable` crossings.
    /// Revision 289 identifies the closed/slashed loc pair as 733/734.
    pub(crate) fn is_slashable_web(&self) -> bool {
        self.kind == TransportKind::Door
            && self.loc_id == 733
            && self.open_loc_id == Some(734)
            && self.dir.is_some()
            && matches!(self.option, 0 | 1)
    }
}

/// Transport edges indexed by exact pinned takeoff tiles, operable footprint
/// stands, or radius-one interact anchors (`graph.at` lists edge indexes).
#[derive(Debug, Default)]
pub struct TransportGraph {
    pub edges: Vec<TransportEdge>,
    pub at: HashMap<WorldTile, Vec<usize>>,
    /// Rotated footprint geometry aligned with `edges`. Absent geometry,
    /// including an empty vector on synthetic graphs, retains radius one.
    pub approaches: Vec<Option<api::query::loc_approach::LocApproach>>,
    /// Any-tile teleport edges (spells + jewellery rubs), kept out of
    /// `edges`/`at` so the default [`crate::router::find`] never sees
    /// them. [`crate::router::find_allow_teleports`] unions them in from
    /// any node. `at` is a wire-only placeholder, never indexed.
    pub teleports: Vec<TransportEdge>,
    /// Zones + `(z − origin_z) / divisor + offset` derived from
    /// `wilderness_levels.rs2` / `wilderness_zones.dbrow`. Empty on graphs
    /// that did not see those sources (fixtures).
    pub wilderness: WildernessRules,
    /// Optional packed exclusion-zone table. `None` for legacy grids and
    /// intentionally synthetic worlds.
    pub zones: Option<crate::zones::ZoneTable>,
    /// The selected quest family the bake consumed stage signals from
    /// (digest + extractor schema). `None` when it consumed none; then no
    /// edge carries [`TransportEdge::quest_gates`].
    pub quest_family: Option<QuestFamilyId>,
}

impl TransportGraph {
    /// The same footprint/face predicate used by live loc interactions.
    #[inline(always)]
    pub fn admissible_from(
        &self,
        collision: &WorldCollision,
        index: usize,
        from: WorldTile,
    ) -> bool {
        let edge = &self.edges[index];
        if edge.takeoff.is_some_and(|takeoff| from != takeoff)
            || !collision.standable(from)
            || from.level != edge.at.level
        {
            return false;
        }
        match self.approaches.get(index).copied().flatten() {
            Some(approach) => approach.can_operate(
                edge.at,
                from,
                collision.walkable_word(from.x, from.z, from.level) as i32,
            ),
            None => match wall_door_takeoff(edge, collision) {
                Some(takeoff) => from == takeoff,
                None => (from.x - edge.at.x).abs().max((from.z - edge.at.z).abs()) <= 1,
            },
        }
    }

    pub(crate) fn takeoff_bounds(&self, index: usize) -> (WorldTile, WorldTile) {
        let edge = &self.edges[index];
        if let Some(takeoff) = edge.takeoff {
            return (takeoff, takeoff);
        }
        let at = edge.at;
        let approach = self.approaches.get(index).copied().flatten();
        (
            WorldTile {
                x: at.x - 1,
                z: at.z - 1,
                level: at.level,
            },
            WorldTile {
                x: at.x + approach.map_or(1, |shape| i32::from(shape.width)),
                z: at.z + approach.map_or(1, |shape| i32::from(shape.length)),
                level: at.level,
            },
        )
    }

    /// Index pinned transports only at their exact admissible takeoff;
    /// footprint edges use operable stands and the rest keep their anchor.
    pub fn rebuild_index(&mut self, collision: &WorldCollision) {
        self.at.clear();
        for (index, edge) in self.edges.iter().enumerate() {
            if let Some(takeoff) = edge.takeoff {
                if self.admissible_from(collision, index, takeoff) {
                    self.at.entry(takeoff).or_default().push(index);
                }
                continue;
            }
            if self.approaches.get(index).copied().flatten().is_none() {
                self.at.entry(edge.at).or_default().push(index);
                continue;
            }
            let (min, max) = self.takeoff_bounds(index);
            for x in min.x..=max.x {
                for z in min.z..=max.z {
                    let from = WorldTile {
                        x,
                        z,
                        level: min.level,
                    };
                    if self.admissible_from(collision, index, from) {
                        self.at.entry(from).or_default().push(index);
                    }
                }
            }
        }
        // Nearly every router probe misses this sparse index. Footprint
        // stands can fill the last slots of a hash-table size class, making
        // those misses scan multiple control groups. Keep spare capacity
        // once at load/bake time instead of paying that cost for every tile.
        self.at.reserve(self.at.len());
    }
}

/// The one tile a straight wall-door crossing starts from. The engine opens a
/// straight wall loc from its own tile, from the tile across the wall, or
/// from a side neighbour of the loc tile that its walls do not cut off
/// (`ReachStrategy.ts:79-80` and `reachWall1` `WALL_STRAIGHT`, lines
/// 101-127); the crossing itself is then the one step through the wall, so it
/// starts on the wall's near side. A forward crossing (`to == at + dir`)
/// starts on the loc tile `at`; a reverse crossing lands on `at`
/// (`to == at`) and starts on the tile across the wall, `at - dir`. From a
/// side neighbour the player opens the door and still steps onto the near
/// side before crossing, which the walk search reaches at the same cost.
/// Admitting every tile within one of `at` let a route "cross" from a tile
/// the wall separates from the door (the Varrock loc 1530 at (3241,3382)
/// entered from (3241,3381)).
///
/// `None` keeps radius-one admission: non-door edges, directionless doors,
/// relative landings, and a door whose loc tile is not standable (a closed
/// diagonal door blocks its own tile and is opened from a cardinal
/// neighbour, `transport/doors.rs` `diagonal_door_crossings`).
#[inline(always)]
fn wall_door_takeoff(edge: &TransportEdge, collision: &WorldCollision) -> Option<WorldTile> {
    if edge.kind != TransportKind::Door || edge.player_delta.is_some() {
        return None;
    }
    let (dx, dz) = dir_delta(edge.dir?);
    if edge.to == edge.at {
        return Some(WorldTile {
            x: edge.at.x - dx,
            z: edge.at.z - dz,
            level: edge.at.level,
        });
    }
    let forward = edge.to.x == edge.at.x + dx
        && edge.to.z == edge.at.z + dz
        && edge.to.level == edge.at.level;
    (forward && collision.standable(edge.at)).then_some(edge.at)
}
/// The `(dx, dz)` tile step of a crossing direction.
fn dir_delta(dir: DoorDir) -> (i32, i32) {
    match dir {
        DoorDir::N => (0, 1),
        DoorDir::S => (0, -1),
        DoorDir::E => (1, 0),
        DoorDir::W => (-1, 0),
    }
}

/// `ladder_cellar`'s +6400/-6400 z shift (m8aq `CELLAR_SHIFT`).
pub const CELLAR_SHIFT: i32 = 6400;

// ---------------------------------------------------------------------------
// Al Kharid border toll and the Shantay northbound hop (item-gated gates).
// ---------------------------------------------------------------------------

/// The Shantay henge doorway loc id (`shantay_pass_henge_doorway` in
/// `pack/loc.pack`): the two loc-4031 Door edges both interact it, and
/// the traveller drives the gated branch's pass-handover chat dialogs
/// for this loc (see [`crate::traveller`]).
pub const SHANTAY_HENGE_LOC_ID: i32 = 4031;
/// `border_gate_toll_left` in `pack/loc.pack`: the southern Al Kharid
/// border gate (m51_50 (4,27) = (3268,3227)).
pub const AL_KHARID_TOLL_LEFT_LOC_ID: i32 = 2882;
/// `border_gate_toll_right` in `pack/loc.pack`: the northern Al Kharid
/// border gate (m51_50 (4,28) = (3268,3228)).
pub const AL_KHARID_TOLL_RIGHT_LOC_ID: i32 = 2883;
/// `wildinlever` in `pack/loc.pack`: the Ardougne lever (m40_51 (1,47) =
/// (2561,3311)) that teleports into the Wilderness.
pub const ARDOUGNE_WILDERNESS_LEVER_LOC_ID: i32 = 1814;

/// Dialogue-driven Door hops: the Al Kharid border toll (`border_gate.rs2`
/// `oploc1` → border-guard `p_choice3`), the Shantay henge
/// (`shantay_pass.rs2` `oploc1` pass handover / first-crossing disclaimer)
/// and the Ardougne Wilderness lever (`wilderness_lever.rs2`
/// `[oploc1,wildinlever]`: while `%warning_wilderness_teleport_lever` is
/// false, a warning `~mesbox` and a `~p_choice3_header` come before the pull).
pub(crate) fn is_dialogue_door_loc(loc_id: i32) -> bool {
    loc_id == SHANTAY_HENGE_LOC_ID
        || loc_id == AL_KHARID_TOLL_LEFT_LOC_ID
        || loc_id == AL_KHARID_TOLL_RIGHT_LOC_ID
        || loc_id == ARDOUGNE_WILDERNESS_LEVER_LOC_ID
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
