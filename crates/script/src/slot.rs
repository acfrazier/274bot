//! Per-uid script runner. `SlotScript` owns at most one compiled `Script`
//! and gates it on operator intent (`want_run`) and client presence
//! (`on_is_up`). `tick` runs on the caller's pump at a game-tick edge and
//! must return; panics are caught, never abort the process.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

use crate::ctx::{Script, ScriptCtx};
#[cfg(feature = "load")]
use crate::isolate_fb::{IsolateBuf, SnapshotFingerprint};
#[cfg(feature = "load")]
use crate::load::{LoadIsolate, LoadShape};
use api::random::{DetectedRandom, RandomClaim};

/// Lifecycle of the script slot. `paused` covers both operator Pause and
/// the not-`is_up` gate; `stopping` is the Load-join window (later task).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running,
    Paused,
    Stopping,
    Error,
}

/// Host-owned second phase of a bank Withdraw-X operation. The first phase
/// sent the X menu action; this record authorizes one count response only
/// while the same bank session remains current and before its deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingWithdrawXPhase {
    Dialog,
    Settlement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingWithdrawResult {
    WithdrawX,
    WithdrawLoad,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingFillBaseline {
    pub bank_item_id: i32,
    pub before_used: usize,
    pub before_count: i32,
    pub before_stock: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingWithdrawX {
    pub item_id: i32,
    pub count: i32,
    pub before: i32,
    pub target: i32,
    pub bank_generation: u64,
    pub phase: PendingWithdrawXPhase,
    pub deadline: Option<Instant>,
    pub remaining: Duration,
    pub result: PendingWithdrawResult,
    pub fill: Option<PendingFillBaseline>,
}

impl PendingWithdrawX {
    pub fn waiting_dialog(
        item_id: i32,
        count: i32,
        before: i32,
        target: i32,
        bank_generation: u64,
    ) -> Self {
        let remaining = Duration::from_millis(3000);
        Self {
            item_id,
            count,
            before,
            target,
            bank_generation,
            phase: PendingWithdrawXPhase::Dialog,
            deadline: Some(Instant::now() + remaining),
            remaining,
            result: PendingWithdrawResult::WithdrawX,
            fill: None,
        }
    }

    pub fn waiting_load_dialog(
        bank_item_id: i32,
        count: i32,
        before_used: usize,
        before_count: i32,
        before_stock: i32,
        bank_generation: u64,
    ) -> Self {
        let mut pending = Self::waiting_dialog(bank_item_id, count, 0, 0, bank_generation);
        pending.result = PendingWithdrawResult::WithdrawLoad;
        pending.fill = Some(PendingFillBaseline {
            bank_item_id,
            before_used,
            before_count,
            before_stock,
        });
        pending
    }

    pub fn waiting_settlement(mut self) -> Self {
        self.phase = PendingWithdrawXPhase::Settlement;
        self.remaining = Duration::from_millis(4000);
        self.deadline = Some(Instant::now() + self.remaining);
        self
    }

    pub fn expired(self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn freeze(&mut self) {
        if let Some(deadline) = self.deadline.take() {
            self.remaining = deadline.saturating_duration_since(Instant::now());
        }
    }

    fn resume(&mut self) {
        if self.deadline.is_none() {
            self.deadline = Some(Instant::now() + self.remaining);
        }
    }
}

/// Per-uid runner. Compiled XOR Load (a JS isolate) — never both.
pub struct SlotScript {
    pub want_run: bool,
    state: RunState,
    compiled: Option<Box<dyn Script>>,
    /// JS Load isolate, spawned by `start_load` on Start (not on Load).
    #[cfg(feature = "load")]
    load: Option<LoadIsolate>,
    /// Per-slot last-post snapshot fingerprint (delta posts: only the
    /// fields that changed are re-sent) and the `NavWorld` identity the
    /// packed banks keyframe on. Cleared on Start so the first post is a
    /// keyframe.
    #[cfg(feature = "load")]
    last_snapshot: Option<SnapshotFingerprint>,
    #[cfg(feature = "load")]
    last_world_id: Option<usize>,
    /// Reusable FlatBuffer builder for this slot's host→isolate snapshot
    /// posts. One per slot; the V8 isolate thread holds its own for
    /// interact/paint. Never a JSON document on either path.
    #[cfg(feature = "load")]
    ipc: IsolateBuf,
    last_error: Option<String>,
    /// Isolate log lines not yet taken by the panel (`take_pending_logs`).
    pending_logs: Vec<String>,
    /// Dispatched game ticks since the last Start.
    ticks: u64,
    pending_withdraw_x: Option<PendingWithdrawX>,
    withdraw_x_result_seq: u64,
    withdraw_x_result: bool,
    withdraw_load_result_seq: u64,
    withdraw_load_result: bool,
    work_epoch: u64,
}

impl Default for SlotScript {
    fn default() -> Self {
        Self::new()
    }
}

impl SlotScript {
    pub fn new() -> Self {
        SlotScript {
            want_run: false,
            state: RunState::Idle,
            compiled: None,
            #[cfg(feature = "load")]
            load: None,
            #[cfg(feature = "load")]
            last_snapshot: None,
            #[cfg(feature = "load")]
            last_world_id: None,
            #[cfg(feature = "load")]
            ipc: IsolateBuf::new(),
            last_error: None,
            pending_logs: Vec::new(),
            ticks: 0,
            pending_withdraw_x: None,
            withdraw_x_result_seq: 0,
            withdraw_x_result: false,
            withdraw_load_result_seq: 0,
            withdraw_load_result: false,
            work_epoch: 0,
        }
    }

    /// True when either a compiled script or a JS isolate is installed.
    fn has_instance(&self) -> bool {
        self.compiled.is_some() || self.load_active()
    }

    /// Install a compiled script and start it. Refuses (no silent replace)
    /// while Running, Paused, or Stopping; allowed from Idle and Error
    /// (a fresh Start clears the previous error).
    pub fn start_compiled(&mut self, script: Box<dyn Script>) -> Result<(), String> {
        match self.state {
            RunState::Running | RunState::Paused | RunState::Stopping => {
                Err("script already active: stop it first".to_string())
            }
            RunState::Idle | RunState::Error => {
                if self.load_active() {
                    return Err("loaded script active: stop it first".to_string());
                }
                self.compiled = Some(script);
                self.want_run = true;
                self.last_error = None;
                self.ticks = 0;
                self.pending_withdraw_x = None;
                self.withdraw_x_result_seq = 0;
                self.withdraw_x_result = false;
                self.withdraw_load_result_seq = 0;
                self.withdraw_load_result = false;
                self.state = RunState::Running;
                Ok(())
            }
        }
    }

    /// Start a JS Load isolate (the isolate is spawned here, on Start, not
    /// at Load). Same state gating as [`SlotScript::start_compiled`].
    #[cfg(feature = "load")]
    pub fn start_load(
        &mut self,
        source: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        let loadouts = crate::loadouts_store::LoadoutsStore::with_default_path();
        self.start_load_with_loadouts(source, shape, siblings, loadouts.loadouts())
    }

    /// Start with explicit loadouts, avoiding operator filesystem inputs in
    /// disposable live fixtures. Commands are posted before the first tick.
    #[cfg(feature = "load")]
    pub fn start_load_with_loadouts(
        &mut self,
        source: String,
        shape: LoadShape,
        siblings: Vec<(String, String)>,
        loadouts: &[crate::loadouts_store::Loadout],
    ) -> Result<(), String> {
        match self.state {
            RunState::Running | RunState::Paused | RunState::Stopping => {
                Err("script already active: stop it first".to_string())
            }
            RunState::Idle | RunState::Error => {
                if self.compiled.is_some() {
                    return Err("compiled script active: stop it first".to_string());
                }
                let isolate = LoadIsolate::spawn(source, shape, siblings)?;
                isolate.post_loadouts(loadouts);
                self.load = Some(isolate);
                self.want_run = true;
                self.last_error = None;
                self.ticks = 0;
                self.pending_withdraw_x = None;
                self.withdraw_x_result_seq = 0;
                self.withdraw_x_result = false;
                self.withdraw_load_result_seq = 0;
                self.withdraw_load_result = false;
                // Fresh isolate: the first posted snapshot is a keyframe.
                self.last_snapshot = None;
                self.last_world_id = None;
                self.state = RunState::Running;
                Ok(())
            }
        }
    }

    /// Operator Pause: `want_run` stays false (survives login) until
    /// Resume. Instance kept. No-op when there is no instance.
    pub fn pause(&mut self) {
        self.want_run = false;
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.freeze();
        }
        if self.has_instance() && self.state == RunState::Running {
            #[cfg(feature = "load")]
            if let Some(isolate) = &self.load {
                isolate.pause();
            }
            self.state = RunState::Paused;
        }
    }

    /// Operator Resume: `want_run` back on. Assumes the client is up; the
    /// next `on_is_up(false)` re-gates if it is not. No-op when there is
    /// no instance or the slot errored.
    pub fn resume(&mut self) {
        self.want_run = true;
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.resume();
        }
        if self.has_instance() && self.state == RunState::Paused {
            #[cfg(feature = "load")]
            if let Some(isolate) = &self.load {
                isolate.resume();
            }
            self.state = RunState::Running;
        }
    }

    /// Operator Stop: join the Load isolate, run the compiled teardown
    /// hook, drop the instance, Idle.
    pub fn stop(&mut self) {
        #[cfg(feature = "load")]
        if let Some(isolate) = self.load.take() {
            isolate.join();
        }
        if let Some(mut script) = self.compiled.take() {
            script.on_stop();
        }
        #[cfg(feature = "load")]
        {
            self.last_snapshot = None;
            self.last_world_id = None;
            self.ipc = IsolateBuf::new();
        }
        self.want_run = false;
        self.pending_withdraw_x = None;
        self.work_epoch = self.work_epoch.wrapping_add(1);
        self.state = RunState::Idle;
    }

    /// Recompute the gate from client presence. With an instance, the slot
    /// is Running only when `up && want_run`; every other combination is
    /// Paused. Without an instance the state is untouched (Idle, or Error
    /// after a panic — `is_up` must not resurrect or wipe an error).
    pub fn on_is_up(&mut self, up: bool) {
        if !up {
            if self.pending_withdraw_x.is_some() {
                self.complete_current_withdrawal(false);
            }
            self.work_epoch = self.work_epoch.wrapping_add(1);
        }
        if !self.has_instance() {
            return;
        }
        self.state = if up && self.want_run {
            RunState::Running
        } else {
            RunState::Paused
        };
    }

    /// Re-gate a started script and invalidate deferred actions and snapshot
    /// deltas at a connection boundary. Operator run intent is retained.
    pub fn reset_session_work(&mut self) {
        self.on_is_up(false);
        #[cfg(feature = "load")]
        {
            if let Some(isolate) = &self.load {
                isolate.reset_session_work();
            }
            self.last_snapshot = None;
            self.last_world_id = None;
        }
    }

    /// Current host-owned Withdraw-X continuation, if one is armed.
    pub fn pending_withdraw_x(&self) -> Option<PendingWithdrawX> {
        self.pending_withdraw_x
    }

    /// Replace the one bounded Withdraw-X continuation for this slot.
    pub fn set_pending_withdraw_x(&mut self, pending: Option<PendingWithdrawX>) {
        self.pending_withdraw_x = pending;
    }

    /// Freeze the monotonic deadline without discarding the operation.
    pub fn freeze_pending_withdraw_x(&mut self) {
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.freeze();
        }
    }

    /// Resume a previously frozen monotonic deadline.
    pub fn resume_pending_withdraw_x(&mut self) {
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.resume();
        }
    }

    /// Last host-owned Withdraw-X result posted to this isolate.
    pub fn withdraw_x_result(&self) -> (u64, bool) {
        (self.withdraw_x_result_seq, self.withdraw_x_result)
    }

    /// Last host-owned withdrawLoad result posted to this isolate.
    pub fn withdraw_load_result(&self) -> (u64, bool) {
        (self.withdraw_load_result_seq, self.withdraw_load_result)
    }

    /// Lifecycle stamp used to reject work that raced a stop/reconnect.
    pub fn work_epoch(&self) -> u64 {
        self.work_epoch
    }

    /// Complete the current operation and advance the posted result token.
    pub fn complete_withdraw_x(&mut self, result: bool) {
        self.pending_withdraw_x = None;
        self.withdraw_x_result_seq = self.withdraw_x_result_seq.wrapping_add(1);
        self.withdraw_x_result = result;
    }

    pub fn complete_withdraw_load(&mut self, result: bool) {
        self.pending_withdraw_x = None;
        self.withdraw_load_result_seq = self.withdraw_load_result_seq.wrapping_add(1);
        self.withdraw_load_result = result;
    }

    /// Complete the armed shared withdrawal continuation on its result channel.
    pub fn complete_current_withdrawal(&mut self, result: bool) {
        match self.pending_withdraw_x.map(|pending| pending.result) {
            Some(PendingWithdrawResult::WithdrawLoad) => self.complete_withdraw_load(result),
            Some(PendingWithdrawResult::WithdrawX) | None => self.complete_withdraw_x(result),
        }
    }

    /// Post the host's FlatBuffer snapshot blob into a Load isolate (no-op
    /// for a compiled script). Call it before [`SlotScript::on_game_tick`]
    /// so the posted blob is what the tick's JS reads.
    #[cfg(feature = "load")]
    pub fn post_snapshot(&self, bytes: Vec<u8>) {
        if let Some(isolate) = &self.load {
            isolate.post_snapshot(bytes);
        }
    }

    /// Post the merged operator settings bag into a Load isolate.
    #[cfg(feature = "load")]
    pub fn post_settings_bag(&self, bag: &serde_json::Map<String, serde_json::Value>) {
        if let Some(isolate) = &self.load {
            isolate.post_settings_bag(bag);
        }
    }

    #[cfg(feature = "load")]
    pub fn post_loadouts(&self, loadouts: &[crate::loadouts_store::Loadout]) {
        if let Some(isolate) = &self.load {
            isolate.post_loadouts(loadouts);
        }
    }

    /// Start a JS Load isolate and optionally post the operator settings bag.
    #[cfg(feature = "load")]
    pub fn start_load_with_settings(
        &mut self,
        source: String,
        shape: LoadShape,
        bag: Option<&serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.start_load(source, shape, siblings)?;
        if let Some(bag) = bag {
            self.post_settings_bag(bag);
        }
        Ok(())
    }

    /// Encode `input` into this slot's reusable isolate buffer and return
    /// the finished bytes. Stores the new last-post fingerprint so the
    /// next observe is a delta. Disjoint-field borrow of `ipc` and
    /// `last_snapshot` — no extra fingerprint clone.
    #[cfg(feature = "load")]
    pub fn encode_snapshot_delta(
        &mut self,
        input: &crate::isolate_fb::SnapshotInput<'_>,
        force_banks: bool,
    ) -> Vec<u8> {
        let (bytes, fp) =
            self.ipc
                .encode_snapshot_delta(self.last_snapshot.as_ref(), input, force_banks);
        self.last_snapshot = Some(fp);
        bytes
    }

    /// The `NavWorld` identity the packed banks were posted against
    /// (`None` before the first post / after a Start). A world rebuild
    /// changes the identity, forcing the banks table onto the next post.
    #[cfg(feature = "load")]
    pub fn last_world_id(&self) -> Option<usize> {
        self.last_world_id
    }

    /// Store the `NavWorld` identity the packed banks were just posted
    /// against.
    #[cfg(feature = "load")]
    pub fn store_last_world_id(&mut self, id: Option<usize>) {
        self.last_world_id = id;
    }

    /// Drain the Load isolate's forwarded interact requests (the shim
    /// Bank/Banking queue), in tick order; empty for a compiled script or
    /// no isolate. The host dispatches them through the slot Driver.
    #[cfg(feature = "load")]
    pub fn drain_interacts(&self) -> Vec<crate::shim::InteractReq> {
        match &self.load {
            Some(isolate) => isolate.drain_interacts(),
            None => Vec::new(),
        }
    }

    /// The slot script's latest recorded paint frame (a Load isolate's
    /// host handle forwards it after every tick that painted); `None` for
    /// a compiled script or a slot that has not painted. The host copies
    /// it onto the status row so the TUI/panel views can show it in the
    /// chat pane in place of the game chat.
    #[cfg(feature = "load")]
    pub fn paint(&self) -> Option<crate::shim::ScriptPaint> {
        self.load.as_ref().and_then(|iso| iso.paint())
    }

    /// The bot instance's random-ignore list (a Load isolate's
    /// `inst.ignoredRandoms?.()`, default `[]`; empty for a compiled
    /// script — there is no instance). Cached on the isolate thread
    /// after each tick; no probe round-trip.
    #[cfg(feature = "load")]
    pub fn ignored_randoms(&self) -> Vec<String> {
        if self.state != RunState::Running || !self.want_run {
            return Vec::new();
        }
        match &self.load {
            Some(isolate) => isolate.ignored_randoms(),
            None => Vec::new(),
        }
    }

    /// Evaluate `expr` in the Load isolate's global scope (test/status
    /// read-back; e.g. the host asserting a posted snapshot field). Errors
    /// when the slot has no Load isolate.
    #[cfg(feature = "load")]
    pub fn probe(&self, expr: &str) -> Result<serde_json::Value, String> {
        match &self.load {
            Some(isolate) => isolate.probe(expr),
            None => Err("no load isolate".to_string()),
        }
    }

    /// Call only on observed server tick. Dispatches the JS isolate's
    /// `on_game_tick` (compiled path) only while Running && want_run. A
    /// compiled panic is caught: the slot goes Error with the message, the
    /// instance is dropped, the run is over.
    pub fn on_game_tick(&mut self, ctx: &mut ScriptCtx<'_>) {
        if self.state != RunState::Running || !self.want_run {
            return;
        }
        #[cfg(feature = "load")]
        if let Some(isolate) = &self.load {
            isolate.on_game_tick(ctx.tick);
            return;
        }
        let Some(script) = self.compiled.as_mut() else {
            return;
        };
        self.ticks += 1;
        let result = catch_unwind(AssertUnwindSafe(|| script.tick(ctx)));
        if let Err(payload) = result {
            self.last_error = Some(format!("script panic: {}", panic_message(&payload)));
            self.state = RunState::Error;
            self.want_run = false;
            self.compiled = None;
        }
    }

    pub fn state(&self) -> RunState {
        self.state
    }

    /// The random-event knock: ask the running compiled script whether it
    /// handles the detected event. Rising edge only (the guardian owns the
    /// per-event signature); JS isolates always answer `Host` — no isolate
    /// hook this tag. Idle / Paused / not-want-run slots answer `Host`
    /// without touching the script.
    pub fn on_random(&mut self, ev: &DetectedRandom) -> RandomClaim {
        if self.state != RunState::Running || !self.want_run {
            return RandomClaim::Host;
        }
        match &mut self.compiled {
            Some(script) => script.on_random(ev),
            // JS Load isolate: the knock has no JS arm this tag.
            None => RandomClaim::Host,
        }
    }

    #[cfg(all(feature = "memory-profile", feature = "load"))]
    pub fn memory_metrics(&self) -> Option<serde_json::Value> {
        self.load.as_ref().map(|i| i.memory_metrics())
    }

    #[cfg(all(feature = "memory-profile", feature = "load"))]
    pub fn memory_progress(&self) -> serde_json::Value {
        let mut value = self
            .load
            .as_ref()
            .map(|i| i.memory_progress())
            .unwrap_or(serde_json::json!({}));
        if let Some(fp) = &self.last_snapshot {
            let rows = |rs: &[crate::isolate_fb::ItemRowFp]| {
                rs.iter()
                    .take(32)
                    .map(|r| serde_json::json!({"name":r.name,"count":r.count,"ops":r.ops}))
                    .collect::<Vec<_>>()
            };
            value["inventory"] = serde_json::json!(rows(&fp.inv));
            value["bank"] = serde_json::json!(rows(&fp.bank));
            value["bank_open"] = serde_json::json!(fp.bank_open);
            value["bank_loaded"] = serde_json::json!(fp.bank_loaded);
        }
        value
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Drain isolate tick / `this.log` lines. Tick errors update
    /// [`Self::last_error`]. Copies are kept for [`Self::take_pending_logs`].
    #[cfg(feature = "load")]
    pub fn drain_logs(&mut self) -> Vec<String> {
        let (logs, stopped) = match &self.load {
            Some(isolate) => {
                let logs = isolate.drain_logs();
                (logs, isolate.stopped())
            }
            None => (Vec::new(), false),
        };
        if let Some(err) = logs
            .iter()
            .rev()
            .find(|l| l.starts_with("tick ") || l.contains("script requested stop"))
        {
            self.last_error = Some(err.clone());
        }
        self.pending_logs.extend(logs.iter().cloned());
        if stopped {
            self.stop();
        }
        logs
    }

    /// Take log lines staged by [`Self::drain_logs`] (panel log pane).
    pub fn take_pending_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_logs)
    }

    /// Whether a JS Load isolate is installed (feature-gated; always false
    /// in a build without the `load` feature).
    #[cfg(feature = "load")]
    pub fn load_active(&self) -> bool {
        self.load.is_some()
    }
    #[cfg(not(feature = "load"))]
    pub fn load_active(&self) -> bool {
        false
    }

    /// Whether a Load-isolate delta fingerprint was stored (always false for
    /// compiled-only slots). Used by host-play tests for OPT-004.
    #[doc(hidden)]
    pub fn has_snapshot_fingerprint(&self) -> bool {
        #[cfg(feature = "load")]
        {
            self.last_snapshot.is_some()
        }
        #[cfg(not(feature = "load"))]
        {
            false
        }
    }
}

