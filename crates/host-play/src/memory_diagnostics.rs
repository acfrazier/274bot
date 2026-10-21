//! Bounded, opt-in diagnostics for memory runs. Never drains application logs.
use api::snapshot::WorldTile;
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::RunState;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Instant;

const GATHERER_RESOURCE_WAIT: &str = "waiting for a resource";
const GATHERER_WOODCUTTING_STAT: usize = 8;
const MAX_GATHERER_CHOP_RECEIPTS: usize = 300;
// A warm Gatherer restart has two cooperative actions (Configure, Construct),
// then host adoption: script/src/slot/compiled.rs::prepare_run / poll_compiled.
// Allow one observed PLAYER_INFO tick per step, not an unbounded worker wait.
const GATHERER_RESTART_TICKS: u64 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GathererRestart {
    Stopped,
    Starting { tick: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DeniedSample {
    tick: u64,
    tile: WorldTile,
    zone: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChopReceipt {
    tick: u64,
    id: i32,
    target: WorldTile,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SafeStandReceipt {
    tick: u64,
    id: i32,
    origin: WorldTile,
    tile: WorldTile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingChop {
    tick: u64,
    id: i32,
    target: WorldTile,
}

struct EligibleTree {
    id: i32,
    origin: WorldTile,
    safe_stands: Vec<WorldTile>,
}

struct GathererGeometry {
    trees: Vec<EligibleTree>,
    candidate_placements: usize,
    excluded_placements: usize,
    world: Option<Arc<nav::world::NavWorld>>,
    filter: Option<nav::zones::ZoneFilterOwned>,
}

#[derive(Default)]
struct GathererBot {
    anchor: Option<WorldTile>,
    last_tick: Option<u64>,
    latest_tile: Option<WorldTile>,
    latest_xp: Option<i32>,
    combat_level: Option<i32>,
    start_tick: Option<u64>,
    start_xp: Option<i32>,
    window_end_tick: Option<u64>,
    window_end_xp: Option<i32>,
    sampled_positions: u32,
    missing_position_samples: u32,
    denied_samples: u32,
    first_denied: Option<DeniedSample>,
    safe_stand_trees: Vec<SafeStandReceipt>,
    deaths: i64,
    deaths_seen: bool,
    native_failure: Option<String>,
    final_run_state: Option<RunState>,
    planned_restart: Option<GathererRestart>,
    final_phase: Option<NativePhase>,
    final_phase_label: Option<Arc<str>>,
    final_event: Option<Arc<str>>,
    native_status_tick: Option<u64>,
    last_status_sample_tick: Option<u64>,
    resource_wait_samples: u32,
    pending_chop: Option<PendingChop>,
    successful_chop_dispatches: u32,
    failed_chop_dispatches: u32,
    window_chop_dispatches: u32,
    chop_receipts: Vec<ChopReceipt>,
}

impl GathererBot {
    fn planned_stop(&mut self) {
        self.planned_restart = Some(GathererRestart::Stopped);
    }

    fn planned_start(&mut self) {
        if self.planned_restart == Some(GathererRestart::Stopped) {
            self.planned_restart = self
                .last_tick
                .map(|tick| GathererRestart::Starting { tick });
        }
    }

    fn observe_run_state(&mut self, state: RunState) {
        self.final_run_state = Some(state);
        if let Some(GathererRestart::Starting { tick }) = self.planned_restart {
            if state == RunState::Running
                && self
                    .last_tick
                    .is_some_and(|now| now.saturating_sub(tick) <= GATHERER_RESTART_TICKS)
            {
                self.planned_restart = None;
            }
        }
    }
}

struct GathererFleet {
    names: Vec<String>,
    budget_ticks: u32,
    radius: i32,
    world: Option<Arc<nav::world::NavWorld>>,
    catalog: Option<Arc<api::gather_methods::GatherCatalog>>,
    anchor: Option<WorldTile>,
    anchor_owner: Option<String>,
    anchor_error: Option<String>,
    geometries: HashMap<i32, Result<Arc<GathererGeometry>, String>>,
    bots: HashMap<String, GathererBot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GathererVerdict {
    Pending,
    Pass { fleet_xp_delta: i64 },
    Fail(String),
}

static GATHERER_FLEET: LazyLock<Mutex<Option<GathererFleet>>> = LazyLock::new(|| Mutex::new(None));
static GATHERER_ENABLED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Slot {
    snapshot: Value,
    logs: VecDeque<String>,
    requests: VecDeque<String>,
    last_capture: Option<Instant>,
    sent_batches: u64,
    request_batches: u64,
}
static SLOTS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
pub(crate) fn clear_gatherer_fleet() {
    GATHERER_ENABLED.store(false, Ordering::Relaxed);
    *GATHERER_FLEET.lock().unwrap() = None;
}

pub(crate) fn configure_gatherer_fleet(
    names: &[String],
    budget_ticks: u32,
    radius: i32,
    world: Option<Arc<nav::world::NavWorld>>,
) {
    let bots = names
        .iter()
        .cloned()
        .map(|name| (name, GathererBot::default()))
        .collect();
    *GATHERER_FLEET.lock().unwrap() = Some(GathererFleet {
        names: names.to_vec(),
        budget_ticks,
        radius,
        world,
        catalog: None,
        anchor: None,
        anchor_owner: None,
        anchor_error: None,
        geometries: HashMap::new(),
        bots,
    });
    GATHERER_ENABLED.store(true, Ordering::Relaxed);
}

pub(crate) fn gatherer_catalog(catalog: Arc<api::gather_methods::GatherCatalog>) {
    let mut all = GATHERER_FLEET.lock().unwrap();
    if let Some(fleet) = all.as_mut() {
        if fleet.catalog.is_none() {
            fleet.catalog = Some(catalog);
        }
    }
}

pub(crate) fn gatherer_anchor(name: &str, c: &client::client::Client) {
    if !GATHERER_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(tile) = current_tile(c) else { return };
    let mut all = GATHERER_FLEET.lock().unwrap();
    let Some(fleet) = all.as_mut() else { return };
    let Some(bot) = fleet.bots.get_mut(name) else {
        return;
    };
    if bot.anchor.is_some() {
        return;
    }
    bot.anchor = Some(tile);
    match fleet.anchor {
        None => {
            fleet.anchor = Some(tile);
            fleet.anchor_owner = Some(name.to_owned());
        }
        Some(anchor) if anchor != tile => {
            let owner = fleet.anchor_owner.as_deref().unwrap_or("unknown");
            fleet.anchor_error.get_or_insert_with(|| {
                format!(
                    "gatherer_anchor_mismatch:{name}:{}:{}:{} differs from {owner}:{}:{}:{}",
                    tile.x, tile.z, tile.level, anchor.x, anchor.z, anchor.level
                )
            });
        }
        Some(_) => {}
    }
}

pub(crate) fn gatherer_started(name: &str) {
    let mut all = GATHERER_FLEET.lock().unwrap();
    let Some(bot) = all.as_mut().and_then(|fleet| fleet.bots.get_mut(name)) else {
        return;
    };
    if bot.start_tick.is_none() {
        if let (Some(tick), Some(xp)) = (bot.last_tick, bot.latest_xp) {
            bot.start_tick = Some(tick);
            bot.start_xp = Some(xp);
        }
    }
}

/// Only the lifecycle harness's own Stop arms a restart allowance.
pub(crate) fn gatherer_planned_stop(name: &str) {
    let mut all = GATHERER_FLEET.lock().unwrap();
    if let Some(bot) = all.as_mut().and_then(|fleet| fleet.bots.get_mut(name)) {
        bot.planned_stop();
    }
}

/// Start the tick bound only after the harness successfully admits its Start.
pub(crate) fn gatherer_planned_start(name: &str) {
    let mut all = GATHERER_FLEET.lock().unwrap();
    if let Some(bot) = all.as_mut().and_then(|fleet| fleet.bots.get_mut(name)) {
        bot.planned_start();
    }
}

pub(crate) fn gatherer_script_error(name: &str, error: &str) {
    let mut all = GATHERER_FLEET.lock().unwrap();
    if let Some(bot) = all.as_mut().and_then(|fleet| fleet.bots.get_mut(name)) {
        bot.native_failure
            .get_or_insert_with(|| format!("script error: {error}"));
    }
}

pub(crate) fn gatherer_native_status(name: &str, state: RunState, status: Option<&ScriptStatus>) {
    let mut all = GATHERER_FLEET.lock().unwrap();
    let Some(fleet) = all.as_mut() else { return };
    let budget_ticks = fleet.budget_ticks;
    let Some(bot) = fleet.bots.get_mut(name) else {
        return;
    };
    bot.observe_run_state(state);
    let Some(status) = status else { return };
    let tick = bot.last_tick;
    bot.native_status_tick = tick;
    bot.final_phase = Some(status.phase);
    bot.final_phase_label = status_arc_text(status, "phase");
    bot.final_event = status_arc_text(status, "last_event");
    if status.card.0 != "Gatherer" {
        bot.native_failure
            .get_or_insert_with(|| format!("unexpected native card {}", status.card.0));
    }
    if let Some(failure) = &status.failure {
        bot.native_failure
            .get_or_insert_with(|| format!("{}: {}", failure.code, failure.message));
    }
    if let Some(deaths) = status_integer(status, "deaths") {
        bot.deaths_seen = true;
        bot.deaths = bot.deaths.max(deaths);
    }
    let new_status_sample = tick.is_some_and(|tick| {
        in_gatherer_window(bot.start_tick, tick, budget_ticks)
            && bot.last_status_sample_tick != Some(tick)
    });
    if new_status_sample {
        bot.last_status_sample_tick = tick;
        if is_resource_wait_sample(
            state,
            status.phase,
            bot.final_phase_label.as_deref(),
            bot.final_event.as_deref(),
        ) {
            bot.resource_wait_samples = bot.resource_wait_samples.saturating_add(1);
        }
    }
}

fn is_resource_wait_sample(
    state: RunState,
    phase: NativePhase,
    phase_label: Option<&str>,
    event: Option<&str>,
) -> bool {
    state == RunState::Running
        && phase == NativePhase::Waiting
        && phase_label == Some("waiting")
        && event == Some(GATHERER_RESOURCE_WAIT)
}

fn status_arc_text(status: &ScriptStatus, key: &str) -> Option<Arc<str>> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(text) => Some(Arc::clone(text)),
            _ => None,
        })
}

fn status_integer(status: &ScriptStatus, key: &str) -> Option<i64> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Integer(value) => Some(*value),
            _ => None,
        })
}

