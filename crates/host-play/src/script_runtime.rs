//! Per-slot script observe/dispatch/hold transaction and script navigation continuation.

use super::*;
#[path = "script_bank.rs"]
mod bank;
use bank::dispatch_observed_bank_op;
#[cfg(all(target_os = "linux", feature = "test-support"))]
pub use bank::harness::measures_captured_native_bank_pick_cpu;
pub(crate) use bank::open_bank_at_here;
#[cfg(any(test, all(feature = "live-harness", feature = "test-support")))]
pub(crate) use bank::BankPickWorkObservation;
#[cfg(test)]
pub(super) fn poll_bank_pick_for_test(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    bank::poll_bank_pick(navs, name, Instant::now());
}
#[cfg(test)]
pub(super) fn dispatch_observed_bank_op_for_test(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    req: &script::shim::InteractReq,
) -> Option<script::slot::PendingBankOp> {
    dispatch_observed_bank_op(driver, snapshot, obj_names, None, req)
}
#[path = "script_nav.rs"]
mod script_nav;
pub(crate) use script_nav::pump_route_work;
#[cfg(test)]
pub(super) use script_nav::{
    approach_tiles, publish_route_owner_refusal, resolve_route_exclusions, route_world_state,
    CarriedWalk, RouteDemandKind, ScriptRouteRequest,
};
pub(super) use script_nav::{
    blocked_zone_detail, hold_script_nav, reset_script_nav, NavBot, PostedWalkOutcome,
    ScriptRouteExclusions, ScriptWalkArm,
};
use script_nav::{log_walk_arm_bot, take_carried_walk};

/// Resolve live-fixture crossing grants through the script walk product path.
#[cfg(feature = "test-support")]
pub fn resolve_script_cross_zones_for_live_test(
    opts: nav_planner::router::FindOptions,
    world: &nav_planner::world::NavWorld,
    from: api::WorldTile,
    to: api::WorldTile,
    state: &nav_planner::WorldState,
    cross: &[&str],
) -> Result<nav_planner::router::FindOptions, String> {
    script_nav::resolve_route_exclusions(
        opts,
        world,
        from,
        to,
        state,
        ScriptRouteExclusions {
            cross: cross.iter().map(|name| Arc::from(*name)).collect(),
            ..Default::default()
        },
    )
    .map(|(opts, _)| opts)
}
#[path = "script_walk.rs"]
mod script_walk;
#[cfg(not(test))]
use script_walk::apply_watchdog_nav_action;
pub(super) use script_walk::{
    abort_script_walk, pause_script, session_freezes_follow, step_bank_fetch_on_bot, step_nav_bot,
    take_manual_walk_ownership,
};
#[cfg(test)]
pub(super) use script_walk::{
    apply_nav_follow_outcome, apply_watchdog_nav_action, owe_combat_prayers_off,
};
use script_walk::{recovery_walk_idle, resumed_walk};
#[path = "script_snapshot.rs"]
mod script_snapshot;
use script_snapshot::pack_cached_reach;
#[cfg(test)]
pub(super) use script_snapshot::{script_snapshot_fb, with_script_snapshot_input};
pub(super) use script_snapshot::{slot_arrival_reach, with_script_snapshot_input_shorts};
#[path = "script_interact.rs"]
mod script_interact;
pub(super) use script_interact::{
    dispatch_native_bank_deposit, dispatch_native_server_ordered,
    dispatch_script_interact_cached_scoped, BankDepositDispatch, InteractContext,
    OrderedDispatchContext,
};
#[cfg(test)]
pub(super) use script_interact::{
    dispatch_script_interact, dispatch_script_interact_cached, validated_duel_accept,
};
#[path = "script_paint.rs"]
mod script_paint;
pub(super) use script_paint::{
    publish_script_paint, script_paint_of, script_paint_select_advertised,
};

#[path = "route_inspect.rs"]
mod route_inspect;
#[cfg(test)]
pub(super) use route_inspect::PostedInspect;

#[cfg(test)]
pub(super) fn poll_inspection_for_test(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    route_inspect::poll_inspection(navs, name);
}

