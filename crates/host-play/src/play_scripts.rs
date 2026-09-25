use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::snapshot::WorldTile;
use nav::router::Route;

use crate::script_runtime;
use crate::script_runtime::{
    abort_script_walk, poll_start, script_slot, script_slot_or_insert, NavBot, ScriptWall,
};
use crate::{debug_enabled, Play};

fn invalidate_bank_pick(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    if let Some(bot) = navs.lock().unwrap().get_mut(name) {
        bot.bank_pick.reset();
    }
}

/// Cloneable overlay source for a catalog Traveller (`InteractReq::Walk`).
/// The panel paints this when WalkTo's [`WalkArm`] is idle so a script
/// walk shows the same path / click as a picker walk.
#[derive(Clone)]
pub struct ScriptNavPaint {
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
}

impl ScriptNavPaint {
    /// Armed route and current hop aim for `name`, if any.
    pub fn of(&self, name: &str) -> (Option<Route>, Option<WorldTile>) {
        match self.navs.lock().unwrap().get(name) {
            Some(b) => (b.route.clone(), b.traveller.current_aim()),
            None => (None, None),
        }
    }
}
/// Cloneable isolate-start handle for slot-thread live pumps that cannot
/// hold `&Play` (the per-frame hook is built before `Play` is stored).
#[derive(Clone)]
pub struct ScriptStartHandle {
    scripts: ScriptWall,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    game_data: Option<Arc<api::game_data::SelectedGameData>>,
    named_banks: Arc<api::named_banks::NamedBankFacts>,
}

impl ScriptStartHandle {
    /// Start a loaded JS bot on `name`'s slot. Same isolate spawn as
    /// [`Play::script_start_load`], without the control-thread wake
    /// (the slot thread is already pumping). Operator loadouts come from
    /// the default store; harness scenario Start uses
    /// [`Self::start_load_with_loadouts`] when the scenario owns fixture
    /// loadouts.
    pub fn start_load(
        &self,
        name: &str,
        source: String,
        shape: script::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        if debug_enabled() {
            eprintln!("[script {name}] start load");
        }
        let slot = script_slot_or_insert(&self.scripts, name);
        let mut slot = slot.lock().unwrap();
        let result = slot.start_load_with_settings_and_game_data(
            source,
            shape,
            settings_bag.as_ref(),
            siblings,
            self.game_data.clone(),
            Arc::clone(&self.named_banks),
        );
        if result.is_ok() {
            invalidate_bank_pick(&self.navs, name);
        }
        if let Err(e) = &result {
            eprintln!("[script {name}] start failed: {e}");
        }
        result
    }

    /// Harness catalog Start with caller-owned loadouts. Uses the existing
    /// explicit-loadout isolate start, then posts `settings_bag` on success.
    /// Does not inspect loadout names and does not affect
    /// [`Play::script_start_load_typed`].
    pub fn start_load_with_loadouts(
        &self,
        name: &str,
        source: String,
        shape: script::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
        loadouts: &[script::Loadout],
    ) -> Result<(), String> {
        if debug_enabled() {
            eprintln!("[script {name}] start load");
        }
        let slot = script_slot_or_insert(&self.scripts, name);
        let mut slot = slot.lock().unwrap();
        let result = slot.start_load_with_loadouts_and_game_data(
            source,
            shape,
            siblings,
            loadouts,
            self.game_data.clone(),
            Arc::clone(&self.named_banks),
        );
        if result.is_ok() {
            invalidate_bank_pick(&self.navs, name);
            if let Some(bag) = settings_bag.as_ref() {
                slot.post_settings_bag(bag);
            }
        }
        if let Err(e) = &result {
            eprintln!("[script {name}] start failed: {e}");
        }
        result
    }