fn in_gatherer_window(start_tick: Option<u64>, tick: u64, budget_ticks: u32) -> bool {
    start_tick
        .is_some_and(|start| tick > start && tick <= start.saturating_add(u64::from(budget_ticks)))
}

fn current_tile(c: &client::client::Client) -> Option<WorldTile> {
    if !c.ingame || c.scene_state != 2 {
        return None;
    }
    let (x, z, level) = crate::player_here_tile(c)?;
    Some(WorldTile { x, z, level })
}

fn geometry_for_combat(
    fleet: &GathererFleet,
    combat: Option<i32>,
) -> Option<&Result<Arc<GathererGeometry>, String>> {
    fleet.geometries.get(&combat?)
}

enum SampledPosition {
    Missing,
    Allowed,
    Denied(DeniedSample),
}

fn build_gatherer_geometry(
    c: &client::client::Client,
    world: &Arc<nav::world::NavWorld>,
    catalog: &api::gather_methods::GatherCatalog,
    anchor: WorldTile,
    combat: i32,
    radius: i32,
) -> Result<Arc<GathererGeometry>, String> {
    let method = catalog
        .method("woodcutting.normal")
        .map_err(|error| format!("gatherer_method_unavailable:{error:?}"))?;
    let targets = match &method.targets {
        api::selected::Knowledge::Known(targets) => targets,
        _ => return Err("gatherer_method_targets_not_complete".to_owned()),
    };
    let target_ids = targets.iter().filter_map(|target| match target.entity {
        api::selected::EntityId::Loc(id)
            if target.class == api::gather_methods::TargetClass::Resource =>
        {
            Some(id)
        }
        _ => None,
    });
    if target_ids.clone().next().is_none() {
        return Err("gatherer_method_has_no_resource_loc_targets".to_owned());
    }
    let region = api::gather_methods::SceneRegionInput {
        min_x: anchor.x.saturating_sub(radius),
        min_z: anchor.z.saturating_sub(radius),
        max_x: anchor.x.saturating_add(radius),
        max_z: anchor.z.saturating_add(radius),
        level: anchor.level,
    };
    let spots = catalog
        .spots(method, &region)
        .map_err(|error| format!("gatherer_method_spots_not_complete:{error:?}"))?;
    let zone_table = world
        .graph
        .zones
        .as_ref()
        .ok_or_else(|| "gatherer_zone_table_unavailable".to_owned())?;
    let zone_filter = nav::zones::ZoneFilterOwned::new(
        zone_table.zones().len(),
        Some(combat),
        &[],
        nav::zones::ZoneExempt::NONE,
    )
    .map_err(|_| "gatherer_zone_filter_allocation_failed".to_owned())?;
    let Some(filter) = zone_filter.view(zone_table) else {
        return Err("gatherer_zone_filter_unavailable".to_owned());
    };
    let mut candidate_placements = 0usize;
    let mut trees = Vec::new();
    for spot in spots {
        let api::selected::EntityId::Loc(id) = spot.entity else {
            continue;
        };
        if !target_ids.clone().any(|target_id| target_id == id) {
            continue;
        }
        candidate_placements = candidate_placements.saturating_add(1);
        let Some(loc) = usize::try_from(id)
            .ok()
            .and_then(|loc_id| c.cache.locs.get(loc_id))
        else {
            continue;
        };
        let Some(approach) = api::query::loc_approach::LocApproach::from_loc_def(
            loc.width,
            loc.length,
            0,
            loc.forceapproach,
        ) else {
            continue;
        };
        if u16::from(approach.width) != spot.width || u16::from(approach.length) != spot.length {
            continue;
        }
        let min_x = spot.origin.x.saturating_sub(1);
        let min_z = spot.origin.z.saturating_sub(1);
        let max_x = spot.origin.x.saturating_add(i32::from(approach.width));
        let max_z = spot.origin.z.saturating_add(i32::from(approach.length));
        let mut safe_stands = Vec::new();
        for x in min_x..=max_x {
            for z in min_z..=max_z {
                let stand = WorldTile {
                    x,
                    z,
                    level: spot.origin.level,
                };
                if world.collision.standable(stand)
                    && approach.can_operate(
                        spot.origin,
                        stand,
                        world.collision.walkable_word(x, z, stand.level) as i32,
                    )
                    && filter
                        .blocking_at(&world.graph.wilderness, stand)
                        .next()
                        .is_none()
                {
                    safe_stands.push(stand);
                }
            }
        }
        if !safe_stands.is_empty() {
            trees.push(EligibleTree {
                id,
                origin: spot.origin,
                safe_stands,
            });
        }
    }
    Ok(Arc::new(GathererGeometry {
        candidate_placements,
        excluded_placements: candidate_placements.saturating_sub(trees.len()),
        trees,
        world: Some(Arc::clone(world)),
        filter: Some(zone_filter),
    }))
}