#[path = "script_observe.rs"]
mod script_observe;
#[cfg(test)]
pub(super) use script_observe::{
    delay_ordered_drain_for_test, drain_observed_host_interacts, install_dispatch_barrier,
    install_ordered_dispatch_hook, post_script_snapshot, script_observe, script_observe_cached,
    script_observe_with_npc_boxes, take_script_interacts, DispatchBarrier,
};
pub(super) use script_observe::{
    deliver_channel_events, nav_world_state_for_observe, observe_script_inv,
    project_npc_boxes_for_isolate_snapshot, projected_npc_boxes,
    script_observe_cached_with_channels, ScriptObservation,
};
/// Per-uid script cell on the wall. Encode/post/drain take the slot lock
/// only — the wall map lock is held briefly for lookup/insert.
pub(super) type ScriptSlot = Arc<Mutex<SlotScript>>;
/// Lock order: wall before slot; never hold slot then wall.
pub(super) type ScriptWall = Arc<Mutex<HashMap<String, ScriptSlot>>>;

pub(super) fn script_slot(wall: &ScriptWall, name: &str) -> Option<ScriptSlot> {
    wall.lock().unwrap().get(name).cloned()
}

pub(super) fn script_slot_or_insert(wall: &ScriptWall, name: &str) -> ScriptSlot {
    wall.lock()
        .unwrap()
        .entry(name.to_string())
        .or_insert_with(|| {
            static NEXT_INCARNATION: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(1);
            let incarnation = NEXT_INCARNATION
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |next| next.checked_add(1),
                )
                .expect("slot incarnation exhausted");
            let mut slot = SlotScript::new();
            slot.bind_incarnation(incarnation);
            bind_js_surface_factory(&mut slot);
            Arc::new(Mutex::new(slot))
        })
        .clone()
}

fn shared_js_surface_factory() -> Arc<script::load::surface::JsSurfaceFactory> {
    static FACTORY: std::sync::LazyLock<Arc<script::load::surface::JsSurfaceFactory>> =
        std::sync::LazyLock::new(|| Arc::new(script::load::surface::JsSurfaceFactory::default()));
    Arc::clone(&FACTORY)
}

fn bind_js_surface_factory(slot: &mut SlotScript) {
    slot.bind_surface_factory(shared_js_surface_factory());
}

pub(super) fn js_start_card(
    source: String,
    shape: script::LoadShape,
    siblings: Vec<(String, String)>,
) -> Arc<script::load::surface::JsStartCard> {
    let source = Arc::<str>::from(source);
    let family = script::resolve_api_family(&source)
        .ok()
        .map(|(_, family)| family);
    Arc::new(script::load::surface::JsStartCard {
        source,
        shape,
        siblings: Arc::from(siblings),
        family,
    })
}

pub(super) fn js_surface_start(
    card: Arc<script::load::surface::JsStartCard>,
    settings_bag: Option<Arc<script::native::SettingsBag>>,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    named_banks: Arc<api::named_banks::NamedBankFacts>,
    loadouts: Arc<[api::loadout::Loadout]>,
) -> script::surface::SurfaceStart {
    let card: Arc<dyn script::surface::SurfaceCard> = card;
    script::surface::SurfaceStart {
        card,
        settings_bag,
        game_data,
        named_banks,
        loadouts,
    }
}

/// Resolve `name`'s lifecycle and read its latest Start under one lock; a
/// slot that no longer exists owes nothing.
pub(super) fn poll_start(wall: &ScriptWall, name: &str) -> script::StartPoll {
    let Some(slot) = script_slot(wall, name) else {
        return script::StartPoll::NotOwed;
    };
    let Ok(mut slot) = slot.lock() else {
        return script::StartPoll::NotOwed;
    };
    let poll = slot.poll_start();
    if let script::StartPoll::Settled(outcome) = &poll {
        log_start_outcome(name, outcome, slot.state());
    }
    poll
}

fn log_start_outcome(name: &str, outcome: &script::StartOutcome, state: script::RunState) {
    use api::host_log;
    use api::hostlog::{Category, Level};
    match outcome {
        script::StartOutcome::Ready => {
            let status = match state {
                script::RunState::Running => "running",
                script::RunState::Starting => "starting",
                _ => "ready",
            };
            host_log!(
                Category::ScriptLifecycle,
                Level::Info,
                slot = name,
                "start {status}"
            );
        }
        script::StartOutcome::Rejected(error) => {
            host_log!(stderr; Category::ScriptLifecycle, Level::Error, slot = name, "start rejected: {error}");
        }
        script::StartOutcome::Failed(error) => {
            host_log!(stderr; Category::ScriptLifecycle, Level::Error, slot = name, "start failed: {error}");
        }
        script::StartOutcome::Cancelled => {
            host_log!(stderr; Category::ScriptLifecycle, Level::Warn, slot = name, "start cancelled");
        }
    }
}

