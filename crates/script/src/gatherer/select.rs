use super::area::WorkArea;
use super::settings::{method_level, GathererSettings, TargetPreference};
use crate::native::{WalkDanger, WalkDangerQuery, WalkGoals, WalkGoalsError};
use api::gather_methods::{
    known_rows, AccessPolicy, GatherCatalog, GatherMethod, GatherSkill, GatherSpot,
    SceneRegionInput, TargetClass,
};
use api::selected::{EntityId, Knowledge, Truth};
use api::snapshot::{LocView, NpcView, WorldStateView, WorldTile};
use std::sync::Arc;

const MAX_PRODUCTS: usize = 8;
const MAX_AVOID: usize = 8;
const HAZARD_WAIT_TICKS: u64 = 60;
const NO_NPC_INDEX: i32 = -1;
pub const RESOURCE_APPROACH_RADIUS: u16 = 1;
// NPC_INFO streams nearby actors, not every NPC in the 104-tile map build.
// The selected 289 engine uses a 15-tile Chebyshev view radius.
const NPC_VIEW_RADIUS: u32 = 15;
// Leave enough view margin for the resource observation walk settlement.
const OBSERVATION_RADIUS: i32 = NPC_VIEW_RADIUS as i32 - RESOURCE_APPROACH_RADIUS as i32;
const OBSERVATION_WIDTH: i32 = OBSERVATION_RADIUS * 2 + 1;
const OBSERVATION_WINDOW_TICKS: u64 = 100;
const MAX_FISHING_APPROACHES: u8 = 8;

/// Session-owned evidence and approach budgets, never evicted while selecting
/// other placements. Yield or a bounded runner survey renewal clears budgets.
#[derive(Default)]
pub struct FishingSurvey {
    placements: Vec<FishingPlacementSurvey>,
}

struct FishingPlacementSurvey {
    method: u16,
    spot: u32,
    covered: u64,
    covered_more: Vec<u64>,
    started: Option<u64>,
    approaches: u8,
}

impl FishingSurvey {
    fn placement(&mut self, method: u16, spot: u32) -> &mut FishingPlacementSurvey {
        let placements = &mut self.placements;
        let index = placements
            .iter()
            .position(|row| row.method == method && row.spot == spot);
        let index = index.unwrap_or_else(|| {
            placements.push(FishingPlacementSurvey {
                method,
                spot,
                covered: 0,
                covered_more: Vec::new(),
                started: None,
                approaches: 0,
            });
            placements.len() - 1
        });
        &mut placements[index]
    }

    fn reset_live(&mut self, method: u16, spot: u32) {
        if let Some(row) = self
            .placements
            .iter_mut()
            .find(|row| row.method == method && row.spot == spot)
        {
            row.covered = 0;
            row.covered_more.fill(0);
            row.started = None;
        }
    }

    pub fn progress(&mut self) {
        self.placements.clear();
    }
}

/// Partition the content rectangle into view-sized cells. Each cell's stand
/// is the nearest point in its radius-one-safe observation rectangle, rather
/// than an NPC origin (which may be water or outside the required view).
fn observation_cells(bounds: SceneRegionInput) -> impl Iterator<Item = SceneRegionInput> + Clone {
    let columns = (bounds.max_x - bounds.min_x + OBSERVATION_WIDTH) / OBSERVATION_WIDTH;
    let rows = (bounds.max_z - bounds.min_z + OBSERVATION_WIDTH) / OBSERVATION_WIDTH;
    (0..rows).flat_map(move |row| {
        (0..columns).map(move |column| {
            let min_x = bounds.min_x + column * OBSERVATION_WIDTH;
            let min_z = bounds.min_z + row * OBSERVATION_WIDTH;
            SceneRegionInput {
                min_x,
                min_z,
                max_x: (min_x + OBSERVATION_WIDTH - 1).min(bounds.max_x),
                max_z: (min_z + OBSERVATION_WIDTH - 1).min(bounds.max_z),
                level: bounds.level,
            }
        })
    })
}

fn observation_stand(cell: SceneRegionInput, here: WorldTile) -> WorldTile {
    WorldTile {
        x: here.x.clamp(
            cell.max_x - OBSERVATION_RADIUS,
            cell.min_x + OBSERVATION_RADIUS,
        ),
        z: here.z.clamp(
            cell.max_z - OBSERVATION_RADIUS,
            cell.min_z + OBSERVATION_RADIUS,
        ),
        level: cell.level,
    }
}

/// Prefer the old ranked stand's legal radius-one arrivals. Only an unreachable
/// preferred tier permits the wider observation set, so reachable old choices
/// retain their work-area and resource-origin preference.
fn observation_goals(
    cells: impl Iterator<Item = SceneRegionInput> + Clone,
    collision: Option<nav::contract::CollisionView<'_>>,
    preferred_stand: Option<WorldTile>,
) -> Result<WalkGoals, WalkGoalsError> {
    let capacity = cells
        .clone()
        .map(|cell| {
            let width = (2 * NPC_VIEW_RADIUS as i32 + cell.min_x - cell.max_x + 1) as usize;
            let height = (2 * NPC_VIEW_RADIUS as i32 + cell.min_z - cell.max_z + 1) as usize;
            width * height
        })
        .fold(0usize, usize::saturating_add)
        .min(WalkGoals::MAX + 1);
    let mut goals = Vec::with_capacity(capacity);
    for cell in cells {
        let radius = NPC_VIEW_RADIUS as i32;
        for x in cell.max_x - radius..=cell.min_x + radius {
            for z in cell.max_z - radius..=cell.min_z + radius {
                let tile = WorldTile {
                    x,
                    z,
                    level: cell.level,
                };
                if collision.is_some_and(|view| !view.standable(tile)) {
                    continue;
                }
                let key = (tile.level, tile.x, tile.z);
                if let Err(index) = goals
                    .binary_search_by_key(&key, |goal: &WorldTile| (goal.level, goal.x, goal.z))
                {
                    goals.insert(index, tile);
                    if goals.len() > WalkGoals::MAX {
                        return Err(WalkGoalsError::TooMany {
                            count: goals.len(),
                            limit: WalkGoals::MAX,
                        });
                    }
                }
            }
        }
    }
    if let Some(stand) = preferred_stand {
        let mut preferred = [stand; 9];
        let mut count = 0;
        let radius = i32::from(RESOURCE_APPROACH_RADIUS);
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                let tile = WorldTile {
                    x: stand.x + dx,
                    z: stand.z + dz,
                    level: stand.level,
                };
                if collision.is_none_or(|view| view.standable(tile)) {
                    preferred[count] = tile;
                    count += 1;
                }
            }
        }
        WalkGoals::with_preferred(goals, &preferred[..count])
    } else {
        WalkGoals::new(goals)
    }
}

fn region_visible(bounds: SceneRegionInput, here: WorldTile) -> bool {
    bounds.level == here.level
        && [bounds.min_x, bounds.max_x]
            .into_iter()
            .all(|x| x.abs_diff(here.x) <= NPC_VIEW_RADIUS)
        && [bounds.min_z, bounds.max_z]
            .into_iter()
            .all(|z| z.abs_diff(here.z) <= NPC_VIEW_RADIUS)
}

impl FishingPlacementSurvey {
    fn observe(
        &mut self,
        bounds: SceneRegionInput,
        world: &WorldStateView,
        here: WorldTile,
        now: u64,
    ) {
        if self
            .started
            .is_some_and(|started| now.saturating_sub(started) > OBSERVATION_WINDOW_TICKS)
        {
            self.covered = 0;
            self.covered_more.fill(0);
            self.started = None;
        }
        for (index, cell) in observation_cells(bounds).enumerate() {
            if region_loaded(cell, world) && region_visible(cell, here) {
                self.started.get_or_insert(now);
                if index < u64::BITS as usize {
                    self.covered |= 1 << index;
                } else {
                    let word = index / u64::BITS as usize - 1;
                    if word >= self.covered_more.len() {
                        self.covered_more.resize(word + 1, 0);
                    }
                    self.covered_more[word] |= 1 << (index % u64::BITS as usize);
                }
            }
        }
    }

    fn unobserved(&self, index: usize) -> bool {
        if index < u64::BITS as usize {
            self.covered & (1 << index) == 0
        } else {
            self.covered_more
                .get(index / u64::BITS as usize - 1)
                .is_none_or(|word| word & (1 << (index % u64::BITS as usize)) == 0)
        }
    }

    fn unobserved_cells(
        &self,
        bounds: SceneRegionInput,
    ) -> impl Iterator<Item = SceneRegionInput> + Clone + '_ {
        observation_cells(bounds)
            .enumerate()
            .filter(|(index, _)| self.unobserved(*index))
            .map(|(_, cell)| cell)
    }

    fn next_stand(&self, bounds: SceneRegionInput, here: WorldTile) -> Option<WorldTile> {
        self.unobserved_cells(bounds)
            .map(|cell| observation_stand(cell, here))
            .min_by_key(|stand| distance(here, *stand))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementClass {
    Live,
    Depleted,
    Hazard,
    Avoided,
    Unloaded,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvoidedTile {
    pub tile: WorldTile,
    pub until: u32,
}

#[derive(Debug, Clone)]
pub struct TargetPlan {
    /// Resource identity: for fishing this is the NPC type, not its instance index.
    pub entity: EntityId,
    pub tile: WorldTile,
    pub op: Arc<str>,
    pub alias: Arc<str>,
    pub products: [i32; MAX_PRODUCTS],
    pub products_len: u8,
    pub skill_stat: i32,
    pub method_index: u16,
    /// NPC slot index captured from the observed row; -1 for loc/object targets.
    pub npc_index: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Target(PlacementClass),
    Exhausted {
        wait_until: u64,
        absent: u16,
    },
    Absent {
        absent: u16,
    },
    InvalidGoals(WalkGoalsError),
    /// Unobserved fishing cells remain, but none have a radius-one Area goal.
    NoStand,
    Danger(DangerFailure),
}

#[derive(Debug, Clone)]
pub struct SelectedTarget {
    pub plan: TargetPlan,
    pub class: PlacementClass,
    /// Shared only by the one selected observation request.
    pub observation_goals: Option<WalkGoals>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DangerFailure {
    Denied { zone: Arc<str> },
    Unavailable,
}

impl std::fmt::Display for DangerFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied { zone } => write!(
                formatter,
                "no safe resource access: danger zone {zone} is not allowed"
            ),
            Self::Unavailable => {
                formatter.write_str("no navigation data is available for the resource danger check")
            }
        }
    }
}

/// A selection-local borrow. Only a terminal rejection owns its zone label.
struct DangerCheck<'a> {
    query: Option<&'a dyn WalkDangerQuery>,
    rejected: Option<WalkDanger<'a>>,
}

impl<'a> DangerCheck<'a> {
    fn new(query: Option<&'a dyn WalkDangerQuery>) -> Self {
        Self {
            query,
            rejected: None,
        }
    }

    fn allows(
        &mut self,
        tile: WorldTile,
        entity: Option<EntityId>,
        arrival: nav::contract::ArrivalKind,
    ) -> bool {
        let loc_id = match entity {
            Some(EntityId::Loc(id)) => Some(id),
            _ => None,
        };
        let decision = self.query.map_or(WalkDanger::Unavailable, |query| {
            query.check(tile, loc_id, RESOURCE_APPROACH_RADIUS, arrival)
        });
        if decision == WalkDanger::Allowed {
            true
        } else {
            self.rejected.get_or_insert(decision);
            false
        }
    }

    fn allows_live(
        &mut self,
        tile: WorldTile,
        entity: EntityId,
        snapshot: Option<api::snapshot::SnapshotView<'_>>,
    ) -> bool {
        if !resource_arrived(snapshot, entity, tile) {
            return self.allows(tile, Some(entity), nav::contract::ArrivalKind::Reach);
        }
        let Some(query) = self.query else {
            return true;
        };
        let loc_id = match entity {
            EntityId::Loc(id) => Some(id),
            _ => None,
        };
        let decision = query.check(
            tile,
            loc_id,
            RESOURCE_APPROACH_RADIUS,
            nav::contract::ArrivalKind::Reach,
        );
        if matches!(decision, WalkDanger::Denied { .. }) {
            self.rejected.get_or_insert(decision);
            false
        } else {
            // An arrived operation needs no navigation data, but physical
            // arrival must not override a known denied access policy.
            true
        }
    }

    fn failure(&self) -> Option<DangerFailure> {
        match self.rejected {
            Some(WalkDanger::Denied { zone }) => Some(DangerFailure::Denied {
                zone: Arc::from(zone),
            }),
            Some(WalkDanger::Unavailable) => Some(DangerFailure::Unavailable),
            Some(WalkDanger::Allowed) | None => None,
        }
    }
}

pub(super) fn resource_arrived(
    snapshot: Option<api::snapshot::SnapshotView<'_>>,
    entity: EntityId,
    target: WorldTile,
) -> bool {
    snapshot.is_some_and(|snapshot| {
        snapshot.local_player().is_some_and(|player| match entity {
            EntityId::Loc(id) => snapshot.walk_loc_arrived(
                player.value.player.network,
                target,
                i32::from(RESOURCE_APPROACH_RADIUS),
                id,
            ),
            _ => snapshot.walk_arrived(
                player.value.player.network,
                target,
                i32::from(RESOURCE_APPROACH_RADIUS),
            ),
        })
    })
}

impl SelectedTarget {
    /// One approach policy for Gatherer and finite quest gathering.
    /// Unrestricted live NPC ops own client-side approach; locs settle at
    /// their footprint. Restricted approaches use only admitted access goals.
    /// Arrival uses the observed server position, never the rendered actor pose.
    pub(crate) fn approach(
        &self,
        snapshot: api::snapshot::SnapshotView<'_>,
        required_after: api::quest_progress::EvidenceStamp,
        walk_danger: Option<&dyn WalkDangerQuery>,
    ) -> Result<Option<crate::native::WalkRequest>, WalkGoalsError> {
        let observation_approach =
            self.class == PlacementClass::Unloaded && matches!(self.plan.entity, EntityId::Npc(_));
        let loc_id = match self.plan.entity {
            EntityId::Loc(id) => Some(id),
            _ => None,
        };
        let radius = if self.observation_goals.is_some() {
            0
        } else {
            RESOURCE_APPROACH_RADIUS
        };
        let arrival = if observation_approach {
            nav::contract::ArrivalKind::Area
        } else {
            nav::contract::ArrivalKind::Reach
        };
        let arrived = resource_arrived(Some(snapshot), self.plan.entity, self.plan.tile);
        let safe_goals = match walk_danger {
            Some(query) => match query.safe_access_goals(
                self.plan.tile,
                loc_id,
                radius,
                arrival,
                self.observation_goals.as_ref(),
            ) {
                Ok(goals) => goals,
                Err(_)
                    if arrived
                        && query.check(self.plan.tile, loc_id, radius, arrival)
                            == WalkDanger::Unavailable =>
                {
                    None
                }
                Err(error) => return Err(error),
            },
            None => None,
        };
        let needs_walk = if self.class == PlacementClass::Unloaded {
            true
        } else if let Some(goals) = safe_goals.as_ref() {
            snapshot
                .local_player()
                .is_none_or(|player| !goals.as_slice().contains(&player.value.player.network))
        } else {
            self.plan.npc_index < 0 && !arrived
        };
        Ok(needs_walk.then(|| {
            let restricted = safe_goals.is_some();
            crate::native::WalkRequest {
                target: self.plan.tile,
                candidate_goals: safe_goals.or_else(|| self.observation_goals.clone()),
                loc_id: if restricted { None } else { loc_id },
                radius: if restricted { 0 } else { radius },
                arrival: if restricted {
                    nav::contract::ArrivalKind::Area
                } else {
                    arrival
                },
                options: crate::native::WalkOptions::default(),
                required_after,
                evidence: None,
                cross: Vec::new().into_boxed_slice(),
                protect: false,
                food_guard: false,
                allow: Default::default(),
            }
        }))
    }
}

#[derive(Debug, Clone)]
pub struct SelectionResult {
    pub target: Option<SelectedTarget>,
    pub outcome: Selection,
    pub zone_gated: u16,
}

impl AvoidedTile {
    pub const EMPTY: Self = Self {
        tile: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        until: 0,
    };
}

pub struct SelectionObservation<'a> {
    pub world: &'a WorldStateView,
    pub locs: &'a [LocView],
    pub npcs: &'a [NpcView],
    pub here: WorldTile,
    pub now: u64,
    pub skill_stat: i32,
    pub fishing: &'a mut FishingSurvey,
    /// Packed collision used to keep Area r=1 observation stands off tiles
    /// with no legal goals. `None` skips the filter (unit fixtures without a pack).
    pub collision: Option<nav::contract::CollisionView<'a>>,
    pub snapshot: Option<api::snapshot::SnapshotView<'a>>,
    pub walk_danger: Option<&'a dyn crate::native::WalkDangerQuery>,
}

pub struct ReturnObservation<'a> {
    pub here: WorldTile,
    pub now: u64,
    pub skill_stat: i32,
    pub avoided: &'a [AvoidedTile; MAX_AVOID],
    /// Packed collision used to keep Area r=1 return stands off tiles with
    /// no legal goals. `None` skips the filter (unit fixtures without a pack).
    pub collision: Option<nav::contract::CollisionView<'a>>,
    pub walk_danger: Option<&'a dyn crate::native::WalkDangerQuery>,
}

