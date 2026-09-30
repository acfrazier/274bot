use super::area::WorkArea;
use super::settings::{method_level, GathererSettings, TargetPreference};
use api::gather_methods::{known_rows, GatherCatalog, GatherMethod, GatherSpot, TargetClass};
use api::selected::{EntityId, Knowledge, Truth};
use api::snapshot::{LocView, WorldStateView, WorldTile};
use std::sync::Arc;

const MAX_PRODUCTS: usize = 8;
const MAX_AVOID: usize = 8;

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
    pub until: u64,
}

#[derive(Debug, Clone)]
pub struct TargetPlan {
    pub entity: EntityId,
    pub tile: WorldTile,
    pub op: Arc<str>,
    pub alias: Arc<str>,
    pub products: [i32; MAX_PRODUCTS],
    pub products_len: u8,
    pub skill_stat: i32,
    pub respawn_max: u32,
    pub class: TargetClass,
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

pub fn classify_placement(
    spot: &GatherSpot,
    method: &GatherMethod,
    world: &WorldStateView,
    locs: &[LocView],
    avoided: &[AvoidedTile; MAX_AVOID],
    now: u64,
) -> PlacementClass {
    // The observed build rectangle is the authoritative loaded-scene test.
    // A placement on another plane is equally unloaded, even if its x/z are
    // inside the rectangle of the current plane.
    let loaded = spot.origin.level == world.level
        && spot.origin.x >= world.map_base_x
        && spot.origin.x < world.map_base_x.saturating_add(104)
        && spot.origin.z >= world.map_base_z
        && spot.origin.z < world.map_base_z.saturating_add(104);
    if !loaded {
        return PlacementClass::Unloaded;
    }
    if avoided
        .iter()
        .any(|entry| entry.until > now && entry.tile == spot.origin)
    {
        return PlacementClass::Avoided;
    }
    let Some(targets) = known_targets(method) else {
        return PlacementClass::Absent;
    };
    for loc in locs.iter().filter(|loc| loc.tile == spot.origin) {
        let Some(target) = targets.iter().find(|target| {
            matches!(target.respawn, Knowledge::Known(_))
                && match target.entity {
                    EntityId::Loc(id) => id == loc.id,
                    _ => false,
                }
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
    PlacementClass::Absent
}

pub struct SelectionObservation<'a> {
    pub world: &'a WorldStateView,
    pub locs: &'a [LocView],
    pub here: WorldTile,
    pub now: u64,
    pub skill_stat: i32,
}

/// Select the best live or not-yet-loaded placement. The pass is allocation
/// free; copies are made only after a candidate wins at a boundary.
pub fn select(
    catalog: &GatherCatalog,
    method_indices: &[usize],
    settings: &GathererSettings,
    area: WorkArea,
    avoided: &[AvoidedTile; MAX_AVOID],
    observation: SelectionObservation<'_>,
) -> SelectionResult {
    let SelectionObservation {
        world,
        locs,
        here,
        now,
        skill_stat,
    } = observation;
    let preference = settings.target_preference_kind();
    let mut absent: u16 = 0;
    let mut zone_gated: u16 = 0;
    let mut wait_until = now;
    let mut non_absent = false;
    let mut best: Option<(&GatherMethod, &GatherSpot, PlacementClass, i64)> = None;
    for &method_index in method_indices {
        let Some(method) = catalog.methods().get(method_index) else {
            continue;
        };
        let region = area.region();
        let Ok(spots) = catalog.spots(method, &region) else {
            continue;
        };
        for spot in spots {
            if !known_targets(method).is_some_and(|targets| {
                targets.iter().any(|target| {
                    target.entity == spot.entity && matches!(target.respawn, Knowledge::Known(_))
                })
            }) {
                continue;
            }
            if catalog.access(method, spot).unwrap_or(Truth::False) != Truth::True {
                zone_gated = zone_gated.saturating_add(1);
                continue;
            }
            let class = classify_placement(spot, method, world, locs, avoided, now);
            let respawn = respawn_max(catalog, method, spot);
            match class {
                PlacementClass::Live | PlacementClass::Unloaded => {
                    non_absent = true;
                    let distance = distance(here, spot.origin);
                    if better_candidate(
                        preference,
                        best.as_ref()
                            .map(|(current_method, current_spot, _, distance)| {
                                (
                                    i64::from(method_level(current_method, skill_stat)),
                                    *distance,
                                    current_spot.id.0,
                                )
                            }),
                        method,
                        skill_stat,
                        distance,
                        spot.id.0,
                    ) {
                        best = Some((method, spot, class, distance));
                    }
                }
                PlacementClass::Depleted | PlacementClass::Hazard | PlacementClass::Avoided => {
                    non_absent = true;
                    let until = now.saturating_add(u64::from(respawn));
                    wait_until = wait_until.max(until);
                }
                PlacementClass::Absent => absent = absent.saturating_add(1),
            }
        }
    }

    if let Some((method, spot, class, _)) = best {
        return SelectionResult {
            target: Some(SelectedTarget {
                plan: make_plan(catalog, method, spot, skill_stat),
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

fn known_targets(method: &GatherMethod) -> Option<&[api::gather_methods::GatherTarget]> {
    match &method.targets {
        Knowledge::Known(rows) => Some(rows),
        Knowledge::Partial { known, .. } => Some(known),
        Knowledge::Unknown(_) => None,
    }
}

fn respawn_max(catalog: &GatherCatalog, method: &GatherMethod, spot: &GatherSpot) -> u32 {
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
    // Keep the catalog's max as-is. The catalog argument is intentionally
    // retained in the signature so future target-specific respawn joins do not
    // need to change the selection call shape.
    let _ = catalog;
    scale.max_ticks
}

fn make_plan(
    catalog: &GatherCatalog,
    method: &GatherMethod,
    spot: &GatherSpot,
    skill_stat: i32,
) -> TargetPlan {
    let (op_slot, op_label) = catalog.op(method).ok().flatten().unwrap_or((1, "Mine"));
    let _ = op_slot;
    let mut products = [0; MAX_PRODUCTS];
    let mut products_len = 0;
    for product in known_rows(&method.products).iter().take(MAX_PRODUCTS) {
        products[products_len] = product.item;
        products_len += 1;
    }
    let alias = catalog
        .alias(spot.entity)
        .or_else(|| catalog.alias(target_entity(method)))
        .unwrap_or(method.id.0.as_ref());
    TargetPlan {
        entity: spot.entity,
        tile: spot.origin,
        op: Arc::from(op_label),
        alias: Arc::from(alias),
        products,
        products_len: products_len as u8,
        skill_stat,
        respawn_max: respawn_max(catalog, method, spot),
        class: known_targets(method)
            .and_then(|rows| rows.iter().find(|target| target.entity == spot.entity))
            .map_or(TargetClass::Resource, |target| target.class),
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

    #[test]
    fn placement_states_follow_scene_evidence_and_avoid_expiry() {
        let world = WorldStateView {
            map_base_x: 3200,
            map_base_z: 3200,
            level: 0,
            ..WorldStateView::default()
        };
        let mut method = GatherMethod {
            id: api::selected::FactKey::new("woodcutting.normal"),
            skill: api::gather_methods::GatherSkill::Woodcutting,
            resources: Arc::from([]),
            targets: Knowledge::Known(Arc::from([api::gather_methods::GatherTarget {
                entity: EntityId::Loc(10),
                op: 1,
                class: TargetClass::Resource,
                respawn: Knowledge::Known(None),
            }])),
            products: Knowledge::Known(Arc::from([])),
            tools: Knowledge::Known(Arc::from([])),
            consumes: Knowledge::Known(Arc::from([])),
            requirements: Knowledge::Known(Arc::from([])),
            spots: Knowledge::Known(Arc::from([])),
        };
        let mut spot = GatherSpot {
            id: api::gather_methods::SpotId(1),
            entity: EntityId::Loc(10),
            origin: WorldTile {
                x: 3304,
                z: 3200,
                level: 0,
            },
            width: 1,
            length: 1,
            movement: Knowledge::Known(None),
            source: api::selected::SourceSpan {
                file: Arc::from("test"),
                first: 1,
                last: 1,
            },
        };
        let mut avoided = [AvoidedTile::EMPTY; MAX_AVOID];
        assert_eq!(
            classify_placement(&spot, &method, &world, &[], &avoided, 1),
            PlacementClass::Unloaded
        );
        spot.origin.x = 3303;
        assert_eq!(
            classify_placement(&spot, &method, &world, &[], &avoided, 1),
            PlacementClass::Absent
        );
        let mut loc = LocView {
            typecode: 0,
            info: 0,
            id: 10,
            name: None,
            description: None,
            actions: vec![],
            tile: spot.origin,
            distance: 0,
            layer: api::snapshot::LocLayer::Ground,
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
        };
        for (id, class, expected) in [
            (10, TargetClass::Resource, PlacementClass::Live),
            (11, TargetClass::Depleted, PlacementClass::Depleted),
            (12, TargetClass::Hazard, PlacementClass::Hazard),
        ] {
            method.targets = Knowledge::Known(Arc::from([api::gather_methods::GatherTarget {
                entity: EntityId::Loc(id),
                op: 1,
                class,
                respawn: Knowledge::Known(None),
            }]));
            loc.id = id;
            assert_eq!(
                classify_placement(
                    &spot,
                    &method,
                    &world,
                    std::slice::from_ref(&loc),
                    &avoided,
                    1
                ),
                expected
            );
        }
        avoided[0] = AvoidedTile {
            tile: spot.origin,
            until: 3,
        };
        assert_eq!(
            classify_placement(
                &spot,
                &method,
                &world,
                std::slice::from_ref(&loc),
                &avoided,
                2
            ),
            PlacementClass::Avoided
        );
        assert_eq!(
            classify_placement(
                &spot,
                &method,
                &world,
                std::slice::from_ref(&loc),
                &avoided,
                3
            ),
            PlacementClass::Hazard
        );
        spot.origin.level = 1;
        assert_eq!(
            classify_placement(
                &spot,
                &method,
                &world,
                std::slice::from_ref(&loc),
                &avoided,
                2
            ),
            PlacementClass::Unloaded
        );
    }
}