/// Test-only Start that waits (bounded) for the isolate to settle through
/// the public observe path. Start returns before V8 setup; fixtures that
/// drive observes against a live script start from Ready, and a setup
/// failure comes back as the `Err` Start used to return.
#[cfg(test)]
pub(super) trait SettledStart {
    fn start_load_settled(
        &mut self,
        source: String,
        shape: script::LoadShape,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String>;
    fn start_load_with_loadouts_settled(
        &mut self,
        source: String,
        shape: script::LoadShape,
        siblings: Vec<(String, String)>,
        loadouts: &[api::loadout::Loadout],
    ) -> Result<(), String>;
}

#[cfg(test)]
fn settle_start(slot: &mut SlotScript) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match slot.poll_start() {
            script::StartPoll::Settled(script::StartOutcome::Ready) => return Ok(()),
            script::StartPoll::Settled(script::StartOutcome::Failed(e)) => return Err(e),
            script::StartPoll::Settled(script::StartOutcome::Rejected(e)) => {
                return Err(e.to_string())
            }
            script::StartPoll::Settled(script::StartOutcome::Cancelled) => {
                return Err("start cancelled".into())
            }
            script::StartPoll::NotOwed => panic!("no Start is pending"),
            script::StartPoll::Pending if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2))
            }
            script::StartPoll::Pending => panic!("script setup did not settle: {:?}", slot.state()),
        }
    }
}

#[cfg(test)]
impl SettledStart for SlotScript {
    fn start_load_settled(
        &mut self,
        source: String,
        shape: script::LoadShape,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        bind_js_surface_factory(self);
        let start = js_surface_start(
            js_start_card(source, shape, siblings),
            None,
            None,
            Arc::new(api::named_banks::NamedBankFacts::default()),
            Arc::from(Vec::<api::loadout::Loadout>::new()),
        );
        self.start_surface(start)
            .map_err(|error| error.to_string())?;
        settle_start(self)
    }

    fn start_load_with_loadouts_settled(
        &mut self,
        source: String,
        shape: script::LoadShape,
        siblings: Vec<(String, String)>,
        loadouts: &[api::loadout::Loadout],
    ) -> Result<(), String> {
        bind_js_surface_factory(self);
        let start = js_surface_start(
            js_start_card(source, shape, siblings),
            None,
            None,
            Arc::new(api::named_banks::NamedBankFacts::default()),
            Arc::from(loadouts.to_vec()),
        );
        self.start_surface(start)
            .map_err(|error| error.to_string())?;
        settle_start(self)
    }
}

/// One frame's script facts, taken under a single wall + slot lock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ScriptFrame {
    pub(super) state: Option<script::RunState>,
    /// The watchdog's clocks are frozen (guardian hold, pause).
    pub(super) frozen: bool,
    /// The watchdog has seen no game packets for 10 s.
    pub(super) no_game_packets: bool,
}

/// Capture one script-state projection for the frame's ownership, inventory
/// and capacity gates. Script observation re-checks the state under its own
/// dispatch fence.
pub(super) fn script_frame(scripts: &ScriptWall, name: &str) -> ScriptFrame {
    script_slot(scripts, name)
        .and_then(|slot| {
            slot.lock().ok().map(|slot| ScriptFrame {
                state: Some(slot.state()),
                frozen: slot.watchdog().frozen(),
                no_game_packets: slot.watchdog().no_game_packets(),
            })
        })
        .unwrap_or_default()
}

pub(super) fn script_movement_owned(state: Option<script::RunState>) -> bool {
    matches!(
        state,
        Some(
            script::RunState::Starting
                | script::RunState::Running
                | script::RunState::Paused
                | script::RunState::Stopping
        )
    )
}

/// A script is running or paused on the slot (frozen `scriptActive()`,
/// `AutoRelogin.ts:138-141`): started and not stopping. An offline slot's
/// started script reads Paused.
pub(super) fn script_active(scripts: &ScriptWall, name: &str) -> bool {
    script_slot(scripts, name)
        .and_then(|slot| slot.lock().ok().map(|slot| slot.state()))
        .is_some_and(|state| {
            matches!(
                state,
                script::RunState::Starting | script::RunState::Running | script::RunState::Paused
            )
        })
}

/// Puzzle-board post tests: the production observe path
/// ([`with_script_snapshot_input`] via [`script_snapshot_fb`]) over a
/// client whose main modal holds both a hint panel (a TYPE_INV without
/// `obj_ops`) and the piece container.
#[cfg(test)]
#[path = "script_runtime_tests.rs"]
mod tests;
