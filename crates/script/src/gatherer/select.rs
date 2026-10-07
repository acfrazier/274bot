use super::area::WorkArea;
use super::settings::{method_level, GathererSettings, TargetPreference};
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
/// other placements. Only actual gathering progress renews approach budgets.
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
fn observation_cells(bounds: SceneRegionInput) -> impl Iterator<Item = SceneRegionInput> {
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

    fn next_stand(&self, bounds: SceneRegionInput, here: WorldTile) -> Option<WorldTile> {
        observation_cells(bounds)
            .enumerate()
            .filter(|(index, _)| {
                if *index < u64::BITS as usize {
                    self.covered & (1 << index) == 0
                } else {
                    self.covered_more
                        .get(index / u64::BITS as usize - 1)
                        .is_none_or(|word| word & (1 << (index % u64::BITS as usize)) == 0)
                }
            })
            .map(|(_, cell)| observation_stand(cell, here))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Target(PlacementClass),
    Exhausted { wait_until: u64, absent: u16 },
    Absent { absent: u16 },
}

#[derive(Debug, Clone)]
pub struct SelectedTarget {
    pub plan: TargetPlan,
    pub class: PlacementClass,
}

impl SelectedTarget {
    /// One approach policy for Gatherer and finite quest gathering.
    /// Live NPC ops own client-side approach; locs settle at their footprint.
    /// Arrival uses the observed server position, never the rendered actor pose.
    pub(crate) fn approach(
        &self,
        snapshot: api::snapshot::SnapshotView<'_>,
        required_after: api::quest_progress::EvidenceStamp,
    ) -> Option<crate::native::WalkRequest> {
        let observation_approach =
            self.class == PlacementClass::Unloaded && matches!(self.plan.entity, EntityId::Npc(_));
        let loc_id = match self.plan.entity {
            EntityId::Loc(id) => Some(id),
            _ => None,
        };
        let needs_walk = if self.class == PlacementClass::Unloaded {
            true
        } else if self.plan.npc_index >= 0 {
            false
        } else {
            !snapshot.local_player().is_some_and(|player| match loc_id {
                Some(id) => snapshot.walk_loc_arrived(
                    player.value.player.network,
                    self.plan.tile,
                    i32::from(RESOURCE_APPROACH_RADIUS),
                    id,
                ),
                None => snapshot.walk_arrived(
                    player.value.player.network,
                    self.plan.tile,
                    i32::from(RESOURCE_APPROACH_RADIUS),
                ),
            })
        };
        needs_walk.then(|| crate::native::WalkRequest {
            target: self.plan.tile,
            loc_id,
            radius: RESOURCE_APPROACH_RADIUS,
            arrival: if observation_approach {
                nav::arrival::ArrivalKind::Area
            } else {
                nav::arrival::ArrivalKind::Reach
            },
            options: crate::native::WalkOptions::default(),
            required_after,
            evidence: None,
            cross: Vec::new().into_boxed_slice(),
            protect: false,
            food_guard: false,
            allow: Default::default(),
        })
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
}

pub struct ReturnObservation<'a> {
    pub here: WorldTile,
    pub now: u64,
    pub skill_stat: i32,
    pub avoided: &'a [AvoidedTile; MAX_AVOID],
    /// Packed collision used to keep Area r=1 return stands off tiles with
    /// no legal goals. `None` skips the filter (unit fixtures without a pack).
    pub collision: Option<&'a nav::collision::WorldCollision>,
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
    if !region_loaded(bounds, world) {
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
    // This direct classifier handles a single fully visible envelope. The
    // selector accumulates partial empty views for wider envelopes below.
    if !region_visible(bounds, here) {
        return PlacementClass::Unloaded;
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
    } = observation;
    let preference = settings.target_preference_kind();
    let region = area.region();
    let mut absent: u16 = 0;
    let mut zone_gated: u16 = 0;
    let mut wait_until = now;
    let mut non_absent = false;
    let mut best_live = None;

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
                    let survey = fishing.placement(method_index, spot.id.0);
                    survey.observe(bounds, world, here, now);
                    if let Some(next) = survey.next_stand(bounds, here) {
                        if survey.approaches >= MAX_FISHING_APPROACHES {
                            // Budget exhaustion is not proof of absence. Let
                            // existing exhaustion handling bound an unseen spot.
                            class = PlacementClass::Avoided;
                        } else {
                            class = PlacementClass::Unloaded;
                        }
                        stand = next;
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
                        // The live pass already considered every eligible NPC.
                        non_absent = true;
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
                    PlacementClass::Live => non_absent = true,
                }
            }
        }
    }

    if let Some((method_index, method, spot, class, tile, npc_index, _)) = best_unloaded {
        if method.skill == GatherSkill::Fishing {
            let survey = fishing.placement(method_index, spot.id.0);
            survey.approaches = survey.approaches.saturating_add(1);
        }
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
            }),
            outcome: Selection::Target(class),
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
) -> Option<TargetPlan> {
    let preference = settings.target_preference_kind();
    let mut best = None;
    for &method_index in method_indices {
        let Some(method) = catalog.methods().get(method_index) else {
            continue;
        };
        let Ok(method_index) = u16::try_from(method_index) else {
            continue;
        };
        if method.skill == GatherSkill::Fishing {
            let Some(spots) = complete_spots(method) else {
                continue;
            };
            for spot in spots {
                let Some(stand) = fishing_return_stand(catalog, method, spot, area, &observation)
                else {
                    continue;
                };
                consider_candidate(
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
                );
            }
        } else {
            let Ok(spots) = catalog.spots(method, &area.region()) else {
                continue;
            };
            for spot in spots {
                if !known_resource_target(method, spot.entity)
                    || catalog.access(method, spot).unwrap_or(Truth::False) != Truth::True
                    || avoid_until(spot, method.skill, observation.avoided) > observation.now
                    || !has_area_arrival(observation.collision, spot.origin)
                {
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
        }
    }
    let (method_index, method, spot, _, tile, npc_index, _) = best?;
    Some(make_plan(
        catalog,
        method_index,
        method,
        spot,
        tile,
        npc_index,
        observation.skill_stat,
    ))
}

fn fishing_return_stand(
    catalog: &GatherCatalog,
    method: &GatherMethod,
    spot: &GatherSpot,
    area: WorkArea,
    observation: &ReturnObservation<'_>,
) -> Option<WorldTile> {
    if !fishing_spot_eligible(spot, area)
        || !known_resource_target(method, spot.entity)
        || catalog.access(method, spot).unwrap_or(Truth::False) != Truth::True
        || avoid_until(spot, method.skill, observation.avoided) > observation.now
    {
        return None;
    }
    // Observe the resource from the resource, never from `here`. Clamping
    // toward the player is how a Draynor bank trip lands a stand inside the
    // booth (the spots sit inside NPC view of the interior). Packed Area
    // goals reuse `standable` over radius one; an all-water origin clamp is
    // replaced by the nearest observation-safe stand Walk can arrive at.
    observation_cells(movement_bounds(spot)?)
        .filter_map(|cell| {
            legal_fishing_return_stand(cell, spot.origin, area, observation.collision)
        })
        .min_by_key(|stand| {
            (
                i64::from(!area.contains(*stand)),
                distance(spot.origin, *stand),
            )
        })
}

fn has_area_arrival(collision: Option<&nav::collision::WorldCollision>, stand: WorldTile) -> bool {
    collision.is_none_or(|collision| {
        nav::arrival::area_has_standable_goal(stand, i32::from(RESOURCE_APPROACH_RADIUS), |tile| {
            collision.standable(tile)
        })
    })
}

fn legal_fishing_return_stand(
    cell: SceneRegionInput,
    origin: WorldTile,
    area: WorkArea,
    collision: Option<&nav::collision::WorldCollision>,
) -> Option<WorldTile> {
    let preferred = observation_stand(cell, origin);
    if has_area_arrival(collision, preferred) {
        return Some(preferred);
    }
    let min_x = cell.max_x - OBSERVATION_RADIUS;
    let max_x = cell.min_x + OBSERVATION_RADIUS;
    let min_z = cell.max_z - OBSERVATION_RADIUS;
    let max_z = cell.min_z + OBSERVATION_RADIUS;
    if min_x > max_x || min_z > max_z {
        return None;
    }
    let mut best = None;
    for x in min_x..=max_x {
        for z in min_z..=max_z {
            let stand = WorldTile {
                x,
                z,
                level: cell.level,
            };
            if !has_area_arrival(collision, stand) {
                continue;
            }
            let key = (i64::from(!area.contains(stand)), distance(origin, stand));
            if best.as_ref().is_none_or(|(_, current)| key < *current) {
                best = Some((stand, key));
            }
        }
    }
    best.map(|(stand, _)| stand)
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

fn tile_loaded(tile: WorldTile, world: &WorldStateView) -> bool {
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
mod tests {
    use super::*;
    use api::gather_methods::{GatherTarget, SpotId};
    use api::snapshot::{LocLayer, LocView, NpcView};
    use std::collections::HashMap;

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
            selected.approach(view, stamp).is_none(),
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

    fn real_catalog() -> Arc<GatherCatalog> {
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
            here,
            now: 1,
            skill_stat: 10,
            avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
            collision: None,
        };
        assert_eq!(
            fishing_return_stand(&catalog, method, &spot, area, &observation),
            Some(expected_return),
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
                fishing: &mut FishingSurvey::default(),
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
                fishing: &mut FishingSurvey::default(),
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
                fishing: &mut FishingSurvey::default(),
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
                    here,
                    now: 1,
                    skill_stat: 10,
                    avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                    collision: Some(&collision),
                },
            )
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
                here: edgeville,
                now: 1,
                skill_stat: 10,
                avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                collision: Some(&collision),
            },
        )
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
            let target = resource_return_target(
                &catalog,
                &[index],
                &settings,
                area,
                ReturnObservation {
                    here: bank_interior,
                    now: 1,
                    skill_stat: 10,
                    avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                    collision: Some(&collision),
                },
            )
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
        let target = resource_return_target(
            &catalog,
            &[index],
            &settings,
            area,
            ReturnObservation {
                here,
                now: 1,
                skill_stat: 30,
                avoided: &[AvoidedTile::EMPTY; MAX_AVOID],
                collision: Some(&collision),
            },
        )
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
                fishing: &mut FishingSurvey::default(),
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
                fishing: &mut FishingSurvey::default(),
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
                        fishing: &mut survey,
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
                    fishing: &mut survey,
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
                    fishing: &mut survey,
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
                fishing: &mut FishingSurvey::default(),
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
                fishing: &mut FishingSurvey::default(),
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
                fishing: &mut FishingSurvey::default(),
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
}