    /// Start a compiled registry card on `name`'s slot. Same constructor as
    /// [`Play::script_start`], without the control-thread wake (the slot
    /// thread is already pumping).
    pub fn start_compiled(&self, name: &str, id: script::CompiledId) -> Result<(), String> {
        if debug_enabled() {
            eprintln!("[script {name}] start compiled {}", id.0);
        }
        let make = script::factory(id).ok_or_else(|| format!("not ported: {}", id.0))?;
        let slot = script_slot_or_insert(&self.scripts, name);
        let mut slot = slot.lock().unwrap();
        let result = slot.start_compiled(make(), self.game_data.clone());
        if result.is_ok() {
            invalidate_bank_pick(&self.navs, name);
        }
        if let Err(e) = &result {
            eprintln!("[script {name}] start failed: {e}");
        }
        result
    }

    /// `name`'s latest Start, read atomically (see [`Play::script_poll_start`]).
    pub fn poll_start(&self, name: &str) -> script::StartPoll {
        poll_start(&self.scripts, name)
    }
}
impl Play {
    /// Overlay handle for catalog `walk` / `ctx.walk` Traveller (Play's
    /// per-uid NavBot). WalkTo's [`WalkArm`] map is a different latch.
    pub fn script_nav_paint(&self) -> ScriptNavPaint {
        ScriptNavPaint {
            navs: Arc::clone(&self.navs),
        }
    }
    /// Whether `name` is a slot this play controls (spawned or armed), so
    /// script control can never create an entry no thread drives.
    fn slot_active(&self, name: &str) -> bool {
        self.spawned.contains(name) || self.arms.contains_key(name)
    }

    /// Paint is status-owned once published. Script lifecycle commands clear
    /// it directly because a disconnected slot has no observe pass. Call only
    /// after releasing the script-slot guard: script -> status is the sole
    /// permitted nesting order, and this path does not need to nest them.
    fn clear_script_paint_status(&self, name: &str) {
        if let Some(status) = self
            .statuses
            .lock()
            .unwrap()
            .iter_mut()
            .find(|status| status.username == name)
        {
            status.script_paint = None;
        }
    }

    /// Start a compiled script on `name`'s slot. `Err("no slot: {name}")`
    /// when no running slot owns that name, `Err("not ported: {id}")` when
    /// the picker id has no ported script yet, or `Err` when the slot
    /// already runs one. The slot thread gates it on `is_up`.
    ///
    /// The compiled card is started with this Play's own selected-revision
    /// pin — the same facts a Load isolate is spawned with.
    pub fn script_start(&self, name: &str, id: script::CompiledId) -> Result<(), String> {
        if !self.slot_active(name) {
            return Err(format!("no slot: {name}"));
        }
        let make = script::factory(id).ok_or_else(|| format!("not ported: {}", id.0))?;
        let slot = script_slot_or_insert(&self.scripts, name);
        let mut slot = slot.lock().unwrap();
        slot.start_compiled(make(), self.game_data.clone())?;
        invalidate_bank_pick(&self.navs, name);
        drop(slot);
        self.wake(name);
        Ok(())
    }

    /// Start a loaded JS bot on `name`'s slot: the isolate is spawned here,
    /// on Start (never at Load). Same slot gating as
    /// [`Play::script_start`].
    pub fn script_start_load(
        &self,
        name: &str,
        source: String,
        shape: script::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.script_start_load_typed(name, source, shape, settings_bag, siblings)
            .map_err(|e| e.to_string())
    }

    /// Initial load result with operational refusals separate from loader errors.
    pub fn script_start_load_typed(
        &self,
        name: &str,
        source: String,
        shape: script::LoadShape,
        settings_bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), script::StartLoadError> {
        if !self.slot_active(name) {
            return Err(script::StartLoadError::Refused(format!("no slot: {name}")));
        }
        if debug_enabled() {
            eprintln!("[script {name}] start load");
        }
        let slot = script_slot_or_insert(&self.scripts, name);
        let mut slot = slot.lock().unwrap();
        let result = slot.start_load_with_settings_and_game_data_typed(
            source,
            shape,
            settings_bag.as_ref(),
            siblings,
            self.game_data.clone(),
            Arc::clone(&self.named_banks),
        );
        if let Err(e) = &result {
            eprintln!("[script {name}] start failed: {e}");
        }
        result?;
        invalidate_bank_pick(&self.navs, name);
        drop(slot);
        self.wake(name);
        Ok(())
    }

