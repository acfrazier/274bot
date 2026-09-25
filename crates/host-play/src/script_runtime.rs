//! Per-slot script observe/dispatch/hold transaction and script navigation continuation.

use super::*;
#[path = "script_bank.rs"]
mod bank;
use bank::{
    action_slot, all_slot, deposit_all_backpack, dispatch_observed_bank_op, open_bank_at_here,
    withdraw_id,
};
pub(super) use bank::{fill_withdraw_action, nearest_bank_booth};
#[path = "script_nav.rs"]
mod script_nav;
use script_nav::log_walk_arm_bot;
#[cfg(test)]
pub(super) use script_nav::{approach_tiles, ScriptRouteRequest};
pub(super) use script_nav::{reset_script_nav, NavBot, PostedWalkOutcome, ScriptWalkArm};
#[path = "script_walk.rs"]
mod script_walk;
#[cfg(test)]
pub(super) use script_walk::apply_nav_follow_outcome;
pub(super) use script_walk::{
    abort_script_walk, bank_fetch_freezes_follow, step_bank_fetch_on_bot, step_nav_bot,
};
use script_walk::{apply_watchdog_nav_action, recovery_walk_idle};
#[path = "script_snapshot.rs"]
mod script_snapshot;
use script_snapshot::pack_cached_reach;
#[cfg(test)]
pub(super) use script_snapshot::{script_snapshot_fb, with_script_snapshot_input};
pub(super) use script_snapshot::{slot_arrival_reach, with_script_snapshot_input_shorts};
#[path = "script_interact.rs"]
mod script_interact;
#[cfg(test)]
pub(super) use script_interact::dispatch_script_interact;
pub(super) use script_interact::dispatch_script_interact_cached;
#[path = "script_paint.rs"]
mod script_paint;
pub(super) use script_paint::{
    publish_script_paint, script_paint_of, script_paint_select_advertised,
};

#[path = "route_inspect.rs"]
mod route_inspect;
#[cfg(test)]
pub(super) use route_inspect::PostedInspect;

#[path = "script_observe.rs"]
mod script_observe;
pub(super) use script_observe::{
    nav_world_state_for_observe, observe_script_inv, project_npc_boxes_for_isolate_snapshot,
    projected_npc_boxes, script_observe_cached,
};
#[cfg(test)]
pub(super) use script_observe::{
    post_script_snapshot, script_observe, script_observe_with_npc_boxes, take_script_interacts,
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
        .or_insert_with(|| Arc::new(Mutex::new(SlotScript::new())))
        .clone()
}

/// Resolve `name`'s lifecycle and read its latest Start under one lock; a
/// slot that no longer exists owes nothing.
pub(super) fn poll_start(wall: &ScriptWall, name: &str) -> script::StartPoll {
    match script_slot(wall, name) {
        Some(slot) => slot.lock().unwrap().poll_start(),
        None => script::StartPoll::NotOwed,
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
        loadouts: &[script::Loadout],
    ) -> Result<(), String>;
}

#[cfg(test)]
fn settle_start(slot: &mut SlotScript) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match slot.poll_start() {
            script::StartPoll::Settled(script::StartOutcome::Ready) => return Ok(()),
            script::StartPoll::Settled(script::StartOutcome::Failed(e)) => return Err(e),
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
        self.start_load(source, shape, siblings)?;
        settle_start(self)
    }

    fn start_load_with_loadouts_settled(
        &mut self,
        source: String,
        shape: script::LoadShape,
        siblings: Vec<(String, String)>,
        loadouts: &[script::Loadout],
    ) -> Result<(), String> {
        self.start_load_with_loadouts(source, shape, siblings, loadouts)?;
        settle_start(self)
    }
}

/// the per-observe inventory view (the observe re-checks the gate inside).
pub(super) fn script_running(scripts: &ScriptWall, name: &str) -> bool {
    script_slot(scripts, name)
        .is_some_and(|s| s.lock().unwrap().state() == script::RunState::Running)
}

/// Puzzle-board post tests: the production observe path
/// ([`with_script_snapshot_input`] via [`script_snapshot_fb`]) over a
/// client whose main modal holds both a hint panel (a TYPE_INV without
/// `obj_ops`) and the piece container.
#[cfg(test)]
#[path = "script_runtime_tests.rs"]
mod tests;
