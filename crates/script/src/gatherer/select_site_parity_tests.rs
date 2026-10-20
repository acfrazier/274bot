use super::*;
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
                    reason: "published site key has no selected fishing resource option".to_owned(),
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
                            reason: "site option resolves to a non-fishing method".to_owned(),
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
                            reason: "resolved fishing method has no catalog index".to_owned(),
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
                                Knowledge::Known(Some(bounds)) => ("known", Some((*bounds).into())),
                                Knowledge::Known(None) => ("known_absent", None),
                                Knowledge::Partial { known, .. } => {
                                    ("partial", known.as_ref().map(|bounds| (*bounds).into()))
                                }
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
                        reason: "method placement, target, or movement metadata is incomplete"
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
            let anchor =
                crate::gatherer::widen::site_anchor(catalog, &method_indices, &site.region, &[], 1);
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

fn bank_accesses(world: &nav::world::NavWorld, selected: &SelectedGameData) -> Vec<BankAccess> {
    let placements = selected
        .bank_placements()
        .expect("selected bank placement facts");
    let facts =
        nav::named_banks::resolve(api::named_banks::BANK_CATALOG, &placements.rows, |tile| {
            world.collision.standable(tile)
        });
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

fn nearest_inside_area_stand(world: &nav::world::NavWorld, area: WorkArea) -> Option<WorldTile> {
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
                    mode: super::super::area::AreaMode::Site,
                    anchor: site.anchor,
                    radius: site.radius,
                },
                &[AvoidedTile::EMPTY; super::MAX_AVOID],
                SelectionObservation {
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
                mode: super::super::area::AreaMode::Site,
                anchor: site.anchor,
                radius: site.radius,
            },
            ReturnObservation {
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
            Some("preferred goals are not the exact legacy radius-one legal goals".to_owned()),
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
            Some("a route search exhausted its budget; this is not a NoPath proof".to_owned()),
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
    markdown.push_str("| Published site ID | Key | Methods | Reason |\n|---|---|---|---|\n");
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
    std::fs::write(&markdown_path, markdown_report(report)).expect("write route census Markdown");
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
    let world = super::tests::real_nav_world();
    let lease = super::tests::test_real_nav_lease();
    let catalog = super::tests::real_catalog();
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
            mode: super::super::area::AreaMode::Site,
            anchor: site.anchor,
            radius: site.radius,
        };
        let (nearest, farthest) = nearest_and_farthest_bank(&all_bank_accesses, site.anchor);
        let inside = nearest_inside_area_stand(world, area);
        let mut starts = Vec::<StartCase>::new();
        if let Some(bank) = nearest {
            starts.push(bank_start("site_bank", bank));
        } else {
            gaps.push(CoverageGap {
                site_id: Some(site.site_id.clone()),
                key: Some(site.key.clone()),
                method: None,
                reason: "no real-pack bank access tile exists on the runtime anchor's level"
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
                reason: "no standable tile exists inside the runtime site work area".to_owned(),
            });
        }
        if let Some(bank) = farthest {
            starts.push(bank_start("far_bank", bank));
        } else {
            gaps.push(CoverageGap {
                site_id: Some(site.site_id.clone()),
                key: Some(site.key.clone()),
                method: None,
                reason: "no far-bank access tile exists on the runtime anchor's level".to_owned(),
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
                let pick = pick_target(&catalog, &site, start.tile, lease.collision(), mode);
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
                    super::tests::area_goals(&world.collision, selected_target.plan.tile);
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