    /// Cloneable isolate-start handle for slot-thread live pumps that
    /// cannot hold `&Play`.
    pub fn script_start_handle(&self) -> ScriptStartHandle {
        ScriptStartHandle {
            scripts: Arc::clone(&self.scripts),
            navs: Arc::clone(&self.navs),
            game_data: self.game_data.clone(),
            named_banks: Arc::clone(&self.named_banks),
        }
    }

    /// Pause `name`'s script (operator Pause; survives login until
    /// Resume re-arms it). No-op when the slot has no script.
    pub fn script_pause(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            let abort = slot.lock().unwrap().pause();
            if abort {
                abort_script_walk(&self.navs, name);
            }
        }
        self.wake(name);
    }

    /// Resume `name`'s script; the next `on_is_up` re-gates it.
    pub fn script_resume(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            slot.lock().unwrap().resume();
        }
        self.wake(name);
    }

    /// Stop `name`'s script: teardown hook, instance dropped, Idle. Clear its
    /// published paint even while the client is offline.
    pub fn script_stop(&self, name: &str) {
        let slot = script_slot(&self.scripts, name);
        let mut guard = slot.as_ref().map(|slot| slot.lock().unwrap());
        if let Some(slot) = guard.as_mut() {
            slot.stop();
        }
        // Keep the script admission lock until its bank epoch is invalidated.
        // A previously dequeued worker can no longer publish or arm a route.
        invalidate_bank_pick(&self.navs, name);
        drop(guard);
        self.clear_script_paint_status(name);
        self.wake(name);
    }

    /// Stop only the exact script lifetime inspected by a reload warning.
    /// Identity, generation, and Stop are one slot-lock transaction; a newer
    /// script can never be stopped by an older confirmation.
    pub fn script_stop_if_identity_generation(
        &self,
        name: &str,
        identity: &str,
        generation: u64,
    ) -> bool {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return false;
        };
        {
            let mut slot = slot.lock().unwrap();
            if slot.source_identity() != Some(identity) || slot.runtime_generation() != generation {
                return false;
            }
            slot.stop();
            invalidate_bank_pick(&self.navs, name);
        }
        self.clear_script_paint_status(name);
        self.wake(name);
        true
    }

    pub fn script_attach_identity(&self, name: &str, identity: impl Into<String>) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            slot.lock().unwrap().attach_source_identity(identity);
        }
    }

    pub fn script_runtime_generation(&self, name: &str) -> Option<u64> {
        script_slot(&self.scripts, name).map(|slot| slot.lock().unwrap().runtime_generation())
    }

    pub fn script_source_identity(&self, name: &str) -> Option<String> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().unwrap().source_identity().map(str::to_string))
    }

    pub fn script_post_settings_fenced(
        &self,
        name: &str,
        bag: &serde_json::Map<String, serde_json::Value>,
        identity: &str,
        generation: u64,
    ) -> bool {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return false;
        };
        let accepted = slot
            .lock()
            .unwrap()
            .post_settings_bag_fenced(bag, identity, generation);
        accepted
    }

    /// One-shot script-local paint button for `name`. No-op when there is
    /// no slot, the slot is not Running, `id` is empty, `generation` does
    /// not match the last forwarded frame, or that frame does not advertise
    /// `id`. Never walks, pauses, or stops.
    pub fn script_paint_click(&self, name: &str, id: &str, generation: u64) {
        if id.is_empty() {
            return;
        }
        let Some(slot) = script_slot(&self.scripts, name) else {
            return;
        };
        let slot = slot.lock().unwrap();
        if slot.state() != script::RunState::Running {
            return;
        }
        let Some(paint) = slot.paint() else {
            return;
        };
        if paint.generation != generation {
            return;
        }
        if !paint.buttons.iter().any(|b| b.id == id) {
            return;
        }
        slot.paint_click(id);
    }

    /// Persistent strip/rail/tabs selection for `name`. No-op when there is
    /// no slot, the slot is not Running, `key` or `name` is empty,
    /// `generation` does not match the last forwarded frame, or that frame
    /// does not advertise `name` under `key`. Never walks, pauses, or stops.
    pub fn script_paint_select(&self, name: &str, key: &str, select_name: &str, generation: u64) {
        if key.is_empty() || select_name.is_empty() {
            return;
        }
        let Some(slot) = script_slot(&self.scripts, name) else {
            return;
        };
        let slot = slot.lock().unwrap();
        if slot.state() != script::RunState::Running {
            return;
        }
        let Some(paint) = slot.paint() else {
            return;
        };
        if paint.generation != generation {
            return;
        }
        if !script_runtime::script_paint_select_advertised(&paint, key, select_name) {
            return;
        }
        slot.paint_select(key, select_name);
    }

    /// `name`'s script lifecycle state; `Idle` when the slot has none.
    pub fn script_state(&self, name: &str) -> script::RunState {
        script_slot(&self.scripts, name)
            .map(|slot| slot.lock().unwrap().state())
            .unwrap_or(script::RunState::Idle)
    }

    /// Resolve `name`'s non-blocking isolate Start/Stop. The slot thread
    /// does this every observe; a slot that is offline or queued for login
    /// has no observe, so the UI pumps it (see [`Self::pump_script_lifecycles`]).
    pub fn pump_script_lifecycle(&self, name: &str) {
        if let Some(slot) = script_slot(&self.scripts, name) {
            if let Ok(mut slot) = slot.lock() {
                slot.observe_lifecycle();
            }
        }
    }

    /// Resolve every slot's pending Start/Stop once. Called per UI frame by
    /// the panel and the TUI. A slot its own thread holds is skipped: that
    /// thread is observing it right now.
    pub fn pump_script_lifecycles(&self) {
        let slots: Vec<_> = self.scripts.lock().unwrap().values().cloned().collect();
        for slot in slots {
            if let Ok(mut slot) = slot.try_lock() {
                slot.observe_lifecycle();
            }
        }
    }

    /// `name`'s latest operator Load Start: Start returns before V8 setup,
    /// so the assignment and the load diagnostic are committed from the
    /// settled outcome, not from Start's `Ok`. The lifecycle observe, the
    /// outcome take and the in-flight check happen under one slot lock, so
    /// the slot thread's own observe can never settle it unseen between
    /// them. A removed slot owes nothing.
    pub fn script_poll_start(&self, name: &str) -> script::StartPoll {
        poll_start(&self.scripts, name)
    }

    #[cfg(feature = "memory-profile")]
    pub fn memory_script_metrics(&self, name: &str) -> Option<serde_json::Value> {
        script_slot(&self.scripts, name).and_then(|slot| slot.lock().unwrap().memory_metrics())
    }

    #[cfg(feature = "memory-profile")]
    pub fn memory_script_progress(&self, name: &str) -> serde_json::Value {
        script_slot(&self.scripts, name)
            .map(|slot| slot.lock().unwrap().memory_progress())
            .unwrap_or(serde_json::Value::Null)
    }

    /// `name`'s script `last_error`; `None` when the slot has none.
    pub fn script_last_error(&self, name: &str) -> Option<String> {
        script_slot(&self.scripts, name)
            .and_then(|slot| slot.lock().unwrap().last_error().map(str::to_string))
    }

    /// Latest bounded ScriptRunner.stop receipt. This is non-consuming and
    /// independent of [`Self::script_take_pending_logs`].
    pub fn script_lifecycle_receipt(&self, name: &str) -> Option<script::ScriptLifecycleReceipt> {
        script_slot(&self.scripts, name).and_then(|slot| slot.lock().unwrap().lifecycle_receipt())
    }

    /// Isolate log lines staged since the last take (panel log pane).
    pub fn script_take_pending_logs(&self, name: &str) -> Vec<String> {
        let Some(slot) = script_slot(&self.scripts, name) else {
            return Vec::new();
        };
        // A slot its own thread holds keeps its lines for the next frame;
        // a poisoned slot is a bug and still panics, as `lock` did.
        let mut slot = match slot.try_lock() {
            Ok(slot) => slot,
            Err(std::sync::TryLockError::WouldBlock) => return Vec::new(),
            Err(std::sync::TryLockError::Poisoned(e)) => panic!("script slot poisoned: {e}"),
        };
        slot.take_pending_logs()
    }
}