/// Bank-return and death-recovery selection. A temporary avoid deadline is
/// not the same as a missing stand: callers wait and retry instead of
/// blocking the run.
#[derive(Debug, Clone)]
pub enum ResourceReturn {
    Ready(SelectedTarget),
    ExcludedUntil { until: u64 },
    Unavailable { reason: &'static str },
    InvalidGoals(WalkGoalsError),
    Danger(DangerFailure),
}

impl ResourceReturn {
    #[cfg(test)]
    pub fn plan(self) -> Option<TargetPlan> {
        match self {
            Self::Ready(selected) => Some(selected.plan),
            Self::ExcludedUntil { .. }
            | Self::Unavailable { .. }
            | Self::InvalidGoals(_)
            | Self::Danger(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReturnStand {
    Ready(WorldTile),
    Excluded(u64),
    Access,
    Metadata,
    Area,
    Collision,
}

const RETURN_UNAVAILABLE_COLLISION: &str = "no standable observation tile in the work area";
const RETURN_UNAVAILABLE_ACCESS: &str = "no accessible resource in the work area";
const RETURN_UNAVAILABLE_AREA: &str = "no resource placement in the work area";
const RETURN_UNAVAILABLE_METADATA: &str = "resource metadata does not name a usable target";

fn note_unavailable(slot: &mut Option<&'static str>, reason: &'static str) {
    const ORDER: [&str; 4] = [
        RETURN_UNAVAILABLE_COLLISION,
        RETURN_UNAVAILABLE_ACCESS,
        RETURN_UNAVAILABLE_AREA,
        RETURN_UNAVAILABLE_METADATA,
    ];
    let rank = |reason: &str| {
        ORDER
            .iter()
            .position(|entry| *entry == reason)
            .unwrap_or(ORDER.len())
    };
    if slot.is_none_or(|current| rank(reason) < rank(current)) {
        *slot = Some(reason);
    }
}

pub struct PlacementScene<'a> {
    pub world: &'a WorldStateView,
    pub locs: &'a [LocView],
    pub npcs: &'a [NpcView],
    pub hazard_npcs: &'a [i32],
}

type Candidate<'a> = (
    u16,
    &'a GatherMethod,
    &'a GatherSpot,
    PlacementClass,
    WorldTile,
    i32,
    i64,
);

/// Classify one fixed loc placement against the scene and its content-derived
/// replacement ids. Placements outside the observed rectangle are approached,
/// not treated as absent or depleted.
pub fn classify_placement(
    spot: &GatherSpot,
    method: &GatherMethod,
    scene: PlacementScene<'_>,
    avoided: &[AvoidedTile; MAX_AVOID],
    now: u64,
) -> PlacementClass {
    let PlacementScene {
        world,
        locs,
        npcs,
        hazard_npcs,
    } = scene;
    if !tile_loaded(spot.origin, world) {
        return PlacementClass::Unloaded;
    }
    if is_avoided(spot.origin, avoided, now) {
        return PlacementClass::Avoided;
    }
    let Some(targets) = known_targets(method) else {
        return PlacementClass::Absent;
    };
    for loc in locs.iter().filter(|loc| loc.tile == spot.origin) {
        let Some(target) = targets.iter().find(|target| {
            matches!(target.respawn, Knowledge::Known(_))
                && matches!(target.entity, EntityId::Loc(id) if id == loc.id)
        }) else {
            continue;
        };
        return match target.class {
            TargetClass::Resource => PlacementClass::Live,
            TargetClass::Depleted => PlacementClass::Depleted,
            TargetClass::Hazard => PlacementClass::Hazard,
            TargetClass::Unclassified => PlacementClass::Absent,
        };
    }
    if has_hazard_npc(npcs, hazard_npcs, spot.origin) {
        return PlacementClass::Hazard;
    }
    PlacementClass::Absent
}

fn classify_fishing_placement(
    spot: &GatherSpot,
    method: &GatherMethod,
    scene: PlacementScene<'_>,
    here: WorldTile,
    avoided: &[AvoidedTile; MAX_AVOID],
    now: u64,
) -> PlacementClass {
    let PlacementScene {
        world,
        npcs,
        hazard_npcs,
        ..
    } = scene;
    let Some(bounds) = movement_bounds(spot) else {
        return PlacementClass::Absent;
    };
    // Live actors were considered first. An exclusion or hazard in the
    // observed part of a moving spot's envelope says nothing about the rest.
    if !region_loaded(bounds, world) || !region_visible(bounds, here) {
        return PlacementClass::Unloaded;
    }
    if avoided
        .iter()
        .any(|entry| u64::from(entry.until) > now && region_contains(bounds, entry.tile))
    {
        return PlacementClass::Avoided;
    }
    let Some(targets) = known_targets(method) else {
        return PlacementClass::Absent;
    };
    for npc in npcs.iter().filter(|npc| region_contains(bounds, npc.tile)) {
        let Some(id) = npc_type_id(npc) else {
            continue;
        };
        if let Some(target) = targets.iter().find(|target| {
            matches!(target.respawn, Knowledge::Known(_))
                && target.entity == EntityId::Npc(id)
                && matches!(target.class, TargetClass::Depleted | TargetClass::Hazard)
        }) {
            return match target.class {
                TargetClass::Depleted => PlacementClass::Depleted,
                TargetClass::Hazard => PlacementClass::Hazard,
                TargetClass::Resource | TargetClass::Unclassified => unreachable!(),
            };
        }
        if hazard_npcs.contains(&id) {
            return PlacementClass::Hazard;
        }
    }
    if npcs.iter().any(|npc| {
        region_contains(bounds, npc.tile)
            && npc_type_id(npc).is_some_and(|id| {
                spot.entity == EntityId::Npc(id)
                    && targets.iter().any(|target| {
                        target.entity == EntityId::Npc(id)
                            && target.class == TargetClass::Resource
                            && matches!(target.respawn, Knowledge::Known(_))
                    })
            })
    }) {
        return PlacementClass::Live;
    }
    PlacementClass::Absent
}

/// Select live resources before considering any unloaded placement. The live
/// pass is allocation-free and only copies a plan after its winner is known.
pub fn select(
    catalog: &GatherCatalog,
    method_indices: &[usize],
    settings: &GathererSettings,
    area: WorkArea,
    avoided: &[AvoidedTile; MAX_AVOID],
    observation: SelectionObservation<'_>,
) -> SelectionResult {
    select_with_access(
        catalog,
        method_indices,
        settings,
        area,
        avoided,
        observation,
        None,
    )
}

/// Quest Paths own their quest-state gates. They may attempt a state-gated
/// resource, but never an intercepted or substituted yield.
pub(crate) fn select_for_quest(
    catalog: &GatherCatalog,
    method_indices: &[usize],
    settings: &GathererSettings,
    area: WorkArea,
    observation: SelectionObservation<'_>,
    snapshot: api::snapshot::SnapshotView<'_>,
) -> SelectionResult {
    select_with_access(
        catalog,
        method_indices,
        settings,
        area,
        &[AvoidedTile::EMPTY; MAX_AVOID],
        observation,
        Some(snapshot),
    )
}

fn select_with_access(
    catalog: &GatherCatalog,
    method_indices: &[usize],
    settings: &GathererSettings,
    area: WorkArea,
    avoided: &[AvoidedTile; MAX_AVOID],
    observation: SelectionObservation<'_>,
    quest_snapshot: Option<api::snapshot::SnapshotView<'_>>,
) -> SelectionResult {
    let access = if quest_snapshot.is_some() {
        AccessPolicy::Possible
    } else {
        AccessPolicy::Usable
    };
    let SelectionObservation {
        world,
        locs,
        npcs,
        here,
        now,
        skill_stat,
        fishing,
        collision,
        snapshot,
        walk_danger,
    } = observation;
    let preference = settings.target_preference_kind();
    let region = area.region();
    let mut absent: u16 = 0;
    let mut zone_gated: u16 = 0;
    let mut wait_until = now;
    let mut non_absent = false;
    let mut blocked_stand = false;
    let mut best_live = None;
    let mut danger = DangerCheck::new(walk_danger);

    for &method_index in method_indices {
        let Some(method) = catalog.methods().get(method_index) else {
            continue;
        };
        if quest_snapshot.is_some_and(|snapshot| {
            !super::supply::method_ready(snapshot, method, true).is_ok_and(|ready| ready)
        }) {
            continue;
        }
        let Ok(method_index) = u16::try_from(method_index) else {
            continue;
        };
        if method.skill == GatherSkill::Fishing {
            let Some(spots) = complete_spots(method) else {
                continue;
            };
            for spot in spots.iter().filter(|spot| {
                fishing_spot_eligible(spot, area) && known_resource_target(method, spot.entity)
            }) {
                if !access_allowed(catalog, method, spot, access) {
                    zone_gated = zone_gated.saturating_add(1);
                    continue;
                }
                let Some(bounds) = movement_bounds(spot) else {
                    continue;
                };
                for npc in npcs.iter().filter(|npc| region_contains(bounds, npc.tile)) {
                    if access == AccessPolicy::Possible && !area.contains(npc.tile) {
                        continue;
                    }
                    let Some(type_id) = npc_type_id(npc) else {
                        continue;
                    };
                    if spot.entity != EntityId::Npc(type_id) || is_avoided(npc.tile, avoided, now) {
                        continue;
                    }
                    let Ok(index) = i32::try_from(npc.index) else {
                        continue;
                    };
                    if !danger.allows_live(npc.tile, spot.entity, snapshot) {
                        continue;
                    }
                    fishing.reset_live(method_index, spot.id.0);
                    consider_candidate(
                        &mut best_live,
                        preference,
                        (
                            method_index,
                            method,
                            spot,
                            PlacementClass::Live,
                            npc.tile,
                            index,
                            distance(here, npc.tile),
                        ),
                        skill_stat,
                    );
                }
            }
        } else {
            let Ok(spots) = catalog.spots(method, &region) else {
                continue;
            };
            for spot in spots.filter(|spot| known_resource_target(method, spot.entity)) {
                if !access_allowed(catalog, method, spot, access) {
                    zone_gated = zone_gated.saturating_add(1);
                    continue;
                }
                if classify_placement(
                    spot,
                    method,
                    PlacementScene {
                        world,
                        locs,
                        npcs,
                        hazard_npcs: catalog.hazard_npcs(),
                    },
                    avoided,
                    now,
                ) == PlacementClass::Live
                {
                    if !danger.allows_live(spot.origin, spot.entity, snapshot) {
                        continue;
                    }
                    consider_candidate(
                        &mut best_live,
                        preference,
                        (
                            method_index,
                            method,
                            spot,
                            PlacementClass::Live,
                            spot.origin,
                            NO_NPC_INDEX,
                            distance(here, spot.origin),
                        ),
                        skill_stat,
                    );
                }
            }
        }
    }

    if let Some((method_index, method, spot, class, tile, npc_index, _)) = best_live {
        return SelectionResult {
            target: Some(SelectedTarget {
                plan: make_plan(
                    catalog,
                    method_index,
                    method,
                    spot,
                    tile,
                    npc_index,
                    skill_stat,
                ),
                class,
                observation_goals: None,
            }),
            outcome: Selection::Target(class),
            zone_gated,
        };
    }

    let mut best_unloaded = None;
    for &method_index in method_indices {
        let Some(method) = catalog.methods().get(method_index) else {
            continue;
        };
        if quest_snapshot.is_some_and(|snapshot| {
            !super::supply::method_ready(snapshot, method, true).is_ok_and(|ready| ready)
        }) {
            continue;
        }
        let Ok(method_index) = u16::try_from(method_index) else {
            continue;
        };
        if method.skill == GatherSkill::Fishing {
            let Some(spots) = complete_spots(method) else {
                continue;
            };
            for spot in spots.iter().filter(|spot| {
                fishing_spot_eligible(spot, area) && known_resource_target(method, spot.entity)
            }) {
                if catalog.access(method, spot).unwrap_or(Truth::False) != Truth::True {
                    continue;
                }
                let mut class = classify_fishing_placement(
                    spot,
                    method,
                    PlacementScene {
                        world,
                        locs,
                        npcs,
                        hazard_npcs: catalog.hazard_npcs(),
                    },
                    here,
                    avoided,
                    now,
                );
                let bounds = movement_bounds(spot).expect("eligible fishing movement");
                let mut stand = spot.origin;
                if matches!(class, PlacementClass::Unloaded | PlacementClass::Absent) {
                    // Quest gather keeps only Live targets and discards any
                    // computed observation stand. Do not scan collision for a
                    // stand that will not be chosen.
                    if quest_snapshot.is_some() {
                        continue;
                    }
                    let survey = fishing.placement(method_index, spot.id.0);
                    survey.observe(bounds, world, here, now);
                    if survey.approaches >= MAX_FISHING_APPROACHES {
                        // Budget exhaustion is not proof of absence. Bound the
                        // unseen spot without another rectangle search.
                        class = PlacementClass::Avoided;
                    } else if let Some(next) = survey
                        .unobserved_cells(bounds)
                        .filter_map(|cell| {
                            let clamp = observation_stand(cell, here);
                            legal_fishing_return_stand(
                                cell,
                                here,
                                clamp,
                                area,
                                collision,
                                &mut danger,
                            )
                            .map(|stand| (stand, clamp))
                        })
                        .min_by_key(|(stand, clamp)| {
                            (distance(*clamp, *stand), distance(here, *stand))
                        })
                        .map(|(stand, _)| stand)
                    {
                        class = PlacementClass::Unloaded;
                        stand = next;
                    } else if survey.next_stand(bounds, here).is_some() {
                        blocked_stand = true;
                        continue;
                    } else {
                        class = PlacementClass::Absent;
                    }
                }
                match class {
                    PlacementClass::Unloaded => {
                        consider_candidate(
                            &mut best_unloaded,
                            preference,
                            (
                                method_index,
                                method,
                                spot,
                                class,
                                stand,
                                NO_NPC_INDEX,
                                distance(here, stand),
                            ),
                            skill_stat,
                        );
                    }
                    PlacementClass::Depleted | PlacementClass::Hazard | PlacementClass::Avoided => {
                        non_absent = true;
                        let respawn = u64::from(respawn_max(method, spot));
                        let until = match class {
                            PlacementClass::Hazard => {
                                now.saturating_add(respawn.max(HAZARD_WAIT_TICKS))
                            }
                            PlacementClass::Avoided => {
                                now.max(avoid_until(spot, method.skill, avoided))
                            }
                            _ => now.saturating_add(respawn),
                        };
                        wait_until = wait_until.max(until);
                    }
                    PlacementClass::Absent => absent = absent.saturating_add(1),
                    PlacementClass::Live => {
                        // Allowed live actors returned in the first pass.
                    }
                }
            }
        } else {
            let Ok(spots) = catalog.spots(method, &region) else {
                continue;
            };
            for spot in spots.filter(|spot| known_resource_target(method, spot.entity)) {
                if catalog.access(method, spot).unwrap_or(Truth::False) != Truth::True {
                    continue;
                }
                let class = classify_placement(
                    spot,
                    method,
                    PlacementScene {
                        world,
                        locs,
                        npcs,
                        hazard_npcs: catalog.hazard_npcs(),
                    },
                    avoided,
                    now,
                );
                if class != PlacementClass::Live
                    && class != PlacementClass::Absent
                    // A live scene can prove arrival at a replaced or avoided
                    // resource too. Waiting there needs no navigation data.
                    && !danger.allows_live(spot.origin, spot.entity, snapshot)
                {
                    continue;
                }
                let respawn = u64::from(respawn_max(method, spot));
                match class {
                    PlacementClass::Unloaded => {
                        consider_candidate(
                            &mut best_unloaded,
                            preference,
                            (
                                method_index,
                                method,
                                spot,
                                class,
                                spot.origin,
                                NO_NPC_INDEX,
                                distance(here, spot.origin),
                            ),
                            skill_stat,
                        );
                    }
                    PlacementClass::Depleted | PlacementClass::Hazard | PlacementClass::Avoided => {
                        non_absent = true;
                        let until = match class {
                            PlacementClass::Hazard => {
                                now.saturating_add(respawn.max(HAZARD_WAIT_TICKS))
                            }
                            PlacementClass::Avoided => {
                                now.max(avoid_until(spot, method.skill, avoided))
                            }
                            _ => now.saturating_add(respawn),
                        };
                        wait_until = wait_until.max(until);
                    }
                    PlacementClass::Absent => absent = absent.saturating_add(1),
                    // Allowed live locs returned in the first pass.
                    PlacementClass::Live => {}
                }
            }
        }
    }

    if let Some((method_index, method, spot, class, tile, npc_index, _)) = best_unloaded {
        let plan = make_plan(
            catalog,
            method_index,
            method,
            spot,
            tile,
            npc_index,
            skill_stat,
        );
        let mut goals = None;
        if method.skill == GatherSkill::Fishing {
            let survey = fishing.placement(method_index, spot.id.0);
            let bounds = movement_bounds(spot).expect("eligible fishing movement");
            match observation_goals(survey.unobserved_cells(bounds), collision, Some(tile)) {
                Ok(candidates) => goals = Some(candidates),
                Err(error) => {
                    return SelectionResult {
                        target: None,
                        outcome: if error == WalkGoalsError::Empty {
                            Selection::NoStand
                        } else {
                            Selection::InvalidGoals(error)
                        },
                        zone_gated,
                    };
                }
            }
            survey.approaches = survey.approaches.saturating_add(1);
        }
        return SelectionResult {
            target: Some(SelectedTarget {
                plan,
                class,
                observation_goals: goals,
            }),
            outcome: Selection::Target(class),
            zone_gated,
        };
    }
    if !non_absent {
        if let Some(failure) = danger.failure() {
            return SelectionResult {
                target: None,
                outcome: Selection::Danger(failure),
                zone_gated,
            };
        }
    }
    if blocked_stand && !non_absent {
        return SelectionResult {
            target: None,
            outcome: Selection::NoStand,
            zone_gated,
        };
    }
    if !non_absent && absent > 0 {
        return SelectionResult {
            target: None,
            outcome: Selection::Absent { absent },
            zone_gated,
        };
    }
    SelectionResult {
        target: None,
        outcome: Selection::Exhausted { wait_until, absent },
        zone_gated,
    }
}

/// Banking can end inside the gathering bound while its resources remain
/// unobserved. Return to a selected placement's observation stand at the
/// resource site — inside the gathering area or next to the resource — not
/// a point clamped toward the player's current tile, the work-area centroid,
/// or a fishing actor's occupied tile.
pub fn resource_return_target(
    catalog: &GatherCatalog,
    method_indices: &[usize],
    settings: &GathererSettings,
    area: WorkArea,
    observation: ReturnObservation<'_>,
) -> ResourceReturn {
    let preference = settings.target_preference_kind();
    let mut best = None;
    let mut excluded_until: Option<u64> = None;
    let mut unavailable = None;
    let mut danger = DangerCheck::new(observation.walk_danger);
    for &method_index in method_indices {
        let Some(method) = catalog.methods().get(method_index) else {
            note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_METADATA);
            continue;
        };
        let Ok(method_index) = u16::try_from(method_index) else {
            note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_METADATA);
            continue;
        };
        if method.skill == GatherSkill::Fishing {
            let Some(spots) = complete_spots(method) else {
                note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_METADATA);
                continue;
            };
            for spot in spots {
                match fishing_return_stand(catalog, method, spot, area, &observation, &mut danger) {
                    ReturnStand::Ready(stand) => consider_candidate(
                        &mut best,
                        preference,
                        (
                            method_index,
                            method,
                            spot,
                            PlacementClass::Unloaded,
                            stand,
                            NO_NPC_INDEX,
                            distance(observation.here, stand),
                        ),
                        observation.skill_stat,
                    ),
                    ReturnStand::Excluded(until) => {
                        excluded_until =
                            Some(excluded_until.map_or(until, |current| current.min(until)));
                    }
                    ReturnStand::Access => {
                        note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_ACCESS);
                    }
                    ReturnStand::Metadata => {
                        note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_METADATA);
                    }
                    ReturnStand::Area => {
                        note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_AREA);
                    }
                    ReturnStand::Collision => {
                        note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_COLLISION);
                    }
                }
            }
        } else {
            let Ok(spots) = catalog.spots(method, &area.region()) else {
                note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_AREA);
                continue;
            };
            let mut saw_spot = false;
            for spot in spots {
                saw_spot = true;
                if !known_resource_target(method, spot.entity) {
                    note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_METADATA);
                    continue;
                }
                if catalog.access(method, spot).unwrap_or(Truth::False) != Truth::True {
                    note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_ACCESS);
                    continue;
                }
                if !has_area_arrival(observation.collision, spot.origin) {
                    note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_COLLISION);
                    continue;
                }
                let until = avoid_until(spot, method.skill, observation.avoided);
                if until > observation.now {
                    excluded_until =
                        Some(excluded_until.map_or(until, |current| current.min(until)));
                    continue;
                }
                if !danger.allows(spot.origin, None, nav::contract::ArrivalKind::Area) {
                    continue;
                }
                consider_candidate(
                    &mut best,
                    preference,
                    (
                        method_index,
                        method,
                        spot,
                        PlacementClass::Unloaded,
                        spot.origin,
                        NO_NPC_INDEX,
                        distance(observation.here, spot.origin),
                    ),
                    observation.skill_stat,
                );
            }
            if !saw_spot {
                note_unavailable(&mut unavailable, RETURN_UNAVAILABLE_AREA);
            }
        }
    }
    if let Some((method_index, method, spot, _, tile, npc_index, _)) = best {
        let plan = make_plan(
            catalog,
            method_index,
            method,
            spot,
            tile,
            npc_index,
            observation.skill_stat,
        );
        let mut goals = None;
        if method.skill == GatherSkill::Fishing {
            let bounds = movement_bounds(spot).expect("eligible fishing movement");
            match observation_goals(observation_cells(bounds), observation.collision, Some(tile)) {
                Ok(candidates) => goals = Some(candidates),
                Err(WalkGoalsError::Empty) => {
                    return ResourceReturn::Unavailable {
                        reason: RETURN_UNAVAILABLE_COLLISION,
                    };
                }
                Err(error) => return ResourceReturn::InvalidGoals(error),
            }
        }
        ResourceReturn::Ready(SelectedTarget {
            plan,
            class: PlacementClass::Unloaded,
            observation_goals: goals,
        })
    } else if let Some(until) = excluded_until {
        ResourceReturn::ExcludedUntil { until }
    } else if let Some(failure) = danger.failure() {
        ResourceReturn::Danger(failure)
    } else {
        ResourceReturn::Unavailable {
            reason: unavailable.unwrap_or(RETURN_UNAVAILABLE_METADATA),
        }
    }
}

fn fishing_return_stand(
    catalog: &GatherCatalog,
    method: &GatherMethod,
    spot: &GatherSpot,
    area: WorkArea,
    observation: &ReturnObservation<'_>,
    danger: &mut DangerCheck<'_>,
) -> ReturnStand {
    if !known_resource_target(method, spot.entity) {
        return ReturnStand::Metadata;
    }
    if catalog.access(method, spot).unwrap_or(Truth::False) != Truth::True {
        return ReturnStand::Access;
    }
    if !fishing_spot_eligible(spot, area) {
        return ReturnStand::Area;
    }
    let until = avoid_until(spot, method.skill, observation.avoided);
    if until > observation.now {
        return ReturnStand::Excluded(until);
    }
    // Observe the resource from the resource, never from `here`. Clamping
    // toward the player is how a Draynor bank trip lands a stand inside the
    // booth (the spots sit inside NPC view of the interior). Packed Area
    // goals reuse `standable` over radius one; an all-water origin clamp is
    // replaced by the nearest observation-safe stand Walk can arrive at.
    let Some(bounds) = movement_bounds(spot) else {
        return ReturnStand::Metadata;
    };
    observation_cells(bounds)
        .filter_map(|cell| {
            legal_fishing_return_stand(
                cell,
                spot.origin,
                spot.origin,
                area,
                observation.collision,
                danger,
            )
        })
        .min_by_key(|stand| {
            (
                i64::from(!area.contains(*stand)),
                distance(spot.origin, *stand),
            )
        })
        .map(ReturnStand::Ready)
        .unwrap_or(ReturnStand::Collision)
}

fn has_area_arrival(collision: Option<nav::contract::CollisionView<'_>>, stand: WorldTile) -> bool {
    collision.is_none_or(|view| view.area_has_goal(stand, RESOURCE_APPROACH_RADIUS))
}

/// Preferred observation stand for one cell, then a legal Area-arrival
/// stand in that cell's observation rectangle ranked by `rank_by` (ties by
/// `origin`). Shared by the initial unloaded approach (`rank_by` = the
/// player-clamped stand) and the bank-return path (`rank_by` = the spot
/// origin, matching prior return ranking).
fn legal_fishing_return_stand(
    cell: SceneRegionInput,
    origin: WorldTile,
    rank_by: WorldTile,
    area: WorkArea,
    collision: Option<nav::contract::CollisionView<'_>>,
    danger: &mut DangerCheck<'_>,
) -> Option<WorldTile> {
    if danger.query.is_none() {
        danger.rejected.get_or_insert(WalkDanger::Unavailable);
        return None;
    }
    let preferred = observation_stand(cell, origin);
    if has_area_arrival(collision, preferred)
        && danger.allows(preferred, None, nav::contract::ArrivalKind::Area)
    {
        return Some(preferred);
    }
    let min_x = cell.max_x - OBSERVATION_RADIUS;
    let max_x = cell.min_x + OBSERVATION_RADIUS;
    let min_z = cell.max_z - OBSERVATION_RADIUS;
    let max_z = cell.min_z + OBSERVATION_RADIUS;
    if min_x > max_x || min_z > max_z {
        return None;
    }
    #[cfg(test)]
    FISHING_STAND_SEARCHES.with(|count| count.set(count.get().saturating_add(1)));
    let mut best = None;
    for x in min_x..=max_x {
        for z in min_z..=max_z {
            let stand = WorldTile {
                x,
                z,
                level: cell.level,
            };
            if !has_area_arrival(collision, stand)
                || !danger.allows(stand, None, nav::contract::ArrivalKind::Area)
            {
                continue;
            }
            let key = (
                i64::from(!area.contains(stand)),
                distance(rank_by, stand),
                distance(origin, stand),
            );
            if best.as_ref().is_none_or(|(_, current)| key < *current) {
                best = Some((stand, key));
            }
        }
    }
    best.map(|(stand, _)| stand)
}

#[cfg(test)]
thread_local! {
    static FISHING_STAND_SEARCHES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn consider_candidate<'a>(
    best: &mut Option<Candidate<'a>>,
    preference: TargetPreference,
    candidate: Candidate<'a>,
    skill_stat: i32,
) {
    let (method_index, method, spot, class, tile, npc_index, candidate_distance) = candidate;
    let better = better_candidate(
        preference,
        best.as_ref()
            .map(|(_, current_method, current_spot, _, _, _, distance)| {
                (
                    i64::from(method_level(current_method, skill_stat)),
                    *distance,
                    current_spot.id.0,
                )
            }),
        method,
        skill_stat,
        candidate_distance,
        spot.id.0,
    );
    if better {
        *best = Some((
            method_index,
            method,
            spot,
            class,
            tile,
            npc_index,
            candidate_distance,
        ));
    }
}

fn complete_spots(method: &GatherMethod) -> Option<&[GatherSpot]> {
    match &method.spots {
        Knowledge::Known(spots) => Some(spots),
        Knowledge::Partial { .. } | Knowledge::Unknown(_) => None,
    }
}

fn known_resource_target(method: &GatherMethod, entity: EntityId) -> bool {
    known_targets(method).is_some_and(|targets| {
        targets.iter().any(|target| {
            target.entity == entity
                && target.class == TargetClass::Resource
                && matches!(target.respawn, Knowledge::Known(_))
        })
    })
}

fn movement_bounds(spot: &GatherSpot) -> Option<SceneRegionInput> {
    match &spot.movement {
        Knowledge::Known(Some(region)) => Some(*region),
        Knowledge::Known(None) => Some(SceneRegionInput {
            min_x: spot.origin.x,
            min_z: spot.origin.z,
            max_x: spot.origin.x,
            max_z: spot.origin.z,
            level: spot.origin.level,
        }),
        Knowledge::Partial { .. } | Knowledge::Unknown(_) => None,
    }
}

fn fishing_spot_eligible(spot: &GatherSpot, area: WorkArea) -> bool {
    movement_bounds(spot).is_some_and(|bounds| regions_intersect(bounds, area.region()))
}

fn regions_intersect(a: SceneRegionInput, b: SceneRegionInput) -> bool {
    a.level == b.level
        && a.min_x <= b.max_x
        && b.min_x <= a.max_x
        && a.min_z <= b.max_z
        && b.min_z <= a.max_z
}

fn region_contains(region: SceneRegionInput, tile: WorldTile) -> bool {
    region.level == tile.level
        && (region.min_x..=region.max_x).contains(&tile.x)
        && (region.min_z..=region.max_z).contains(&tile.z)
}

fn region_loaded(region: SceneRegionInput, world: &WorldStateView) -> bool {
    region.level == world.level
        && region.min_x >= world.map_base_x
        && region.max_x < world.map_base_x.saturating_add(104)
        && region.min_z >= world.map_base_z
        && region.max_z < world.map_base_z.saturating_add(104)
}

pub(super) fn tile_loaded(tile: WorldTile, world: &WorldStateView) -> bool {
    tile.level == world.level
        && tile.x >= world.map_base_x
        && tile.x < world.map_base_x.saturating_add(104)
        && tile.z >= world.map_base_z
        && tile.z < world.map_base_z.saturating_add(104)
}