fn sample_position(
    geometry: Option<&Result<Arc<GathererGeometry>, String>>,
    tick: u64,
    tile: Option<WorldTile>,
) -> SampledPosition {
    let (Some(Ok(geometry)), Some(tile)) = (geometry, tile) else {
        return SampledPosition::Missing;
    };
    let (Some(world), Some(zone_filter), Some(zone_table)) = (
        geometry.world.as_ref(),
        geometry.filter.as_ref(),
        geometry
            .world
            .as_ref()
            .and_then(|world| world.graph.zones.as_ref()),
    ) else {
        return SampledPosition::Missing;
    };
    let Some(filter) = zone_filter.view(zone_table) else {
        return SampledPosition::Missing;
    };
    let blocking = filter.blocking_at(&world.graph.wilderness, tile).next();
    match blocking {
        Some(zone) => SampledPosition::Denied(DeniedSample { tick, tile, zone }),
        None => SampledPosition::Allowed,
    }
}

pub(crate) fn gatherer_frame(c: &client::client::Client, name: &str) {
    if !GATHERER_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let tick = c.gens.player_info;
    let xp = c.stat_xp.get(GATHERER_WOODCUTTING_STAT).copied();
    let combat = c.local_player.as_ref().map(|player| player.combat_level);
    let tile = current_tile(c);
    let mut all = GATHERER_FLEET.lock().unwrap();
    let Some(fleet) = all.as_mut() else { return };
    let Some(bot) = fleet.bots.get(name) else {
        return;
    };
    let budget_ticks = fleet.budget_ticks;
    let new_tick = bot.last_tick != Some(tick);
    let should_sample = new_tick && bot.start_tick.is_some_and(|start| tick > start);
    let combat = combat.filter(|level| *level > 0);
    let anchor = bot.anchor;
    if let (Some(anchor), Some(combat), Some(world), Some(catalog)) =
        (anchor, combat, fleet.world.as_ref(), fleet.catalog.as_ref())
    {
        fleet.geometries.entry(combat).or_insert_with(|| {
            build_gatherer_geometry(c, world, catalog, anchor, combat, fleet.radius)
        });
    }
    let geometry = combat.and_then(|level| fleet.geometries.get(&level));
    let sampled = if should_sample {
        sample_position(geometry, tick, tile)
    } else {
        SampledPosition::Missing
    };
    let Some(bot) = fleet.bots.get_mut(name) else {
        return;
    };
    if new_tick {
        bot.last_tick = Some(tick);
    }
    bot.latest_tile = tile;
    if let Some(xp) = xp {
        bot.latest_xp = Some(xp);
        if bot
            .start_tick
            .is_some_and(|start| tick == start.saturating_add(u64::from(budget_ticks)))
        {
            bot.window_end_tick = Some(tick);
            bot.window_end_xp = Some(xp);
        }
    }
    if let Some(combat) = combat {
        bot.combat_level = Some(combat);
    }
    if should_sample {
        match sampled {
            SampledPosition::Missing => {
                bot.missing_position_samples = bot.missing_position_samples.saturating_add(1);
            }
            SampledPosition::Allowed => {
                bot.sampled_positions = bot.sampled_positions.saturating_add(1);
                if let (Some(Ok(geometry)), Some(tile)) = (geometry, tile) {
                    for tree in &geometry.trees {
                        let already_seen = bot
                            .safe_stand_trees
                            .iter()
                            .any(|receipt| receipt.id == tree.id && receipt.origin == tree.origin);
                        if tree.safe_stands.contains(&tile) && !already_seen {
                            bot.safe_stand_trees.push(SafeStandReceipt {
                                tick,
                                id: tree.id,
                                origin: tree.origin,
                                tile,
                            });
                        }
                    }
                }
            }
            SampledPosition::Denied(denied) => {
                bot.sampled_positions = bot.sampled_positions.saturating_add(1);
                bot.denied_samples = bot.denied_samples.saturating_add(1);
                bot.first_denied.get_or_insert(denied);
            }
        }
    }
}

fn pending_chop(reqs: &[script::shim::InteractReq], tick: u64) -> Option<PendingChop> {
    let [script::shim::InteractReq::Loc {
        id: Some(id),
        x,
        z,
        level,
        action,
    }] = reqs
    else {
        return None;
    };
    if action.as_str() != "Chop down" {
        return None;
    }
    Some(PendingChop {
        tick,
        id: *id,
        target: WorldTile {
            x: *x,
            z: *z,
            level: *level,
        },
    })
}

fn record_chop_dispatch(
    bot: &mut GathererBot,
    pending: Option<PendingChop>,
    wrote: bool,
    budget_ticks: u32,
) {
    let Some(pending) = pending else { return };
    if !wrote {
        bot.failed_chop_dispatches = bot.failed_chop_dispatches.saturating_add(1);
        return;
    }
    bot.successful_chop_dispatches = bot.successful_chop_dispatches.saturating_add(1);
    if !in_gatherer_window(bot.start_tick, pending.tick, budget_ticks) {
        return;
    }
    bot.window_chop_dispatches = bot.window_chop_dispatches.saturating_add(1);
    if bot.chop_receipts.len() < MAX_GATHERER_CHOP_RECEIPTS {
        bot.chop_receipts.push(ChopReceipt {
            tick: pending.tick,
            id: pending.id,
            target: pending.target,
        });
    }
}

pub(crate) fn gatherer_requests(name: &str, reqs: &[script::shim::InteractReq]) {
    if !GATHERER_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let mut all = GATHERER_FLEET.lock().unwrap();
    let Some(fleet) = all.as_mut() else { return };
    let Some(bot) = fleet.bots.get_mut(name) else {
        return;
    };
    bot.pending_chop = bot.last_tick.and_then(|tick| pending_chop(reqs, tick));
}

pub(crate) fn gatherer_sent(name: &str, wrote: bool) {
    if !GATHERER_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let mut all = GATHERER_FLEET.lock().unwrap();
    let Some(fleet) = all.as_mut() else { return };
    let budget_ticks = fleet.budget_ticks;
    let Some(bot) = fleet.bots.get_mut(name) else {
        return;
    };
    let pending = bot.pending_chop.take();
    record_chop_dispatch(bot, pending, wrote, budget_ticks);
}

fn qualifying_chop<'a>(
    geometry: &'a GathererGeometry,
    bot: &GathererBot,
    start_tick: u64,
    end_tick: u64,
) -> Option<&'a EligibleTree> {
    bot.chop_receipts.iter().find_map(|receipt| {
        if receipt.tick <= start_tick || receipt.tick > end_tick {
            return None;
        }
        geometry.trees.iter().find(|tree| {
            tree.id == receipt.id
                && tree.origin == receipt.target
                && bot.safe_stand_trees.iter().any(|stand| {
                    stand.tick > start_tick
                        && stand.tick <= end_tick
                        && stand.id == tree.id
                        && stand.origin == tree.origin
                        && tree.safe_stands.contains(&stand.tile)
                })
        })
    })
}