/// Best-effort panic payload to string. Downcasts the usual `&str` and
/// `String` payloads; also unwraps a re-boxed `Box<dyn Any + Send>` payload,
/// the shape `catch_unwind` can re-arm in some unwind runtimes.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    if let Some(inner) = payload.downcast_ref::<Box<dyn std::any::Any + Send>>() {
        if let Some(s) = inner.downcast_ref::<&str>() {
            return (*s).to_string();
        }
        if let Some(s) = inner.downcast_ref::<String>() {
            return s.clone();
        }
    }
    "(no message)".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::test_support::NullDriver;

    struct Noop;

    impl Script for Noop {
        fn name(&self) -> &str {
            "noop"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
    }

    #[cfg(feature = "load")]
    #[test]
    fn script_requested_stop_cleans_slot_work_and_allows_fresh_restart() {
        let source = r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() {
        (globalThis.__rs2b0t_host.interact ||= []).push({op: 'set-camera-yaw', yaw: 123});
        ScriptRunner.stop('finished');
    }
}
"#;
        let mut slot = SlotScript::new();
        slot.start_load_with_loadouts(source.into(), LoadShape::CompatClass, vec![], &[])
            .unwrap();
        let input = crate::isolate_fb::tests::empty_input(1);
        slot.encode_snapshot_delta(&input, false);
        slot.store_last_world_id(Some(123));
        slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
        let epoch = slot.work_epoch();
        slot.load.as_ref().unwrap().on_game_tick(1);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut logs = Vec::new();
        while slot.state() == RunState::Running && Instant::now() < deadline {
            logs.extend(slot.drain_logs());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(slot.state(), RunState::Idle, "{logs:?}");
        assert!(logs
            .iter()
            .any(|line| line.contains("script requested stop")));
        assert!(!slot.has_instance());
        assert!(slot.pending_withdraw_x().is_none());
        assert_ne!(slot.work_epoch(), epoch);
        assert!(!slot.has_snapshot_fingerprint());
        assert_eq!(slot.last_world_id(), None);
        assert!(slot.drain_interacts().is_empty());
        assert!(slot.last_error().unwrap().contains("script requested stop"));
        assert_eq!(slot.take_pending_logs(), logs);
        slot.on_is_up(true);
        assert_eq!(
            slot.state(),
            RunState::Idle,
            "login cannot restart a stopped card"
        );
        slot.start_load_with_loadouts(
            "export default class T extends LoopingBot { loop() { this.n = (this.n || 0) + 1; } }"
                .into(),
            LoadShape::CompatClass,
            vec![],
            &[],
        )
        .unwrap();
        slot.load.as_ref().unwrap().on_game_tick(2);
        assert_eq!(slot.probe("__rs_bot.n").unwrap(), 1);
        assert_eq!(slot.state(), RunState::Running);
        assert!(slot.last_error().is_none());
        slot.stop();
    }

    #[cfg(feature = "load")]
    #[test]
    fn stop_releases_snapshot_storage_and_restart_emits_keyframe() {
        let mut slot = SlotScript::new();
        slot.start_compiled(Box::new(Noop)).unwrap();
        let text = "x".repeat(1024 * 1024);
        let mut input = crate::isolate_fb::tests::empty_input(1);
        input.chat_text = Some(&text);
        let first = slot.encode_snapshot_delta(&input, false);
        assert!(first.len() > text.len());
        slot.store_last_world_id(Some(123));
        slot.last_error = Some("retained diagnostic".into());
        slot.pending_logs.push("retained log".into());
        slot.pause();
        assert!(slot.has_snapshot_fingerprint());
        assert_eq!(slot.last_world_id(), Some(123));
        slot.resume();
        let delta = slot.encode_snapshot_delta(&input, false);
        assert!(delta.len() < 1024);
        slot.stop();
        assert!(!slot.has_snapshot_fingerprint());
        assert_eq!(slot.last_world_id(), None);
        assert_eq!(std::mem::take(&mut slot.ipc).into_backing_capacity(), 0);
        assert_eq!(slot.last_error.as_deref(), Some("retained diagnostic"));
        assert_eq!(slot.pending_logs, ["retained log"]);
        slot.start_compiled(Box::new(Noop)).unwrap();
        assert_eq!(slot.encode_snapshot_delta(&input, false), first);
        // The earlier owned packet remains intact after reuse and Stop.
        assert!(crate::isolate_fb::SnapshotReader::from_bytes(&first).is_ok());
    }

    #[test]
    fn on_random_defaults_to_host_and_override_claims_handle() {
        use api::random::{DetectedRandom, RandomClaim, RandomKind};

        struct ClaimHandle;
        impl Script for ClaimHandle {
            fn name(&self) -> &str {
                "claim-handle"
            }
            fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
            fn on_random(&mut self, _ev: &DetectedRandom) -> RandomClaim {
                RandomClaim::Handle
            }
        }

        let ev = DetectedRandom {
            kind: RandomKind::Dialog,
            name: "genie".to_string(),
            ours: true,
            npc_index: Some(0),
        };

        // Default: Host.
        let mut s = SlotScript::new();
        s.start_compiled(Box::new(Noop)).unwrap();
        assert_eq!(s.on_random(&ev), RandomClaim::Host);

        // Override: Handle.
        s.stop();
        s.start_compiled(Box::new(ClaimHandle)).unwrap();
        assert_eq!(s.on_random(&ev), RandomClaim::Handle);

        // Paused: Host — the knock only fires while Running.
        s.pause();
        assert_eq!(s.on_random(&ev), RandomClaim::Host);

        // Idle (stopped): Host.
        s.stop();
        assert_eq!(s.on_random(&ev), RandomClaim::Host);
    }

    #[test]
    fn ticks_counts_dispatched_ticks_since_start() {
        let mut s = SlotScript::new();
        s.start_compiled(Box::new(Noop)).unwrap();
        let mut d = NullDriver::default();
        s.on_game_tick(&mut ScriptCtx {
            driver: &mut d,
            tick: 1,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
        });
        s.on_game_tick(&mut ScriptCtx {
            driver: &mut d,
            tick: 2,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
        });
        assert_eq!(s.ticks, 2);

        // Paused ticks do not count.
        s.pause();
        s.on_game_tick(&mut ScriptCtx {
            driver: &mut d,
            tick: 3,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
        });
        assert_eq!(s.ticks, 2);

        // A fresh Start resets the counter.
        s.stop();
        s.start_compiled(Box::new(Noop)).unwrap();
        s.on_game_tick(&mut ScriptCtx {
            driver: &mut d,
            tick: 4,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: None,
            obj_names: None,
        });
        assert_eq!(s.ticks, 1);
    }

    #[test]
    fn pending_withdraw_x_pause_freezes_while_stop_and_reconnect_abort() {
        let mut slot = SlotScript::new();
        slot.start_compiled(Box::new(Noop)).unwrap();
        slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
        assert_eq!(
            slot.pending_withdraw_x().unwrap().remaining,
            Duration::from_millis(3000)
        );

        slot.pause();
        let paused = slot
            .pending_withdraw_x()
            .expect("Pause retains pending work");
        assert!(paused.deadline.is_none(), "Pause freezes monotonic time");
        slot.resume();
        assert!(
            slot.pending_withdraw_x().unwrap().deadline.is_some(),
            "Resume restores the remaining deadline"
        );

        slot.stop();
        assert!(
            slot.pending_withdraw_x().is_none(),
            "Stop aborts pending work"
        );

        slot.start_compiled(Box::new(Noop)).unwrap();
        slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3)));
        let before_reset = slot.withdraw_x_result();
        slot.reset_session_work();
        assert!(
            slot.pending_withdraw_x().is_none(),
            "reconnect/session reset aborts pending work"
        );
        assert_eq!(
            slot.withdraw_x_result(),
            (before_reset.0.wrapping_add(1), false),
            "session reset publishes an explicit abort even if a later bank reuses the generation"
        );

        slot.set_pending_withdraw_x(Some(PendingWithdrawX::waiting_load_dialog(
            2, 20, 1, 0, 20, 3,
        )));
        let before_load_reset = slot.withdraw_load_result();
        slot.reset_session_work();
        assert_eq!(
            slot.withdraw_load_result(),
            (before_load_reset.0.wrapping_add(1), false),
            "withdrawLoad gets the same explicit abort on generation-reusing session reset"
        );

        let settlement = PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3).waiting_settlement();
        assert_eq!(settlement.remaining, Duration::from_millis(4000));

        let mut expired = PendingWithdrawX::waiting_dialog(2, 7, 0, 7, 3);
        expired.deadline = Some(Instant::now() - Duration::from_millis(1));
        assert!(expired.expired(), "expiry uses monotonic wall time");
    }
}