fn npc_type_id(npc: &NpcView) -> Option<i32> {
    i32::try_from(npc.r#type?).ok()
}

fn has_hazard_npc(npcs: &[NpcView], hazard_npcs: &[i32], tile: WorldTile) -> bool {
    npcs.iter()
        .any(|npc| npc.tile == tile && npc_type_id(npc).is_some_and(|id| hazard_npcs.contains(&id)))
}

fn is_avoided(tile: WorldTile, avoided: &[AvoidedTile; MAX_AVOID], now: u64) -> bool {
    avoided
        .iter()
        .any(|entry| u64::from(entry.until) > now && entry.tile == tile)
}

fn avoid_until(spot: &GatherSpot, skill: GatherSkill, avoided: &[AvoidedTile; MAX_AVOID]) -> u64 {
    avoided
        .iter()
        .filter(|entry| {
            entry.until > 0
                && if skill == GatherSkill::Fishing {
                    movement_bounds(spot).is_some_and(|bounds| region_contains(bounds, entry.tile))
                } else {
                    entry.tile == spot.origin
                }
        })
        .map(|entry| u64::from(entry.until))
        .max()
        .unwrap_or(0)
}

fn better_candidate(
    preference: TargetPreference,
    current: Option<(i64, i64, u32)>,
    method: &GatherMethod,
    skill_stat: i32,
    distance: i64,
    spot_id: u32,
) -> bool {
    let level = i64::from(method_level(method, skill_stat));
    let Some((current_level, current_distance, current_spot)) = current else {
        return true;
    };
    match preference {
        TargetPreference::BestTier => {
            (level, -distance, -(i64::from(spot_id)))
                > (current_level, -current_distance, -(i64::from(current_spot)))
        }
        TargetPreference::Nearest => {
            (distance, -level, i64::from(spot_id))
                < (current_distance, -current_level, i64::from(current_spot))
        }
    }
}

fn access_allowed(
    catalog: &GatherCatalog,
    method: &GatherMethod,
    spot: &GatherSpot,
    policy: AccessPolicy,
) -> bool {
    match catalog.access(method, spot) {
        Ok(Truth::True) => true,
        Ok(Truth::Unknown) => policy == AccessPolicy::Possible,
        Ok(Truth::False) | Err(_) => false,
    }
}

fn known_targets(method: &GatherMethod) -> Option<&[api::gather_methods::GatherTarget]> {
    match &method.targets {
        Knowledge::Known(rows) => Some(rows),
        Knowledge::Partial { known, .. } => Some(known),
        Knowledge::Unknown(_) => None,
    }
}

fn respawn_max(method: &GatherMethod, spot: &GatherSpot) -> u32 {
    let Some(targets) = known_targets(method) else {
        return 0;
    };
    let Some(target) = targets.iter().find(|target| {
        target.entity == spot.entity && matches!(target.respawn, Knowledge::Known(_))
    }) else {
        return 0;
    };
    let Knowledge::Known(Some(fact)) = &target.respawn else {
        return 0;
    };
    let Knowledge::Known(scale) = &fact.scale else {
        return 0;
    };
    scale.max_ticks
}

fn make_plan(
    catalog: &GatherCatalog,
    method_index: u16,
    method: &GatherMethod,
    spot: &GatherSpot,
    tile: WorldTile,
    npc_index: i32,
    skill_stat: i32,
) -> TargetPlan {
    let (op_slot, op_label) = catalog.op(method).ok().flatten().unwrap_or((1, "Mine"));
    let _ = op_slot;
    let mut products = [0; MAX_PRODUCTS];
    let mut products_len = 0;
    for product in known_rows(&method.products).iter() {
        if products_len == MAX_PRODUCTS {
            break;
        }
        if !products[..products_len].contains(&product.item) {
            products[products_len] = product.item;
            products_len += 1;
        }
    }
    if method.skill == GatherSkill::Mining {
        for &gem in catalog.incidental_gem_ids() {
            if products_len == MAX_PRODUCTS {
                break;
            }
            if !products[..products_len].contains(&gem) {
                products[products_len] = gem;
                products_len += 1;
            }
        }
    }
    let alias = catalog
        .alias(spot.entity)
        .or_else(|| catalog.alias(target_entity(method)))
        .unwrap_or(method.id.0.as_ref());
    TargetPlan {
        entity: spot.entity,
        tile,
        op: Arc::from(op_label),
        alias: Arc::from(alias),
        products,
        products_len: products_len as u8,
        skill_stat,
        method_index,
        npc_index,
    }
}

fn target_entity(method: &GatherMethod) -> EntityId {
    known_targets(method)
        .and_then(|rows| rows.first())
        .map_or(EntityId::Loc(-1), |target| target.entity)
}

fn distance(a: WorldTile, b: WorldTile) -> i64 {
    if a.level != b.level {
        return i64::MAX;
    }
    i64::from((a.x - b.x).abs().max((a.z - b.z).abs()))
}

#[cfg(test)]
pub(crate) fn test_real_nav_world() -> &'static std::sync::Arc<nav::world::NavWorld> {
    static WORLD: std::sync::LazyLock<std::sync::Arc<nav::world::NavWorld>> =
        std::sync::LazyLock::new(|| {
            let pack = std::env::var_os("NAV_PACK")
                .map(std::path::PathBuf::from)
                .expect("NAV_PACK must name the reviewed real 289 nav pack");
            std::sync::Arc::new(
                nav::world::NavWorld::load_pack(&pack).unwrap_or_else(|error| {
                    panic!(
                    "load real 289 nav pack {} (set NAV_PACK to the reviewed 289 bake): {error}",
                    pack.display()
                )
                }),
            )
        });
    &WORLD
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use api::gather_methods::{GatherTarget, SpotId};
    use api::snapshot::{LocLayer, LocView, NpcView};
    use std::collections::HashMap;

    fn empty_collision_fixture() -> nav::collision::WorldCollision {
        let (walk, blocked) = nav::collision::pack_walk(&[0; 4]);
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 1,
            height: 1,
            walk,
            blocked,
            flags: None,
        }
    }

    /// The one script test site that constructs a `NavWorld` (§21 §5.3
    /// exception scope). Other script tests obtain contract leases through
    /// it; `configure` edits the world (zones, wilderness) before wrapping.
    pub(crate) fn test_nav_lease(
        configure: impl FnOnce(&mut nav::world::NavWorld),
    ) -> nav::contract::NavLease {
        test_nav_lease_over(empty_collision_fixture(), configure)
    }

    pub(crate) fn test_nav_world_over(
        collision: nav::collision::WorldCollision,
        configure: impl FnOnce(&mut nav::world::NavWorld),
    ) -> std::sync::Arc<nav::world::NavWorld> {
        // The checker registers the type path; the constructor is reached
        // through it rather than as a second qualified symbol.
        type NavWorld = nav::world::NavWorld;
        let mut world = NavWorld::from_parts(collision, Default::default(), Vec::new());
        configure(&mut world);
        std::sync::Arc::new(world)
    }

    /// Same, over a caller-built packed collision.
    pub(crate) fn test_nav_lease_over(
        collision: nav::collision::WorldCollision,
        configure: impl FnOnce(&mut nav::world::NavWorld),
    ) -> nav::contract::NavLease {
        nav::contract::NavLease::wrap(
            test_nav_world_over(collision, configure),
            nav::contract::PackDigest::of(b"script test fixture"),
        )
        .expect("lease ids are not exhausted in tests")
    }

    /// A lease over a small synthetic collision the test keeps for its raw
    /// oracle: the grid is copied once (fixture-sized, never a real pack).
    pub(crate) fn test_nav_lease_for_collision(
        collision: &nav::collision::WorldCollision,
    ) -> nav::contract::NavLease {
        assert!(
            collision.width * collision.height <= 1 << 16,
            "copy only fixture grids; the real pack is leased by test_real_nav_lease"
        );
        test_nav_lease_over(
            nav::collision::WorldCollision {
                origin: collision.origin,
                width: collision.width,
                height: collision.height,
                walk: collision.walk.clone(),
                blocked: collision.blocked.clone(),
                flags: collision.flags.clone(),
            },
            |_| {},
        )
    }

    /// A lease over the shared real 289 pack world, without copying it.
    pub(crate) fn test_real_nav_lease() -> nav::contract::NavLease {
        nav::contract::NavLease::wrap(
            std::sync::Arc::clone(super::test_real_nav_world()),
            nav::contract::PackDigest::of(b"real 289 pack"),
        )
        .expect("lease ids are not exhausted in tests")
    }

    #[test]
    fn empty_nav_lease_has_an_open_origin() {
        let lease = test_nav_lease(|_| {});
        assert!(lease.collision().standable(WorldTile {
            x: 0,
            z: 0,
            level: 0,
        }));
    }

    #[test]
    fn loc_arrival_consumers_use_server_position_while_rendering_trails() {
        use api::quest_progress::EvidenceStamp;
        use api::selected::RunKey;
        use api::snapshot::{GameSnapshot, SnapshotView};

        let tile = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let rendered = WorldTile { x: 3195, ..tile };
        let mut client = client::client::Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.map_build_base_x = tile.x - 52;
        client.map_build_base_z = tile.z - 52;
        client.minusedlevel = tile.level;
        client.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        let mut player = crate::quester::families::tests::local_player(rendered);
        player.player.network = tile;
        player.player.actor.moving = true;
        snapshot.seed_local_player(player);
        snapshot.seed_locs(vec![loc(1, tile)]);
        let selected = SelectedTarget {
            class: PlacementClass::Live,
            observation_goals: None,
            plan: TargetPlan {
                entity: EntityId::Loc(1),
                tile,
                op: Arc::from("Chop down"),
                alias: Arc::from("Tree"),
                products: [0; MAX_PRODUCTS],
                products_len: 0,
                skill_stat: 8,
                method_index: 0,
                npc_index: NO_NPC_INDEX,
            },
        };
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert!(view.walk_loc_arrived(tile, tile, 1, 1));
        assert!(
            selected.approach(view, stamp, None).unwrap().is_none(),
            "the observed server arrival must not launch a redundant approach"
        );
        let mut ledger = None;
        crate::quester::families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(
                crate::quester::families::reach::loc_arrived(&tick.cx, &loc(1, tile)),
                "quest loc arrival must use the same server position as native Walk"
            );
        });
    }

    struct DenyDanger;

    impl crate::native::WalkDangerQuery for DenyDanger {
        fn check(
            &self,
            _target: WorldTile,
            _loc_id: Option<i32>,
            _radius: u16,
            _arrival: nav::contract::ArrivalKind,
        ) -> crate::native::WalkDanger<'_> {
            crate::native::WalkDanger::Denied {
                zone: "Draynor jail guards",
            }
        }

        fn safe_access_goals(
            &self,
            _target: WorldTile,
            _loc_id: Option<i32>,
            _radius: u16,
            _arrival: nav::contract::ArrivalKind,
            _existing: Option<&WalkGoals>,
        ) -> Result<Option<WalkGoals>, WalkGoalsError> {
            Err(WalkGoalsError::Empty)
        }
    }

    fn danger_tree_selection(
        walk_danger: Option<&dyn crate::native::WalkDangerQuery>,
        arrived: bool,
    ) -> SelectionResult {
        danger_tree_selection_case(walk_danger, arrived, PlacementClass::Live, false)
    }

    fn danger_tree_selection_case(
        walk_danger: Option<&dyn crate::native::WalkDangerQuery>,
        arrived: bool,
        class: PlacementClass,
        quest: bool,
    ) -> SelectionResult {
        use api::quest_progress::EvidenceStamp;
        use api::selected::RunKey;
        use api::snapshot::{GameSnapshot, SnapshotView};

        let catalog = real_catalog();
        let method = catalog.method("woodcutting.normal").unwrap();
        let tile = WorldTile {
            x: 3113,
            z: 3235,
            level: 0,
        };
        let spot = complete_spots(method)
            .unwrap()
            .iter()
            .find(|spot| spot.origin == tile && known_resource_target(method, spot.entity))
            .expect("the shipped Draynor jail tree is in the normal-tree catalog");
        let EntityId::Loc(id) = spot.entity else {
            panic!("a normal tree is a loc");
        };
        let here = if arrived {
            WorldTile {
                x: tile.x - 1,
                ..tile
            }
        } else {
            WorldTile {
                x: 3109,
                z: 3228,
                ..tile
            }
        };
        let mut client = client::client::Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.map_build_base_x = tile.x - 52;
        client.map_build_base_z = tile.z - 52;
        client.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        snapshot.seed_local_player(crate::quester::families::tests::local_player(here));
        snapshot.seed_inventory(
            vec![api::snapshot::ItemView {
                def: crate::quester::families::tests::def(1351, "Bronze axe"),
                container: api::snapshot::ItemContainer::Inventory,
                action_family: api::snapshot::ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![],
                component_id: 3214,
            }],
            28,
        );
        snapshot.seed_equipment(vec![]);
        snapshot.seed_stats(vec![api::snapshot::StatView {
            index: 8,
            name: "woodcutting".into(),
            effective: 1,
            base: 1,
            xp: 0,
            used: true,
        }]);
        let observed_id = if class == PlacementClass::Depleted {
            known_targets(method)
                .unwrap()
                .iter()
                .find_map(|target| match (target.class, target.entity) {
                    (TargetClass::Depleted, EntityId::Loc(id)) => Some(id),
                    _ => None,
                })
                .expect("normal trees publish a depleted stump")
        } else {
            id
        };
        let locs = [loc(observed_id, tile)];
        snapshot.seed_locs(locs.to_vec());
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert_eq!(view.walk_loc_arrived(here, tile, 1, id), arrived);
        if quest {
            assert_eq!(
                super::super::supply::method_ready(view, method, true),
                Ok(true)
            );
        }
        let mut fishing = FishingSurvey::default();
        let world = scene_around(tile);
        let observation = SelectionObservation {
            world: &world,
            locs: &locs,
            npcs: &[],
            here,
            now: 1,
            skill_stat: 8,
            fishing: &mut fishing,
            collision: None,
            snapshot: Some(view),
            walk_danger,
        };
        let indices = [method_index(&catalog, method)];
        let settings = GathererSettings::default();
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: tile,
            radius: 0,
        };
        if quest {
            select_for_quest(&catalog, &indices, &settings, area, observation, view)
        } else {
            let mut avoided = [AvoidedTile::EMPTY; MAX_AVOID];
            if class == PlacementClass::Avoided {
                avoided[0] = AvoidedTile { tile, until: 81 };
            }
            select(&catalog, &indices, &settings, area, &avoided, observation)
        }
    }

    #[test]
    fn danger_denied_tree_is_not_ranked_as_a_target() {
        let result = danger_tree_selection(Some(&DenyDanger), false);
        assert!(
            result.target.is_none(),
            "a fully denied tree must be ineligible"
        );
        assert_eq!(
            result.outcome,
            Selection::Danger(DangerFailure::Denied {
                zone: Arc::from("Draynor jail guards")
            })
        );
    }

    #[test]
    fn danger_without_navigation_data_refuses_a_tree_that_needs_a_walk() {
        let result = danger_tree_selection(None, false);
        assert!(
            result.target.is_none(),
            "missing danger data must not admit a nav walk"
        );
        assert_eq!(
            result.outcome,
            Selection::Danger(DangerFailure::Unavailable)
        );
        let Selection::Danger(failure) = result.outcome else {
            unreachable!()
        };
        assert!(failure.to_string().contains("no navigation data"));
    }

    #[test]
    fn danger_without_navigation_data_keeps_a_tree_already_in_operating_range() {
        let result = danger_tree_selection(None, true);
        assert!(
            result.target.is_some(),
            "an arrived loc operation does not need walk admission"
        );
    }

    #[test]
    fn danger_without_navigation_waits_for_depleted_tree_in_gatherer_and_quester() {
        for quest in [false, true] {
            let result = danger_tree_selection_case(None, true, PlacementClass::Depleted, quest);
            let allowed = danger_tree_selection_case(
                Some(&crate::native::AllowWalkDanger),
                true,
                PlacementClass::Depleted,
                quest,
            );
            assert_eq!(result.outcome, allowed.outcome, "quest={quest}");
            assert!(matches!(
                result.outcome,
                Selection::Exhausted {
                    wait_until: 200,
                    absent: 0
                }
            ));
        }
    }

    #[test]
    fn danger_without_navigation_waits_for_depleted_tree_in_quester() {
        let result = danger_tree_selection_case(None, true, PlacementClass::Depleted, true);
        assert_eq!(
            result.outcome,
            Selection::Exhausted {
                wait_until: 200,
                absent: 0,
            }
        );
    }

    #[test]
    fn danger_without_navigation_waits_for_avoided_tree_in_operating_range() {
        let result = danger_tree_selection_case(None, true, PlacementClass::Avoided, false);
        assert_eq!(
            result.outcome,
            Selection::Exhausted {
                wait_until: 81,
                absent: 0
            }
        );
    }

    #[test]
    fn danger_non_live_tree_still_requires_permission_when_not_arrived_or_denied() {
        for quest in [false, true] {
            let distant = danger_tree_selection_case(None, false, PlacementClass::Depleted, quest);
            assert_eq!(
                distant.outcome,
                Selection::Danger(DangerFailure::Unavailable)
            );
            let denied = danger_tree_selection_case(
                Some(&DenyDanger),
                true,
                PlacementClass::Depleted,
                quest,
            );
            assert!(matches!(
                denied.outcome,
                Selection::Danger(DangerFailure::Denied { .. })
            ));
        }
    }

    #[test]
    fn danger_denied_tree_is_ineligible_even_when_already_in_operating_range() {
        let result = danger_tree_selection(Some(&DenyDanger), true);
        assert!(
            result.target.is_none(),
            "physical arrival must not bypass denied access permissions"
        );
        assert!(matches!(
            result.outcome,
            Selection::Danger(DangerFailure::Denied { .. })
        ));
    }

    struct SafeAccessQuery {
        result: Result<Option<WalkGoals>, WalkGoalsError>,
        calls: std::cell::Cell<u32>,
    }

    impl WalkDangerQuery for SafeAccessQuery {
        fn check(
            &self,
            _target: WorldTile,
            _loc_id: Option<i32>,
            _radius: u16,
            _arrival: nav::contract::ArrivalKind,
        ) -> WalkDanger<'_> {
            WalkDanger::Allowed
        }

        fn safe_access_goals(
            &self,
            _target: WorldTile,
            _loc_id: Option<i32>,
            _radius: u16,
            _arrival: nav::contract::ArrivalKind,
            _existing: Option<&WalkGoals>,
        ) -> Result<Option<WalkGoals>, WalkGoalsError> {
            self.calls.set(self.calls.get() + 1);
            self.result.clone()
        }
    }

    fn safe_access_fixture(
        here: WorldTile,
    ) -> (
        SelectedTarget,
        api::snapshot::GameSnapshot,
        api::quest_progress::EvidenceStamp,
    ) {
        let tile = WorldTile {
            x: 3116,
            z: 3229,
            level: 0,
        };
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_world(scene_around(tile));
        snapshot.seed_scene(api::snapshot::SceneView {
            available: true,
            base_x: tile.x - 50,
            base_z: tile.z - 50,
            level: tile.level,
            width: 104,
            height: 104,
            collision_flags: vec![0; 104 * 104],
        });
        snapshot.seed_local_player(crate::quester::families::tests::local_player(here));
        snapshot.seed_locs(vec![loc(1, tile)]);
        let selected = SelectedTarget {
            class: PlacementClass::Live,
            observation_goals: None,
            plan: TargetPlan {
                entity: EntityId::Loc(1),
                tile,
                op: Arc::from("Chop down"),
                alias: Arc::from("Tree"),
                products: [0; MAX_PRODUCTS],
                products_len: 0,
                skill_stat: 8,
                method_index: 0,
                npc_index: NO_NPC_INDEX,
            },
        };
        let stamp = api::quest_progress::EvidenceStamp {
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        (selected, snapshot, stamp)
    }

    #[test]
    fn safe_access_goals_restrict_mixed_stand_resource_walk() {
        let safe = WorldTile {
            x: 3116,
            z: 3228,
            level: 0,
        };
        let query = SafeAccessQuery {
            result: Ok(Some(WalkGoals::new(vec![safe]).unwrap())),
            calls: std::cell::Cell::new(0),
        };
        let (selected, snapshot, stamp) = safe_access_fixture(WorldTile { x: 3109, ..safe });
        let request = selected
            .approach(
                api::snapshot::SnapshotView::new(Some(&snapshot), stamp),
                stamp,
                Some(&query),
            )
            .unwrap()
            .expect("the chosen resource still needs an approach");
        assert_eq!(query.calls.get(), 1);
        assert_eq!(request.candidate_goals.unwrap().as_slice(), &[safe]);
        assert_eq!(request.loc_id, None);
        assert_eq!(request.radius, 0);
        assert_eq!(request.arrival, nav::contract::ArrivalKind::Area);
        assert!(selected.observation_goals.is_none());
    }

    #[test]
    fn safe_access_goals_move_from_denied_operating_tile() {
        let safe = WorldTile {
            x: 3116,
            z: 3228,
            level: 0,
        };
        let denied = WorldTile {
            x: 3115,
            z: 3229,
            ..safe
        };
        let query = SafeAccessQuery {
            result: Ok(Some(WalkGoals::new(vec![safe]).unwrap())),
            calls: std::cell::Cell::new(0),
        };
        let (selected, snapshot, stamp) = safe_access_fixture(denied);
        let view = api::snapshot::SnapshotView::new(Some(&snapshot), stamp);
        assert!(resource_arrived(
            Some(view),
            selected.plan.entity,
            selected.plan.tile
        ));
        let request = selected
            .approach(view, stamp, Some(&query))
            .unwrap()
            .expect("an unsafe operating tile must not settle the safe approach");
        assert_eq!(request.candidate_goals.unwrap().as_slice(), &[safe]);
    }

    #[test]
    fn safe_access_goals_keep_safe_operating_tile() {
        let safe = WorldTile {
            x: 3116,
            z: 3228,
            level: 0,
        };
        let query = SafeAccessQuery {
            result: Ok(Some(WalkGoals::new(vec![safe]).unwrap())),
            calls: std::cell::Cell::new(0),
        };
        let (selected, snapshot, stamp) = safe_access_fixture(safe);
        assert!(selected
            .approach(
                api::snapshot::SnapshotView::new(Some(&snapshot), stamp),
                stamp,
                Some(&query),
            )
            .unwrap()
            .is_none());
        assert_eq!(query.calls.get(), 1);
    }

    #[test]
    fn safe_access_goals_leave_unrestricted_walk_unchanged() {
        let query = SafeAccessQuery {
            result: Ok(None),
            calls: std::cell::Cell::new(0),
        };
        let (selected, snapshot, stamp) = safe_access_fixture(WorldTile {
            x: 3109,
            z: 3228,
            level: 0,
        });
        let request = selected
            .approach(
                api::snapshot::SnapshotView::new(Some(&snapshot), stamp),
                stamp,
                Some(&query),
            )
            .unwrap()
            .unwrap();
        assert!(request.candidate_goals.is_none());
        assert_eq!(request.loc_id, Some(1));
        assert_eq!(request.radius, RESOURCE_APPROACH_RADIUS);
        assert_eq!(request.arrival, nav::contract::ArrivalKind::Reach);
        assert_eq!(query.calls.get(), 1);
    }

    #[test]
    fn safe_access_goals_preserve_unrestricted_observation_goals() {
        let query = SafeAccessQuery {
            result: Ok(None),
            calls: std::cell::Cell::new(0),
        };
        let (mut selected, snapshot, stamp) = safe_access_fixture(WorldTile {
            x: 3109,
            z: 3228,
            level: 0,
        });
        selected.class = PlacementClass::Unloaded;
        selected.plan.entity = EntityId::Npc(1);
        let preferred = WorldTile {
            x: 3110,
            z: 3228,
            level: 0,
        };
        let fallback = WorldTile {
            x: 3111,
            ..preferred
        };
        selected.observation_goals =
            Some(WalkGoals::with_preferred(vec![preferred, fallback], &[preferred]).unwrap());
        let request = selected
            .approach(
                api::snapshot::SnapshotView::new(Some(&snapshot), stamp),
                stamp,
                Some(&query),
            )
            .unwrap()
            .unwrap();
        let goals = request.candidate_goals.unwrap();
        assert_eq!(goals.preferred(), &[preferred]);
        assert_eq!(goals.as_slice(), &[preferred, fallback]);
        assert_eq!(request.loc_id, None);
        assert_eq!(request.radius, 0);
        assert_eq!(request.arrival, nav::contract::ArrivalKind::Area);
    }

    #[test]
    fn safe_access_goals_reject_invalid_restrictions() {
        let query = SafeAccessQuery {
            result: Err(WalkGoalsError::Empty),
            calls: std::cell::Cell::new(0),
        };
        let (selected, snapshot, stamp) = safe_access_fixture(WorldTile {
            x: 3109,
            z: 3228,
            level: 0,
        });
        assert!(matches!(
            selected.approach(
                api::snapshot::SnapshotView::new(Some(&snapshot), stamp),
                stamp,
                Some(&query),
            ),
            Err(WalkGoalsError::Empty)
        ));
    }

    #[test]
    fn safe_access_goals_unavailable_data_keeps_arrived_operation_only() {
        struct UnavailableAccess;

        impl WalkDangerQuery for UnavailableAccess {
            fn check(
                &self,
                _target: WorldTile,
                _loc_id: Option<i32>,
                _radius: u16,
                _arrival: nav::contract::ArrivalKind,
            ) -> WalkDanger<'_> {
                WalkDanger::Unavailable
            }

            fn safe_access_goals(
                &self,
                _target: WorldTile,
                _loc_id: Option<i32>,
                _radius: u16,
                _arrival: nav::contract::ArrivalKind,
                _existing: Option<&WalkGoals>,
            ) -> Result<Option<WalkGoals>, WalkGoalsError> {
                Err(WalkGoalsError::Empty)
            }
        }

        for arrived in [false, true] {
            let here = if arrived {
                WorldTile {
                    x: 3115,
                    z: 3229,
                    level: 0,
                }
            } else {
                WorldTile {
                    x: 3109,
                    z: 3228,
                    level: 0,
                }
            };
            let (selected, snapshot, stamp) = safe_access_fixture(here);
            let result = selected.approach(
                api::snapshot::SnapshotView::new(Some(&snapshot), stamp),
                stamp,
                Some(&UnavailableAccess),
            );
            if arrived {
                assert!(matches!(result, Ok(None)));
            } else {
                assert!(matches!(result, Err(WalkGoalsError::Empty)));
            }
        }
    }

    struct DenyOneTile(WorldTile);

    impl WalkDangerQuery for DenyOneTile {
        fn check(
            &self,
            target: WorldTile,
            _loc_id: Option<i32>,
            _radius: u16,
            _arrival: nav::contract::ArrivalKind,
        ) -> WalkDanger<'_> {
            if target == self.0 {
                WalkDanger::Denied {
                    zone: "denied nearest resource",
                }
            } else {
                WalkDanger::Allowed
            }
        }

        fn safe_access_goals(
            &self,
            target: WorldTile,
            _loc_id: Option<i32>,
            _radius: u16,
            _arrival: nav::contract::ArrivalKind,
            _existing: Option<&WalkGoals>,
        ) -> Result<Option<WalkGoals>, WalkGoalsError> {
            if target == self.0 {
                Err(WalkGoalsError::Empty)
            } else {
                Ok(None)
            }
        }
    }

    #[test]
    fn danger_filter_runs_before_live_unloaded_and_return_ranking() {
        let catalog = real_catalog();
        let method = catalog.method("woodcutting.normal").unwrap();
        let index = method_index(&catalog, method);
        let here = WorldTile {
            x: 3113,
            z: 3235,
            level: 0,
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: here,
            radius: 16,
        };
        let locs: Vec<_> = catalog
            .spots(method, &area.region())
            .unwrap()
            .filter(|spot| known_resource_target(method, spot.entity))
            .filter(|spot| catalog.access(method, spot) == Ok(Truth::True))
            .map(|spot| {
                let EntityId::Loc(id) = spot.entity else {
                    unreachable!()
                };
                loc(id, spot.origin)
            })
            .collect();
        assert!(locs.iter().any(|loc| loc.tile == here));
        assert!(locs.iter().any(|loc| loc.tile != here));
        let deny = DenyOneTile(here);
        for live in [true, false] {
            let scene = scene_around(if live {
                here
            } else {
                WorldTile { x: 2000, ..here }
            });
            let result = select(
                &catalog,
                &[index],
                &GathererSettings::default(),
                area,
                &[AvoidedTile::EMPTY; MAX_AVOID],
                SelectionObservation {
                    world: &scene,
                    locs: if live { &locs } else { &[] },
                    npcs: &[],
                    here,
                    now: 1,
                    skill_stat: 8,
                    fishing: &mut FishingSurvey::default(),
                    collision: None,
                    snapshot: None,
                    walk_danger: Some(&deny),
                },
            );
            let target = result.target.expect("an allowed alternative remains");
            assert_ne!(target.plan.tile, here);
            assert_eq!(
                target.class,
                if live {
                    PlacementClass::Live
                } else {
                    PlacementClass::Unloaded
                }
            );
        }
        let returned = resource_return_target(
            &catalog,
            &[index],
            &GathererSettings::default(),
            area,
            ReturnObservation {
                here,
                now: 1,
                skill_stat: 8,
                avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                collision: None,
                walk_danger: Some(&deny),
            },
        )
        .plan()
        .expect("return ranking retains an allowed alternative");
        assert_ne!(returned.tile, here);
    }

    #[test]
    fn danger_filter_searches_another_fishing_stand_before_rejecting_the_cell() {
        let here = WorldTile {
            x: 3000,
            z: 3000,
            level: 0,
        };
        let cell = SceneRegionInput {
            min_x: here.x,
            min_z: here.z,
            max_x: here.x,
            max_z: here.z,
            level: here.level,
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: here,
            radius: 16,
        };
        let deny = DenyOneTile(here);
        let mut danger = DangerCheck::new(Some(&deny));
        let stand = legal_fishing_return_stand(cell, here, here, area, None, &mut danger)
            .expect("the cell contains other allowed access sets");
        assert_ne!(stand, here);
        assert!(danger.allows(stand, None, nav::contract::ArrivalKind::Area));
    }

    #[test]
    fn danger_without_navigation_data_skips_fishing_stand_search() {
        let here = WorldTile {
            x: 3000,
            z: 3000,
            level: 0,
        };
        let cell = SceneRegionInput {
            min_x: here.x,
            min_z: here.z,
            max_x: here.x,
            max_z: here.z,
            level: here.level,
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: here,
            radius: 16,
        };
        let mut danger = DangerCheck::new(None);
        take_fishing_stand_searches();
        assert!(legal_fishing_return_stand(cell, here, here, area, None, &mut danger).is_none());
        assert_eq!(take_fishing_stand_searches(), 0);
        assert_eq!(danger.failure(), Some(DangerFailure::Unavailable));
    }

    fn target(entity: EntityId, class: TargetClass) -> GatherTarget {
        GatherTarget {
            entity,
            op: 1,
            class,
            respawn: Knowledge::Known(None),
        }
    }

    fn method(skill: GatherSkill, targets: Vec<GatherTarget>) -> GatherMethod {
        GatherMethod {
            id: api::selected::FactKey::new("fixture.method"),
            skill,
            resources: Arc::from([]),
            targets: Knowledge::Known(Arc::from(targets)),
            products: Knowledge::Known(Arc::from([])),
            tools: Knowledge::Known(Arc::from([])),
            consumes: Knowledge::Known(Arc::from([])),
            requirements: Knowledge::Known(Arc::from([])),
            spots: Knowledge::Known(Arc::from([])),
        }
    }

    fn spot(entity: EntityId, origin: WorldTile) -> GatherSpot {
        GatherSpot {
            id: SpotId(1),
            entity,
            origin,
            width: 1,
            length: 1,
            movement: Knowledge::Known(None),
            source: api::selected::SourceSpan {
                file: Arc::from("test"),
                first: 1,
                last: 1,
            },
        }
    }

    fn loc(id: i32, tile: WorldTile) -> LocView {
        LocView {
            typecode: 0,
            info: 0,
            id,
            name: None,
            description: None,
            actions: vec![],
            tile,
            distance: 0,
            layer: LocLayer::Ground,
            shape: 10,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: true,
            block_range: true,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }
    }

    fn npc(index: usize, type_id: usize, tile: WorldTile) -> NpcView {
        NpcView {
            index,
            r#type: Some(type_id),
            name: None,
            actions: vec![],
            tile,
            distance: 0,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: tile,
            x: 0,
            z: 0,
            yaw: 0,
        }
    }

    pub(super) fn real_catalog() -> Arc<GatherCatalog> {
        let data = api::game_data::for_revision(client::io::ClientRevision::R289)
            .expect("embedded game data");
        api::selected::FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
            .expect("start catalog preparation")
            .join()
            .expect("catalog worker")
            .expect("prepared gathering catalog")
    }

    fn method_index(catalog: &GatherCatalog, method: &GatherMethod) -> usize {
        catalog
            .methods()
            .iter()
            .position(|candidate| std::ptr::eq(candidate, method))
            .expect("method belongs to catalog")
    }

    fn scene_around(tile: WorldTile) -> WorldStateView {
        WorldStateView {
            map_base_x: tile.x.saturating_sub(52),
            map_base_z: tile.z.saturating_sub(52),
            level: tile.level,
            ..WorldStateView::default()
        }
    }

    fn radius_one_tiles(centre: WorldTile) -> impl Iterator<Item = WorldTile> {
        let radius = i32::from(RESOURCE_APPROACH_RADIUS);
        (-radius..=radius).flat_map(move |dx| {
            (-radius..=radius).map(move |dz| WorldTile {
                x: centre.x + dx,
                z: centre.z + dz,
                level: centre.level,
            })
        })
    }

    fn fishing_cover(method: &GatherMethod, area: WorkArea) -> Vec<WorldTile> {
        complete_spots(method)
            .into_iter()
            .flatten()
            .filter(|spot| fishing_spot_eligible(spot, area))
            .flat_map(|spot| {
                let bounds = movement_bounds(spot).expect("eligible spots have movement");
                [
                    spot.origin,
                    WorldTile {
                        x: bounds.min_x,
                        z: bounds.min_z,
                        level: bounds.level,
                    },
                    WorldTile {
                        x: bounds.max_x,
                        z: bounds.max_z,
                        level: bounds.level,
                    },
                ]
            })
            .collect()
    }

    /// Standable grid over the covered tiles. `blocked` cells are `SQ_BLOCKED`
    /// (Catherby water); every other cell is standable, including Draynor shore.
    fn synthetic_collision(
        cover: impl IntoIterator<Item = WorldTile>,
        blocked: impl IntoIterator<Item = WorldTile>,
    ) -> nav::collision::WorldCollision {
        let blocked: Vec<WorldTile> = blocked.into_iter().collect();
        let pad = OBSERVATION_RADIUS + i32::from(RESOURCE_APPROACH_RADIUS);
        let mut min_x = i32::MAX;
        let mut min_z = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_z = i32::MIN;
        for tile in cover.into_iter().chain(blocked.iter().copied()) {
            min_x = min_x.min(tile.x - pad);
            min_z = min_z.min(tile.z - pad);
            max_x = max_x.max(tile.x + pad);
            max_z = max_z.max(tile.z + pad);
        }
        assert!(
            min_x <= max_x && min_z <= max_z,
            "synthetic collision needs at least one covered tile"
        );
        let width = usize::try_from(max_x - min_x + 1).expect("collision width");
        let height = usize::try_from(max_z - min_z + 1).expect("collision height");
        let mut flags = vec![0u32; 4 * width * height];
        let block = client::dash3d::CollisionFlag::SQ_BLOCKED as u32;
        for tile in &blocked {
            let Ok(lx) = usize::try_from(tile.x - min_x) else {
                continue;
            };
            let Ok(lz) = usize::try_from(tile.z - min_z) else {
                continue;
            };
            if tile.level == 0 && lx < width && lz < height {
                flags[lz * width + lx] = block;
            }
        }
        let (walk, packed) = nav::collision::pack_walk(&flags);
        nav::collision::WorldCollision {
            origin: WorldTile {
                x: min_x,
                z: min_z,
                level: 0,
            },
            width,
            height,
            walk,
            blocked: packed,
            flags: None,
        }
    }

    fn area_arrival_ok(collision: &nav::collision::WorldCollision, stand: WorldTile) -> bool {
        nav::arrival::area_has_standable_goal(stand, i32::from(RESOURCE_APPROACH_RADIUS), |tile| {
            collision.standable(tile)
        })
    }

    const FISHING_GUILD: SceneRegionInput = SceneRegionInput {
        min_x: 2599,
        min_z: 3410,
        max_x: 2612,
        max_z: 3426,
        level: 0,
    };
    const ARDOUGNE_START: WorldTile = WorldTile {
        x: 2661,
        z: 3306,
        level: 0,
    };
    const BLOCKED_GUILD_CLAMP: WorldTile = WorldTile {
        x: 2613,
        z: 3412,
        level: 0,
    };
    pub(super) fn real_nav_world() -> &'static nav::world::NavWorld {
        super::test_real_nav_world().as_ref()
    }

    fn fishing_guild_spot(
        catalog: &GatherCatalog,
    ) -> (&GatherMethod, usize, &api::gather_methods::GatherSpot) {
        for method in catalog
            .methods()
            .iter()
            .filter(|method| method.skill == GatherSkill::Fishing)
        {
            for spot in complete_spots(method).unwrap_or(&[]) {
                if !known_resource_target(method, spot.entity)
                    || catalog.access(method, spot).ok() != Some(Truth::True)
                {
                    continue;
                }
                if movement_bounds(spot)
                    .is_some_and(|bounds| regions_intersect(bounds, FISHING_GUILD))
                {
                    return (method, method_index(catalog, method), spot);
                }
            }
        }
        panic!("289 catalog must include a Fishing Guild NPC movement");
    }

    fn guild_work_area() -> WorkArea {
        WorkArea {
            mode: super::super::area::AreaMode::Site,
            anchor: WorldTile {
                x: 2605,
                z: 3418,
                level: 0,
            },
            radius: 12,
        }
    }

    pub(super) fn guild_fishing_settings(
        method: &GatherMethod,
        area: WorkArea,
    ) -> GathererSettings {
        GathererSettings {
            skill: "Fishing".into(),
            fishing_method: method.id.0.to_string(),
            location: "Site".into(),
            site: "fishing.fishing_guild".into(),
            radius: area.radius,
            ..GathererSettings::default()
        }
    }

    fn take_fishing_stand_searches() -> u32 {
        super::FISHING_STAND_SEARCHES.with(|count| count.replace(0))
    }

    pub(super) fn select_unloaded_fishing(
        catalog: &GatherCatalog,
        method: &GatherMethod,
        index: usize,
        area: WorkArea,
        here: WorldTile,
        collision: Option<nav::contract::CollisionView<'_>>,
    ) -> SelectionResult {
        select(
            catalog,
            &[index],
            &guild_fishing_settings(method, area),
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision,
                world: &scene_around(here),
                locs: &[],
                npcs: &[],
                here,
                now: 1,
                skill_stat: 68,
            },
        )
    }

    pub(super) fn area_goals(
        collision: &nav::collision::WorldCollision,
        stand: WorldTile,
    ) -> Vec<WorldTile> {
        let radius = i32::from(RESOURCE_APPROACH_RADIUS);
        (-radius..=radius)
            .flat_map(move |dx| {
                (-radius..=radius).map(move |dz| WorldTile {
                    x: stand.x + dx,
                    z: stand.z + dz,
                    level: stand.level,
                })
            })
            .filter(|tile| collision.standable(*tile))
            .collect()
    }

    fn route_observation(
        world: &nav::world::NavWorld,
        from: WorldTile,
        goals: &WalkGoals,
        state: &nav::world_state::WorldState,
    ) -> Result<nav::router::Route, nav::router::RouteError> {
        let preferred = nav::router::find_first_with(
            &world.collision,
            &world.graph,
            from,
            goals.preferred(),
            nav::router::FindOptions::default(),
            state,
        )
        .into_route();
        preferred.or_else(|error| {
            if goals.fallback().is_empty() {
                return Err(error);
            }
            nav::router::find_first_with(
                &world.collision,
                &world.graph,
                from,
                goals.fallback(),
                nav::router::FindOptions::default(),
                state,
            )
            .into_route()
        })
    }

    #[test]
    fn large_fishing_spot_returns_to_and_observes_cells_beyond_the_eighth() {
        let catalog = real_catalog();
        let method = catalog.method("fishing.rarefish.op3").unwrap();
        let mut spot = complete_spots(method)
            .unwrap()
            .iter()
            .find(|spot| catalog.access(method, spot) == Ok(Truth::True))
            .unwrap()
            .clone();
        let origin = spot.origin;
        let bounds = SceneRegionInput {
            min_x: origin.x,
            min_z: origin.z,
            max_x: origin.x + OBSERVATION_WIDTH * 9 - 1,
            max_z: origin.z + OBSERVATION_WIDTH - 1,
            level: origin.level,
        };
        spot.movement = Knowledge::Known(Some(bounds));
        let here = WorldTile {
            x: bounds.max_x + 30,
            z: origin.z + OBSERVATION_RADIUS,
            ..origin
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: origin,
            radius: 1,
        };
        let expected_survey = WorldTile {
            x: bounds.max_x - OBSERVATION_RADIUS,
            z: here.z,
            ..origin
        };
        // Return stands at the resource, not the cell nearest the player.
        let expected_return = WorldTile {
            x: origin.x + OBSERVATION_RADIUS,
            z: origin.z + OBSERVATION_RADIUS,
            ..origin
        };
        let observation = ReturnObservation {
            walk_danger: Some(&crate::native::AllowWalkDanger),
            here,
            now: 1,
            skill_stat: 10,
            avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
            collision: None,
        };
        assert_eq!(
            fishing_return_stand(
                &catalog,
                method,
                &spot,
                area,
                &observation,
                &mut DangerCheck::new(observation.walk_danger),
            ),
            ReturnStand::Ready(expected_return),
            "Return must not reject a spot solely because its movement spans nine cells"
        );
        let mut fishing = FishingSurvey::default();
        let survey = fishing.placement(0, spot.id.0);
        assert_eq!(survey.next_stand(bounds, here), Some(expected_survey));
        survey.observe(bounds, &scene_around(expected_survey), expected_survey, 2);
        assert_ne!(
            survey.next_stand(bounds, expected_survey),
            Some(expected_survey),
            "arrival must record the ninth cell before selecting the next observation"
        );
    }

    #[test]
    fn large_fishing_survey_records_cells_beyond_the_inline_mask() {
        let bounds = SceneRegionInput {
            min_x: 3000,
            min_z: 3000,
            max_x: 3000 + OBSERVATION_WIDTH * 65 - 1,
            max_z: 3000 + OBSERVATION_WIDTH - 1,
            level: 0,
        };
        let here = WorldTile {
            x: bounds.max_x + 30,
            z: bounds.min_z + OBSERVATION_RADIUS,
            level: 0,
        };
        let expected = WorldTile {
            x: bounds.max_x - OBSERVATION_RADIUS,
            ..here
        };
        let mut fishing = FishingSurvey::default();
        let survey = fishing.placement(0, 0);
        assert_eq!(survey.next_stand(bounds, here), Some(expected));
        survey.observe(bounds, &scene_around(expected), expected, 1);
        assert_ne!(survey.next_stand(bounds, expected), Some(expected));
        survey.observe(
            bounds,
            &scene_around(here),
            here,
            OBSERVATION_WINDOW_TICKS + 2,
        );
        assert_eq!(
            survey.next_stand(bounds, here),
            Some(expected),
            "expired observations reopen distant cells without changing the approach budget"
        );
    }

    #[test]
    fn placements_follow_observed_rectangle_replacements_and_avoid_expiry() {
        let world = WorldStateView {
            map_base_x: 3200,
            map_base_z: 3200,
            level: 0,
            ..WorldStateView::default()
        };
        let mut method = method(
            GatherSkill::Woodcutting,
            vec![
                target(EntityId::Loc(10), TargetClass::Resource),
                target(EntityId::Loc(11), TargetClass::Depleted),
                target(EntityId::Loc(12), TargetClass::Hazard),
            ],
        );
        let mut placement = spot(
            EntityId::Loc(10),
            WorldTile {
                x: 3304,
                z: 3200,
                level: 0,
            },
        );
        let avoided = [AvoidedTile::EMPTY; MAX_AVOID];
        assert_eq!(
            classify_placement(
                &placement,
                &method,
                PlacementScene {
                    world: &world,
                    locs: &[],
                    npcs: &[],
                    hazard_npcs: &[]
                },
                &avoided,
                1
            ),
            PlacementClass::Unloaded
        );

        // At the loaded edge: player is 19 tiles from the final scene tile;
        // this placement is 30 tiles past that edge, but still inside radius.
        let here = WorldTile {
            x: 3284,
            z: 3200,
            level: 0,
        };
        placement.origin.x = 3334;
        let area = WorkArea {
            mode: super::super::area::AreaMode::Start,
            anchor: here,
            radius: 64,
        };
        assert!(area.contains(placement.origin));
        assert_eq!(
            classify_placement(
                &placement,
                &method,
                PlacementScene {
                    world: &world,
                    locs: &[],
                    npcs: &[],
                    hazard_npcs: &[]
                },
                &avoided,
                1
            ),
            PlacementClass::Unloaded
        );

        placement.origin.x = 3303;
        assert_eq!(
            classify_placement(
                &placement,
                &method,
                PlacementScene {
                    world: &world,
                    locs: &[],
                    npcs: &[],
                    hazard_npcs: &[]
                },
                &avoided,
                1
            ),
            PlacementClass::Absent
        );
        for (id, class, expected) in [
            (10, TargetClass::Resource, PlacementClass::Live),
            (11, TargetClass::Depleted, PlacementClass::Depleted),
            (12, TargetClass::Hazard, PlacementClass::Hazard),
        ] {
            method.targets = Knowledge::Known(Arc::from([target(EntityId::Loc(id), class)]));
            assert_eq!(
                classify_placement(
                    &placement,
                    &method,
                    PlacementScene {
                        world: &world,
                        locs: &[loc(id, placement.origin)],
                        npcs: &[],
                        hazard_npcs: &[],
                    },
                    &avoided,
                    1,
                ),
                expected
            );
        }
        method.targets = Knowledge::Known(Arc::from([
            target(EntityId::Loc(10), TargetClass::Resource),
            target(EntityId::Loc(12), TargetClass::Hazard),
        ]));
        let hazard_npcs = [900];
        assert_eq!(
            classify_placement(
                &placement,
                &method,
                PlacementScene {
                    world: &world,
                    locs: &[],
                    npcs: &[npc(4, 900, placement.origin)],
                    hazard_npcs: &hazard_npcs,
                },
                &avoided,
                1,
            ),
            PlacementClass::Hazard
        );

        let mut avoided = avoided;
        avoided[0] = AvoidedTile {
            tile: placement.origin,
            until: 3,
        };
        assert_eq!(
            classify_placement(
                &placement,
                &method,
                PlacementScene {
                    world: &world,
                    locs: &[loc(12, placement.origin)],
                    npcs: &[],
                    hazard_npcs: &[],
                },
                &avoided,
                2,
            ),
            PlacementClass::Avoided
        );
        assert_eq!(
            classify_placement(
                &placement,
                &method,
                PlacementScene {
                    world: &world,
                    locs: &[loc(12, placement.origin)],
                    npcs: &[],
                    hazard_npcs: &[],
                },
                &avoided,
                3,
            ),
            PlacementClass::Hazard
        );
        placement.origin.level = 1;
        assert_eq!(
            classify_placement(
                &placement,
                &method,
                PlacementScene {
                    world: &world,
                    locs: &[],
                    npcs: &[],
                    hazard_npcs: &[]
                },
                &avoided,
                2
            ),
            PlacementClass::Unloaded
        );
    }

    #[test]
    fn fishing_spots_use_movement_boxes_and_do_not_target_hazards() {
        let bounds = SceneRegionInput {
            min_x: 3290,
            min_z: 3290,
            max_x: 3310,
            max_z: 3310,
            level: 0,
        };
        let origin = WorldTile {
            x: 3280,
            z: 3280,
            level: 0,
        };
        let mut fish_spot = spot(EntityId::Npc(309), origin);
        fish_spot.movement = Knowledge::Known(Some(bounds));
        let fish_method = method(
            GatherSkill::Fishing,
            vec![target(EntityId::Npc(309), TargetClass::Resource)],
        );
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: WorldTile {
                x: 3300,
                z: 3300,
                level: 0,
            },
            radius: 2,
        };
        assert!(!area.contains(origin));
        assert!(fishing_spot_eligible(&fish_spot, area));

        let world = WorldStateView {
            map_base_x: 3250,
            map_base_z: 3250,
            level: 0,
            ..WorldStateView::default()
        };
        let target_tile = WorldTile {
            x: 3301,
            z: 3302,
            level: 0,
        };
        assert_eq!(
            classify_fishing_placement(
                &fish_spot,
                &fish_method,
                PlacementScene {
                    world: &world,
                    locs: &[],
                    npcs: &[npc(42, 309, target_tile)],
                    hazard_npcs: &[],
                },
                area.anchor,
                &[AvoidedTile::EMPTY; MAX_AVOID],
                1,
            ),
            PlacementClass::Live
        );
        assert_eq!(
            classify_fishing_placement(
                &fish_spot,
                &fish_method,
                PlacementScene {
                    world: &world,
                    locs: &[],
                    npcs: &[],
                    hazard_npcs: &[],
                },
                area.anchor,
                &[AvoidedTile::EMPTY; MAX_AVOID],
                1,
            ),
            PlacementClass::Absent
        );
        assert_eq!(
            classify_fishing_placement(
                &fish_spot,
                &fish_method,
                PlacementScene {
                    world: &world,
                    locs: &[],
                    npcs: &[npc(43, 900, target_tile)],
                    hazard_npcs: &[900],
                },
                area.anchor,
                &[AvoidedTile::EMPTY; MAX_AVOID],
                1,
            ),
            PlacementClass::Hazard
        );
        assert!(!known_resource_target(&fish_method, EntityId::Npc(900)));

        fish_spot.movement = Knowledge::Known(Some(SceneRegionInput {
            min_x: 3330,
            min_z: 3290,
            max_x: 3340,
            max_z: 3310,
            level: 0,
        }));
        assert!(fishing_spot_eligible(
            &fish_spot,
            WorkArea {
                anchor: WorldTile {
                    x: 3335,
                    z: 3300,
                    level: 0,
                },
                ..area
            }
        ));
        assert_eq!(
            classify_fishing_placement(
                &fish_spot,
                &fish_method,
                PlacementScene {
                    world: &WorldStateView {
                        map_base_x: 3200,
                        map_base_z: 3200,
                        level: 0,
                        ..WorldStateView::default()
                    },
                    locs: &[],
                    npcs: &[],
                    hazard_npcs: &[],
                },
                area.anchor,
                &[AvoidedTile::EMPTY; MAX_AVOID],
                1,
            ),
            PlacementClass::Unloaded
        );
    }

    #[test]
    fn selected_fishing_target_uses_npc_type_index_and_live_tile() {
        let catalog = real_catalog();
        let method = catalog
            .method("fishing.saltfish.op1")
            .expect("saltfish fishing method");
        let method_index = method_index(&catalog, method);
        let spot = complete_spots(method)
            .expect("complete fishing placements")
            .iter()
            .find(|spot| {
                known_resource_target(method, spot.entity)
                    && catalog.access(method, spot).ok() == Some(Truth::True)
                    && movement_bounds(spot).is_some_and(|bounds| {
                        [
                            WorldTile {
                                x: bounds.min_x,
                                z: bounds.min_z,
                                level: bounds.level,
                            },
                            WorldTile {
                                x: bounds.max_x,
                                z: bounds.max_z,
                                level: bounds.level,
                            },
                            WorldTile {
                                x: bounds.min_x + (bounds.max_x - bounds.min_x) / 2,
                                z: bounds.min_z + (bounds.max_z - bounds.min_z) / 2,
                                level: bounds.level,
                            },
                        ]
                        .into_iter()
                        .any(|tile| tile != spot.origin && region_contains(bounds, tile))
                    })
            })
            .expect("a moving fishing spot in the pinned content");
        let bounds = movement_bounds(spot).expect("fishing movement bounds");
        let tile = [
            WorldTile {
                x: bounds.min_x,
                z: bounds.min_z,
                level: bounds.level,
            },
            WorldTile {
                x: bounds.max_x,
                z: bounds.max_z,
                level: bounds.level,
            },
            WorldTile {
                x: bounds.min_x + (bounds.max_x - bounds.min_x) / 2,
                z: bounds.min_z + (bounds.max_z - bounds.min_z) / 2,
                level: bounds.level,
            },
        ]
        .into_iter()
        .find(|tile| *tile != spot.origin && region_contains(bounds, *tile))
        .expect("a non-origin tile in the movement box");
        let EntityId::Npc(type_id) = spot.entity else {
            panic!("fishing spot identity must be an NPC type");
        };
        let type_index = usize::try_from(type_id).expect("NPC type id is nonnegative");
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: tile,
            radius: 2,
        };
        let settings = GathererSettings {
            skill: "Fishing".into(),
            location: "Custom".into(),
            custom_tile: Some(tile),
            radius: 2,
            ..GathererSettings::default()
        };
        let world = scene_around(tile);
        let observed_npcs = [npc(42, type_index, tile)];
        let selected = select(
            &catalog,
            &[method_index],
            &settings,
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision: None,
                world: &world,
                locs: &[],
                npcs: &observed_npcs,
                here: tile,
                now: 1,
                skill_stat: 10,
            },
        );
        let target = selected.target.expect("live fish selected");
        assert_eq!(target.class, PlacementClass::Live);
        assert_eq!(target.plan.entity, EntityId::Npc(type_id));
        assert_eq!(target.plan.npc_index, 42);
        assert_eq!(target.plan.tile, tile);
        let moved_tile = [
            WorldTile {
                x: tile.x.saturating_add(1),
                ..tile
            },
            WorldTile {
                x: tile.x.saturating_sub(1),
                ..tile
            },
            WorldTile {
                z: tile.z.saturating_add(1),
                ..tile
            },
            WorldTile {
                z: tile.z.saturating_sub(1),
                ..tile
            },
        ]
        .into_iter()
        .find(|moved| *moved != tile && area.contains(*moved) && region_contains(bounds, *moved))
        .expect("a moved fishing instance remains in its eligible movement box");
        let moved_npc = [npc(43, type_index, moved_tile)];
        let reacquired = select(
            &catalog,
            &[method_index],
            &settings,
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision: None,
                world: &world,
                locs: &[],
                npcs: &moved_npc,
                here: tile,
                now: 2,
                skill_stat: 10,
            },
        )
        .target
        .expect("the moved spot NPC is re-acquired");
        assert_eq!(reacquired.plan.entity, EntityId::Npc(type_id));
        assert_eq!(reacquired.plan.npc_index, 43);
        assert_eq!(reacquired.plan.tile, moved_tile);
        let hazard_id = catalog
            .hazard_npcs()
            .iter()
            .copied()
            .find(|id| *id != type_id)
            .expect("a content-derived hazard NPC id distinct from the fish type");
        let observed_hazard = [npc(
            43,
            usize::try_from(hazard_id).expect("hazard id fits an NPC type"),
            tile,
        )];
        let hazard_selection = select(
            &catalog,
            &[method_index],
            &settings,
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision: None,
                world: &world,
                locs: &[],
                npcs: &observed_hazard,
                here: tile,
                now: 2,
                skill_stat: 10,
            },
        );
        if let Some(candidate) = hazard_selection.target {
            assert_eq!(candidate.plan.entity, EntityId::Npc(type_id));
            assert_eq!(candidate.class, PlacementClass::Unloaded);
            assert_eq!(candidate.plan.npc_index, NO_NPC_INDEX);
        }
    }

    #[test]
    fn bank_return_stand_observes_real_harpoon_placements_for_all_area_modes() {
        use super::super::area::AreaMode;

        let catalog = real_catalog();
        let method = catalog.method("fishing.rarefish.op3").unwrap();
        let index = method_index(&catalog, method);
        let here = WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        };
        let settings = GathererSettings {
            skill: "Fishing".into(),
            fishing_method: method.id.0.to_string(),
            radius: 40,
            ..GathererSettings::default()
        };
        let water = WorldTile {
            x: 2850,
            z: 3423,
            level: 0,
        };
        let collision = synthetic_collision(
            fishing_cover(
                method,
                WorkArea {
                    mode: AreaMode::Custom,
                    anchor: WorldTile {
                        x: 2848,
                        z: 3426,
                        level: 0,
                    },
                    radius: settings.radius,
                },
            ),
            radius_one_tiles(water),
        );
        let lease = test_nav_lease_for_collision(&collision);
        for mode in [AreaMode::Auto, AreaMode::Start, AreaMode::Custom] {
            let area = WorkArea {
                mode,
                anchor: WorldTile {
                    x: 2848,
                    z: 3426,
                    level: 0,
                },
                radius: settings.radius,
            };
            assert!(
                area.contains(here),
                "Catherby bank is inside the gathering bound"
            );
            let target = resource_return_target(
                &catalog,
                &[index],
                &settings,
                area,
                ReturnObservation {
                    walk_danger: Some(&crate::native::AllowWalkDanger),
                    here,
                    now: 1,
                    skill_stat: 10,
                    avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                    collision: Some(lease.collision()),
                },
            )
            .plan()
            .expect("the selected content supplies a resource observation stand");
            assert_ne!(target.tile, area.anchor);
            assert!(distance(here, target.tile) > i64::from(NPC_VIEW_RADIUS));
            assert_eq!(target.npc_index, NO_NPC_INDEX);
            assert!(
                complete_spots(method).unwrap().iter().any(|spot| {
                    spot.entity == target.entity
                        && fishing_spot_eligible(spot, area)
                        && observation_cells(movement_bounds(spot).unwrap()).any(|cell| {
                            let radius = i32::from(RESOURCE_APPROACH_RADIUS);
                            (-radius..=radius).all(|dx| {
                                (-radius..=radius).all(|dz| {
                                    region_visible(
                                        cell,
                                        WorldTile {
                                            x: target.tile.x + dx,
                                            z: target.tile.z + dz,
                                            level: target.tile.level,
                                        },
                                    )
                                })
                            })
                        })
                }),
                "every accepted radius-one arrival observes a selected movement cell"
            );
        }
    }

    #[test]
    fn catherby_edgeville_return_skips_the_all_water_harpoon_stand() {
        use super::super::area::AreaMode;

        let catalog = real_catalog();
        let method = catalog.method("fishing.rarefish.op3").unwrap();
        let index = method_index(&catalog, method);
        let water = WorldTile {
            x: 2850,
            z: 3423,
            level: 0,
        };
        let edgeville = WorldTile {
            x: 3094,
            z: 3493,
            level: 0,
        };
        let anchor = WorldTile {
            x: 2848,
            z: 3426,
            level: 0,
        };
        let area = WorkArea {
            mode: AreaMode::Custom,
            anchor,
            radius: 40,
        };
        let collision = synthetic_collision(fishing_cover(method, area), radius_one_tiles(water));
        let lease = test_nav_lease_for_collision(&collision);
        let settings = GathererSettings {
            skill: "Fishing".into(),
            fishing_method: method.id.0.to_string(),
            radius: area.radius,
            bank: "Edgeville".into(),
            ..GathererSettings::default()
        };
        assert!(
            complete_spots(method).unwrap().iter().any(|spot| {
                fishing_spot_eligible(spot, area)
                    && known_resource_target(method, spot.entity)
                    && observation_cells(movement_bounds(spot).unwrap())
                        .any(|cell| observation_stand(cell, spot.origin) == water)
            }),
            "pinned 289 Catherby harpoon still origin-clamps the eastern placement to {water:?}"
        );
        assert!(
            !area_arrival_ok(&collision, water),
            "{water:?} has no legal radius-one Area goal"
        );
        let target = resource_return_target(
            &catalog,
            &[index],
            &settings,
            area,
            ReturnObservation {
                walk_danger: Some(&crate::native::AllowWalkDanger),
                here: edgeville,
                now: 1,
                skill_stat: 10,
                avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                collision: Some(lease.collision()),
            },
        )
        .plan()
        .expect("Catherby harpoon supplies a resource return stand");
        assert_ne!(
            target.tile, water,
            "Edgeville ranking used to pick the all-water eastern stand"
        );
        assert!(
            area_arrival_ok(&collision, target.tile),
            "return stand {:?} must have a legal radius-one Area goal",
            target.tile
        );
        assert_ne!(target.tile, edgeville);
        assert_eq!(target.npc_index, NO_NPC_INDEX);
        assert!(
            complete_spots(method).unwrap().iter().any(|spot| {
                spot.entity == target.entity
                    && fishing_spot_eligible(spot, area)
                    && observation_cells(movement_bounds(spot).unwrap()).any(|cell| {
                        let radius = i32::from(RESOURCE_APPROACH_RADIUS);
                        (-radius..=radius).all(|dx| {
                            (-radius..=radius).all(|dz| {
                                region_visible(
                                    cell,
                                    WorldTile {
                                        x: target.tile.x + dx,
                                        z: target.tile.z + dz,
                                        level: target.tile.level,
                                    },
                                )
                            })
                        })
                    })
            }),
            "every accepted radius-one arrival observes a selected movement cell"
        );
    }

    #[test]
    fn draynor_bank_return_stand_is_at_the_saltfish_spots_not_the_booth() {
        use super::super::area::AreaMode;

        let catalog = real_catalog();
        let method = catalog
            .method("fishing.saltfish.op3")
            .expect("Draynor bait fishing");
        let index = method_index(&catalog, method);
        let bank_interior = WorldTile {
            x: 3092,
            z: 3241,
            level: 0,
        };
        let north_door = WorldTile {
            x: 3091,
            z: 3247,
            level: 0,
        };
        let spots = [
            WorldTile {
                x: 3085,
                z: 3230,
                level: 0,
            },
            WorldTile {
                x: 3086,
                z: 3227,
                level: 0,
            },
        ];
        let anchor = WorldTile {
            x: 3086,
            z: 3229,
            level: 0,
        };
        let settings = GathererSettings {
            skill: "Fishing".into(),
            fishing_method: method.id.0.to_string(),
            radius: 8,
            ..GathererSettings::default()
        };
        assert!(
            complete_spots(method).unwrap().iter().any(|spot| {
                let bounds = movement_bounds(spot).unwrap_or(SceneRegionInput {
                    min_x: spot.origin.x,
                    min_z: spot.origin.z,
                    max_x: spot.origin.x,
                    max_z: spot.origin.z,
                    level: spot.origin.level,
                });
                spots.iter().any(|tile| region_contains(bounds, *tile))
            }),
            "pinned 289 content must include the Draynor saltfish envelope"
        );
        for (mode, radius) in [
            (AreaMode::Custom, 8_u16),
            (AreaMode::Start, 8),
            (AreaMode::Auto, 40),
        ] {
            let area = WorkArea {
                mode,
                anchor,
                radius,
            };
            let collision = synthetic_collision(
                fishing_cover(method, area).into_iter().chain([
                    bank_interior,
                    north_door,
                    anchor,
                    spots[0],
                    spots[1],
                ]),
                std::iter::empty::<WorldTile>(),
            );
            let lease = test_nav_lease_for_collision(&collision);
            let target = resource_return_target(
                &catalog,
                &[index],
                &settings,
                area,
                ReturnObservation {
                    walk_danger: Some(&crate::native::AllowWalkDanger),
                    here: bank_interior,
                    now: 1,
                    skill_stat: 10,
                    avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                    collision: Some(lease.collision()),
                },
            )
            .plan()
            .expect("Draynor saltfish supplies a resource return stand");
            assert_ne!(
                target.tile, bank_interior,
                "{mode:?}: clamp-toward-here used to pick the bank interior"
            );
            assert_ne!(target.tile, north_door);
            assert!(
                area.contains(target.tile)
                    || spots.iter().any(|spot| distance(target.tile, *spot) <= 1),
                "{mode:?}: stand {target:?} must sit in the gathering area or next to a spot"
            );
            assert!(
                spots
                    .iter()
                    .any(|spot| distance(target.tile, *spot) <= i64::from(NPC_VIEW_RADIUS)),
                "{mode:?}: the spots must stay in view from {target:?}"
            );
            assert!(
                distance(target.tile, north_door) > i64::from(NPC_VIEW_RADIUS)
                    || target.tile.z < north_door.z,
                "{mode:?}: a north-door click route loses the spots (dz ≈ 17)"
            );
            assert_eq!(target.npc_index, NO_NPC_INDEX);
            let player_clamp = observation_cells(
                complete_spots(method)
                    .unwrap()
                    .iter()
                    .find(|spot| {
                        fishing_spot_eligible(spot, area)
                            && known_resource_target(method, spot.entity)
                    })
                    .and_then(movement_bounds)
                    .expect("eligible Draynor saltfish movement"),
            )
            .map(|cell| observation_stand(cell, bank_interior))
            .min_by_key(|stand| distance(bank_interior, *stand))
            .expect("a toward-player observation clamp");
            assert_ne!(
                target.tile, player_clamp,
                "{mode:?}: return must not reuse the player-clamped observation stand {player_clamp:?}"
            );
        }
    }

    #[test]
    fn draynor_real_router_returns_to_the_spots_not_the_bank_booth() {
        let Some(_pack) =
            crate::ctx::test_support::local_asset_path("NAV_PACK", "", "real 289 nav pack")
        else {
            return;
        };
        let world = real_nav_world();
        let lease = test_real_nav_lease();
        let catalog = real_catalog();
        let method = catalog.method("fishing.saltfish.op3").unwrap();
        let bank = WorldTile {
            x: 3092,
            z: 3242,
            level: 0,
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Site,
            anchor: WorldTile {
                x: 3086,
                z: 3229,
                level: 0,
            },
            radius: 8,
        };
        let ResourceReturn::Ready(selected) = resource_return_target(
            &catalog,
            &[method_index(&catalog, method)],
            &guild_fishing_settings(method, area),
            area,
            ReturnObservation {
                walk_danger: Some(&crate::native::AllowWalkDanger),
                here: bank,
                now: 1,
                skill_stat: 68,
                avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                collision: Some(lease.collision()),
            },
        ) else {
            panic!("Draynor must supply an observation return");
        };
        let goals = selected.observation_goals.as_ref().unwrap();
        assert!(
            goals.fallback().contains(&bank),
            "reproduce the review's admitted booth stand"
        );
        assert!(!goals.preferred().contains(&bank));
        let mut state = nav::world_state::WorldState::empty().with_map_members(true);
        state.stats = (0..=20).map(|skill| (skill, 99)).collect();
        let route = route_observation(world, bank, goals, &state).unwrap();
        assert!(goals.preferred().contains(&route.dest));
        assert!(area.contains(route.dest));
        assert_ne!(route.dest, bank);
        assert!(route.ticks > 0.0);
        let old = nav::router::find_first_with(
            &world.collision,
            &world.graph,
            bank,
            &area_goals(&world.collision, selected.plan.tile),
            nav::router::FindOptions::default(),
            &state,
        )
        .into_route()
        .unwrap();
        assert_eq!(
            route, old,
            "the router must preserve the old resource-origin choice"
        );
    }

    #[test]
    fn loc_bank_return_uses_the_resource_origin_not_an_observation_clamp() {
        let catalog = real_catalog();
        let method = catalog.method("woodcutting.willow").unwrap();
        let index = method_index(&catalog, method);
        let here = WorldTile {
            x: 3092,
            z: 3241,
            level: 0,
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: WorldTile {
                x: 3083,
                z: 3237,
                level: 0,
            },
            radius: 16,
        };
        assert!(area.contains(here));
        let settings = GathererSettings {
            skill: "Woodcutting".into(),
            woodcutting_resources: vec!["willow".into()],
            radius: area.radius,
            ..GathererSettings::default()
        };
        let collision = synthetic_collision(
            catalog
                .spots(method, &area.region())
                .expect("willow placements")
                .map(|spot| spot.origin),
            std::iter::empty::<WorldTile>(),
        );
        let lease = test_nav_lease_for_collision(&collision);
        let target = resource_return_target(
            &catalog,
            &[index],
            &settings,
            area,
            ReturnObservation {
                walk_danger: Some(&crate::native::AllowWalkDanger),
                here,
                now: 1,
                skill_stat: 30,
                avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                collision: Some(lease.collision()),
            },
        )
        .plan()
        .expect("willow origin is the loc return stand");
        assert!(
            catalog
                .spots(method, &area.region())
                .expect("willow placements")
                .any(|spot| known_resource_target(method, spot.entity)
                    && catalog.access(method, spot).ok() == Some(Truth::True)
                    && spot.origin == target.tile),
            "loc return walks to a resource origin, not a player-clamped observation stand: {:?}",
            target.tile
        );
        assert_ne!(target.tile, here);
    }

    #[test]
    fn fishing_bank_return_approaches_unobserved_spots_before_reacquiring_the_actor() {
        let catalog = real_catalog();
        let method = catalog.method("fishing.rarefish.op3").unwrap();
        let index = method_index(&catalog, method);
        let anchor = WorldTile {
            x: 2840,
            z: 3436,
            level: 0,
        };
        let here = WorldTile { x: 2828, ..anchor };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Start,
            anchor,
            radius: 12,
        };
        let settings = GathererSettings {
            skill: "Fishing".into(),
            fishing_method: method.id.0.to_string(),
            radius: area.radius,
            ..GathererSettings::default()
        };
        let world = scene_around(here);
        let approach = select(
            &catalog,
            &[index],
            &settings,
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision: None,
                world: &world,
                locs: &[],
                npcs: &[],
                here,
                now: 1,
                skill_stat: 10,
            },
        );
        let target = approach.target.expect(
            "returning inside the work area does not establish observation of distant fishing NPCs",
        );
        assert_eq!(target.class, PlacementClass::Unloaded);
        assert_eq!(target.plan.npc_index, NO_NPC_INDEX);
        let EntityId::Npc(type_id) = target.plan.entity else {
            panic!("harpoon spot must be NPC-backed");
        };
        let spot = complete_spots(method)
            .unwrap()
            .iter()
            .find(|spot| spot.entity == target.plan.entity && fishing_spot_eligible(spot, area))
            .unwrap();
        assert!(
            region_loaded(movement_bounds(spot).unwrap(), &world),
            "the regression needs a loaded map but an unobserved NPC region"
        );

        let visible = [npc(43, type_id as usize, spot.origin)];
        let reacquired = select(
            &catalog,
            &[index],
            &settings,
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision: None,
                world: &world,
                locs: &[],
                npcs: &visible,
                here: target.plan.tile,
                now: 2,
                skill_stat: 10,
            },
        )
        .target
        .expect("the fresh actor observation resumes fishing");
        assert_eq!(reacquired.class, PlacementClass::Live);
        assert_eq!(reacquired.plan.npc_index, 43);
        assert_eq!(reacquired.plan.entity, target.plan.entity);
    }

    #[test]
    fn fishing_empty_arrival_decides_and_moved_east_edge_is_reacquired() {
        let catalog = real_catalog();
        let method = catalog.method("fishing.rarefish.op3").unwrap();
        let index = method_index(&catalog, method);
        let anchor = WorldTile {
            x: 2840,
            z: 3436,
            level: 0,
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Start,
            anchor,
            radius: 12,
        };
        let settings = GathererSettings {
            skill: "Fishing".into(),
            fishing_method: method.id.0.to_string(),
            radius: area.radius,
            ..GathererSettings::default()
        };
        for moved in [false, true] {
            let mut survey = FishingSurvey::default();
            let mut here = WorldTile {
                x: 2844,
                z: 3430,
                level: 0,
            };
            let east = WorldTile {
                x: 2860,
                z: 3426,
                level: 0,
            };
            let mut decided = false;
            for now in 1..=9 {
                let world = scene_around(here);
                let actors = [npc(43, 321, east)];
                let visible = if moved && distance(here, east) <= 15 {
                    actors.as_slice()
                } else {
                    &[]
                };
                let result = select(
                    &catalog,
                    &[index],
                    &settings,
                    area,
                    &[AvoidedTile::EMPTY; MAX_AVOID],
                    SelectionObservation {
                        snapshot: None,
                        walk_danger: Some(&crate::native::AllowWalkDanger),
                        fishing: &mut survey,
                        collision: None,
                        world: &world,
                        locs: &[],
                        npcs: visible,
                        here,
                        now,
                        skill_stat: 10,
                    },
                );
                match result.target {
                    Some(target) if target.class == PlacementClass::Live => {
                        assert!(moved);
                        assert_eq!(target.plan.tile, east);
                        decided = true;
                        eprintln!("moved Catherby east-edge spot reacquired at selection {now}, stand {here:?}");
                        break;
                    }
                    Some(target) => {
                        assert_eq!(target.class, PlacementClass::Unloaded);
                        assert_ne!(
                            target.plan.tile, here,
                            "arrival at the old radius-one stand must not reselect the same stand"
                        );
                        here = WorldTile {
                            z: target.plan.tile.z + 1,
                            ..target.plan.tile
                        };
                    }
                    None => {
                        assert!(
                            !moved,
                            "the content east-edge actor must be observed, not lost"
                        );
                        assert!(matches!(
                            result.outcome,
                            Selection::Absent { .. } | Selection::Exhausted { .. }
                        ));
                        eprintln!(
                            "removed Catherby spot decided {:?} at selection {now}, stand {here:?}",
                            result.outcome
                        );
                        decided = true;
                        break;
                    }
                }
            }
            assert!(
                decided,
                "removed/moved Catherby spot must reach a decision within the approach bound"
            );
        }
    }

    #[test]
    fn catherby_moved_spot_is_surveyed_while_old_tile_is_avoided() {
        let catalog = real_catalog();
        let method = catalog.method("fishing.saltfish.op1").unwrap();
        let index = method_index(&catalog, method);
        let anchor = WorldTile {
            x: 2836,
            z: 3431,
            level: 0,
        };
        let here = WorldTile {
            x: 2838,
            z: 3432,
            level: 0,
        };
        // A real destination observed for NPC 2171 in the baseline move cell.
        let moved = WorldTile {
            x: 2859,
            z: 3426,
            level: 0,
        };
        assert!(distance(here, moved) > i64::from(NPC_VIEW_RADIUS));
        let area = WorkArea {
            mode: super::super::area::AreaMode::Site,
            anchor,
            radius: 32,
        };
        let settings = GathererSettings {
            skill: "Fishing".into(),
            fishing_method: method.id.0.to_string(),
            location: "Site".into(),
            site: "fishing.catherby".into(),
            radius: area.radius,
            ..GathererSettings::default()
        };
        let mut avoided = [AvoidedTile::EMPTY; MAX_AVOID];
        avoided[0] = AvoidedTile {
            tile: anchor,
            until: 9753,
        };
        let world = scene_around(here);
        let mut fishing = FishingSurvey::default();
        let approach = select(
            &catalog,
            &[index],
            &settings,
            area,
            &avoided,
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                world: &world,
                locs: &[],
                npcs: &[],
                here,
                now: 9723,
                skill_stat: 10,
                fishing: &mut fishing,
                collision: None,
            },
        );
        let target = approach.target.expect(
            "excluding the old tile must not suppress observing another saltfish in the shared movement area",
        );
        assert_eq!(target.class, PlacementClass::Unloaded);
        assert_eq!(target.plan.entity, EntityId::Npc(320));
        assert_ne!(target.plan.tile, anchor);
        assert!(distance(target.plan.tile, moved) <= i64::from(NPC_VIEW_RADIUS));
        let actors = [npc(2171, 320, moved)];
        let reacquired = select(
            &catalog,
            &[index],
            &settings,
            area,
            &avoided,
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                world: &world,
                locs: &[],
                npcs: &actors,
                here: target.plan.tile,
                now: 9724,
                skill_stat: 10,
                fishing: &mut fishing,
                collision: None,
            },
        )
        .target
        .expect("the visible moved actor must be selectable");
        assert_eq!(reacquired.class, PlacementClass::Live);
        assert_eq!(reacquired.plan.npc_index, 2171);
        assert_eq!(reacquired.plan.tile, moved);
    }

    #[test]
    fn fishing_wide_freshfish_envelope_is_fully_observed_then_absent() {
        let catalog = real_catalog();
        let method = catalog.method("fishing.freshfish.op1").unwrap();
        let index = method_index(&catalog, method);
        let placement = complete_spots(method)
            .unwrap()
            .iter()
            .find(|spot| {
                spot.entity == EntityId::Npc(317)
                    && catalog.access(method, spot).ok() == Some(Truth::True)
                    && movement_bounds(spot).is_some_and(|bounds| bounds.max_x - bounds.min_x > 31)
            })
            .expect("content NPC 317 has an envelope wider than one actor view");
        let mut here = placement.origin;
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: here,
            radius: 1,
        };
        let settings = GathererSettings {
            skill: "Fishing".into(),
            ..GathererSettings::default()
        };
        let mut stands = Vec::new();
        let mut survey = FishingSurvey::default();
        for now in 1..=9 {
            let world = scene_around(here);
            let result = select(
                &catalog,
                &[index],
                &settings,
                area,
                &[AvoidedTile::EMPTY; MAX_AVOID],
                SelectionObservation {
                    snapshot: None,
                    walk_danger: Some(&crate::native::AllowWalkDanger),
                    fishing: &mut survey,
                    collision: None,
                    world: &world,
                    locs: &[],
                    npcs: &[],
                    here,
                    now,
                    skill_stat: 10,
                },
            );
            let Some(target) = result.target else {
                assert!(matches!(result.outcome, Selection::Absent { .. }));
                assert!(!stands.is_empty());
                assert!(
                    stands.len() <= 4,
                    "the wide envelope needs only a small bounded survey"
                );
                eprintln!(
                    "wide NPC 317 envelope absent at selection {now} after {} approach stands",
                    stands.len()
                );
                return;
            };
            assert!(
                !stands.contains(&target.plan.tile),
                "empty survey must not repeat a covered stand"
            );
            stands.push(target.plan.tile);
            here = target.plan.tile;
        }
        panic!("the wide fishing envelope never reached Absent");
    }

    #[test]
    fn fishing_reapproach_is_capped_even_without_view_or_arrival_progress() {
        let catalog = real_catalog();
        let method = catalog.method("fishing.rarefish.op3").unwrap();
        let index = method_index(&catalog, method);
        let here = WorldTile {
            x: 2844,
            z: 3430,
            level: 0,
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: here,
            radius: 1,
        };
        let settings = GathererSettings {
            skill: "Fishing".into(),
            ..GathererSettings::default()
        };
        let world = WorldStateView {
            level: 1,
            ..scene_around(here)
        };
        let placements = complete_spots(method)
            .unwrap()
            .iter()
            .filter(|spot| {
                fishing_spot_eligible(spot, area)
                    && known_resource_target(method, spot.entity)
                    && catalog.access(method, spot).ok() == Some(Truth::True)
            })
            .count();
        let mut survey = FishingSurvey::default();
        for now in 1..=(placements as u64 * 8 + 1) {
            let result = select(
                &catalog,
                &[index],
                &settings,
                area,
                &[AvoidedTile::EMPTY; MAX_AVOID],
                SelectionObservation {
                    snapshot: None,
                    walk_danger: Some(&crate::native::AllowWalkDanger),
                    fishing: &mut survey,
                    collision: None,
                    world: &world,
                    locs: &[],
                    npcs: &[],
                    here,
                    now,
                    skill_stat: 10,
                },
            );
            if result.target.is_none() {
                assert!(matches!(
                    result.outcome,
                    Selection::Absent { .. } | Selection::Exhausted { .. }
                ));
                eprintln!("no-progress fishing approach cap decided {:?} at selection {now} for {placements} placements", result.outcome);
                return;
            }
        }
        panic!(
            "same-placement reapproaches must be capped even when observation makes no progress"
        );
    }

    #[test]
    fn fishing_content_envelopes_have_radius_one_safe_stands() {
        let catalog = real_catalog();
        for id in [310, 317, 320, 325, 1191] {
            let mut checked = 0;
            for method in catalog
                .methods()
                .iter()
                .filter(|method| method.skill == GatherSkill::Fishing)
            {
                for spot in complete_spots(method)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|spot| spot.entity == EntityId::Npc(id))
                {
                    let bounds = movement_bounds(spot).unwrap();
                    let cells: Vec<_> = observation_cells(bounds).collect();
                    assert!(!cells.is_empty(), "NPC {id} has no cells: {bounds:?}");
                    let survey = FishingPlacementSurvey {
                        method: 0,
                        spot: 0,
                        covered: 0,
                        covered_more: Vec::new(),
                        started: None,
                        approaches: 0,
                    };
                    for cell in cells {
                        let stand = survey.next_stand(cell, spot.origin).unwrap();
                        for dx in -1..=1 {
                            for dz in -1..=1 {
                                assert!(
                                    region_visible(
                                        cell,
                                        WorldTile {
                                            x: stand.x + dx,
                                            z: stand.z + dz,
                                            level: stand.level,
                                        }
                                    ),
                                    "radius-one arrival must cover NPC {id}'s whole cell"
                                );
                            }
                        }
                    }
                    checked += 1;
                }
            }
            assert!(checked > 0, "content coverage must include NPC {id}");
            eprintln!("checked {checked} radius-one-safe content envelopes for NPC {id}");
        }
    }

    #[test]
    fn fishing_guild_ardougne_approach_picks_a_legal_stand() {
        let Some(_pack) =
            crate::ctx::test_support::local_asset_path("NAV_PACK", "", "real 289 nav pack")
        else {
            return;
        };
        let catalog = real_catalog();
        let (method, index, spot) = fishing_guild_spot(&catalog);
        let bounds = movement_bounds(spot).expect("guild movement");
        let cell = observation_cells(bounds)
            .next()
            .expect("guild observation cell");
        let clamped = observation_stand(cell, ARDOUGNE_START);
        assert_eq!(
            clamped, BLOCKED_GUILD_CLAMP,
            "Ardougne still clamps onto the blocked south-east observation corner"
        );
        let world = real_nav_world();
        let lease = test_real_nav_lease();
        assert!(
            !area_arrival_ok(&world.collision, clamped),
            "{clamped:?} must stay an empty Area r=1 goal on the reference pack"
        );
        let area = guild_work_area();
        assert!(fishing_spot_eligible(spot, area));
        let selected = select_unloaded_fishing(
            &catalog,
            method,
            index,
            area,
            ARDOUGNE_START,
            Some(lease.collision()),
        );
        let target = selected
            .target
            .expect("Fishing Guild must supply an observation stand from Ardougne");
        assert_eq!(target.class, PlacementClass::Unloaded);
        assert_ne!(
            target.plan.tile, BLOCKED_GUILD_CLAMP,
            "initial approach must not walk the blocked clamp"
        );
        let neighbor = WorldTile {
            x: 2612,
            z: 3412,
            level: 0,
        };
        assert!(
            area_arrival_ok(&world.collision, neighbor),
            "{neighbor:?} must stay a legal Area r=1 stand one tile from the clamp"
        );
        assert_eq!(
            target.plan.tile, neighbor,
            "initial approach must pick the legal stand nearest the blocked clamp, not the player-tied west-edge scan winner"
        );
        assert_eq!(
            distance(clamped, target.plan.tile),
            1,
            "selected stand {:?} is not nearest the clamp {clamped:?}",
            target.plan.tile
        );
        assert!(
            area_arrival_ok(&world.collision, target.plan.tile),
            "selected stand {:?} must have a radius-one Area goal",
            target.plan.tile
        );
        let goals = area_goals(&world.collision, target.plan.tile);
        assert!(
            !goals.is_empty(),
            "Area r=1 around {:?} must list standable goals",
            target.plan.tile
        );
        let mut state = nav::world_state::WorldState::empty().with_map_members(true);
        state.stats = (0..=20).map(|skill| (skill, 99)).collect();
        let routed = nav::router::find_first_with(
            &world.collision,
            &world.graph,
            ARDOUGNE_START,
            &goals,
            nav::router::FindOptions::default(),
            &state,
        );
        let route = routed.route().unwrap_or_else(|error| {
            panic!(
                "Ardougne to {:?} Area goals must route: {error:?}",
                target.plan.tile
            )
        });
        eprintln!(
            "Fishing Guild Ardougne stand {:?} routed to {:?}",
            target.plan.tile, route.dest
        );
    }

    #[test]
    fn fishing_approach_keeps_a_legal_clamped_stand() {
        let Some(_pack) =
            crate::ctx::test_support::local_asset_path("NAV_PACK", "", "real 289 nav pack")
        else {
            return;
        };
        let catalog = real_catalog();
        let (method, index, spot) = fishing_guild_spot(&catalog);
        let bounds = movement_bounds(spot).expect("guild movement");
        let cell = observation_cells(bounds)
            .next()
            .expect("guild observation cell");
        let world = real_nav_world();
        let lease = test_real_nav_lease();
        let area = guild_work_area();
        let mut legal_here = None;
        let mid_x = (cell.min_x + cell.max_x) / 2;
        let mid_z = (cell.min_z + cell.max_z) / 2;
        for here in [
            WorldTile {
                x: mid_x,
                z: cell.max_z + OBSERVATION_RADIUS + 8,
                level: 0,
            },
            WorldTile {
                x: cell.min_x - OBSERVATION_RADIUS - 8,
                z: mid_z,
                level: 0,
            },
            WorldTile {
                x: mid_x,
                z: cell.min_z - OBSERVATION_RADIUS - 8,
                level: 0,
            },
            WorldTile {
                x: cell.max_x + OBSERVATION_RADIUS + 8,
                z: mid_z,
                level: 0,
            },
        ] {
            let clamped = observation_stand(cell, here);
            if area_arrival_ok(&world.collision, clamped) {
                legal_here = Some((here, clamped));
                break;
            }
        }
        let (here, clamped) = legal_here.expect(
            "at least one side of the Fishing Guild observation rectangle must clamp onto a legal stand",
        );
        let selected =
            select_unloaded_fishing(&catalog, method, index, area, here, Some(lease.collision()));
        let target = selected
            .target
            .expect("a legal clamp must remain an unloaded observation stand");
        assert_eq!(target.class, PlacementClass::Unloaded);
        assert_eq!(
            target.plan.tile, clamped,
            "already-legal clamps must keep the player-nearest observation stand"
        );
    }

    #[test]
    fn fishing_approach_fails_when_observation_rectangle_has_no_arrival() {
        let catalog = real_catalog();
        let method = catalog
            .method("fishing.saltfish.op3")
            .expect("Draynor bait");
        let index = method_index(&catalog, method);
        let spot = complete_spots(method)
            .unwrap()
            .iter()
            .find(|spot| {
                known_resource_target(method, spot.entity)
                    && catalog.access(method, spot).ok() == Some(Truth::True)
                    && movement_bounds(spot).is_some_and(|bounds| {
                        bounds.max_x - bounds.min_x <= 8 && bounds.max_z - bounds.min_z <= 8
                    })
            })
            .expect("a compact saltfish envelope");
        let bounds = movement_bounds(spot).unwrap();
        let here = WorldTile {
            x: bounds.max_x + 40,
            z: bounds.min_z - 40,
            level: bounds.level,
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: spot.origin,
            radius: 1,
        };
        let mut blocked = Vec::new();
        for eligible in complete_spots(method).unwrap().iter().filter(|candidate| {
            fishing_spot_eligible(candidate, area)
                && known_resource_target(method, candidate.entity)
        }) {
            let Some(eligible_bounds) = movement_bounds(eligible) else {
                continue;
            };
            for cell in observation_cells(eligible_bounds) {
                let pad = OBSERVATION_RADIUS + i32::from(RESOURCE_APPROACH_RADIUS);
                let min_x = cell.max_x - pad;
                let max_x = cell.min_x + pad;
                let min_z = cell.max_z - pad;
                let max_z = cell.min_z + pad;
                for x in min_x..=max_x {
                    for z in min_z..=max_z {
                        blocked.push(WorldTile {
                            x,
                            z,
                            level: cell.level,
                        });
                    }
                }
            }
        }
        let collision = synthetic_collision(
            fishing_cover(method, area)
                .into_iter()
                .chain([here, spot.origin]),
            blocked,
        );
        let lease = test_nav_lease_for_collision(&collision);
        let settings = GathererSettings {
            skill: "Fishing".into(),
            fishing_method: method.id.0.to_string(),
            location: "Custom".into(),
            custom_tile: Some(spot.origin),
            radius: area.radius,
            ..GathererSettings::default()
        };
        let selected = select(
            &catalog,
            &[index],
            &settings,
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision: Some(lease.collision()),
                world: &scene_around(here),
                locs: &[],
                npcs: &[],
                here,
                now: 1,
                skill_stat: 10,
            },
        );
        assert!(selected.target.is_none());
        assert_eq!(selected.outcome, Selection::NoStand);
    }

    #[test]
    fn fishing_stand_search_does_not_repeat_on_waiting_ticks() {
        let Some(_pack) =
            crate::ctx::test_support::local_asset_path("NAV_PACK", "", "real 289 nav pack")
        else {
            return;
        };
        use api::obj_names::ItemDefView;
        use api::quest_progress::EvidenceStamp;
        use api::selected::RunKey;
        use api::snapshot::{
            GameSnapshot, ItemActionFamily, ItemContainer, ItemView, SnapshotView, StatView,
        };

        let catalog = real_catalog();
        let (method, index, _spot) = fishing_guild_spot(&catalog);
        let lease = test_real_nav_lease();
        let area = guild_work_area();
        let settings = guild_fishing_settings(method, area);
        let select_now = |fishing: &mut FishingSurvey, now: u64| {
            select(
                &catalog,
                &[index],
                &settings,
                area,
                &[AvoidedTile::EMPTY; MAX_AVOID],
                SelectionObservation {
                    snapshot: None,
                    walk_danger: Some(&crate::native::AllowWalkDanger),
                    fishing,
                    collision: Some(lease.collision()),
                    world: &scene_around(ARDOUGNE_START),
                    locs: &[],
                    npcs: &[],
                    here: ARDOUGNE_START,
                    now,
                    skill_stat: 68,
                },
            )
        };
        let mut survey = FishingSurvey::default();
        take_fishing_stand_searches();
        let mut chosen = 0u32;
        for now in 1..=u64::from(MAX_FISHING_APPROACHES)
            .saturating_mul(8)
            .saturating_add(2)
        {
            if select_now(&mut survey, now).target.is_none() {
                break;
            }
            chosen = chosen.saturating_add(1);
        }
        assert!(
            chosen > 0,
            "Ardougne Guild approach must choose a stand before waiting"
        );
        let approach_searches = take_fishing_stand_searches();
        assert!(
            approach_searches > 0,
            "choosing a blocked-clamp stand must run the rectangle search"
        );

        for now in 1..=8 {
            assert!(
                select_now(&mut survey, 100 + now).target.is_none(),
                "exhausted approaches must not choose another stand on waiting tick {now}"
            );
        }
        let waiting_searches = take_fishing_stand_searches();
        eprintln!(
            "fishing stand searches: {approach_searches} while choosing {chosen} stands, {waiting_searches} across 8 waiting ticks"
        );
        assert_eq!(
            waiting_searches, 0,
            "waiting ticks must not repeat the rectangle search; a per-tick search would fail this"
        );

        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_world(WorldStateView {
            map_base_x: ARDOUGNE_START.x.saturating_sub(52),
            map_base_z: ARDOUGNE_START.z.saturating_sub(52),
            level: ARDOUGNE_START.level,
            members: true,
            ..WorldStateView::default()
        });
        snapshot.seed_stats(vec![StatView {
            index: 10,
            name: "fishing".into(),
            effective: 68,
            base: 68,
            xp: 0,
            used: true,
        }]);
        let tool_id = known_rows(&method.tools)
            .first()
            .map(|tool| tool.item)
            .expect("guild fishing method has a tool");
        snapshot.seed_inventory(
            vec![ItemView {
                def: ItemDefView {
                    id: tool_id,
                    name: Some("tool".into()),
                    stackable: false,
                    members: false,
                    base_value: 0,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: -1,
            }],
            28,
        );
        snapshot.seed_equipment(vec![]);
        snapshot.seed_npcs(vec![]);
        snapshot.seed_locs(vec![]);
        snapshot.seed_local_player(crate::quester::families::tests::local_player(
            ARDOUGNE_START,
        ));
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let view = SnapshotView::new(Some(&snapshot), stamp);
        assert_eq!(
            super::super::supply::method_ready(view, method, true),
            Ok(true),
            "quest snapshot must be ready so the unloaded pass is reachable"
        );
        take_fishing_stand_searches();
        let mut quest_survey = FishingSurvey::default();
        for now in 1..=8 {
            let selected = select_for_quest(
                &catalog,
                &[index],
                &settings,
                area,
                SelectionObservation {
                    snapshot: None,
                    walk_danger: Some(&crate::native::AllowWalkDanger),
                    fishing: &mut quest_survey,
                    collision: Some(lease.collision()),
                    world: &scene_around(ARDOUGNE_START),
                    locs: &[],
                    npcs: &[],
                    here: ARDOUGNE_START,
                    now,
                    skill_stat: 68,
                },
                view,
            );
            assert!(
                selected
                    .target
                    .as_ref()
                    .is_none_or(|target| target.class != PlacementClass::Unloaded),
                "quest gather must not keep an unloaded stand on waiting tick {now}"
            );
        }
        let quest_searches = take_fishing_stand_searches();
        eprintln!("quest gather rectangle searches across 8 waiting ticks: {quest_searches}");
        assert_eq!(
            quest_searches, 0,
            "quest gather must not compute a discarded observation stand"
        );
    }

    #[test]
    fn fishing_npc_bounds_have_legal_initial_stands_on_every_side() {
        let Some(_pack) =
            crate::ctx::test_support::local_asset_path("NAV_PACK", "", "real 289 nav pack")
        else {
            return;
        };
        let catalog = real_catalog();
        let world = real_nav_world();
        let lease = test_real_nav_lease();
        let mut seen = std::collections::HashSet::new();
        let mut previously_blocked = Vec::new();
        let mut checked = 0usize;
        for method in catalog
            .methods()
            .iter()
            .filter(|method| method.skill == GatherSkill::Fishing)
        {
            for spot in complete_spots(method).unwrap_or(&[]) {
                if !known_resource_target(method, spot.entity)
                    || catalog.access(method, spot).ok() != Some(Truth::True)
                {
                    continue;
                }
                let Some(bounds) = movement_bounds(spot) else {
                    continue;
                };
                let EntityId::Npc(npc) = spot.entity else {
                    continue;
                };
                let key = (
                    npc,
                    bounds.min_x,
                    bounds.min_z,
                    bounds.max_x,
                    bounds.max_z,
                    bounds.level,
                );
                if !seen.insert(key) {
                    continue;
                }
                let area = WorkArea {
                    mode: super::super::area::AreaMode::Custom,
                    anchor: spot.origin,
                    radius: 64,
                };
                for cell in observation_cells(bounds) {
                    let min_x = cell.max_x - OBSERVATION_RADIUS;
                    let max_x = cell.min_x + OBSERVATION_RADIUS;
                    let min_z = cell.max_z - OBSERVATION_RADIUS;
                    let max_z = cell.min_z + OBSERVATION_RADIUS;
                    if min_x > max_x || min_z > max_z {
                        continue;
                    }
                    let mid_x = (min_x + max_x) / 2;
                    let mid_z = (min_z + max_z) / 2;
                    let samples = [
                        ("east", max_x + 8, mid_z),
                        ("west", min_x - 8, mid_z),
                        ("north", mid_x, max_z + 8),
                        ("south", mid_x, min_z - 8),
                        ("se", max_x + 8, min_z - 8),
                        ("sw", min_x - 8, min_z - 8),
                        ("ne", max_x + 8, max_z + 8),
                        ("nw", min_x - 8, max_z + 8),
                    ];
                    for (side, x, z) in samples {
                        let here = WorldTile {
                            x,
                            z,
                            level: bounds.level,
                        };
                        let clamped = observation_stand(cell, here);
                        let old_ok = area_arrival_ok(&world.collision, clamped);
                        let legal = legal_fishing_return_stand(
                            cell,
                            here,
                            clamped,
                            area,
                            Some(lease.collision()),
                            &mut DangerCheck::new(Some(&crate::native::AllowWalkDanger)),
                        );
                        let stand = legal.unwrap_or(clamped);
                        let new_ok = area_arrival_ok(&world.collision, stand);
                        checked += 1;
                        if !old_ok {
                            previously_blocked.push(format!(
                                "{:?} {} {} start=({},{}) clamp={:?} legal={:?} legal_ok={new_ok}",
                                spot.entity, method.id.0, side, here.x, here.z, clamped, legal
                            ));
                        }
                        assert!(
                            new_ok,
                            "initial stand {:?} for {:?} {} from ({},{}) must have Area r=1 arrival",
                            stand, spot.entity, side, here.x, here.z
                        );
                    }
                }
            }
        }
        assert!(checked > 0, "289 fishing NPC movements must be sampled");
        eprintln!(
            "fishing initial stands: {checked} samples, {} previously blocked clamps:\n{}",
            previously_blocked.len(),
            previously_blocked.join("\n")
        );
    }

    #[test]
    fn fishing_empty_views_expire_without_renewing_the_approach_budget() {
        let bounds = SceneRegionInput {
            min_x: 3000,
            max_x: 3047,
            min_z: 3000,
            max_z: 3008,
            level: 0,
        };
        let mut survey = FishingPlacementSurvey {
            method: 0,
            spot: 0,
            covered: 0,
            covered_more: Vec::new(),
            started: None,
            approaches: 3,
        };
        let here = WorldTile {
            x: 3014,
            z: 3004,
            level: 0,
        };
        survey.observe(bounds, &scene_around(here), here, 1);
        assert_eq!(survey.covered, 1);
        let later = WorldTile { x: 3035, ..here };
        survey.observe(
            bounds,
            &scene_around(later),
            later,
            OBSERVATION_WINDOW_TICKS + 2,
        );
        assert_eq!(
            survey.covered, 2,
            "stale empty views cannot prove the whole moving envelope absent"
        );
        assert_eq!(
            survey.approaches, 3,
            "expiry must not reopen an unbounded approach loop"
        );
    }

    #[test]
    fn a_live_lower_tier_beats_an_unloaded_higher_tier_and_tier_falls_back() {
        let catalog = real_catalog();
        let low = catalog.method("woodcutting.normal").expect("normal trees");
        let high = catalog.method("woodcutting.oak").expect("oak trees");
        let low_index = method_index(&catalog, low);
        let high_index = method_index(&catalog, high);
        let mut high_by_tile = HashMap::new();
        for candidate in complete_spots(high).expect("oak placements") {
            if known_resource_target(high, candidate.entity)
                && catalog.access(high, candidate).ok() == Some(Truth::True)
            {
                high_by_tile
                    .entry((
                        candidate.origin.x,
                        candidate.origin.z,
                        candidate.origin.level,
                    ))
                    .or_insert(candidate);
            }
        }
        let mut pair = None;
        'placements: for lower in complete_spots(low).expect("normal placements") {
            if !known_resource_target(low, lower.entity)
                || catalog.access(low, lower).ok() != Some(Truth::True)
            {
                continue;
            }
            for distance in 50..=64 {
                for (dx, dz) in [(distance, 0), (-distance, 0), (0, distance), (0, -distance)] {
                    let Some(x) = lower.origin.x.checked_add(dx) else {
                        continue;
                    };
                    let Some(z) = lower.origin.z.checked_add(dz) else {
                        continue;
                    };
                    if let Some(higher) = high_by_tile.get(&(x, z, lower.origin.level)).copied() {
                        pair = Some((lower, higher, dx, dz));
                        break 'placements;
                    }
                }
            }
        }
        let (lower, higher, dx, dz) = pair.expect("near-edge normal/oak placements in content");
        let mut world = scene_around(lower.origin);
        if dx > 0 {
            world.map_base_x = lower.origin.x.saturating_sub(84);
        } else if dx < 0 {
            world.map_base_x = lower.origin.x.saturating_sub(20);
        } else if dz > 0 {
            world.map_base_z = lower.origin.z.saturating_sub(84);
        } else {
            world.map_base_z = lower.origin.z.saturating_sub(20);
        }
        let methods = [low_index, high_index];
        let settings = GathererSettings::default();
        let area = WorkArea {
            mode: super::super::area::AreaMode::Start,
            anchor: lower.origin,
            radius: 64,
        };
        let here = lower.origin;
        let lower_id = match lower.entity {
            EntityId::Loc(id) => id,
            _ => panic!("woodcutting placement uses loc identity"),
        };
        let lower_live = [loc(lower_id, lower.origin)];
        let live_result = select(
            &catalog,
            &methods,
            &settings,
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision: None,
                world: &world,
                locs: &lower_live,
                npcs: &[],
                here,
                now: 10,
                skill_stat: 8,
            },
        );
        let selected = live_result.target.expect("live lower-tier resource");
        assert_eq!(selected.class, PlacementClass::Live);
        assert_eq!(selected.plan.entity, lower.entity);
        assert_eq!(selected.plan.tile, lower.origin);

        assert_eq!(higher.origin.x - lower.origin.x, dx);
        assert_eq!(higher.origin.z - lower.origin.z, dz);
        let fallback = select(
            &catalog,
            &methods,
            &settings,
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision: None,
                world: &world,
                locs: &[],
                npcs: &[],
                here,
                now: 10,
                skill_stat: 8,
            },
        );
        let selected = fallback.target.expect("highest tier unloaded fallback");
        assert_eq!(selected.class, PlacementClass::Unloaded);
        assert_eq!(selected.plan.entity, higher.entity);
    }

    #[test]
    fn depleted_placements_wait_for_the_content_respawn_bound() {
        let catalog = real_catalog();
        let method = catalog.method("woodcutting.oak").expect("oak method");
        let method_index = method_index(&catalog, method);
        let spot = complete_spots(method)
            .expect("oak placements")
            .iter()
            .find(|spot| {
                known_resource_target(method, spot.entity)
                    && catalog.access(method, spot).ok() == Some(Truth::True)
            })
            .expect("eligible oak placement");
        let depleted_id = known_targets(method)
            .expect("known oak targets")
            .iter()
            .find_map(|target| match target.entity {
                EntityId::Loc(id)
                    if target.class == TargetClass::Depleted
                        && matches!(target.respawn, Knowledge::Known(_)) =>
                {
                    Some(id)
                }
                _ => None,
            })
            .expect("oak stump id");
        let expected_wait = u64::from(respawn_max(method, spot));
        assert!(expected_wait > 0, "oak respawn scale comes from content");
        let area = WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: spot.origin,
            radius: 2,
        };
        let world = scene_around(spot.origin);
        let locs = [loc(depleted_id, spot.origin)];
        let settings = GathererSettings::default();
        let selected = select(
            &catalog,
            &[method_index],
            &settings,
            area,
            &[AvoidedTile::EMPTY; MAX_AVOID],
            SelectionObservation {
                snapshot: None,
                walk_danger: Some(&crate::native::AllowWalkDanger),
                fishing: &mut FishingSurvey::default(),
                collision: None,
                world: &world,
                locs: &locs,
                npcs: &[],
                here: spot.origin,
                now: 100,
                skill_stat: 8,
            },
        );
        assert!(selected.target.is_none());
        assert!(matches!(
            selected.outcome,
            Selection::Exhausted { wait_until, .. }
                if wait_until >= 100 + expected_wait
        ));
    }

    #[test]
    fn target_yield_contains_content_derived_incidental_mining_gems() {
        let catalog = real_catalog();
        let method = catalog.method("mining.coal").expect("coal method");
        let index = method_index(&catalog, method);
        let spot = complete_spots(method)
            .expect("complete coal placements")
            .iter()
            .find(|spot| {
                known_resource_target(method, spot.entity)
                    && catalog.access(method, spot).ok() == Some(Truth::True)
            })
            .expect("eligible coal placement");
        let plan = make_plan(
            &catalog,
            u16::try_from(index).expect("catalog method index fits"),
            method,
            spot,
            spot.origin,
            NO_NPC_INDEX,
            4,
        );
        for gem in catalog.incidental_gem_ids() {
            assert!(
                plan.products[..usize::from(plan.products_len)].contains(gem),
                "incidental gem product {gem} missing from target yield"
            );
        }
    }

    #[test]
    fn draynor_small_net_return_waits_out_a_shared_placement_exclusion() {
        use super::super::area::AreaMode;

        let catalog = real_catalog();
        let method = catalog
            .method("fishing.saltfish.op1")
            .expect("Draynor small-net fishing");
        let index = method_index(&catalog, method);
        let bank = WorldTile {
            x: 3092,
            z: 3243,
            level: 0,
        };
        let area = WorkArea {
            mode: AreaMode::Custom,
            anchor: WorldTile {
                x: 3086,
                z: 3229,
                level: 0,
            },
            radius: 12,
        };
        let settings = GathererSettings {
            skill: "Fishing".into(),
            fishing_method: method.id.0.to_string(),
            radius: area.radius,
            ..GathererSettings::default()
        };
        let spots: Vec<_> = complete_spots(method)
            .expect("complete small-net placements")
            .iter()
            .filter(|spot| {
                fishing_spot_eligible(spot, area)
                    && known_resource_target(method, spot.entity)
                    && catalog.access(method, spot).ok() == Some(Truth::True)
            })
            .collect();
        assert!(
            spots.len() >= 2,
            "Draynor small-net must offer both movement placements, got {}",
            spots.len()
        );
        let shared = movement_bounds(spots[0]).expect("eligible spots have movement");
        assert!(
            spots.iter().all(|spot| {
                movement_bounds(spot).is_some_and(|bounds| {
                    bounds.min_x == shared.min_x
                        && bounds.max_x == shared.max_x
                        && bounds.min_z == shared.min_z
                        && bounds.max_z == shared.max_z
                        && bounds.level == shared.level
                })
            }),
            "the Draynor small-net placements share one movement envelope"
        );
        let collision = synthetic_collision(
            fishing_cover(method, area)
                .into_iter()
                .chain([bank, area.anchor]),
            std::iter::empty::<WorldTile>(),
        );
        let lease = test_nav_lease_for_collision(&collision);
        let observation = |now, avoided: &[AvoidedTile; MAX_AVOID]| {
            resource_return_target(
                &catalog,
                &[index],
                &settings,
                area,
                ReturnObservation {
                    walk_danger: Some(&crate::native::AllowWalkDanger),
                    here: bank,
                    now,
                    skill_stat: 1,
                    avoided,
                    collision: Some(lease.collision()),
                },
            )
        };
        assert!(
            matches!(
                observation(117, &[AvoidedTile::EMPTY; MAX_AVOID]),
                ResourceReturn::Ready(_)
            ),
            "both placements are ready before any exclusion"
        );
        let mut avoided = [AvoidedTile::EMPTY; MAX_AVOID];
        avoided[0] = AvoidedTile {
            tile: WorldTile {
                x: 3085,
                z: 3231,
                level: 0,
            },
            until: 167,
        };
        match observation(117, &avoided) {
            ResourceReturn::ExcludedUntil { until } => assert_eq!(until, 167),
            other => panic!("temporary exclusion must wait, got {other:?}"),
        }
        let ResourceReturn::Ready(selected) = observation(167, &avoided) else {
            panic!("the shared exclusion must lift at its deadline");
        };
        let plan = selected.plan;
        assert_ne!(plan.tile, bank);
        assert!(
            spots
                .iter()
                .any(|spot| distance(plan.tile, spot.origin) <= i64::from(NPC_VIEW_RADIUS)),
            "after the deadline the return stand must observe a small-net placement"
        );
    }

    #[test]
    fn observation_candidates_deduplicate_before_the_typed_limit() {
        let bounds = SceneRegionInput {
            min_x: 0,
            max_x: 29,
            min_z: 0,
            max_z: 0,
            level: 0,
        };
        let goals = observation_goals(observation_cells(bounds), None, None).unwrap();
        assert_eq!(goals.as_slice().len(), 992);
        assert!(goals.as_slice().windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(
            observation_goals(std::iter::empty(), None, None).unwrap_err(),
            WalkGoalsError::Empty
        );
        let too_large = SceneRegionInput {
            max_x: OBSERVATION_WIDTH * 12 - 1,
            ..bounds
        };
        assert_eq!(
            observation_goals(observation_cells(too_large), None, None).unwrap_err(),
            WalkGoalsError::TooMany {
                count: WalkGoals::MAX + 1,
                limit: WalkGoals::MAX,
            }
        );
    }

    #[test]
    fn generated_fishing_movement_candidates_fit_the_reviewed_bound() {
        for revision in [
            client::io::ClientRevision::R274,
            client::io::ClientRevision::R289,
        ] {
            let data = api::game_data::for_revision(revision).unwrap();
            let catalog =
                api::selected::FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
                    .unwrap()
                    .join()
                    .unwrap()
                    .unwrap();
            let mut maximum = 0;
            let mut checked = 0;
            for method in catalog
                .methods()
                .iter()
                .filter(|method| method.skill == GatherSkill::Fishing)
            {
                for spot in complete_spots(method).unwrap_or(&[]) {
                    let Some(bounds) = movement_bounds(spot) else {
                        continue;
                    };
                    let goals = observation_goals(observation_cells(bounds), None, None).unwrap();
                    maximum = maximum.max(goals.as_slice().len());
                    checked += 1;
                    assert!(goals.as_slice().iter().all(|goal| {
                        observation_cells(bounds).any(|cell| region_visible(cell, *goal))
                    }));
                }
            }
            assert!(checked > 0);
            assert!(maximum <= 961, "reviewed movement census grew to {maximum}");
            assert!(WalkGoals::MAX >= maximum + 63);
            eprintln!("{revision:?}: {checked} movement rows, maximum {maximum} exact goals");
        }
    }

    #[test]
    fn catherby_candidates_route_from_the_guild_and_keep_the_island_start() {
        let Some(_pack) =
            crate::ctx::test_support::local_asset_path("NAV_PACK", "", "real 289 nav pack")
        else {
            return;
        };
        let world = real_nav_world();
        let lease = test_real_nav_lease();
        let catalog = real_catalog();
        let method = catalog.method("fishing.rarefish.op3").unwrap();
        let index = method_index(&catalog, method);
        let here = WorldTile {
            x: 2611,
            z: 3399,
            level: 0,
        };
        let area = WorkArea {
            mode: super::super::area::AreaMode::Site,
            anchor: WorldTile {
                x: 2840,
                z: 3436,
                level: 0,
            },
            radius: 32,
        };
        let selected =
            select_unloaded_fishing(&catalog, method, index, area, here, Some(lease.collision()))
                .target
                .expect("Catherby must provide an unloaded observation request");
        let goals = selected.observation_goals.as_ref().unwrap().as_slice();
        let old_goals = area_goals(&world.collision, selected.plan.tile);
        let mut state = nav::world_state::WorldState::empty().with_map_members(true);
        state.stats = (0..=20).map(|skill| (skill, 99)).collect();
        let old = nav::router::find_first_with(
            &world.collision,
            &world.graph,
            here,
            &old_goals,
            nav::router::FindOptions::default(),
            &state,
        );
        assert!(
            old.route().is_err(),
            "the old locally legal clamp must reproduce the disconnected-island failure"
        );
        let route = route_observation(
            world,
            here,
            selected.observation_goals.as_ref().unwrap(),
            &state,
        )
        .expect("one source-reachable observation goal is enough");
        assert!(!selected
            .observation_goals
            .as_ref()
            .unwrap()
            .preferred()
            .contains(&route.dest));
        assert!(goals.contains(&route.dest));
        assert!(route.ticks > 0.0);
        let island = *old_goals.first().expect("old clamp has legal island goals");
        assert!(
            goals.contains(&island),
            "never discard non-mainland components"
        );
        let island_route = route_observation(
            world,
            island,
            selected.observation_goals.as_ref().unwrap(),
            &state,
        )
        .expect("the player's island is a valid start");
        assert_eq!(island_route.dest, island);
        assert_eq!(island_route.ticks, 0.0);
        eprintln!(
            "Catherby: advisory={:?}, old_goals={}, candidates={}, guild_goal={:?}, guild_ticks={}, island={island:?}",
            selected.plan.tile, old_goals.len(), goals.len(), route.dest, route.ticks
        );

        let ResourceReturn::Ready(returned) = resource_return_target(
            &catalog,
            &[index],
            &guild_fishing_settings(method, area),
            area,
            ReturnObservation {
                walk_danger: Some(&crate::native::AllowWalkDanger),
                here,
                now: 1,
                skill_stat: 68,
                avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                collision: Some(lease.collision()),
            },
        ) else {
            panic!("bank return must retain the same multi-goal policy");
        };
        let return_route = route_observation(
            world,
            here,
            returned.observation_goals.as_ref().unwrap(),
            &state,
        );
        assert!(return_route.is_ok());
    }

    mod site_parity_tests {
        use super::super::*;
        use api::game_data::{GatherSiteOption, SelectedGameData};
        use api::gather_methods::{GatherCatalog, GatherSkill, SceneRegionInput};
        use api::selected::Knowledge;
        use api::snapshot::WorldTile;
        use serde::Serialize;
        use std::collections::BTreeSet;
        use std::path::{Path, PathBuf};
        use std::time::Instant;

        const LEGACY_BASELINE: &str = "e9b575be4";
        const DEFAULT_SITE_RADIUS: u16 = 12;
        const CATHERBY_REGRESSION_RADIUS: u16 = 32;
        const CATHERBY_GUILD_START: WorldTile = WorldTile {
            x: 2611,
            z: 3399,
            level: 0,
        };
        const CATHERBY_ISLAND_CLAMP: WorldTile = WorldTile {
            x: 2846,
            z: 3417,
            level: 0,
        };
        const CATHERBY_TUNA_SWORD_KEY: &str = "fishing.harpoon.tool_311.products_359_371";

        #[derive(Debug)]
        struct SiteOption {
            site_id: String,
            label: String,
            region: SceneRegionInput,
            key: String,
            expected_count: usize,
            declared_methods: Vec<String>,
            method_indices: Vec<usize>,
            method_facts: Vec<MethodRegionFact>,
            profile: String,
            anchor: WorldTile,
            radius: u16,
        }

        #[derive(Debug, Clone, Serialize)]
        struct MethodRegionFact {
            method: String,
            region_spots: usize,
            usable_resource_spots: usize,
            coverage_complete: bool,
            movement_evidence: Vec<MovementEvidence>,
            region_error: Option<String>,
        }

        #[derive(Debug, Clone, Serialize)]
        struct MovementEvidence {
            spot_id: u32,
            entity: String,
            origin: WorldTile,
            status: String,
            bounds: Option<RegionEvidence>,
        }

        #[derive(Debug, Clone, Serialize)]
        struct SiteAudit {
            site_id: String,
            label: String,
            region: RegionEvidence,
            key: String,
            expected_count: usize,
            selectable: bool,
            option_gap: Option<String>,
            declared_methods: Vec<String>,
            supported_methods_in_region: Vec<String>,
            method_facts: Vec<MethodRegionFact>,
            profile: String,
            area_radius: u16,
            runtime_anchor: Option<WorldTile>,
            status: String,
        }

        #[derive(Debug, Clone)]
        struct BankAccess {
            name: String,
            stand_tile: WorldTile,
            access_tile: WorldTile,
        }

        #[derive(Debug, Clone, Serialize)]
        struct BankEvidence {
            name: String,
            stand_tile: WorldTile,
            access_tile: WorldTile,
        }

        #[derive(Debug, Clone)]
        struct StartCase {
            kind: &'static str,
            tile: WorldTile,
            bank: Option<BankEvidence>,
        }

        #[derive(Debug, Clone, Copy, Serialize)]
        struct RegionEvidence {
            min_x: i32,
            min_z: i32,
            max_x: i32,
            max_z: i32,
            level: i32,
        }

        impl From<SceneRegionInput> for RegionEvidence {
            fn from(bounds: SceneRegionInput) -> Self {
                Self {
                    min_x: bounds.min_x,
                    min_z: bounds.min_z,
                    max_x: bounds.max_x,
                    max_z: bounds.max_z,
                    level: bounds.level,
                }
            }
        }

        #[derive(Debug, Clone, Serialize)]
        struct RouteEvidence {
            status: String,
            error: Option<String>,
            tier: Option<String>,
            destination: Option<WorldTile>,
            cost_ticks: Option<f64>,
            settled: usize,
            elapsed_us: u128,
            proof: String,
            preferred_status: String,
            preferred_error: Option<String>,
            preferred_settled: usize,
            preferred_elapsed_us: u128,
            preferred_proof: String,
            fallback_status: Option<String>,
            fallback_error: Option<String>,
            fallback_only_status: Option<String>,
            fallback_only_error: Option<String>,
            fallback_only_settled: usize,
            fallback_only_elapsed_us: u128,
            fallback_only_proof: Option<String>,
        }

        #[derive(Debug, Clone, Serialize)]
        struct CensusCase {
            site_id: String,
            site_label: String,
            region: RegionEvidence,
            key: String,
            expected_count: usize,
            declared_methods: Vec<String>,
            supported_methods_in_region: Vec<String>,
            profile: String,
            area_anchor: WorldTile,
            area_radius: u16,
            start_kind: String,
            start_tile: WorldTile,
            bank: Option<BankEvidence>,
            mode: String,
            selection_status: String,
            selection_detail: Option<String>,
            selected_method: Option<String>,
            ranked_stand: Option<WorldTile>,
            old_goal_count: usize,
            preferred_goal_count: usize,
            fallback_goal_count: usize,
            goal_sets_match: Option<bool>,
            before: Option<RouteEvidence>,
            after: Option<RouteEvidence>,
            classification: String,
            reason: Option<String>,
        }

        #[derive(Debug, Clone, Serialize)]
        struct SiteExclusion {
            site_id: String,
            label: String,
            key: String,
            expected_count: usize,
            methods: Vec<String>,
            reason: String,
        }

        #[derive(Debug, Clone, Serialize)]
        struct CoverageGap {
            site_id: Option<String>,
            key: Option<String>,
            method: Option<String>,
            reason: String,
        }
        struct CensusReport<'a> {
            cases: &'a [CensusCase],
            site_audit: &'a [SiteAudit],
            exclusions: &'a [SiteExclusion],
            gaps: &'a [CoverageGap],
            totals: &'a serde_json::Value,
            summary: &'a serde_json::Value,
            metadata: &'a serde_json::Value,
        }

        #[derive(Debug, Clone, Copy)]
        enum SelectionMode {
            Approach,
            BankReturn,
        }

        impl SelectionMode {
            fn label(self) -> &'static str {
                match self {
                    Self::Approach => "unloaded_approach",
                    Self::BankReturn => "bank_return",
                }
            }
        }

        struct TargetPick {
            selected: Option<SelectedTarget>,
            status: String,
            detail: Option<String>,
            defect: bool,
        }

        fn site_options(
            selected: &SelectedGameData,
            catalog: &GatherCatalog,
            gaps: &mut Vec<CoverageGap>,
            exclusions: &mut Vec<SiteExclusion>,
            audit: &mut Vec<SiteAudit>,
        ) -> (Vec<SiteOption>, usize, usize) {
            let sites: Vec<&GatherSiteOption> = selected.gather_sites_for("fishing").collect();
            let mut options = Vec::new();
            let mut published_keys = 0usize;
            for site in &sites {
                if site.keys.is_empty() {
                    gaps.push(CoverageGap {
                        site_id: Some(site.id.clone()),
                        key: None,
                        method: None,
                        reason: "published fishing site has no selectable keys".to_owned(),
                    });
                    audit.push(SiteAudit {
                        site_id: site.id.clone(),
                        label: site.label.clone(),
                        region: site.region.into(),
                        key: "<no keys>".to_owned(),
                        expected_count: 0,
                        selectable: false,
                        option_gap: None,
                        declared_methods: Vec::new(),
                        supported_methods_in_region: Vec::new(),
                        method_facts: Vec::new(),
                        profile: "default".to_owned(),
                        area_radius: DEFAULT_SITE_RADIUS,
                        runtime_anchor: None,
                        status: "site_without_keys".to_owned(),
                    });
                    continue;
                }
                for site_key in &site.keys {
                    published_keys += 1;
                    let option = selected.gather_option("fishing", &site_key.key);
                    let declared_methods = option.map_or_else(Vec::new, |row| row.methods.clone());
                    let Some(option) = option else {
                        exclusions.push(SiteExclusion {
                            site_id: site.id.clone(),
                            label: site.label.clone(),
                            key: site_key.key.clone(),
                            expected_count: site_key.count,
                            methods: declared_methods.clone(),
                            reason: "published site key has no selected fishing resource option"
                                .to_owned(),
                        });
                        gaps.push(CoverageGap {
                            site_id: Some(site.id.clone()),
                            key: Some(site_key.key.clone()),
                            method: None,
                            reason: "site key does not resolve through gather_option(fishing, key)"
                                .to_owned(),
                        });
                        audit.push(SiteAudit {
                            site_id: site.id.clone(),
                            label: site.label.clone(),
                            region: site.region.into(),
                            key: site_key.key.clone(),
                            expected_count: site_key.count,
                            selectable: false,
                            option_gap: Some("missing-resource-option".to_owned()),
                            declared_methods,
                            supported_methods_in_region: Vec::new(),
                            method_facts: Vec::new(),
                            profile: "default".to_owned(),
                            area_radius: DEFAULT_SITE_RADIUS,
                            runtime_anchor: None,
                            status: "unresolved_site_key".to_owned(),
                        });
                        continue;
                    };
                    let mut method_indices = Vec::new();
                    let mut method_facts = Vec::new();
                    for method_id in &option.methods {
                        let Ok(method) = catalog.method(method_id) else {
                            if option.selectable {
                                gaps.push(CoverageGap {
                            site_id: Some(site.id.clone()),
                            key: Some(site_key.key.clone()),
                            method: Some(method_id.clone()),
                            reason:
                                "resource option method is absent from the prepared fishing catalog"
                                    .to_owned(),
                        });
                            }
                            continue;
                        };
                        if method.skill != GatherSkill::Fishing {
                            if option.selectable {
                                gaps.push(CoverageGap {
                                    site_id: Some(site.id.clone()),
                                    key: Some(site_key.key.clone()),
                                    method: Some(method_id.clone()),
                                    reason: "site option resolves to a non-fishing method"
                                        .to_owned(),
                                });
                            }
                            continue;
                        }
                        let Some(index) = catalog
                            .methods()
                            .iter()
                            .position(|candidate| std::ptr::eq(candidate, method))
                        else {
                            if option.selectable {
                                gaps.push(CoverageGap {
                                    site_id: Some(site.id.clone()),
                                    key: Some(site_key.key.clone()),
                                    method: Some(method_id.clone()),
                                    reason: "resolved fishing method has no catalog index"
                                        .to_owned(),
                                });
                            }
                            continue;
                        };
                        if !method_indices.contains(&index) {
                            method_indices.push(index);
                        }
                        let placement_and_target_metadata_complete =
                            matches!(&method.spots, Knowledge::Known(_))
                                && matches!(&method.targets, Knowledge::Known(_));
                        let mut region_spots = 0usize;
                        let mut usable_resource_spots = 0usize;
                        let mut movement_evidence = Vec::new();
                        let mut region_error = None;
                        match catalog.spots(method, &site.region) {
                            Ok(spots) => {
                                for spot in spots {
                                    region_spots += 1;
                                    let (status, bounds) = match &spot.movement {
                                        Knowledge::Known(Some(bounds)) => {
                                            ("known", Some((*bounds).into()))
                                        }
                                        Knowledge::Known(None) => ("known_absent", None),
                                        Knowledge::Partial { known, .. } => (
                                            "partial",
                                            known.as_ref().map(|bounds| (*bounds).into()),
                                        ),
                                        Knowledge::Unknown(_) => ("unknown", None),
                                    };
                                    movement_evidence.push(MovementEvidence {
                                        spot_id: spot.id.0,
                                        entity: format!("{:?}", spot.entity),
                                        origin: spot.origin,
                                        status: status.to_owned(),
                                        bounds,
                                    });
                                    if crate::gatherer::widen::usable_candidate(
                                        catalog,
                                        method,
                                        spot,
                                        &[],
                                        1,
                                    ) {
                                        usable_resource_spots += 1;
                                    }
                                }
                            }
                            Err(error) => region_error = Some(format!("{error:?}")),
                        }
                        let coverage_complete = placement_and_target_metadata_complete
                            && region_error.is_none()
                            && movement_evidence.iter().all(|movement| {
                                movement.status == "known" || movement.status == "known_absent"
                            });
                        if option.selectable && !coverage_complete {
                            gaps.push(CoverageGap {
                                site_id: Some(site.id.clone()),
                                key: Some(site_key.key.clone()),
                                method: Some(method_id.clone()),
                                reason:
                                    "method placement, target, or movement metadata is incomplete"
                                        .to_owned(),
                            });
                        }
                        method_facts.push(MethodRegionFact {
                            method: method_id.clone(),
                            region_spots,
                            usable_resource_spots,
                            coverage_complete,
                            movement_evidence,
                            region_error,
                        });
                    }
                    if !option.selectable {
                        exclusions.push(SiteExclusion {
                            site_id: site.id.clone(),
                            label: site.label.clone(),
                            key: site_key.key.clone(),
                            expected_count: site_key.count,
                            methods: option.methods.clone(),
                            reason: format!(
                                "resource option is not selectable: {}",
                                option.gap.as_deref().unwrap_or("no gap code")
                            ),
                        });
                    }
                    let supported_methods_in_region = method_facts
                        .iter()
                        .filter(|fact| fact.usable_resource_spots > 0)
                        .map(|fact| fact.method.clone())
                        .collect::<Vec<_>>();
                    let mut profiles = vec![("default", DEFAULT_SITE_RADIUS)];
                    if site.id == "fishing.catherby"
                        && site_key.key == CATHERBY_TUNA_SWORD_KEY
                        && option.selectable
                    {
                        profiles.push(("catherby32_regression", CATHERBY_REGRESSION_RADIUS));
                    }
                    if !option.selectable {
                        for (profile, radius) in profiles {
                            audit.push(SiteAudit {
                                site_id: site.id.clone(),
                                label: site.label.clone(),
                                region: site.region.into(),
                                key: site_key.key.clone(),
                                expected_count: site_key.count,
                                selectable: false,
                                option_gap: option.gap.clone(),
                                declared_methods: option.methods.clone(),
                                supported_methods_in_region: supported_methods_in_region.clone(),
                                method_facts: method_facts.clone(),
                                profile: profile.to_owned(),
                                area_radius: radius,
                                runtime_anchor: None,
                                status: "not_selectable".to_owned(),
                            });
                        }
                        continue;
                    }
                    if method_indices.is_empty() {
                        gaps.push(CoverageGap {
                            site_id: Some(site.id.clone()),
                            key: Some(site_key.key.clone()),
                            method: None,
                            reason: "selectable site key resolved to no method indices".to_owned(),
                        });
                        for (profile, radius) in profiles {
                            audit.push(SiteAudit {
                                site_id: site.id.clone(),
                                label: site.label.clone(),
                                region: site.region.into(),
                                key: site_key.key.clone(),
                                expected_count: site_key.count,
                                selectable: true,
                                option_gap: option.gap.clone(),
                                declared_methods: option.methods.clone(),
                                supported_methods_in_region: supported_methods_in_region.clone(),
                                method_facts: method_facts.clone(),
                                profile: profile.to_owned(),
                                area_radius: radius,
                                runtime_anchor: None,
                                status: "no_method_indices".to_owned(),
                            });
                        }
                        continue;
                    }
                    let anchor = crate::gatherer::widen::site_anchor(
                        catalog,
                        &method_indices,
                        &site.region,
                        &[],
                        1,
                    );
                    if anchor.is_none() {
                        gaps.push(CoverageGap {
                    site_id: Some(site.id.clone()),
                    key: Some(site_key.key.clone()),
                    method: None,
                    reason:
                        "runtime site_anchor found no accessible resource in the published region"
                            .to_owned(),
                });
                    }
                    for (profile, radius) in profiles {
                        audit.push(SiteAudit {
                            site_id: site.id.clone(),
                            label: site.label.clone(),
                            region: site.region.into(),
                            key: site_key.key.clone(),
                            expected_count: site_key.count,
                            selectable: true,
                            option_gap: option.gap.clone(),
                            declared_methods: option.methods.clone(),
                            supported_methods_in_region: supported_methods_in_region.clone(),
                            method_facts: method_facts.clone(),
                            profile: profile.to_owned(),
                            area_radius: radius,
                            runtime_anchor: anchor,
                            status: if anchor.is_some() {
                                "anchored".to_owned()
                            } else {
                                "no_runtime_anchor".to_owned()
                            },
                        });
                        if let Some(anchor) = anchor {
                            options.push(SiteOption {
                                site_id: site.id.clone(),
                                label: site.label.clone(),
                                region: site.region,
                                key: site_key.key.clone(),
                                expected_count: site_key.count,
                                declared_methods: option.methods.clone(),
                                method_indices: method_indices.clone(),
                                method_facts: method_facts.clone(),
                                profile: profile.to_owned(),
                                anchor,
                                radius,
                            });
                        }
                    }
                }
            }
            (options, sites.len(), published_keys)
        }

        #[derive(Debug)]
        struct SiteCase {
            site_id: String,
            label: String,
            region: SceneRegionInput,
            key: String,
            expected_count: usize,
            declared_methods: Vec<String>,
            method_indices: Vec<usize>,
            selected_method: Option<String>,
            profile: String,
            anchor: WorldTile,
            radius: u16,
        }

        fn bank_accesses(
            world: &nav::world::NavWorld,
            selected: &SelectedGameData,
        ) -> Vec<BankAccess> {
            let placements = selected
                .bank_placements()
                .expect("selected bank placement facts");
            let facts = nav::named_banks::resolve(
                api::named_banks::BANK_CATALOG,
                &placements.rows,
                |tile| world.collision.standable(tile),
            );
            facts
                .banks()
                .iter()
                .filter(|bank| bank.routable)
                .map(|bank| BankAccess {
                    name: bank.name.to_owned(),
                    stand_tile: bank
                        .definition
                        .map_or(bank.tile, |definition| definition.tile),
                    access_tile: bank.tile,
                })
                .collect()
        }

        fn chebyshev(left: WorldTile, right: WorldTile) -> u32 {
            left.x.abs_diff(right.x).max(left.z.abs_diff(right.z))
        }

        fn nearest_and_farthest_bank(
            accesses: &[BankAccess],
            anchor: WorldTile,
        ) -> (Option<BankAccess>, Option<BankAccess>) {
            let mut same_level: Vec<&BankAccess> = accesses
                .iter()
                .filter(|bank| bank.access_tile.level == anchor.level)
                .collect();
            same_level.sort_by_key(|bank| {
                (
                    chebyshev(bank.access_tile, anchor),
                    bank.name.as_str(),
                    bank.access_tile.x,
                    bank.access_tile.z,
                )
            });
            let nearest = same_level.first().map(|bank| (**bank).clone());
            let farthest = nearest
                .as_ref()
                .and_then(|nearest| {
                    same_level
                        .iter()
                        .rev()
                        .copied()
                        .find(|bank| bank.access_tile != nearest.access_tile)
                })
                .or_else(|| same_level.last().copied())
                .map(|bank| (*bank).clone());
            (nearest, farthest)
        }

        fn nearest_inside_area_stand(
            world: &nav::world::NavWorld,
            area: WorkArea,
        ) -> Option<WorldTile> {
            let region = area.region();
            (region.min_x..=region.max_x)
                .flat_map(|x| {
                    (region.min_z..=region.max_z).map(move |z| WorldTile {
                        x,
                        z,
                        level: region.level,
                    })
                })
                .filter(|tile| world.collision.standable(*tile))
                .min_by_key(|tile| (chebyshev(*tile, area.anchor), tile.x, tile.z))
        }

        fn bank_start(kind: &'static str, bank: BankAccess) -> StartCase {
            StartCase {
                kind,
                tile: bank.access_tile,
                bank: Some(BankEvidence {
                    name: bank.name,
                    stand_tile: bank.stand_tile,
                    access_tile: bank.access_tile,
                }),
            }
        }

        fn route_status(result: &Result<nav::router::Route, nav::router::RouteError>) -> String {
            match result {
                Ok(_) => "Routed".to_owned(),
                Err(nav::router::RouteError::NoPath) => "NoPath".to_owned(),
                Err(nav::router::RouteError::BudgetExhausted) => "BudgetExhausted".to_owned(),
            }
        }

        fn route_evidence(
            result: Result<nav::router::Route, nav::router::RouteError>,
            search_kind: &str,
            settled: usize,
            elapsed_us: u128,
            proof: String,
        ) -> RouteEvidence {
            let status = route_status(&result);
            let error = result.as_ref().err().map(|error| format!("{error:?}"));
            let route = result.ok();
            RouteEvidence {
                status: status.clone(),
                error: error.clone(),
                tier: Some(search_kind.to_owned()),
                destination: route.as_ref().map(|route| route.dest),
                cost_ticks: route.as_ref().map(|route| route.ticks),
                settled,
                elapsed_us,
                proof: proof.clone(),
                preferred_status: status,
                preferred_error: error,
                preferred_settled: settled,
                preferred_elapsed_us: elapsed_us,
                preferred_proof: proof,
                fallback_status: None,
                fallback_error: None,
                fallback_only_status: None,
                fallback_only_error: None,
                fallback_only_settled: 0,
                fallback_only_elapsed_us: 0,
                fallback_only_proof: None,
            }
        }

        fn legacy_route(
            world: &nav::world::NavWorld,
            from: WorldTile,
            goals: &[WorldTile],
            state: &nav::world_state::WorldState,
        ) -> RouteEvidence {
            let started = Instant::now();
            let search = nav::router::find_first_with(
                &world.collision,
                &world.graph,
                from,
                goals,
                nav::router::FindOptions::default(),
                state,
            );
            let settled = search.settled();
            let proof = format!("{:?}", search.proof());
            route_evidence(
                search.into_route(),
                "e9_ranked_radius_one",
                settled,
                started.elapsed().as_micros(),
                proof,
            )
        }

        fn tiered_route(
            world: &nav::world::NavWorld,
            from: WorldTile,
            goals: &WalkGoals,
            state: &nav::world_state::WorldState,
        ) -> RouteEvidence {
            let started = Instant::now();
            let search = nav::router::find_first_with(
                &world.collision,
                &world.graph,
                from,
                goals.preferred(),
                nav::router::FindOptions::default(),
                state,
            );
            let preferred_elapsed_us = started.elapsed().as_micros();
            let preferred_settled = search.settled();
            let preferred_proof = format!("{:?}", search.proof());
            let preferred = search.into_route();
            let preferred_status = route_status(&preferred);
            let preferred_error = preferred.as_ref().err().map(|error| format!("{error:?}"));
            let mut fallback_status = None;
            let mut fallback_error = None;
            let mut fallback_only_status = None;
            let mut fallback_only_error = None;
            let mut fallback_only_settled = 0;
            let mut fallback_only_elapsed_us = 0;
            let mut fallback_only_proof = None;
            let (result, tier) = match preferred {
                Ok(route) => (Ok(route), Some("preferred".to_owned())),
                Err(error) if goals.fallback().is_empty() => (Err(error), None),
                Err(_) => {
                    fallback_status = Some("SeparateTier".to_owned());
                    let fallback_started = Instant::now();
                    let fallback_search = nav::router::find_first_with(
                        &world.collision,
                        &world.graph,
                        from,
                        goals.fallback(),
                        nav::router::FindOptions::default(),
                        state,
                    );
                    fallback_only_settled = fallback_search.settled();
                    fallback_only_proof = Some(format!("{:?}", fallback_search.proof()));
                    let fallback_result = fallback_search.into_route();
                    fallback_only_elapsed_us = fallback_started.elapsed().as_micros();
                    fallback_only_status = Some(route_status(&fallback_result));
                    fallback_only_error = fallback_result
                        .as_ref()
                        .err()
                        .map(|error| format!("{error:?}"));
                    fallback_error.clone_from(&fallback_only_error);
                    match fallback_result {
                        Ok(route) => (Ok(route), Some("fallback".to_owned())),
                        Err(error) => (Err(error), None),
                    }
                }
            };
            let status = route_status(&result);
            let error = result.as_ref().err().map(|error| format!("{error:?}"));
            let route = result.ok();
            RouteEvidence {
                status,
                error,
                tier,
                destination: route.as_ref().map(|route| route.dest),
                cost_ticks: route.as_ref().map(|route| route.ticks),
                settled: preferred_settled.saturating_add(fallback_only_settled),
                elapsed_us: started.elapsed().as_micros(),
                proof: preferred_proof.clone(),
                preferred_status,
                preferred_error,
                preferred_settled,
                preferred_elapsed_us,
                preferred_proof,
                fallback_status,
                fallback_error,
                fallback_only_status,
                fallback_only_error,
                fallback_only_settled,
                fallback_only_elapsed_us,
                fallback_only_proof,
            }
        }

        fn scene_around(tile: WorldTile) -> super::WorldStateView {
            super::WorldStateView {
                map_base_x: tile.x.saturating_sub(52),
                map_base_z: tile.z.saturating_sub(52),
                level: tile.level,
                ..super::WorldStateView::default()
            }
        }

        fn settings_for_site(site: &SiteCase) -> GathererSettings {
            GathererSettings {
                skill: "Fishing".to_owned(),
                fishing_method: site.key.clone(),
                location: "Site".to_owned(),
                site: site.site_id.clone(),
                radius: site.radius,
                ..GathererSettings::default()
            }
        }

        fn pick_target(
            catalog: &GatherCatalog,
            site: &SiteCase,
            here: WorldTile,
            collision: nav::contract::CollisionView<'_>,
            mode: SelectionMode,
        ) -> TargetPick {
            let settings = settings_for_site(site);
            match mode {
                SelectionMode::Approach => {
                    let world_view = scene_around(here);
                    let mut fishing = super::FishingSurvey::default();
                    let result = super::select(
                        catalog,
                        &site.method_indices,
                        &settings,
                        WorkArea {
                            mode: crate::gatherer::area::AreaMode::Site,
                            anchor: site.anchor,
                            radius: site.radius,
                        },
                        &[AvoidedTile::EMPTY; super::MAX_AVOID],
                        SelectionObservation {
                            snapshot: None,
                            walk_danger: Some(&crate::native::AllowWalkDanger),
                            world: &world_view,
                            locs: &[],
                            npcs: &[],
                            here,
                            now: 1,
                            skill_stat: 99,
                            fishing: &mut fishing,
                            collision: Some(collision),
                        },
                    );
                    if let Some(selected) = result.target {
                        TargetPick {
                            selected: Some(selected),
                            status: "Target".to_owned(),
                            detail: Some(format!("{:?}", result.outcome)),
                            defect: false,
                        }
                    } else {
                        let defect = matches!(
                            result.outcome,
                            Selection::InvalidGoals(_) | Selection::NoStand
                        );
                        TargetPick {
                            selected: None,
                            status: format!("{:?}", result.outcome),
                            detail: None,
                            defect,
                        }
                    }
                }
                SelectionMode::BankReturn => match super::resource_return_target(
                    catalog,
                    &site.method_indices,
                    &settings,
                    WorkArea {
                        mode: crate::gatherer::area::AreaMode::Site,
                        anchor: site.anchor,
                        radius: site.radius,
                    },
                    ReturnObservation {
                        walk_danger: Some(&crate::native::AllowWalkDanger),
                        here,
                        now: 1,
                        skill_stat: 99,
                        avoided: &[AvoidedTile::EMPTY; super::MAX_AVOID],
                        collision: Some(collision),
                    },
                ) {
                    ResourceReturn::Ready(selected) => TargetPick {
                        selected: Some(selected),
                        status: "Target".to_owned(),
                        detail: None,
                        defect: false,
                    },
                    ResourceReturn::ExcludedUntil { until } => TargetPick {
                        selected: None,
                        status: "ExcludedUntil".to_owned(),
                        detail: Some(format!("until={until}")),
                        defect: false,
                    },
                    ResourceReturn::Unavailable { reason } => TargetPick {
                        selected: None,
                        status: "Unavailable".to_owned(),
                        detail: Some(reason.to_owned()),
                        defect: true,
                    },
                    ResourceReturn::InvalidGoals(error) => TargetPick {
                        selected: None,
                        status: "InvalidGoals".to_owned(),
                        detail: Some(format!("{error:?}")),
                        defect: true,
                    },
                    ResourceReturn::Danger(failure) => TargetPick {
                        selected: None,
                        status: "Danger".to_owned(),
                        detail: Some(failure.to_string()),
                        defect: true,
                    },
                },
            }
        }

        fn classify(
            goals: &WalkGoals,
            old_goals: &[WorldTile],
            goal_sets_match: bool,
            before: &RouteEvidence,
            after: &RouteEvidence,
        ) -> (String, Option<String>) {
            if !goal_sets_match {
                return (
                    "defect".to_owned(),
                    Some(
                        "preferred goals are not the exact legacy radius-one legal goals"
                            .to_owned(),
                    ),
                );
            }
            if before.status == "BudgetExhausted"
                || after.status == "BudgetExhausted"
                || before.preferred_status == "BudgetExhausted"
                || after.preferred_status == "BudgetExhausted"
                || (after.fallback_status.as_deref() == Some("Failed")
                    && after.fallback_error.as_deref() == Some("BudgetExhausted"))
                || after.fallback_only_status.as_deref() == Some("BudgetExhausted")
            {
                return (
                    "defect".to_owned(),
                    Some(
                        "a route search exhausted its budget; this is not a NoPath proof"
                            .to_owned(),
                    ),
                );
            }
            match (before.status.as_str(), after.status.as_str()) {
        ("NoPath", "NoPath") => ("unchanged".to_owned(), None),
        ("Routed", "Routed")
            if before.destination == after.destination
                && before.cost_ticks == after.cost_ticks
                && after.tier.as_deref() == Some("preferred") =>
        {
            ("unchanged".to_owned(), None)
        }
        ("NoPath", "Routed")
            if after.tier.as_deref() == Some("fallback")
                && after.destination.is_some_and(|tile| goals.fallback().contains(&tile)) =>
        {
            (
                "newly_reachable".to_owned(),
                Some(format!(
                    "the exact legacy legal goals {old_goals:?} have NoPath (proof {}); the selected resource's wider observation set supplies the routed fallback {:?}; movement bounds and named region are recorded in the site audit",
                    before.proof, after.destination,
                )),
            )
        }
        ("Routed", "Routed") => (
            "defect".to_owned(),
            Some("a previously reachable legacy route changed destination, cost, or tier".to_owned()),
        ),
        _ => (
            "defect".to_owned(),
            Some(format!(
                "route outcome changed from {} to {} without a newly reachable fallback",
                before.status, after.status
            )),
        ),
    }
        }

        fn tile_text(tile: Option<WorldTile>) -> String {
            tile.map_or_else(
                || "-".to_owned(),
                |tile| format!("{},{},L{}", tile.x, tile.z, tile.level),
            )
        }

        fn route_text(route: Option<&RouteEvidence>) -> String {
            let Some(route) = route else {
                return "-".to_owned();
            };
            if route.status == "Routed" {
                format!(
            "Routed to `{}` at {:.2} ticks, tier `{}`, {} settled / {} µs; preferred={} ({} settled / {} µs, proof={})",
            tile_text(route.destination),
            route.cost_ticks.unwrap_or_default(),
            route.tier.as_deref().unwrap_or("unknown"),
            route.settled,
            route.elapsed_us,
            route.preferred_status,
            route.preferred_settled,
            route.preferred_elapsed_us,
            route.preferred_proof
        )
            } else {
                format!(
            "{} ({:?}), preferred={} (proof={}, {} settled / {} µs), fallback={:?}, fallback-alone={:?}",
            route.status,
            route.error,
            route.preferred_status,
            route.preferred_proof,
            route.preferred_settled,
            route.preferred_elapsed_us,
            route.fallback_status,
            route.fallback_only_status
        )
            }
        }

        fn markdown_report(report: &CensusReport<'_>) -> String {
            let cases = report.cases;
            let site_audit = report.site_audit;
            let exclusions = report.exclusions;
            let gaps = report.gaps;
            let totals = report.totals;
            let summary = report.summary;
            let metadata = report.metadata;
            let mut markdown = String::new();
            markdown.push_str("# FIX-025 named fishing-site navigation parity census\n\n");
            markdown.push_str("## Provenance and route contract\n\n");
            markdown.push_str(&format!(
        "- Legacy source: `{LEGACY_BASELINE}`. The selector ranking and `fishing_return_stand` winner selection are retained; historical goal collection is reconstructed by routing the selected `plan.tile` through exact standable radius-one `area_goals`.\n- Site identity and area anchor: selected `GatherSiteOption` IDs, regions, and keys. For each selectable option key, the anchor comes from `widen::site_anchor(catalog, option.methods, site.region, [], tick=1)`, matching runtime. Site radius is `{DEFAULT_SITE_RADIUS}` by default; the known Catherby Guild island case is separately measured at radius `{CATHERBY_REGRESSION_RADIUS}`.\n- Bank starts use the runtime named-bank placement resolver, not arbitrary neighbours behind a teller's counter.\n- Route policy: run the old radius-one goal set alone first, including its own-zone completion; run the wider tier only after preferred failure. `NoPath` is separate from `BudgetExhausted`.\n- An empty loaded scene can legitimately produce `Absent` without requesting an observation walk. Such rows record the unchanged no-walk result, not a missing site or method.\n- Selected content and pack identity: `{}`\n\n",
        serde_json::to_string(metadata).expect("metadata JSON")
    ));
            markdown.push_str("## Summary\n\n");
            markdown.push_str(&format!(
                "- Totals: `{totals}`\n- Classification: `{summary}`\n\n"
            ));
            markdown.push_str("## Published site / key coverage\n\n");
            markdown.push_str("This table contains every `GatherSiteOption` for fishing and every published key. `supported_methods_in_region` are methods with a content-proven usable resource placement in the named region; aliases with no usable placement remain visible in `method_facts`.\n\n");
            markdown.push_str("| Published site ID / label | Region | Key / expected count | Selectable | Declared methods | Supported methods in region | Method placement evidence | Profile | Runtime anchor | Status |\n|---|---|---|---|---|---|---|---|---|---|\n");
            for row in site_audit {
                let method_facts = row
                    .method_facts
                    .iter()
                    .map(|fact| {
                        let movements = fact
                            .movement_evidence
                            .iter()
                            .map(|movement| {
                                format!(
                                    "spot{} {}@{} {}={:?}",
                                    movement.spot_id,
                                    movement.entity,
                                    tile_text(Some(movement.origin)),
                                    movement.status,
                                    movement.bounds
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!(
                            "{}: {}/{} usable (complete={}) movements=[{}]{}",
                            fact.method,
                            fact.usable_resource_spots,
                            fact.region_spots,
                            fact.coverage_complete,
                            movements,
                            fact.region_error
                                .as_deref()
                                .map_or_else(String::new, |error| format!(" error={error}"))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                markdown.push_str(&format!(
            "| `{}`<br>{} | `{}` | `{}` / {} | {} | `{}` | `{}` | `{}` | `{}` r{} | `{}` | {} |\n",
            row.site_id,
            row.label,
            serde_json::to_string(&row.region).expect("site region JSON"),
            row.key,
            row.expected_count,
            row.selectable,
            row.declared_methods.join(", "),
            row.supported_methods_in_region.join(", "),
            method_facts,
            row.profile,
            row.area_radius,
            tile_text(row.runtime_anchor),
            row.status
        ));
            }
            markdown.push_str("\n## Route comparisons\n\n");
            markdown.push_str("Each row is a published site/key option × start × selector mode. The key lists the exact selected resource option; its method group and the runtime site anchor are retained.\n\n");
            markdown.push_str("| Published site ID | Key | Methods | Profile / anchor | Start | Mode | Selection / chosen method | Legacy ranked stand | Goal counts (old / preferred / fallback) | e9 route | Tiered route | Classification / reason |\n|---|---|---|---|---|---|---|---|---|---|---|---|\n");
            for case in cases {
                let methods = case.declared_methods.join(", ");
                let selected_method = case.selected_method.as_deref().map_or("-", |method| method);
                let goals = format!(
                    "{}/{}/{} (exact={})",
                    case.old_goal_count,
                    case.preferred_goal_count,
                    case.fallback_goal_count,
                    case.goal_sets_match.unwrap_or(false)
                );
                let start = format!("{} `{}`", case.start_kind, tile_text(Some(case.start_tile)));
                markdown.push_str(&format!(
            "| `{}`<br>{}<br>`{:?}` | `{}` | `{}` | `{}` r{}<br>`{}` | {} | {} | {}<br>selected `{}` | `{}` | `{}` | {} | {} | **{}**<br>{} |\n",
            case.site_id,
            case.site_label,
            case.region,
            case.key,
            methods,
            case.profile,
            case.area_radius,
            tile_text(Some(case.area_anchor)),
            start,
            case.mode,
            case.selection_status,
            selected_method,
            tile_text(case.ranked_stand),
            goals,
            route_text(case.before.as_ref()),
            route_text(case.after.as_ref()),
            case.classification,
            case.reason.as_deref().unwrap_or("-")
        ));
            }
            markdown.push_str("\n## Explicit content exclusions\n\n");
            markdown
                .push_str("| Published site ID | Key | Methods | Reason |\n|---|---|---|---|\n");
            for exclusion in exclusions {
                markdown.push_str(&format!(
                    "| `{}`<br>{} | `{}` (count {}) | `{}` | {} |\n",
                    exclusion.site_id,
                    exclusion.label,
                    exclusion.key,
                    exclusion.expected_count,
                    exclusion.methods.join(", "),
                    exclusion.reason
                ));
            }
            markdown.push_str("\n## Coverage gaps\n\n");
            if gaps.is_empty() {
                markdown.push_str("None.\n");
            } else {
                markdown.push_str("| Site | Key | Method | Gap |\n|---|---|---|---|\n");
                for gap in gaps {
                    markdown.push_str(&format!(
                        "| `{}` | `{}` | `{}` | {} |\n",
                        gap.site_id.as_deref().unwrap_or("-"),
                        gap.key.as_deref().unwrap_or("-"),
                        gap.method.as_deref().unwrap_or("-"),
                        gap.reason
                    ));
                }
            }
            markdown
        }

        fn hex(bytes: &[u8]) -> String {
            use std::fmt::Write as _;
            let mut value = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                let _ = write!(value, "{byte:02x}");
            }
            value
        }

        fn write_artifacts(output: &Path, report: &CensusReport<'_>) -> PathBuf {
            let markdown_path = output.with_extension("md");
            for path in [output, markdown_path.as_path()] {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("create parity evidence directory");
                }
            }
            let json = serde_json::json!({
                "schema": "fix-025-named-fishing-site-parity-v1",
                "baseline_commit": LEGACY_BASELINE,
                "metadata": report.metadata,
                "totals": report.totals,
                "summary": report.summary,
                "site_audit": report.site_audit,
                "cases": report.cases,
                "content_exclusions": report.exclusions,
                "coverage_gaps": report.gaps,
            });
            std::fs::write(
                output,
                serde_json::to_vec_pretty(&json).expect("serialize route census JSON"),
            )
            .expect("write route census JSON");
            std::fs::write(&markdown_path, markdown_report(report))
                .expect("write route census Markdown");
            markdown_path
        }

        /// Exhaustively compare named published fishing sites and resource keys using
        /// the runtime site anchor, default radius, and Catherby regression radius.
        #[test]
        #[ignore = "requires the reviewed real R289 NAV_PACK and BOT_FISHING_SITE_PARITY_OUT"]
        fn real_fishing_site_route_parity_census() {
            // Historical provenance: e9b575be4's `select` and
            // `resource_return_target` chose the placement and `plan.tile` before any
            // observation goals existed. The later change added candidate construction
            // after those winner selections. `area_goals` therefore reconstructs the
            // historical route target without a second picker or an invented baseline.
            let output = std::env::var_os("BOT_FISHING_SITE_PARITY_OUT")
                .map(PathBuf::from)
                .expect("set BOT_FISHING_SITE_PARITY_OUT to the JSON evidence path");
            let world = super::real_nav_world();
            let lease = super::test_real_nav_lease();
            let catalog = super::real_catalog();
            let selected = api::game_data::for_revision(client::io::ClientRevision::R289)
                .expect("embedded R289 selected game data");
            let all_bank_accesses = bank_accesses(world, &selected);
            let mut state = nav::world_state::WorldState::empty().with_map_members(true);
            state.stats = (0..=20).map(|skill| (skill, 99)).collect();
            let mut site_audit = Vec::<SiteAudit>::new();
            let mut exclusions = Vec::<SiteExclusion>::new();
            let mut gaps = Vec::<CoverageGap>::new();
            let (options, named_site_count, published_key_count) = site_options(
                &selected,
                &catalog,
                &mut gaps,
                &mut exclusions,
                &mut site_audit,
            );
            let mut cases = Vec::<CensusCase>::new();

            for option in &options {
                let site = SiteCase {
                    site_id: option.site_id.clone(),
                    label: option.label.clone(),
                    region: option.region,
                    key: option.key.clone(),
                    expected_count: option.expected_count,
                    declared_methods: option.declared_methods.clone(),
                    method_indices: option.method_indices.clone(),
                    selected_method: None,
                    profile: option.profile.clone(),
                    anchor: option.anchor,
                    radius: option.radius,
                };
                let area = WorkArea {
                    mode: crate::gatherer::area::AreaMode::Site,
                    anchor: site.anchor,
                    radius: site.radius,
                };
                let (nearest, farthest) =
                    nearest_and_farthest_bank(&all_bank_accesses, site.anchor);
                let inside = nearest_inside_area_stand(world, area);
                let mut starts = Vec::<StartCase>::new();
                if let Some(bank) = nearest {
                    starts.push(bank_start("site_bank", bank));
                } else {
                    gaps.push(CoverageGap {
                        site_id: Some(site.site_id.clone()),
                        key: Some(site.key.clone()),
                        method: None,
                        reason:
                            "no real-pack bank access tile exists on the runtime anchor's level"
                                .to_owned(),
                    });
                }
                if let Some(tile) = inside {
                    starts.push(StartCase {
                        kind: "inside_site_area",
                        tile,
                        bank: None,
                    });
                } else {
                    gaps.push(CoverageGap {
                        site_id: Some(site.site_id.clone()),
                        key: Some(site.key.clone()),
                        method: None,
                        reason: "no standable tile exists inside the runtime site work area"
                            .to_owned(),
                    });
                }
                if let Some(bank) = farthest {
                    starts.push(bank_start("far_bank", bank));
                } else {
                    gaps.push(CoverageGap {
                        site_id: Some(site.site_id.clone()),
                        key: Some(site.key.clone()),
                        method: None,
                        reason: "no far-bank access tile exists on the runtime anchor's level"
                            .to_owned(),
                    });
                }
                let catherby_regression = site.site_id == "fishing.catherby"
                    && site.key == CATHERBY_TUNA_SWORD_KEY
                    && site.method_indices.iter().any(|&index| {
                        catalog
                            .methods()
                            .get(index)
                            .is_some_and(|method| method.id.0.as_ref() == "fishing.rarefish.op3")
                    });
                if catherby_regression {
                    if world.collision.standable(CATHERBY_GUILD_START) {
                        starts.push(StartCase {
                            kind: "catherby_guild",
                            tile: CATHERBY_GUILD_START,
                            bank: None,
                        });
                    } else {
                        gaps.push(CoverageGap {
                    site_id: Some(site.site_id.clone()),
                    key: Some(site.key.clone()),
                    method: None,
                    reason: "known Catherby Guild regression start is not standable in the selected pack".to_owned(),
                });
                    }
                }

                for start in &starts {
                    for mode in [SelectionMode::Approach, SelectionMode::BankReturn] {
                        let pick =
                            pick_target(&catalog, &site, start.tile, lease.collision(), mode);
                        let Some(selected_target) = pick.selected else {
                            if pick.defect {
                                gaps.push(CoverageGap {
                                    site_id: Some(site.site_id.clone()),
                                    key: Some(site.key.clone()),
                                    method: None,
                                    reason: format!(
                                        "{} during {}: {}",
                                        pick.status,
                                        mode.label(),
                                        pick.detail.as_deref().unwrap_or("no target detail")
                                    ),
                                });
                            }
                            let classification = if pick.defect {
                                "defect"
                            } else {
                                "not_selected"
                            };
                            cases.push(CensusCase {
                                site_id: site.site_id.clone(),
                                site_label: site.label.clone(),
                                region: site.region.into(),
                                key: site.key.clone(),
                                expected_count: site.expected_count,
                                declared_methods: site.declared_methods.clone(),
                                supported_methods_in_region: option
                                    .method_facts
                                    .iter()
                                    .filter(|fact| fact.usable_resource_spots > 0)
                                    .map(|fact| fact.method.clone())
                                    .collect(),
                                profile: site.profile.clone(),
                                area_anchor: site.anchor,
                                area_radius: site.radius,
                                start_kind: start.kind.to_owned(),
                                start_tile: start.tile,
                                bank: start.bank.clone(),
                                mode: mode.label().to_owned(),
                                selection_status: pick.status,
                                selection_detail: pick.detail.clone(),
                                selected_method: None,
                                ranked_stand: None,
                                old_goal_count: 0,
                                preferred_goal_count: 0,
                                fallback_goal_count: 0,
                                goal_sets_match: None,
                                before: None,
                                after: None,
                                classification: classification.to_owned(),
                                reason: pick.detail,
                            });
                            continue;
                        };
                        let Some(goals) = selected_target.observation_goals.as_ref() else {
                            cases.push(CensusCase {
                                site_id: site.site_id.clone(),
                                site_label: site.label.clone(),
                                region: site.region.into(),
                                key: site.key.clone(),
                                expected_count: site.expected_count,
                                declared_methods: site.declared_methods.clone(),
                                supported_methods_in_region: option
                                    .method_facts
                                    .iter()
                                    .filter(|fact| fact.usable_resource_spots > 0)
                                    .map(|fact| fact.method.clone())
                                    .collect(),
                                profile: site.profile.clone(),
                                area_anchor: site.anchor,
                                area_radius: site.radius,
                                start_kind: start.kind.to_owned(),
                                start_tile: start.tile,
                                bank: start.bank.clone(),
                                mode: mode.label().to_owned(),
                                selection_status: "TargetWithoutObservationGoals".to_owned(),
                                selection_detail: Some(format!(
                                    "selected entity {:?}",
                                    selected_target.plan.entity
                                )),
                                selected_method: catalog
                                    .methods()
                                    .get(usize::from(selected_target.plan.method_index))
                                    .map(|method| method.id.0.to_string()),
                                ranked_stand: Some(selected_target.plan.tile),
                                old_goal_count: 0,
                                preferred_goal_count: 0,
                                fallback_goal_count: 0,
                                goal_sets_match: Some(false),
                                before: None,
                                after: None,
                                classification: "defect".to_owned(),
                                reason: Some(
                                    "a selected fishing target has no observation goals".to_owned(),
                                ),
                            });
                            continue;
                        };
                        let selected_method_index = usize::from(selected_target.plan.method_index);
                        if !site.method_indices.contains(&selected_method_index) {
                            cases.push(CensusCase {
                        site_id: site.site_id.clone(),
                        site_label: site.label.clone(),
                        region: site.region.into(),
                        key: site.key.clone(),
                        expected_count: site.expected_count,
                        declared_methods: site.declared_methods.clone(),
                        supported_methods_in_region: option
                            .method_facts
                            .iter()
                            .filter(|fact| fact.usable_resource_spots > 0)
                            .map(|fact| fact.method.clone())
                            .collect(),
                        profile: site.profile.clone(),
                        area_anchor: site.anchor,
                        area_radius: site.radius,
                        start_kind: start.kind.to_owned(),
                        start_tile: start.tile,
                        bank: start.bank.clone(),
                        mode: mode.label().to_owned(),
                        selection_status: "TargetOutsideSelectedOptionMethods".to_owned(),
                        selection_detail: Some(format!("method index {selected_method_index}")),
                        selected_method: catalog
                            .methods()
                            .get(selected_method_index)
                            .map(|method| method.id.0.to_string()),
                        ranked_stand: Some(selected_target.plan.tile),
                        old_goal_count: 0,
                        preferred_goal_count: goals.preferred().len(),
                        fallback_goal_count: goals.fallback().len(),
                        goal_sets_match: Some(false),
                        before: None,
                        after: None,
                        classification: "defect".to_owned(),
                        reason: Some(
                            "selector chose a method outside the selected named-site option"
                                .to_owned(),
                        ),
                    });
                            continue;
                        }
                        let old_goals =
                            super::area_goals(&world.collision, selected_target.plan.tile);
                        let goal_sets_match =
                            !old_goals.is_empty() && goals.preferred() == old_goals.as_slice();
                        let before = legacy_route(world, start.tile, &old_goals, &state);
                        let after = tiered_route(world, start.tile, goals, &state);
                        let site_for_classification = SiteCase {
                            site_id: site.site_id.clone(),
                            label: site.label.clone(),
                            region: site.region,
                            key: site.key.clone(),
                            expected_count: site.expected_count,
                            declared_methods: site.declared_methods.clone(),
                            selected_method: catalog
                                .methods()
                                .get(selected_method_index)
                                .map(|method| method.id.0.to_string()),
                            method_indices: site.method_indices.clone(),
                            profile: site.profile.clone(),
                            anchor: site.anchor,
                            radius: site.radius,
                        };
                        let (classification, reason) =
                            classify(goals, &old_goals, goal_sets_match, &before, &after);
                        let selected_method = site_for_classification.selected_method;
                        cases.push(CensusCase {
                            site_id: site.site_id.clone(),
                            site_label: site.label.clone(),
                            region: site.region.into(),
                            key: site.key.clone(),
                            expected_count: site.expected_count,
                            declared_methods: site.declared_methods.clone(),
                            supported_methods_in_region: option
                                .method_facts
                                .iter()
                                .filter(|fact| fact.usable_resource_spots > 0)
                                .map(|fact| fact.method.clone())
                                .collect(),
                            profile: site.profile.clone(),
                            area_anchor: site.anchor,
                            area_radius: site.radius,
                            start_kind: start.kind.to_owned(),
                            start_tile: start.tile,
                            bank: start.bank.clone(),
                            mode: mode.label().to_owned(),
                            selection_status: pick.status,
                            selection_detail: pick.detail,
                            selected_method,
                            ranked_stand: Some(selected_target.plan.tile),
                            old_goal_count: old_goals.len(),
                            preferred_goal_count: goals.preferred().len(),
                            fallback_goal_count: goals.fallback().len(),
                            goal_sets_match: Some(goal_sets_match),
                            before: Some(before),
                            after: Some(after),
                            classification,
                            reason,
                        });
                    }
                }
            }

            let unchanged = cases
                .iter()
                .filter(|case| case.classification == "unchanged")
                .count();
            let newly_reachable = cases
                .iter()
                .filter(|case| case.classification == "newly_reachable")
                .count();
            let other_content_justified = cases
                .iter()
                .filter(|case| case.classification == "other_content_justified")
                .count();
            let defects = cases
                .iter()
                .filter(|case| case.classification == "defect")
                .count();
            let not_selected = cases
                .iter()
                .filter(|case| case.classification == "not_selected")
                .count();
            let route_comparisons = cases
                .iter()
                .filter(|case| case.before.is_some() && case.after.is_some())
                .count();
            let totals = serde_json::json!({
                "named_fishing_sites": named_site_count,
                "published_site_keys": published_key_count,
                "site_key_profiles_with_runtime_anchors": options.len(),
                "site_audit_rows": site_audit.len(),
                "content_excluded_site_keys": exclusions.len(),
                "route_case_rows": cases.len(),
                "route_comparisons": route_comparisons,
                "selection_nonroute_rows": not_selected,
                "coverage_gaps": gaps.len(),
            });
            let worst_preferred_by_settles = cases
                .iter()
                .filter_map(|case| {
                    let route = case.after.as_ref()?;
                    (route.preferred_status == "NoPath").then_some((case, route))
                })
                .max_by_key(|(_, route)| route.preferred_settled);
            let worst_preferred_by_time = cases
                .iter()
                .filter_map(|case| {
                    let route = case.after.as_ref()?;
                    (route.preferred_status == "NoPath").then_some((case, route))
                })
                .max_by_key(|(_, route)| route.preferred_elapsed_us);
            let summary = serde_json::json!({
                "unchanged": unchanged,
                "newly_reachable": newly_reachable,
                "other_content_justified_changes": other_content_justified,
                "defects": defects,
                "not_selected": not_selected,
                "worst_failed_preferred_by_settles": worst_preferred_by_settles.map(|(case, route)| serde_json::json!({
                    "site": case.site_id,
                    "key": case.key,
                    "profile": case.profile,
                    "start_kind": case.start_kind,
                    "start_tile": case.start_tile,
                    "mode": case.mode,
                    "preferred_settled": route.preferred_settled,
                    "preferred_elapsed_us": route.preferred_elapsed_us,
                    "combined_elapsed_us": route.elapsed_us,
                    "preferred_proof": route.preferred_proof,
                })),
                "worst_failed_preferred_by_time": worst_preferred_by_time.map(|(case, route)| serde_json::json!({
                    "site": case.site_id,
                    "key": case.key,
                    "profile": case.profile,
                    "start_kind": case.start_kind,
                    "start_tile": case.start_tile,
                    "mode": case.mode,
                    "preferred_settled": route.preferred_settled,
                    "preferred_elapsed_us": route.preferred_elapsed_us,
                    "combined_elapsed_us": route.elapsed_us,
                    "preferred_proof": route.preferred_proof,
                })),
            });
            let pin = catalog.pin();
            let metadata = serde_json::json!({
                "client_revision": format!("{:?}", pin.revision),
                "engine_commit": pin.engine_commit.as_ref(),
                "content_commit": pin.content_commit.as_ref(),
                "cache_id": pin.cache_id.as_ref(),
                "content_id": pin.content_id.as_ref(),
                "nav_sha256": hex(&pin.nav_sha256),
                "flags_sha256": hex(&pin.flags_sha256),
                "manifest_sha256": hex(&pin.manifest_sha256),
                "nav_pack_path": std::env::var_os("NAV_PACK").map(|path| path.to_string_lossy().into_owned()),
                "evidence_json_path": output.display().to_string(),
            });
            let report = CensusReport {
                cases: &cases,
                site_audit: &site_audit,
                exclusions: &exclusions,
                gaps: &gaps,
                totals: &totals,
                summary: &summary,
                metadata: &metadata,
            };
            let markdown_path = write_artifacts(&output, &report);
            assert!(
                named_site_count > 0,
                "selected R289 game data has no published fishing sites; see {}",
                markdown_path.display()
            );
            let audited_site_ids = site_audit
                .iter()
                .map(|row| row.site_id.as_str())
                .collect::<BTreeSet<_>>()
                .len();
            assert_eq!(
                audited_site_ids,
                named_site_count,
                "published fishing site IDs were not all represented; see {}",
                markdown_path.display()
            );
            let audited_site_keys = site_audit
                .iter()
                .filter(|row| row.key != "<no keys>")
                .map(|row| (row.site_id.as_str(), row.key.as_str()))
                .collect::<BTreeSet<_>>()
                .len();
            assert_eq!(
                audited_site_keys,
                published_key_count,
                "published fishing site keys were not all represented; see {}",
                markdown_path.display()
            );
            assert_eq!(
                gaps.len(),
                0,
                "the named-site census has missing methods, anchors, starts, or metadata; see {}",
                markdown_path.display()
            );
            assert_eq!(
                defects,
                0,
                "the real-pack route parity census found {defects} defects; see {}",
                markdown_path.display()
            );
            assert!(
                cases.iter().any(|case| {
                    case.site_id == "fishing.catherby"
                        && case.key == CATHERBY_TUNA_SWORD_KEY
                        && case.start_kind == "catherby_guild"
                        && case.ranked_stand == Some(CATHERBY_ISLAND_CLAMP)
                        && case.classification == "newly_reachable"
                }),
                "the expected Catherby island-clamp improvement was not observed; see {}",
                markdown_path.display()
            );
            assert!(
                route_comparisons > 0,
                "the census did not produce any comparable routes; see {}",
                markdown_path.display()
            );
        }
    }
}