fn gatherer_verdict(fleet: &GathererFleet) -> GathererVerdict {
    if let Some(error) = &fleet.anchor_error {
        return GathererVerdict::Fail(error.clone());
    }
    if fleet.names.len() < 2 {
        return GathererVerdict::Fail("gatherer_fleet_requires_multiple_bots".to_owned());
    }
    let (Some(anchor), Some(anchor_owner)) = (fleet.anchor, fleet.anchor_owner.as_deref()) else {
        return GathererVerdict::Pending;
    };
    let mut xp_delta = 0i64;
    let mut restarting = false;
    for name in &fleet.names {
        let Some(bot) = fleet.bots.get(name) else {
            return GathererVerdict::Fail(format!("gatherer_bot_missing:{name}"));
        };
        let (Some(start_tick), Some(start_xp)) = (bot.start_tick, bot.start_xp) else {
            return GathererVerdict::Pending;
        };
        let end_tick = start_tick.saturating_add(u64::from(fleet.budget_ticks));
        if bot.last_tick.is_none_or(|tick| tick < end_tick)
            || bot.native_status_tick.is_none_or(|tick| tick < end_tick)
        {
            return GathererVerdict::Pending;
        }
        if bot.window_end_tick != Some(end_tick) {
            return GathererVerdict::Fail(format!(
                "gatherer_window_end_sample_missing:{name}:expected_tick={end_tick}:observed={:?}",
                bot.window_end_tick
            ));
        }
        let Some(end_xp) = bot.window_end_xp else {
            return GathererVerdict::Fail(format!(
                "gatherer_window_xp_missing:{name}:expected_tick={end_tick}"
            ));
        };
        if bot.anchor != Some(anchor) {
            return GathererVerdict::Fail(format!("gatherer_bot_anchor_missing:{name}"));
        }
        if let Some(denied) = bot.first_denied {
            return GathererVerdict::Fail(format!(
                "gatherer_denied_position:{name}:tick={}:tile={},{},{}:zone={}",
                denied.tick, denied.tile.x, denied.tile.z, denied.tile.level, denied.zone
            ));
        }
        if bot.missing_position_samples != 0 || bot.sampled_positions == 0 {
            return GathererVerdict::Fail(format!(
                "gatherer_position_evidence_missing:{name}:sampled={}:missing={}",
                bot.sampled_positions, bot.missing_position_samples
            ));
        }
        if !bot.deaths_seen {
            return GathererVerdict::Fail(format!("gatherer_death_status_missing:{name}"));
        }
        if bot.deaths != 0 {
            return GathererVerdict::Fail(format!("gatherer_death:{name}:{}", bot.deaths));
        }
        if let Some(failure) = &bot.native_failure {
            return GathererVerdict::Fail(format!("gatherer_native_failure:{name}:{failure}"));
        }
        if bot.final_phase == Some(NativePhase::Blocked) {
            return GathererVerdict::Fail(format!("gatherer_final_blocked:{name}"));
        }
        match bot.planned_restart {
            Some(GathererRestart::Stopped)
                if matches!(
                    bot.final_run_state,
                    Some(RunState::Running | RunState::Stopping | RunState::Idle)
                ) =>
            {
                restarting = true;
            }
            Some(GathererRestart::Starting { tick }) => {
                if bot
                    .last_tick
                    .is_none_or(|now| now.saturating_sub(tick) > GATHERER_RESTART_TICKS)
                {
                    return GathererVerdict::Fail(format!(
                        "gatherer_restart_timeout:{name}:start_tick={tick}:last_tick={:?}:budget={GATHERER_RESTART_TICKS}",
                        bot.last_tick
                    ));
                }
                if !matches!(
                    bot.final_run_state,
                    Some(RunState::Starting | RunState::Running)
                ) {
                    return GathererVerdict::Fail(format!(
                        "gatherer_not_running:{name}:{:?}",
                        bot.final_run_state
                    ));
                }
                restarting = true;
            }
            _ if bot.final_run_state != Some(RunState::Running) => {
                return GathererVerdict::Fail(format!(
                    "gatherer_not_running:{name}:{:?}",
                    bot.final_run_state
                ));
            }
            _ => {}
        }
        let delta = i64::from(end_xp) - i64::from(start_xp);
        if delta < 0 {
            return GathererVerdict::Fail(format!("gatherer_negative_xp_delta:{name}:{delta}"));
        }
        xp_delta = xp_delta.saturating_add(delta);
        let Some(combat) = bot.combat_level else {
            return GathererVerdict::Fail(format!("gatherer_combat_level_missing:{name}"));
        };
        let Some(Ok(geometry)) = geometry_for_combat(fleet, Some(combat)) else {
            return GathererVerdict::Fail(format!("gatherer_geometry_missing:{name}"));
        };
        if geometry.trees.is_empty() {
            return GathererVerdict::Fail(format!(
                "gatherer_no_eligible_safe_tree:{name}:anchor={},{},{}:combat={combat}",
                anchor.x, anchor.z, anchor.level
            ));
        }
        if qualifying_chop(geometry, bot, start_tick, end_tick).is_none() {
            return GathererVerdict::Fail(format!("gatherer_no_successful_safe_chop:{name}"));
        }
        if delta == 0 && bot.resource_wait_samples == 0 {
            return GathererVerdict::Fail(format!(
                "gatherer_zero_xp_without_resource_wait:{name}:wait_samples={}",
                bot.resource_wait_samples
            ));
        }
    }
    if xp_delta <= 0 {
        return GathererVerdict::Fail(format!(
            "gatherer_fleet_zero_xp:anchor_owner={anchor_owner}:delta={xp_delta}"
        ));
    }
    if restarting {
        return GathererVerdict::Pending;
    }
    GathererVerdict::Pass {
        fleet_xp_delta: xp_delta,
    }
}

pub(crate) fn gatherer_verdict_current() -> Option<GathererVerdict> {
    let all = GATHERER_FLEET.lock().unwrap();
    all.as_ref().map(gatherer_verdict)
}

pub(crate) fn gatherer_verdict_json() -> Option<(GathererVerdict, Value)> {
    let all = GATHERER_FLEET.lock().unwrap();
    let fleet = all.as_ref()?;
    let verdict = gatherer_verdict(fleet);
    let bots: Vec<Value> = fleet
        .names
        .iter()
        .filter_map(|name| {
            let bot = fleet.bots.get(name)?;
            let geometry = geometry_for_combat(fleet, bot.combat_level);
            let (tree_count, candidate_placements, excluded_placements, geometry_error) =
                match geometry {
                    Some(Ok(geometry)) => (
                        geometry.trees.len(),
                        geometry.candidate_placements,
                        geometry.excluded_placements,
                        Value::Null,
                    ),
                    Some(Err(error)) => (0, 0, 0, json!(error)),
                    None => (0, 0, 0, Value::Null),
                };
            let first_denied = bot.first_denied.map(|denied| {
                json!({
                    "tick": denied.tick,
                    "tile": [denied.tile.x, denied.tile.z, denied.tile.level],
                    "zone_id": denied.zone,
                })
            });
            let safe_stands: Vec<Value> = bot
                .safe_stand_trees
                .iter()
                .map(|stand| {
                    json!({
                        "tick": stand.tick,
                        "loc_id": stand.id,
                        "origin": [stand.origin.x, stand.origin.z, stand.origin.level],
                        "tile": [stand.tile.x, stand.tile.z, stand.tile.level],
                    })
                })
                .collect();
            let successful_chops: Vec<Value> = bot
                .chop_receipts
                .iter()
                .map(|receipt| {
                    json!({
                        "tick": receipt.tick,
                        "loc_id": receipt.id,
                        "target": [receipt.target.x, receipt.target.z, receipt.target.level],
                    })
                })
                .collect();
            Some(json!({
                "name": name,
                "anchor": bot.anchor.map(|tile| [tile.x, tile.z, tile.level]),
                "start_tick": bot.start_tick,
                "window_start_tick": bot.start_tick.map(|tick| tick.saturating_add(1)),
                "window_end_tick": bot.window_end_tick,
                "last_tick": bot.last_tick,
                "tick_budget": fleet.budget_ticks,
                "combat_level": bot.combat_level,
                "xp_start": bot.start_xp,
                "xp_end": bot.window_end_xp,
                "xp_latest": bot.latest_xp,
                "xp_delta": bot.start_xp.zip(bot.window_end_xp).map(|(start, end)| i64::from(end) - i64::from(start)),
                "sampled_engine_ticks": bot.sampled_positions,
                "missing_position_samples": bot.missing_position_samples,
                "denied_positions": bot.denied_samples,
                "first_denied_position": first_denied,
                "deaths": bot.deaths,
                "deaths_seen": bot.deaths_seen,
                "native_failure": bot.native_failure.as_deref(),
                "run_state": bot.final_run_state.map(|state| format!("{state:?}")),
                "native_phase": bot.final_phase.map(|phase| format!("{phase:?}")),
                "native_phase_label": bot.final_phase_label.as_deref(),
                "last_event": bot.final_event.as_deref(),
                "native_status_tick": bot.native_status_tick,
                "resource_wait_samples": bot.resource_wait_samples,
                "successful_chop_dispatches": bot.successful_chop_dispatches,
                "failed_chop_dispatches": bot.failed_chop_dispatches,
                "window_chop_dispatches": bot.window_chop_dispatches,
                "safe_stand_receipts": safe_stands,
                "safe_stand_tree_count": bot.safe_stand_trees.len(),
                "successful_chops": successful_chops,
                "eligible_safe_tree_count": tree_count,
                "candidate_tree_placements": candidate_placements,
                "excluded_placements": excluded_placements,
                "geometry_error": geometry_error,
            }))
        })
        .collect();
    let summary = json!({
        "kind": "gatherer_fleet_qualification",
        "geometry_authority": "NavWorld collision + LocApproach::can_operate + ZoneFilter::blocking_at",
        "anchor": fleet.anchor.map(|tile| [tile.x, tile.z, tile.level]),
        "anchor_owner": fleet.anchor_owner.as_deref(),
        "anchor_error": fleet.anchor_error.as_deref(),
        "work_area_radius": fleet.radius,
        "tick_budget": fleet.budget_ticks,
        "fleet_xp_delta": match &verdict {
            GathererVerdict::Pass { fleet_xp_delta } => Some(*fleet_xp_delta),
            _ => None,
        },
        "verdict": match &verdict {
            GathererVerdict::Pending => json!("pending"),
            GathererVerdict::Pass { .. } => json!("pass"),
            GathererVerdict::Fail(error) => json!({"fail": error}),
        },
        "bots": bots,
    });
    Some((verdict, summary))
}

pub(crate) fn enable(names: &[String]) {
    let _ = SLOTS.set(Mutex::new(
        names.iter().map(|n| (n.clone(), Slot::default())).collect(),
    ));
}
fn bounded_push(queue: &mut VecDeque<String>, text: &str) {
    if queue.len() == 32 {
        queue.pop_front();
    }
    queue.push_back(text.chars().take(512).collect());
}
pub(crate) fn logs(name: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let Some(all) = SLOTS.get() else { return };
    if let Some(slot) = all.lock().unwrap().get_mut(name) {
        for line in lines {
            bounded_push(&mut slot.logs, line)
        }
    }
}
pub(crate) fn requests(name: &str, reqs: &[script::shim::InteractReq]) {
    gatherer_requests(name, reqs);
    if reqs.is_empty() {
        return;
    }
    let Some(all) = SLOTS.get() else { return };
    if let Some(slot) = all.lock().unwrap().get_mut(name) {
        slot.request_batches += 1;
        for req in reqs {
            bounded_push(&mut slot.requests, &format!("{req:?}"))
        }
    }
}
pub(crate) fn sent(name: &str, wrote: bool) {
    gatherer_sent(name, wrote);
    let Some(all) = SLOTS.get() else { return };
    if wrote {
        if let Some(slot) = all.lock().unwrap().get_mut(name) {
            slot.sent_batches += 1;
        }
    }
}
pub(crate) fn frame(c: &client::client::Client, name: &str, hold: bool) {
    let Some(all) = SLOTS.get() else { return };
    let mut all = all.lock().unwrap();
    let Some(slot) = all.get_mut(name) else {
        return;
    };
    if slot
        .last_capture
        .is_some_and(|t| t.elapsed().as_millis() < 1000)
    {
        return;
    }
    let guards: Vec<_>=c.npc_ids.iter().take(c.npc_count.max(0) as usize).filter_map(|index| {
        let npc=c.npc.get(*index as usize)?.as_ref()?;
        let kind=c.cache.npcs.get(npc.r#type?)?;
        if kind.name != "Guard" {return None}
        Some(json!({"index":index,"type":npc.r#type,"x":c.map_build_base_x+npc.route_x[0],"z":c.map_build_base_z+npc.route_z[0],"face_entity":npc.face_entity,"animation":npc.primary_anim}))
    }).take(16).collect();
    slot.snapshot = json!({"client_tick":c.gens.player,"loop_cycle":c.loop_cycle,
        "xp":c.stat_xp.get(17),"hp":c.stat_effective_level.get(3),
        "position":crate::player_here_tile(c),"ingame":c.ingame,"scene_state":c.scene_state,
        "drawing":c.draw,"hold":hold,"animation":c.local_player.as_ref().map(|p|p.primary_anim),
        "spot_animation":c.local_player.as_ref().map(|p|p.spotanim_id),
        "spot_animation_start_cycle":c.local_player.as_ref().map(|p|p.spotanim_last_cycle),
        "face_entity":c.local_player.as_ref().map(|p|p.face_entity),
        "chat":c.chat_text.first(),"guards":guards});
    slot.last_capture = Some(Instant::now());
}
pub(crate) fn sample(name: &str) -> Value {
    let Some(all) = SLOTS.get() else {
        return Value::Null;
    };
    let all = all.lock().unwrap();
    let Some(slot) = all.get(name) else {
        return Value::Null;
    };
    // Detailed navigation history belongs to the excluded campaign capture hooks.
    json!({"client":slot.snapshot,"snapshot_age_ms":slot.last_capture.map(|t|t.elapsed().as_millis()),"recent_logs":slot.logs,"recent_requests":slot.requests,"recent_navigation":Value::Null,"request_batches":slot.request_batches,"sent_batches":slot.sent_batches})
}
#[cfg(test)]
mod tests {
    use super::*;

    const R2_STARTS: [u64; 10] = [30, 31, 32, 29, 31, 31, 31, 29, 30, 30];
    const R2_XP: [i32; 10] = [175, 0, 125, 0, 25, 75, 0, 75, 75, 125];
    const R2_WAITS: [u32; 10] = [68, 71, 72, 70, 71, 71, 72, 72, 71, 72];
    const R2_CHOPS: [u64; 10] = [63, 173, 175, 169, 39, 176, 56, 51, 53, 172];
    const R2_STAND_TICKS: [u64; 10] = [65, 173, 177, 169, 41, 178, 56, 51, 55, 172];
    const R3_STARTS: [u64; 10] = [32, 32, 32, 32, 30, 31, 32, 29, 31, 32];
    const R3_XP: [i32; 10] = [225, 0, 0, 50, 25, 0, 0, 50, 75, 225];
    const R3_WAITS: [u32; 10] = [60, 72, 71, 66, 69, 72, 66, 68, 68, 64];
    const R3_CHOPS: [u64; 10] = [35, 35, 35, 35, 33, 36, 35, 34, 36, 35];
    const R3_STAND_TICKS: [u64; 10] = [35, 37, 35, 35, 35, 36, 37, 34, 36, 35];
    const R2_TREE_IDS: [i32; 10] = [1278; 10];
    const R2_TREE_TILES: [(i32, i32); 10] = [(3103, 3222); 10];
    const R2_STAND_TILES: [(i32, i32); 10] = [(3104, 3224); 10];
    const R3_TREE_IDS: [i32; 10] = [1276; 10];
    const R3_TREE_TILES: [(i32, i32); 10] = [
        (3107, 3229),
        (3111, 3226),
        (3111, 3226),
        (3107, 3229),
        (3107, 3229),
        (3105, 3228),
        (3107, 3229),
        (3107, 3229),
        (3107, 3229),
        (3107, 3229),
    ];
    const R3_STAND_TILES: [(i32, i32); 10] = [
        (3108, 3228),
        (3110, 3227),
        (3110, 3227),
        (3108, 3228),
        (3108, 3228),
        (3107, 3228),
        (3108, 3228),
        (3108, 3228),
        (3108, 3228),
        (3108, 3228),
    ];

    // Compact typed replay of `oracle/r2-oracle.json` and
    // `live-raw/danger-select-n10-r3/oracle.json`. Safe stand ticks and tiles
    // are the first matching samples in each run's samples.diagnostics.jsonl;
    // eligibility.json authenticates the tree access tiles. All retained
    // observations keep their original client tick and per-bot XP values.
    struct RetainedFleetReceipt<'a> {
        prefix: &'a str,
        starts: &'a [u64; 10],
        xp: &'a [i32; 10],
        waits: &'a [u32; 10],
        chop_ticks: &'a [u64; 10],
        tree_ids: &'a [i32; 10],
        tree_tiles: &'a [(i32, i32); 10],
        stand_ticks: &'a [u64; 10],
        stand_tiles: &'a [(i32, i32); 10],
        phase: NativePhase,
        phase_label: &'a str,
        event: &'a str,
    }

    fn retained_fleet(receipt: RetainedFleetReceipt<'_>) -> GathererFleet {
        let RetainedFleetReceipt {
            prefix,
            starts,
            xp,
            waits,
            chop_ticks,
            tree_ids,
            tree_tiles,
            stand_ticks,
            stand_tiles,
            phase,
            phase_label,
            event,
        } = receipt;
        let anchor = WorldTile {
            x: 3109,
            z: 3228,
            level: 0,
        };
        let names: Vec<String> = (0..10).map(|index| format!("{prefix}_{index}")).collect();
        let mut bots = HashMap::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            let name = name.clone();
            let start = starts[index];
            let end = start + 300;
            let origin = WorldTile {
                x: tree_tiles[index].0,
                z: tree_tiles[index].1,
                level: 0,
            };
            bots.insert(
                name,
                GathererBot {
                    anchor: Some(anchor),
                    last_tick: Some(end),
                    latest_tile: Some(anchor),
                    latest_xp: Some(xp[index]),
                    combat_level: Some(3),
                    start_tick: Some(start),
                    start_xp: Some(0),
                    window_end_tick: Some(end),
                    window_end_xp: Some(xp[index]),
                    sampled_positions: 1,
                    deaths_seen: true,
                    final_run_state: Some(RunState::Running),
                    final_phase: Some(phase),
                    final_phase_label: Some(Arc::from(phase_label)),
                    final_event: Some(Arc::from(event)),
                    native_status_tick: Some(end),
                    last_status_sample_tick: Some(end),
                    resource_wait_samples: waits[index],
                    successful_chop_dispatches: 1,
                    window_chop_dispatches: 1,
                    safe_stand_trees: vec![SafeStandReceipt {
                        tick: stand_ticks[index],
                        id: tree_ids[index],
                        origin,
                        tile: WorldTile {
                            x: stand_tiles[index].0,
                            z: stand_tiles[index].1,
                            level: 0,
                        },
                    }],
                    chop_receipts: vec![ChopReceipt {
                        tick: chop_ticks[index],
                        id: tree_ids[index],
                        target: origin,
                    }],
                    ..GathererBot::default()
                },
            );
        }
        let tree = |id, x, z, safe_stands: &[(i32, i32)]| EligibleTree {
            id,
            origin: WorldTile { x, z, level: 0 },
            safe_stands: safe_stands
                .iter()
                .map(|(x, z)| WorldTile {
                    x: *x,
                    z: *z,
                    level: 0,
                })
                .collect(),
        };
        let geometry = Arc::new(GathererGeometry {
            trees: vec![
                tree(1278, 3103, 3222, &[(3102, 3222), (3104, 3224)]),
                tree(1276, 3107, 3229, &[(3107, 3228), (3108, 3228)]),
                tree(1276, 3111, 3226, &[(3110, 3226), (3110, 3227)]),
                tree(
                    1276,
                    3105,
                    3228,
                    &[(3104, 3228), (3105, 3227), (3107, 3228)],
                ),
            ],
            candidate_placements: 4,
            excluded_placements: 0,
            world: None,
            filter: None,
        });
        let names_for_owner = names[0].clone();
        let geometries = HashMap::from([(3, Ok(geometry))]);
        GathererFleet {
            names,
            budget_ticks: 300,
            radius: 12,
            world: None,
            catalog: None,
            anchor: Some(anchor),
            anchor_owner: Some(names_for_owner),
            anchor_error: None,
            geometries,
            bots,
        }
    }

    fn r2_fleet() -> GathererFleet {
        retained_fleet(RetainedFleetReceipt {
            prefix: "gm1d8f7qgp",
            starts: &R2_STARTS,
            xp: &R2_XP,
            waits: &R2_WAITS,
            chop_ticks: &R2_CHOPS,
            tree_ids: &R2_TREE_IDS,
            tree_tiles: &R2_TREE_TILES,
            stand_ticks: &R2_STAND_TICKS,
            stand_tiles: &R2_STAND_TILES,
            phase: NativePhase::Working,
            phase_label: "working",
            event: "gathering",
        })
    }

    fn r3_fleet() -> GathererFleet {
        retained_fleet(RetainedFleetReceipt {
            prefix: "gm1dorefg4",
            starts: &R3_STARTS,
            xp: &R3_XP,
            waits: &R3_WAITS,
            chop_ticks: &R3_CHOPS,
            tree_ids: &R3_TREE_IDS,
            tree_tiles: &R3_TREE_TILES,
            stand_ticks: &R3_STAND_TICKS,
            stand_tiles: &R3_STAND_TILES,
            phase: NativePhase::Waiting,
            phase_label: "waiting",
            event: GATHERER_RESOURCE_WAIT,
        })
    }

    fn failure(fleet: &GathererFleet) -> String {
        match gatherer_verdict(fleet) {
            GathererVerdict::Fail(error) => error,
            verdict => panic!("expected failure, got {verdict:?}"),
        }
    }

    #[test]
    fn retained_r2_and_r3_fleets_qualify_on_exact_300_tick_receipts() {
        let r2 = r2_fleet();
        assert_eq!(
            gatherer_verdict(&r2),
            GathererVerdict::Pass {
                fleet_xp_delta: 675
            }
        );
        let r3 = r3_fleet();
        assert_eq!(
            gatherer_verdict(&r3),
            GathererVerdict::Pass {
                fleet_xp_delta: 650
            }
        );
        for fleet in [&r2, &r3] {
            for name in &fleet.names {
                let bot = &fleet.bots[name];
                let start = bot.start_tick.unwrap();
                let end = start + u64::from(fleet.budget_ticks);
                assert_eq!(bot.window_end_tick, Some(end));
                assert_eq!(bot.last_tick, Some(end));
                assert_eq!(bot.native_status_tick, Some(end));
                assert!(bot.safe_stand_trees[0].tick > start);
                assert!(bot.safe_stand_trees[0].tick <= end);
                if bot.window_end_xp == Some(0) {
                    assert!(bot.resource_wait_samples > 0);
                }
            }
        }
        let r2_after_request = &r2.bots["gm1d8f7qgp_4"];
        assert!(r2_after_request.chop_receipts[0].tick < r2_after_request.safe_stand_trees[0].tick);
        assert!(qualifying_chop(
            r2.geometries[&3].as_ref().unwrap(),
            r2_after_request,
            r2_after_request.start_tick.unwrap(),
            r2_after_request.window_end_tick.unwrap(),
        )
        .is_some());
    }

    #[test]
    fn planned_lifecycle_restart_qualifies_but_unplanned_or_overdue_starting_fails() {
        let mut fleet = r2_fleet();
        let name = "gm1d8f7qgp_0";
        let passed = GathererVerdict::Pass {
            fleet_xp_delta: 675,
        };
        assert_eq!(gatherer_verdict(&fleet), passed);
        let start = fleet.bots[name].last_tick.unwrap() + 100;
        {
            let bot = fleet.bots.get_mut(name).unwrap();
            bot.planned_stop();
            bot.last_tick = Some(start);
            bot.observe_run_state(RunState::Idle);
        }
        fleet.bots.get_mut(name).unwrap().planned_start();
        for elapsed in 0..=GATHERER_RESTART_TICKS {
            let bot = fleet.bots.get_mut(name).unwrap();
            bot.last_tick = Some(start + elapsed);
            bot.observe_run_state(RunState::Starting);
            assert_eq!(gatherer_verdict(&fleet), GathererVerdict::Pending);
        }
        // A denied position, death, native failure, missing position or Blocked
        // phase must still fail during the planned restart.
        for bad in 0..5 {
            let bot = fleet.bots.get_mut(name).unwrap();
            match bad {
                0 => bot.missing_position_samples = 1,
                1 => bot.deaths = 1,
                2 => bot.native_failure = Some("restart failure".into()),
                3 => bot.final_phase = Some(NativePhase::Blocked),
                _ => {
                    bot.first_denied = Some(DeniedSample {
                        tick: start,
                        tile: WorldTile {
                            x: 3104,
                            z: 3229,
                            level: 0,
                        },
                        zone: 514,
                    })
                }
            }
            assert!(matches!(gatherer_verdict(&fleet), GathererVerdict::Fail(_)));
            let bot = fleet.bots.get_mut(name).unwrap();
            bot.missing_position_samples = 0;
            bot.deaths = 0;
            bot.native_failure = None;
            bot.final_phase = Some(NativePhase::Working);
            bot.first_denied = None;
        }
        fleet
            .bots
            .get_mut(name)
            .unwrap()
            .observe_run_state(RunState::Running);
        assert_eq!(gatherer_verdict(&fleet), passed);
        assert_eq!(fleet.bots[name].planned_restart, None);

        // The allowance ends at Running. A later unplanned Starting fails.
        fleet
            .bots
            .get_mut(name)
            .unwrap()
            .observe_run_state(RunState::Starting);
        assert!(failure(&fleet).starts_with("gatherer_not_running:"));
        // A Start without a harness Stop cannot grant an allowance either.
        fleet.bots.get_mut(name).unwrap().planned_start();
        assert!(failure(&fleet).starts_with("gatherer_not_running:"));

        let bot = fleet.bots.get_mut(name).unwrap();
        bot.planned_stop();
        bot.observe_run_state(RunState::Idle);
        assert_eq!(gatherer_verdict(&fleet), GathererVerdict::Pending);
        let bot = fleet.bots.get_mut(name).unwrap();
        bot.planned_start();
        let restart = bot.last_tick.unwrap();
        bot.observe_run_state(RunState::Starting);
        assert_eq!(gatherer_verdict(&fleet), GathererVerdict::Pending);
        let bot = fleet.bots.get_mut(name).unwrap();
        bot.last_tick = Some(restart + GATHERER_RESTART_TICKS + 1);
        bot.observe_run_state(RunState::Starting);
        assert!(failure(&fleet).starts_with("gatherer_restart_timeout:"));
        // A late Running cannot erase an overrun.
        fleet
            .bots
            .get_mut(name)
            .unwrap()
            .observe_run_state(RunState::Running);
        assert!(failure(&fleet).starts_with("gatherer_restart_timeout:"));
    }

    #[test]
    fn chop_receipt_requires_one_exact_chop_down_request() {
        use script::shim::InteractReq;

        let request = || InteractReq::Loc {
            id: Some(1278),
            x: 3103,
            z: 3222,
            level: 0,
            action: "Chop down".to_owned(),
        };
        assert_eq!(
            pending_chop(&[request()], 63),
            Some(PendingChop {
                tick: 63,
                id: 1278,
                target: WorldTile {
                    x: 3103,
                    z: 3222,
                    level: 0,
                },
            })
        );
        let wrong_action = [InteractReq::Loc {
            id: Some(1278),
            x: 3103,
            z: 3222,
            level: 0,
            action: "Examine".to_owned(),
        }];
        assert!(pending_chop(&wrong_action, 63).is_none());
        let multiple = [request(), request()];
        assert!(pending_chop(&multiple, 63).is_none());
    }

    #[test]
    fn only_a_successfully_sent_chop_is_a_dispatch_receipt() {
        let mut bot = GathererBot {
            start_tick: Some(30),
            ..GathererBot::default()
        };
        let pending = PendingChop {
            tick: 63,
            id: 1278,
            target: WorldTile {
                x: 3103,
                z: 3222,
                level: 0,
            },
        };
        record_chop_dispatch(&mut bot, Some(pending), false, 300);
        assert_eq!(bot.failed_chop_dispatches, 1);
        assert!(bot.chop_receipts.is_empty());
        record_chop_dispatch(&mut bot, Some(pending), true, 300);
        assert_eq!(bot.successful_chop_dispatches, 1);
        assert_eq!(bot.window_chop_dispatches, 1);
        assert_eq!(bot.chop_receipts.len(), 1);
    }

    #[test]
    fn denied_positions_deaths_native_failures_and_blocked_phase_fail_closed() {
        let mut denied = r2_fleet();
        denied.bots.get_mut("gm1d8f7qgp_0").unwrap().first_denied = Some(DeniedSample {
            tick: 51,
            tile: WorldTile {
                x: 3104,
                z: 3229,
                level: 0,
            },
            zone: 514,
        });
        assert!(failure(&denied).starts_with("gatherer_denied_position:"));

        let mut dead = r2_fleet();
        dead.bots.get_mut("gm1d8f7qgp_0").unwrap().deaths = 1;
        assert!(failure(&dead).starts_with("gatherer_death:"));

        let mut missing_death_status = r2_fleet();
        missing_death_status
            .bots
            .get_mut("gm1d8f7qgp_0")
            .unwrap()
            .deaths_seen = false;
        assert!(failure(&missing_death_status).starts_with("gatherer_death_status_missing:"));

        let mut native_failure = r2_fleet();
        native_failure
            .bots
            .get_mut("gm1d8f7qgp_0")
            .unwrap()
            .native_failure = Some("native error".to_owned());
        assert!(failure(&native_failure).starts_with("gatherer_native_failure:"));

        let mut blocked = r2_fleet();
        blocked.bots.get_mut("gm1d8f7qgp_0").unwrap().final_phase = Some(NativePhase::Blocked);
        assert!(failure(&blocked).starts_with("gatherer_final_blocked:"));

        for (sampled, missing) in [(1, 1), (0, 0)] {
            let mut missing_position = r2_fleet();
            let bot = missing_position.bots.get_mut("gm1d8f7qgp_0").unwrap();
            bot.sampled_positions = sampled;
            bot.missing_position_samples = missing;
            assert!(failure(&missing_position).starts_with("gatherer_position_evidence_missing:"));
        }
    }

    #[test]
    fn safety_failures_after_the_proof_budget_still_fail_the_cell() {
        let mut denied = r2_fleet();
        let bot = denied.bots.get_mut("gm1d8f7qgp_0").unwrap();
        let after_budget = bot.window_end_tick.unwrap() + 1;
        bot.last_tick = Some(after_budget);
        bot.native_status_tick = Some(after_budget);
        bot.first_denied = Some(DeniedSample {
            tick: after_budget,
            tile: WorldTile {
                x: 3104,
                z: 3229,
                level: 0,
            },
            zone: 514,
        });
        assert!(failure(&denied).starts_with("gatherer_denied_position:"));

        let mut blocked = r2_fleet();
        let bot = blocked.bots.get_mut("gm1d8f7qgp_0").unwrap();
        bot.last_tick = Some(after_budget);
        bot.native_status_tick = Some(after_budget);
        bot.final_phase = Some(NativePhase::Blocked);
        assert!(failure(&blocked).starts_with("gatherer_final_blocked:"));

        let mut dead = r2_fleet();
        let bot = dead.bots.get_mut("gm1d8f7qgp_0").unwrap();
        bot.last_tick = Some(after_budget);
        bot.native_status_tick = Some(after_budget);
        bot.deaths = 1;
        assert!(failure(&dead).starts_with("gatherer_death:"));
    }

    #[test]
    fn missing_unsafe_or_late_chop_receipts_fail_closed() {
        let mut no_safe_stand = r2_fleet();
        no_safe_stand
            .bots
            .get_mut("gm1d8f7qgp_0")
            .unwrap()
            .safe_stand_trees
            .clear();
        assert!(failure(&no_safe_stand).starts_with("gatherer_no_successful_safe_chop:"));

        let mut fake_safe_stand = r2_fleet();
        let bot = fake_safe_stand.bots.get_mut("gm1d8f7qgp_0").unwrap();
        bot.safe_stand_trees[0].tile = bot.safe_stand_trees[0].origin;
        assert!(failure(&fake_safe_stand).starts_with("gatherer_no_successful_safe_chop:"));

        let mut no_sent_chop = r2_fleet();
        {
            let bot = no_sent_chop.bots.get_mut("gm1d8f7qgp_0").unwrap();
            bot.chop_receipts.clear();
            bot.window_chop_dispatches = 0;
            record_chop_dispatch(
                bot,
                Some(PendingChop {
                    tick: 63,
                    id: 1278,
                    target: WorldTile {
                        x: 3103,
                        z: 3222,
                        level: 0,
                    },
                }),
                false,
                300,
            );
            assert_eq!(bot.failed_chop_dispatches, 1);
        }
        assert!(failure(&no_sent_chop).starts_with("gatherer_no_successful_safe_chop:"));

        let mut late_chop = r2_fleet();
        let bot = late_chop.bots.get_mut("gm1d8f7qgp_0").unwrap();
        bot.chop_receipts[0].tick = bot.window_end_tick.unwrap() + 1;
        assert!(failure(&late_chop).starts_with("gatherer_no_successful_safe_chop:"));
    }

    #[test]
    fn zero_xp_requires_exact_resource_wait_and_positive_fleet_xp() {
        assert!(is_resource_wait_sample(
            RunState::Running,
            NativePhase::Waiting,
            Some("waiting"),
            Some("waiting for a resource"),
        ));
        assert!(!is_resource_wait_sample(
            RunState::Running,
            NativePhase::Waiting,
            Some("waiting"),
            Some("waiting"),
        ));
        assert!(!is_resource_wait_sample(
            RunState::Running,
            NativePhase::Working,
            Some("working"),
            Some("waiting for a resource"),
        ));
        assert!(!is_resource_wait_sample(
            RunState::Idle,
            NativePhase::Waiting,
            Some("waiting"),
            Some("waiting for a resource"),
        ));
        assert!(!is_resource_wait_sample(
            RunState::Paused,
            NativePhase::Waiting,
            Some("waiting"),
            Some("waiting for a resource"),
        ));

        let mut generic_wait = r2_fleet();
        let bot = generic_wait.bots.get_mut("gm1d8f7qgp_1").unwrap();
        bot.resource_wait_samples = 0;
        bot.final_event = Some(Arc::from("idle"));
        assert!(failure(&generic_wait).starts_with("gatherer_zero_xp_without_resource_wait:"));

        let mut no_fleet_xp = r2_fleet();
        for bot in no_fleet_xp.bots.values_mut() {
            bot.window_end_xp = bot.start_xp;
            bot.resource_wait_samples = 1;
        }
        assert!(failure(&no_fleet_xp).starts_with("gatherer_fleet_zero_xp:"));

        let mut idle = r2_fleet();
        idle.bots.get_mut("gm1d8f7qgp_0").unwrap().final_run_state = Some(RunState::Idle);
        assert!(failure(&idle).starts_with("gatherer_not_running:"));
    }

    #[test]
    fn incomplete_window_evidence_is_pending_or_named_missing_evidence() {
        let mut pending = r2_fleet();
        let bot = pending.bots.get_mut("gm1d8f7qgp_0").unwrap();
        bot.start_tick = None;
        bot.start_xp = None;
        assert_eq!(gatherer_verdict(&pending), GathererVerdict::Pending);

        let mut missing_end = r2_fleet();
        let bot = missing_end.bots.get_mut("gm1d8f7qgp_0").unwrap();
        bot.window_end_tick = None;
        assert!(failure(&missing_end).starts_with("gatherer_window_end_sample_missing:"));

        let mut missing_xp = r2_fleet();
        missing_xp
            .bots
            .get_mut("gm1d8f7qgp_0")
            .unwrap()
            .window_end_xp = None;
        assert!(failure(&missing_xp).starts_with("gatherer_window_xp_missing:"));
    }

    #[test]
    fn logs_remain_bounded_and_keep_latest_utf8_messages() {
        let mut queue = std::collections::VecDeque::new();
        for i in 0..100 {
            super::bounded_push(&mut queue, &format!("{i}:{}", "é".repeat(1024)));
        }
        assert_eq!(queue.len(), 32);
        assert!(queue.front().unwrap().starts_with("68:"));
        assert!(queue.back().unwrap().starts_with("99:"));
        assert!(queue.iter().all(|s| s.chars().count() == 512));
    }
}
