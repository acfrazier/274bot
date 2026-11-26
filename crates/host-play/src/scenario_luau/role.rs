use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

use api::identity::SessionKey;
use api::lifecycle::{Intent, IntentGeneration};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use parking_lot::{Mutex, MutexGuard};
use scenario::file_scenario::{self as grammar, FileCheck, FileJournal, FileRole};
use script::surface::extension::{
    ExtensionEnd, ExtensionEndCause, ExtensionFault, ExtensionFaultCode, ExtensionFunction,
};
use script::{
    surface::{
        extension::{
            ExtensionCall, ExtensionDock, ExtensionMode, ExtensionPayload, ExtensionValue,
            RunBinding,
        },
        SurfaceStart,
    },
    RunState, SlotScript,
};
use script_luau::{ExtensionRegistry, LuauScenarioExtension, LuauStartCard};
use serde_json::{json, Map, Value};

use super::{FileShotRequest, Run};
use crate::{script_runtime::NavBot, SlotArm};

const MAX_RECORDS: usize = 4096;
const LOGOUT_CAP: Duration = Duration::from_secs(30);
const MAINLAND_LANDING: (i32, i32, i32) = (3220, 3220, 0);
const DECODED_LIMIT: usize = 4096;
/// Action root ids stay disjoint from normal walk request ids.
const ACTION_ROOT_BASE: u64 = 1 << 62;
/// Check kinds that observe an action family's own inventory/message domain.
const TERMINAL_CHECKS: &[&str] = &[
    "received",
    "item_count",
    "item_delta",
    "item",
    "item_id",
    "item_at_most",
    "item_id_at_most",
    "equipment_id",
    "count",
];

pub(crate) struct Role {
    id: u64,
    name: String,
    account: String,
    options: FileRole,
    dock: Arc<ExtensionDock>,
    boundary: Mutex<()>,
    ready: AtomicBool,
    gameplay_matched: AtomicBool,
    fenced: AtomicBool,
    started: AtomicBool,
    stopped: AtomicBool,
    shot: Mutex<Option<Shot>>,
    result: OnceLock<Value>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    preamble: Preamble,
    binding: Option<RunBinding>,
    subject: Option<script::slot::SlotExecutionRef>,
    session: Option<u64>,
    journal: FileJournal,
    notes: BTreeMap<String, String>,
    records: Vec<Value>,
    queued: Option<ExtensionCall>,
    seed: Option<SeedCall>,
    relog: Option<RelogCall>,
    checks: Vec<Check>,
    active_step: Option<(String, u64)>,
    last_call_tick: Option<u64>,
    gameplay_checks: usize,
    sealed: bool,
    logout: bool,
    logout_started: Option<(Instant, IntentGeneration, SessionKey)>,
    product_phase: bool,
    retirement: Option<script::slot::RetirementTicket>,
    terminal_shot_written: bool,
    witness: Option<Value>,
    root: Option<WalkRoot>,
    action: Option<super::actions::ActionRoot>,
    paired: Vec<super::actions::PairedControl>,
    next_root: u64,
}

#[derive(Default)]
enum Preamble {
    #[default]
    Begin,
    Mainland {
        session: u64,
        player_info: u64,
    },
    PreRelog {
        session: u64,
        chat: i32,
    },
    Relog {
        intent: IntentGeneration,
        before: SessionKey,
        sent: Instant,
    },
    PostRelog {
        session: u64,
        chat: i32,
    },
    Ready,
}

struct SeedCall {
    sequence: u64,
    pending: FixtureCall,
    observer: api::scenario_observe::ScenarioObserveAdapter,
    events: Box<[Option<api::scenario_observe::AppliedEvent>]>,
}

enum FixtureCall {
    Seed(super::fixtures::PendingSeed),
    Pack(super::plan::seed_pack::PendingPackSeed),
    ServerVar {
        query: super::fixtures::PendingGetvar,
        label: String,
        deadline: u64,
    },
}

struct RelogCall {
    sequence: u64,
    intent: IntentGeneration,
    before: SessionKey,
    snapshot_session: u64,
    sent_at: Instant,
}

struct Shot {
    request: FileShotRequest,
    sequence: Option<u64>,
    result: Option<Result<(u64, std::path::PathBuf), String>>,
    /// False until the tab interaction or the `-later` delay is done.
    visible: bool,
    options: ShotOptions,
    later_from: Option<Instant>,
}

#[derive(Default)]
struct ShotOptions {
    /// Side-tab index selected through the normal SideTab interaction.
    tab: Option<i32>,
    /// Monotonic delay before the separate `-later` capture (LD2 §9.2).
    again_after: Option<Duration>,
}

impl ShotOptions {
    fn parse(args: &mut Map<String, Value>) -> Result<Self, String> {
        // Standard side-tab order: stats 1, quests 2, inventory 3, worn 4,
        // prayer 5, magic 6 (the same indices `InteractReq::SideTab` takes).
        let tab = optional_text(args, "tab")?
            .map(|name| match name.as_str() {
                "stats" => Ok(1),
                "quests" => Ok(2),
                "inventory" => Ok(3),
                "worn" => Ok(4),
                "prayer" => Ok(5),
                "magic" => Ok(6),
                _ => Err("shot.tab: expected inventory, worn, stats, quests, prayer or magic"),
            })
            .transpose()?;
        let again_after = take_optional_u64(args, "again_after_s", 1, 60)?.map(Duration::from_secs);
        Ok(Self { tab, again_after })
    }
}

struct Check {
    seq: u64,
    name: String,
    label: String,
    check: CheckKind,
    last_tick: u64,
    probe: bool,
    /// The action root open when this check was admitted.
    root: Option<u64>,
    outcome: Option<ExtensionPayload>,
}

enum CheckKind {
    Proof(FileCheck),
    Signal(String),
}

impl CheckKind {
    fn parse(name: &str, mut args: Map<String, Value>) -> Result<Self, String> {
        if name == "signal" {
            let name = grammar::take_string(&mut args, "name")?;
            grammar::empty(&args)?;
            Ok(Self::Signal(name))
        } else {
            FileCheck::parse(name, args).map(Self::Proof)
        }
    }
}

struct WalkRoot {
    request: u64,
    route_pending: bool,
    session: u64,
    target: WorldTile,
    radius: u16,
    send: Option<WalkSend>,
}

impl WalkRoot {
    fn closure(&self, snapshot: &GameSnapshot) -> Option<WalkSend> {
        self.send.filter(|send| {
            !self.route_pending
                && self.session == snapshot.gens().session
                && snapshot.gens().player_info > send.raw_player_info
                && snapshot.tile() == Some((send.aim.x, send.aim.z, send.aim.level))
                && snapshot
                    .local_player()
                    .is_some_and(|p| !p.player.actor.moving)
        })
    }
}

/// The exact successful normal-owner movement attempt, not a route admission or queue sample.
#[derive(Clone, Copy)]
pub(crate) struct WalkSend {
    pub(crate) nav_request: u64,
    pub(crate) aim: WorldTile,
    pub(crate) sent_from: WorldTile,
    pub(crate) raw_player_info: u64,
    pub(crate) commit: u64,
}

#[derive(Default)]
pub(crate) struct WalkTrace {
    pub(crate) request: u64,
    pub(crate) send: Option<WalkSend>,
    pub(crate) unsupported_continuation: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct WalkObservation {
    request: u64,
    send: Option<WalkSend>,
    unsupported_continuation: bool,
    outcome: u64,
    failed: bool,
    pending: bool,
}

impl WalkObservation {
    pub(crate) fn capture(bot: &NavBot) -> Option<Self> {
        let trace = bot.file_walk.as_ref()?;
        Some(Self {
            request: trace.request,
            send: trace.send,
            unsupported_continuation: trace.unsupported_continuation,
            outcome: bot.walk_outcome_request_id,
            failed: bot.walk_outcome_failed || bot.walk_outcome_blocked,
            pending: bot.has_route() || bot.route_work_pending() || bot.bank_fetch.is_some(),
        })
    }
}

impl Role {
    pub(super) fn new(id: u64, name: &str, account: &str, options: FileRole) -> Self {
        Self {
            id,
            name: name.into(),
            account: account.into(),
            options,
            dock: Arc::new(ExtensionDock::new()),
            boundary: Mutex::new(()),
            ready: AtomicBool::new(false),
            fenced: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            shot: Mutex::new(None),
            gameplay_matched: AtomicBool::new(false),
            started: AtomicBool::new(false),
            result: OnceLock::new(),
            state: Mutex::new(State::default()),
        }
    }

    pub(crate) fn account(&self) -> &str {
        &self.account
    }
    pub(crate) fn boundary(&self) -> MutexGuard<'_, ()> {
        self.boundary.lock()
    }
    pub(super) fn try_boundary(&self) -> Option<MutexGuard<'_, ()>> {
        self.boundary.try_lock()
    }
    pub(super) fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub(super) fn gameplay_matched(&self) -> bool {
        self.gameplay_matched.load(Ordering::Acquire)
    }
    pub(super) fn not_started(&self) -> bool {
        !self.started.load(Ordering::Acquire)
    }
    pub(super) fn done(&self) -> bool {
        self.result.get().is_some()
    }
    pub(super) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(super) fn resolved(&self) -> bool {
        self.result
            .get()
            .is_some_and(|result| result["resolved"] == true)
    }

    pub(super) fn pending_shot(&self) -> Option<FileShotRequest> {
        self.shot
            .lock()
            .as_ref()
            .filter(|shot| shot.result.is_none() && shot.visible)
            .map(|shot| shot.request.clone())
    }

    pub(super) fn complete_shot(
        &self,
        request: &FileShotRequest,
        presented_tick: u64,
        output: Result<std::path::PathBuf, String>,
    ) -> Result<(), String> {
        let mut shot = self.shot.lock();
        let shot = shot
            .as_mut()
            .filter(|shot| &shot.request == request && shot.result.is_none() && shot.visible)
            .ok_or("capture completion does not match a pending request")?;
        shot.result = Some(output.and_then(|path| {
            if presented_tick <= request.tick || !path.is_file() {
                return Err(
                    "capture did not identify a fresh presented frame and durable file".into(),
                );
            }
            Ok((presented_tick, path))
        }));
        Ok(())
    }

    fn request_shot(
        &self,
        run: &Run,
        snapshot: &GameSnapshot,
        sequence: Option<u64>,
        label: String,
        options: ShotOptions,
    ) -> Result<(), String> {
        if !run.plan.headed {
            return Err("capture requires a headed File plan".into());
        }
        if label.is_empty()
            || label.len() > 96
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        {
            return Err(
                "capture label must contain only letters, digits, hyphens or underscores".into(),
            );
        }
        let mut shot = self.shot.lock();
        if shot.is_some() {
            return Err("capture already pending".into());
        }
        *shot = Some(Shot {
            request: FileShotRequest {
                id: sequence.unwrap_or(u64::MAX),
                account: self.account.clone(),
                label,
                session: snapshot.gens().session,
                tick: u64::from(snapshot.tick()),
                plan_id: run.id,
            },
            sequence,
            result: None,
            visible: options.tab.is_none(),
            options,
            later_from: None,
        });
        Ok(())
    }

    /// Owner-side shot preparation: the normal SideTab interaction before the
    /// first capture, and the monotonic delay before the `-later` capture.
    fn prepare_shot(&self, client: &mut Client, snapshot: &GameSnapshot) -> Result<(), String> {
        let mut slot = self.shot.lock();
        let Some(shot) = slot.as_mut().filter(|shot| !shot.visible) else {
            return Ok(());
        };
        if let Some(started) = shot.later_from {
            let delay = shot
                .options
                .again_after
                .ok_or("later capture has no delay")?;
            if started.elapsed() >= delay {
                shot.request.tick = u64::from(snapshot.tick());
                shot.visible = true;
            }
            return Ok(());
        }
        let tab = shot.options.tab.ok_or("hidden capture has no tab")?;
        let mut interactions = api::interact::Interactions::new(snapshot, client);
        if !matches!(
            interactions.click_side_tab(tab),
            api::interact::SendResult::Sent { .. }
        ) {
            return Err(format!("capture side tab {tab} is unavailable"));
        }
        // The capture must present a frame after this tab selection.
        shot.request.tick = u64::from(snapshot.tick());
        shot.visible = true;
        Ok(())
    }

    fn observe_shot(&self, state: &mut State, snapshot: &GameSnapshot) -> Result<bool, String> {
        let mut slot = self.shot.lock();
        let Some(shot) = slot.as_mut() else {
            return Ok(true);
        };
        if snapshot.gens().session != shot.request.session {
            return Err("capture session changed".into());
        }
        let Some(result) = shot.result.take() else {
            return Ok(false);
        };
        let (presented_tick, path) = result?;
        state.record(json!({"shot":shot.request.label,"request":shot.request.id,"session":shot.request.session,"tab":shot.options.tab,"presented_tick":presented_tick,"path":path}))?;
        if let (Some(_), None) = (shot.options.again_after, shot.later_from) {
            // The first capture is written; the `-later` capture has its own
            // request, presented frame and write identity.
            shot.later_from = Some(Instant::now());
            shot.visible = false;
            shot.request.label.push_str("-later");
            return Ok(false);
        }
        if let Some(sequence) = shot.sequence {
            self.complete(state, sequence, ExtensionPayload::Unit)?;
        } else {
            state.terminal_shot_written = true;
        }
        *slot = None;
        Ok(true)
    }
    pub(crate) fn fenced(&self) -> bool {
        self.fenced.load(Ordering::Acquire)
    }

    pub(super) fn publish_fence(&self) {
        self.fenced.store(true, Ordering::Release);
        self.dock.publish_fence();
    }

    pub(super) fn revoke(&self) {
        self.fenced.store(true, Ordering::Release);
        self.dock.publish_revocation();
    }

    pub(super) fn receipt(&self) -> Value {
        self.result.get().cloned().unwrap_or_else(|| json!({"role": self.name, "account": self.account, "resolved": false, "fenced": self.fenced()}))
    }

    pub(crate) fn owner_frame(
        &self,
        run: &super::FileRun,
        client: &mut Client,
        snapshot: &GameSnapshot,
        slot: Option<&mut SlotScript>,
        inputs: OwnerInputs<'_>,
    ) {
        let run = &run.0;
        if self.done() {
            return;
        }
        run.check_deadline();
        let mut state = self.state.lock();
        let result = self.owner_poll(run, &mut state, client, snapshot, slot, &inputs);
        if let Err(error) = result {
            let phase = if state.product_phase {
                "product-or-oracle"
            } else {
                "fixture"
            };
            run.fail(format!("{} ({phase}): {error}", self.name));
        }
    }

    fn owner_poll(
        &self,
        run: &Run,
        state: &mut State,
        client: &mut Client,
        snapshot: &GameSnapshot,
        slot: Option<&mut SlotScript>,
        inputs: &OwnerInputs<'_>,
    ) -> Result<(), String> {
        if !self.fenced() && state.binding.is_none() {
            if !snapshot.ingame()
                || snapshot.scene_state() != 2
                || client.stream.is_none()
                || inputs.held
            {
                return Ok(());
            }
            if !self.prepare_account(state, client, snapshot, inputs.arm)? {
                return Ok(());
            }
            let slot = slot.ok_or("missing role slot")?;
            self.start(run, state, snapshot, slot, inputs)?;
            return Ok(());
        }
        if let Some(fault) = self.dock.fault() {
            run.fail(format!("{} scenario extension fault: {fault:?}", self.name));
        }
        if self.fenced() {
            return self.retire(run, state, client, snapshot, slot, inputs);
        }
        let Some(slot) = slot else {
            return Err("role slot disappeared".into());
        };
        self.validate_subject(state, slot)?;
        if let Some(relog) = &state.relog {
            if snapshot.gens().session != relog.snapshot_session
                && snapshot.ingame()
                && snapshot.scene_state() == 2
                && client.stream.is_some()
                && !inputs.held
            {
                // The snapshot can see the new session before the slot
                // lifecycle records the clean owned logout and next session
                // (the tutorial preamble waits the same way). Wait for that
                // record within the logout cap; never accept a foreign session.
                if !inputs.arm.confirms_file_relog(relog.intent, relog.before) {
                    if relog.sent_at.elapsed() >= LOGOUT_CAP {
                        return Err("new session does not match the owned relog".into());
                    }
                    return Ok(());
                }
                let seq = relog.sequence;
                state.session = Some(snapshot.gens().session);
                state.relog = None;
                state.last_call_tick = Some(u64::from(snapshot.tick()));
                state.active_step = None;
                state.record(json!({"sequence":seq,"relog":{"new_session":snapshot.gens().session,"tick":snapshot.tick()}}))?;
                self.complete(state, seq, ExtensionPayload::Unit)?;
            } else if inputs.arm.session_key() == Some(relog.before)
                && relog.sent_at.elapsed() >= LOGOUT_CAP
            {
                return Err(
                    "relog did not detach the original session within the logout cap".into(),
                );
            }
            return Ok(());
        }
        if !snapshot.ingame() || client.stream.is_none() {
            return Err("bound File session disconnected outside owned terminal logout".into());
        }
        if state.session != Some(snapshot.gens().session) {
            return Err("bound File session changed".into());
        }
        if !run.plan.scenario.death_allowed
            && snapshot
                .local_player()
                .is_some_and(|p| p.player.actor.health == 0 && p.player.actor.in_combat)
        {
            return Err("role died".into());
        }
        let selected = inputs.selected.ok_or("selected game data is unavailable")?;
        if let Some(walk) = inputs.nav {
            Self::observe_walk(state, snapshot, walk)?;
        }
        // A fixture or relog waits behind any open root: a later seed must
        // never change the closure precondition of an earlier action.
        let queued = if inputs.held || state.root.is_some() || state.action.is_some() {
            None
        } else {
            state.queued.take()
        };
        if let Some(call) = queued {
            if call.name == "relog" {
                let mut args = named(call.arguments)?;
                let why = grammar::take_string(&mut args, "why")?;
                grammar::empty(&args)?;
                let before = inputs
                    .arm
                    .session_key()
                    .ok_or("relog has no current host session")?;
                let intent = inputs
                    .arm
                    .post_intent(Intent::RelogNow)
                    .ok_or("relog intent refused")?
                    .generation();
                state.relog = Some(RelogCall {
                    sequence: call.sequence,
                    intent,
                    before,
                    snapshot_session: snapshot.gens().session,
                    sent_at: Instant::now(),
                });
                state.record(json!({"sequence":call.sequence,"relog":{"why":why,"intent":intent.get(),"before":snapshot.gens().session}}))?;
                return Ok(());
            }
            if call.name == "expect.server_var" {
                self.handle_server_var(run, state, client, snapshot, inputs, call)?;
            } else if call.name == "seed.ext" {
                self.handle_pack_seed(run, state, client, snapshot, inputs, call)?;
            } else {
                self.handle_seed(run, state, client, snapshot, inputs, call)?;
            }
        }
        if let Some(seed) = &mut state.seed {
            let drain = seed.observer.drain(client, &mut seed.events);
            if drain.remaining != 0 || drain.fault.is_some() {
                return Err(format!("seed observation incomplete: {drain:?}"));
            }
            let at = seed
                .observer
                .watermark(client)
                .map_err(|err| format!("seed watermark: {err:?}"))?;
            let (kind, receipt) = match &mut seed.pending {
                FixtureCall::Seed(pending) => (
                    "seed",
                    pending.poll(client, snapshot, at, &seed.events[..drain.written])?,
                ),
                FixtureCall::Pack(pending) => (
                    "seed",
                    pending.poll(client, snapshot, at, &seed.events[..drain.written])?,
                ),
                FixtureCall::ServerVar {
                    query,
                    label,
                    deadline,
                } => {
                    let tick = u64::from(snapshot.tick());
                    if tick > *deadline {
                        return Err(format!("server_var check {label:?} exceeded its deadline"));
                    }
                    let answer = query.poll(snapshot, at)?;
                    if answer.is_none() && tick == *deadline {
                        return Err(format!(
                            "server_var check {label:?} did not match at its deadline"
                        ));
                    }
                    ("server_var", answer)
                }
            };
            if let Some(receipt) = receipt {
                let seq = seed.sequence;
                client.cancel_decoded_domain_observer();
                state.record(json!({"sequence":seq,(kind):receipt}))?;
                state.seed = None;
                self.complete(state, seq, ExtensionPayload::Unit)?;
            }
        }
        self.poll_action(run, state, client, snapshot)?;
        self.evaluate_checks(run, state, snapshot, selected)?;
        self.prepare_shot(client, snapshot)?;
        let shot_complete = self.observe_shot(state, snapshot)?;
        let tick = u64::from(snapshot.tick());
        if !state.sealed {
            if let Some((label, deadline)) = &state.active_step {
                if tick >= *deadline {
                    return Err(format!("step {label:?} exhausted its observed tick budget"));
                }
            } else if state
                .last_call_tick
                .is_some_and(|start| tick >= start + u64::from(run.plan.scenario.idle_ticks))
            {
                return Err("scenario idle tick budget exhausted".into());
            }
        }
        let settled = state.sealed
            && state.seed.is_none()
            && state.queued.is_none()
            && state.checks.iter().all(|check| check.outcome.is_some())
            && state.root.is_none()
            && state.action.is_none();
        if settled {
            if let Some(owed) = state.paired.first() {
                return Err(format!(
                    "silent root {} has no same-item paired level control at level {}",
                    owed.silent_root, owed.level
                ));
            }
        }
        let ready = settled
            && shot_complete
            && run.plan.scenario.proof.check(
                snapshot,
                selected,
                &state.journal,
                snapshot.gens().session,
            )?;
        self.ready.store(ready, Ordering::Release);
        Ok(())
    }

    fn handle_seed(
        &self,
        run: &Run,
        state: &mut State,
        client: &mut Client,
        snapshot: &GameSnapshot,
        inputs: &OwnerInputs<'_>,
        call: ExtensionCall,
    ) -> Result<(), String> {
        let selected = inputs.selected.ok_or("selected game data is unavailable")?;
        use super::fixtures::{BuiltinSeed, SeedPolicy, SeedRequest};
        let operation = BuiltinSeed::parse(call.name).ok_or("unknown File seed operation")?;
        let mut args = named(call.arguments)?;
        let why = grammar::take_string(&mut args, "why")?;
        let definitions = run
            .fixtures
            .get()
            .ok_or("missing bound fixture definitions")?;
        let selection = definitions.selection(operation, &args)?;
        let key = inputs
            .arm
            .session_key()
            .ok_or("seed requires a current host session")?;
        let observer = api::scenario_observe::ScenarioObserveAdapter::arm(
            client,
            key,
            selection,
            DECODED_LIMIT,
        )
        .map_err(|err| format!("seed observation unavailable: {err:?}"))?;
        let policy = if run.plan.scenario.confirmed_only {
            SeedPolicy::ConfirmedOnly
        } else {
            SeedPolicy::AllowUnconfirmed
        };
        let at = observer
            .watermark(client)
            .map_err(|err| format!("seed watermark: {err:?}"))?;
        state.record(json!({
            "sequence": call.sequence, "intervention": operation.name(),
            "why": why, "requested": args, "tick": snapshot.tick(),
            "session": snapshot.gens().session, "client_session": at.client_session,
            "source_before_send": at.source_serial, "application_before_send": at.application_serial,
        }))?;
        let pending = match definitions.start(
            client,
            snapshot,
            selected,
            SeedRequest {
                operation,
                args: &args,
                why: &why,
                policy,
            },
            at,
        ) {
            Ok(pending) => pending,
            Err(error) => {
                state.record(json!({"sequence":call.sequence,"fixture_start_failed":error}))?;
                return Err(error);
            }
        };
        let (command, readback) = pending.sent_commands();
        state.record(json!({
            "sequence": call.sequence, "dispatch": "sent",
            "command": command, "readback_command": readback,
        }))?;
        let captures_events = matches!(
            operation,
            BuiltinSeed::Give
                | BuiltinSeed::ClearInventory
                | BuiltinSeed::SetStat
                | BuiltinSeed::ResetAndAdvanceStat
                | BuiltinSeed::SetVar
                | BuiltinSeed::EnergyFullRunOn
        );
        let events: Box<[_]> = if captures_events {
            std::iter::repeat_with(|| None)
                .take(DECODED_LIMIT)
                .collect()
        } else {
            Box::default()
        };
        state.seed = Some(SeedCall {
            sequence: call.sequence,
            pending: FixtureCall::Seed(pending),
            observer,
            events,
        });
        Ok(())
    }

    /// LD2 §8.4: one typed operation from the path scenario's reviewed pack.
    /// The script supplies typed arguments only, never the command line.
    fn handle_pack_seed(
        &self,
        run: &Run,
        state: &mut State,
        client: &mut Client,
        snapshot: &GameSnapshot,
        inputs: &OwnerInputs<'_>,
        call: ExtensionCall,
    ) -> Result<(), String> {
        use super::plan::seed_pack::{PackSeedRequest, SelectedPackNames};
        let pack = run
            .plan
            .seed_pack
            .as_ref()
            .ok_or("seed.ext requires a seed pack for this path scenario")?;
        let selected = inputs.selected.ok_or("selected game data is unavailable")?;
        let definitions = run
            .fixtures
            .get()
            .ok_or("missing bound fixture definitions")?;
        let mut args = named(call.arguments)?;
        let why = grammar::take_string(&mut args, "why")?;
        let operation = grammar::take_string(&mut args, "op")?;
        let names = SelectedPackNames::new(definitions, selected);
        let selection = pack.selection(&operation, &args, &names)?;
        let key = inputs
            .arm
            .session_key()
            .ok_or("seed requires a current host session")?;
        let observer = api::scenario_observe::ScenarioObserveAdapter::arm(
            client,
            key,
            selection,
            DECODED_LIMIT,
        )
        .map_err(|err| format!("seed observation unavailable: {err:?}"))?;
        let at = observer
            .watermark(client)
            .map_err(|err| format!("seed watermark: {err:?}"))?;
        state.record(json!({
            "sequence": call.sequence, "intervention": "seed.ext", "operation": operation,
            "why": why, "requested": args, "tick": snapshot.tick(),
            "session": snapshot.gens().session, "seed_pack_sha256": pack.sha256(),
            "grant": run.plan.seed_pack_grant, "source_before_send": at.source_serial,
        }))?;
        let policy = if run.plan.scenario.confirmed_only {
            super::fixtures::SeedPolicy::ConfirmedOnly
        } else {
            super::fixtures::SeedPolicy::AllowUnconfirmed
        };
        let pending = pack.start(
            client,
            snapshot,
            &names,
            PackSeedRequest {
                operation_name: &operation,
                args: &args,
                why: &why,
                policy,
                watermark: at,
                grant_identity: run.plan.seed_pack_grant.as_deref(),
            },
        )?;
        state.seed = Some(SeedCall {
            sequence: call.sequence,
            pending: FixtureCall::Pack(pending),
            observer,
            events: std::iter::repeat_with(|| None)
                .take(DECODED_LIMIT)
                .collect(),
        });
        Ok(())
    }

    fn handle_server_var(
        &self,
        run: &Run,
        state: &mut State,
        client: &mut Client,
        snapshot: &GameSnapshot,
        inputs: &OwnerInputs<'_>,
        call: ExtensionCall,
    ) -> Result<(), String> {
        let mut args = named(call.arguments)?;
        let label = optional_text(&mut args, "label")?.unwrap_or_else(|| call.name.to_owned());
        let budget = take_optional_u64(&mut args, "within", 1, 20)?.unwrap_or(20);
        let deadline = state
            .last_call_tick
            .ok_or("server_var was not admitted")?
            .checked_add(budget)
            .ok_or("server_var deadline overflow")?;
        if u64::from(snapshot.tick()) >= deadline {
            return Err("server_var expired before dispatch".into());
        }
        let definitions = run
            .fixtures
            .get()
            .ok_or("missing bound fixture definitions")?;
        let selection = definitions.server_var_selection(&args)?;
        let key = inputs
            .arm
            .session_key()
            .ok_or("server_var requires a current host session")?;
        let observer = api::scenario_observe::ScenarioObserveAdapter::arm(
            client,
            key,
            selection,
            DECODED_LIMIT,
        )
        .map_err(|err| format!("server_var observation unavailable: {err:?}"))?;
        let watermark = observer
            .watermark(client)
            .map_err(|err| format!("server_var watermark: {err:?}"))?;
        let query = definitions.start_server_var(client, snapshot, &args, &label, watermark)?;
        state.seed = Some(SeedCall {
            sequence: call.sequence,
            pending: FixtureCall::ServerVar {
                query,
                label,
                deadline,
            },
            observer,
            events: Box::new([]),
        });
        Ok(())
    }

    fn prepare_account(
        &self,
        state: &mut State,
        client: &mut Client,
        snapshot: &GameSnapshot,
        arm: &SlotArm,
    ) -> Result<bool, String> {
        use scenario::tutorial::{
            chat_baseline, parse_tutorial_reply, TUTORIAL_COMPLETE, TUTORIAL_GETVAR,
            TUTORIAL_SETVAR,
        };
        let gens = snapshot.gens();
        let send = |client: &mut Client, command: &str| {
            api::interact::cheat(client, command)
                .is_sent()
                .then_some(())
                .ok_or_else(|| "guarded account preamble send was refused".to_owned())
        };
        let get_tutorial = |client: &mut Client| -> Result<(), String> {
            send(client, TUTORIAL_SETVAR)?;
            send(client, TUTORIAL_GETVAR)
        };
        let confirmed = |chat| {
            snapshot.chat_lines().iter().any(|line| {
                line.sequence > chat && parse_tutorial_reply(&line.text) == Some(TUTORIAL_COMPLETE)
            })
        };
        let relog = self.options.preamble.is_some();
        match &state.preamble {
            Preamble::Begin if self.options.mainland => {
                // Existing guarded hop: api::interact::mainland_hop and the
                // Table runner's exact MAINLAND_LANDING check.
                send(client, &format!("tele {}", api::interact::OFF_ISLAND_TELE))?;
                send(client, TUTORIAL_SETVAR)?;
                state.preamble = Preamble::Mainland {
                    session: gens.session,
                    player_info: gens.player_info,
                };
            }
            Preamble::Begin if relog => {
                get_tutorial(client)?;
                state.preamble = Preamble::PreRelog {
                    session: gens.session,
                    chat: chat_baseline(snapshot),
                };
            }
            Preamble::Begin | Preamble::Ready => return Ok(true),
            Preamble::Mainland {
                session,
                player_info,
            } => {
                if gens.session != *session {
                    return Err("mainland hop crossed an unowned session".into());
                }
                if gens.player_info > *player_info && snapshot.tile() == Some(MAINLAND_LANDING) {
                    state.record(json!({"preamble":"mainland","tile":MAINLAND_LANDING,"session":gens.session,"player_info":gens.player_info}))?;
                    if !relog {
                        state.preamble = Preamble::Ready;
                        return Ok(true);
                    }
                    get_tutorial(client)?;
                    state.preamble = Preamble::PreRelog {
                        session: gens.session,
                        chat: chat_baseline(snapshot),
                    };
                }
            }
            Preamble::PreRelog { session, chat } => {
                if gens.session != *session {
                    return Err("tutorial preamble crossed an unowned session".into());
                }
                if confirmed(*chat) {
                    let before = arm
                        .session_key()
                        .ok_or("tutorial preamble has no current session")?;
                    let intent = arm
                        .post_intent(Intent::RelogNow)
                        .ok_or("tutorial relog was refused")?
                        .generation();
                    state.preamble = Preamble::Relog {
                        intent,
                        before,
                        sent: Instant::now(),
                    };
                }
            }
            Preamble::Relog {
                intent,
                before,
                sent,
            } => {
                if !arm.confirms_file_relog(*intent, *before) {
                    if arm.session_key() != Some(*before) || sent.elapsed() >= LOGOUT_CAP {
                        return Err("tutorial preamble did not observe its owned relog".into());
                    }
                } else if snapshot
                    .side_tabs()
                    .iter()
                    .any(|tab| tab.index == 3 && tab.available)
                {
                    get_tutorial(client)?;
                    state.preamble = Preamble::PostRelog {
                        session: gens.session,
                        chat: chat_baseline(snapshot),
                    };
                }
            }
            Preamble::PostRelog { session, chat } => {
                if gens.session != *session {
                    return Err("tutorial reseed crossed an unowned session".into());
                }
                if confirmed(*chat) {
                    state.record(json!({"preamble":"tutorial","session":gens.session,"tutorial":TUTORIAL_COMPLETE}))?;
                    state.preamble = Preamble::Ready;
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn start(
        &self,
        run: &Run,
        state: &mut State,
        snapshot: &GameSnapshot,
        slot: &mut SlotScript,
        inputs: &OwnerInputs<'_>,
    ) -> Result<(), String> {
        if slot.state() != RunState::Idle {
            return Err("scenario start requires the role's Idle slot".into());
        }
        let selected = inputs.selected.ok_or("selected game data is unavailable")?;
        let identity = slot
            .next_surface_start_identity()
            .map_err(|err| format!("{err:?}"))?;
        let binding = RunBinding {
            slot_incarnation: identity.slot_incarnation,
            run_key: identity.run_key,
            dock_id: run
                .id
                .checked_mul(16)
                .and_then(|id| id.checked_add(self.id))
                .ok_or("dock identity exhausted")?,
        };
        let extension = LuauScenarioExtension::new(
            Arc::clone(&self.dock),
            binding,
            Arc::clone(registry()),
            run.plan.scenario.case.as_str(),
            self.name.as_str(),
        );
        let card = LuauStartCard::new_scenario(
            run.plan.scenario.name.as_str(),
            run.plan.source.clone(),
            extension,
        );
        let start = SurfaceStart {
            card: Arc::new(card),
            settings_bag: None,
            game_data: Some(Arc::clone(selected)),
            named_banks: Arc::clone(inputs.named_banks),
            wilderness_rules: script::surface::no_wilderness(),
            loadouts: Arc::from([]),
        };
        let admission = inputs
            .start_hold
            .lock()
            .map_err(|_| "script start admission lock poisoned")?;
        if self.fenced() || admission.strong_count() != 0 {
            return Ok(());
        }
        let token = slot
            .admit_surface_loading(start, identity)
            .map_err(|err| format!("{err:?}"))?;
        self.started.store(true, Ordering::Release);
        state.binding = Some(binding);
        state.subject = slot.surface_execution_observation().execution;
        state.session = Some(snapshot.gens().session);
        state.last_call_tick = Some(u64::from(snapshot.tick()));
        drop(admission);
        state.record(json!({"transition":"Loading","run_key":binding.run_key,"dock_id":binding.dock_id,"slot_incarnation":binding.slot_incarnation,"start_serial":identity.start_serial}))?;
        slot.load_admitted_surface(token)
            .map_err(|err| format!("{err:?}"))?;
        if self.fenced() {
            return Ok(());
        }
        if !matches!(slot.state(), RunState::Running | RunState::Paused) {
            return Err("Luau Load returned without a Ready slot".into());
        }
        self.validate_subject(state, slot)?;
        state.record(
            json!({"transition":"Ready","run_key":binding.run_key,"dock_id":binding.dock_id}),
        )?;
        Ok(())
    }

    fn validate_subject(&self, state: &State, slot: &SlotScript) -> Result<(), String> {
        if state.subject.as_ref() != slot.surface_execution_observation().execution.as_ref()
            || self.dock.end().is_some()
        {
            return Err("unexpected scenario slot execution transition".into());
        }
        Ok(())
    }

    fn retire(
        &self,
        run: &Run,
        state: &mut State,
        client: &mut Client,
        snapshot: &GameSnapshot,
        slot: Option<&mut SlotScript>,
        inputs: &OwnerInputs<'_>,
    ) -> Result<(), String> {
        let Some(subject) = state.subject.as_ref() else {
            if run.failure.get().is_none() {
                return Err("fenced role has no execution identity".into());
            }
            self.stopped.store(true, Ordering::Release);
            self.finish_receipt(state, false, "no_execution");
            return Ok(());
        };
        let owner = slot.ok_or("retirement slot unavailable")?;
        let current = owner.surface_execution_observation();
        if run.failure.get().is_some()
            && state.retirement.is_none()
            && current.state == RunState::Idle
            && current.execution.is_none()
            && current.slot_incarnation == subject.slot_incarnation
            && current.start_serial == subject.start_serial
        {
            let kind = match (&current.extension_end, self.dock.end()) {
                (None, None) if current.runtime_generation < subject.runtime_generation => {
                    "failed_before_execution"
                }
                (Some(end), Some(dock_end))
                    if *end == dock_end && Some(end.binding) == state.binding && end.vm_dropped =>
                {
                    "joined_failed_execution"
                }
                _ => {
                    self.finish_receipt(state, false, "unresolved_subject");
                    return Err("failed Start or Stop has no matching terminal evidence".into());
                }
            };
            state.record(json!({
                "retirement":kind,
                "slot_incarnation":current.slot_incarnation,
                "start_serial":current.start_serial,
                "runtime_generation":current.runtime_generation
            }))?;
            self.stopped.store(true, Ordering::Release);
            self.finish_receipt(state, true, "not_requested");
            return Ok(());
        }
        if state.retirement.is_none() {
            client.cancel_decoded_domain_observer();
            if run.failure.get().is_none() {
                let selected = inputs.selected.ok_or("selected game data is unavailable")?;
                let valid = state.sealed
                    && state.root.is_none()
                    && state.action.is_none()
                    && state.paired.is_empty()
                    && state.seed.is_none()
                    && state.checks.iter().all(|check| check.outcome.is_some())
                    && snapshot.ingame()
                    && snapshot.scene_state() == 2
                    && state.session == Some(snapshot.gens().session)
                    && run.plan.scenario.proof.check(
                        snapshot,
                        selected,
                        &state.journal,
                        snapshot.gens().session,
                    )?;
                if !valid {
                    run.fail(format!(
                        "{}: final fenced proof or action closure unavailable",
                        self.name
                    ));
                } else {
                    state.witness = Some(
                        json!({"session":snapshot.gens().session,"tick":snapshot.tick(),"player_info":snapshot.gens().player_info,"tile":snapshot.tile()}),
                    );
                }
            }
            let result = owner.stop_expected_execution(
                subject,
                script::slot::StopAuthorization {
                    plan_id: run.id,
                    step_ordinal: 1,
                    stop_ordinal: 1,
                },
            );
            match result {
                Ok(ticket) => {
                    state.retirement = Some(ticket);
                    crate::play_scripts::retire_script_run_status(inputs.statuses, &self.account);
                }
                Err(err) => {
                    self.finish_receipt(state, false, "unresolved_subject");
                    return Err(format!("owned expected-subject Stop: {err:?}"));
                }
            }
        }
        let ticket = state
            .retirement
            .as_ref()
            .expect("retirement assigned above");
        let current = owner.surface_execution_observation();
        if current.state == RunState::Stopping
            && current.execution.as_ref() == Some(&ticket.execution)
            && current.retirement_ticket.as_ref() == Some(ticket)
        {
            return Ok(());
        }
        let expected_cause = if self.dock.is_finished() {
            ExtensionEndCause::Completed
        } else {
            ExtensionEndCause::Stopped
        };
        let end = current.extension_end.as_ref().filter(|end| {
            Some(end.binding) == state.binding
                && end.vm_dropped
                && self.dock.end().as_ref() == Some(*end)
        });
        if !ticket.matches_idle(&current) || end.is_none() {
            self.finish_receipt(state, false, "wrong_stop_frontier_or_fault");
            return Err(
                "retired execution does not match its expected closed end and idle frontier".into(),
            );
        }
        if run.failure.get().is_none()
            && !end.is_some_and(|end| end.is_clean_end(end.binding, expected_cause))
        {
            run.fail(format!(
                "{}: retired execution has an unexpected cause or fault",
                self.name
            ));
        }
        self.stopped.store(true, Ordering::Release);
        if run.failure.get().is_none() {
            if let Some(label) = &run.plan.scenario.terminal_shot {
                if !state.terminal_shot_written {
                    if self.shot.lock().is_none() {
                        self.request_shot(
                            run,
                            snapshot,
                            None,
                            label.clone(),
                            ShotOptions::default(),
                        )?;
                    }
                    if !self.observe_shot(state, snapshot)? {
                        return Ok(());
                    }
                }
            }
        }
        if state.logout {
            if let Some((started, intent, session)) = state.logout_started {
                if !client.ingame && client.stream.is_none() {
                    if !inputs.arm.confirms_file_logout(intent, session) {
                        self.finish_receipt(state, false, "unattributed_disconnect");
                        return Err("terminal disconnect does not match the owned logout send and server response".into());
                    }
                    state.record(json!({"logout_intent":intent.get(),"logout_session":format!("{session:?}"),"server_logout":true}))?;
                    self.finish_receipt(state, true, "detached");
                } else if started.elapsed() >= LOGOUT_CAP {
                    self.finish_receipt(state, false, "logout_expired");
                    return Err("terminal logout exceeded its observation cap".into());
                }
                return Ok(());
            }
            let session = inputs
                .arm
                .session_key()
                .ok_or("terminal logout has no current host session")?;
            let intent = inputs
                .arm
                .post_intent(api::lifecycle::Intent::Logout)
                .ok_or("terminal logout intent refused")?;
            state.logout_started = Some((Instant::now(), intent.generation(), session));
        } else {
            self.finish_receipt(state, true, "not_requested");
        }
        Ok(())
    }

    fn finish_receipt(&self, state: &State, resolved: bool, logout: &str) {
        let _ = self.result.set(json!({"role":self.name,"role_id":self.id,"account":self.account,"resolved":resolved,"gameplay_checks":state.gameplay_checks,
            "gameplay_witness":state.witness,"logout":logout,"records":state.records,"script_notes":state.notes,
            "outcome_class":if state.product_phase {"product_or_oracle"} else {"fixture"},
            "retirement":state.retirement.as_ref().map(|ticket| json!({
                "slot_incarnation":ticket.execution.slot_incarnation,
                "start_serial":ticket.execution.start_serial,
                "before_generation":ticket.execution.runtime_generation,
                "after_generation":ticket.after_generation,
                "plan_id":ticket.authorization.plan_id,
                "step_ordinal":ticket.authorization.step_ordinal,
                "stop_ordinal":ticket.authorization.stop_ordinal
            })),
            "dock_end":self.dock.end().as_ref().map(end_record),"first_fault":self.dock.fault().as_ref().map(fault_record)}));
    }

    /// This call site is between staging VM rows and the ordinary native dispatch.
    pub(crate) fn dispatch_calls(
        &self,
        run: &super::FileRun,
        snapshot: &GameSnapshot,
        source: Option<(u64, u64, u64)>,
    ) {
        if self.fenced() || !run.0.check_deadline() {
            return;
        }
        let mut state = self.state.lock();
        let Some(binding) = state.binding else {
            return;
        };
        let result = (|| {
            while let Some(call) = self.dock.take(binding).map_err(|err| format!("{err:?}"))? {
                let (sequence, entry) = (call.sequence, call.entry);
                let finishes =
                    call.mode == ExtensionMode::Request && !call.name.starts_with("watch.");
                if let Err(error) = self.admit_call(&run.0, &mut state, snapshot, source, call) {
                    self.dock.latch_fault(
                        binding,
                        Some(sequence),
                        entry,
                        ExtensionFaultCode::Protocol,
                        &error,
                    );
                    return Err(error);
                }
                if finishes {
                    self.dock
                        .finish_request(binding, sequence)
                        .map_err(|err| format!("{err:?}"))?;
                }
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            run.0.fail(format!("{} scenario call: {error}", self.name));
        }
    }

    fn admit_call(
        &self,
        run: &Run,
        state: &mut State,
        snapshot: &GameSnapshot,
        source: Option<(u64, u64, u64)>,
        call: ExtensionCall,
    ) -> Result<(), String> {
        let seq = call.sequence;
        let tick = u64::from(snapshot.tick());
        let session = snapshot.gens().session;
        if state.session != Some(session) {
            return Err("command has a stale session".into());
        }
        if state.sealed {
            return Err("scenario stream already sealed".into());
        }
        state.last_call_tick = Some(tick);
        let name = call.name;
        if name.starts_with("seed.") || matches!(name, "relog" | "expect.server_var") {
            // An open root is not an overtake: the owner starts this queued
            // fixture only after the root closes (see `owner_poll`).
            if state.queued.is_some() || state.seed.is_some() || state.relog.is_some() {
                return Err("fixture or relog cannot overtake another operation".into());
            }
            if name == "relog" && state.checks.iter().any(|check| check.outcome.is_none()) {
                return Err("relog cannot cross an open check".into());
            }
            state.queued = Some(call);
            return Ok(());
        }
        if let Some((scope @ ("expect" | "watch" | "probe"), check)) = name.split_once('.') {
            let mut args = named(call.arguments)?;
            let within = take_optional_u64(&mut args, "within", 1, 12000)?.unwrap_or(20);
            let label = optional_text(&mut args, "label")?.unwrap_or_else(|| check.to_owned());
            let last_tick = tick.checked_add(within).ok_or("check deadline overflow")?;
            state.record(json!({"sequence":seq,"label":label,"admission":scope,"check":check,"arguments":args,"tick":tick,"last_tick":last_tick,"session":session,"source":source}))?;
            let check_name = check.to_owned();
            let check = CheckKind::parse(check, args)?;
            if state.checks.len() == MAX_RECORDS {
                return Err("scenario check capacity exhausted".into());
            }
            state.checks.push(Check {
                seq,
                name: check_name,
                label,
                check,
                last_tick,
                probe: scope == "probe",
                root: state.action.as_ref().map(|root| root.request),
                outcome: None,
            });
            return Ok(());
        }
        match name {
            "step" => {
                let mut values = positional(call.arguments, 2, 2)?.into_iter();
                let label = text(values.next().expect("validated arity"))?;
                let why = text(values.next().expect("validated arity"))?;
                if state.checks.iter().any(|check| check.outcome.is_none()) {
                    return Err("next step would leave an admitted check open".into());
                }
                let last_tick = tick
                    .checked_add(u64::from(run.plan.scenario.idle_ticks))
                    .ok_or("step deadline overflow")?;
                state.active_step = Some((label.clone(), last_tick));
                state.record(json!({"sequence":seq,"tick":tick,"last_tick":last_tick,"step":label,"why":why}))?;
            }
            "phase" => {
                let phase = text(
                    positional(call.arguments, 1, 1)?
                        .pop()
                        .expect("validated arity"),
                )?;
                match phase.as_str() {
                    "fixture" => state.product_phase = false,
                    "product" => state.product_phase = true,
                    _ => return Err("phase: expected fixture or product".into()),
                }
                state.record(json!({"sequence":seq,"phase":phase}))?;
            }
            "mark" => {
                let label = positional(call.arguments, 0, 1)?
                    .pop()
                    .map(text)
                    .transpose()?;
                let (client_session, source_serial, application_serial) =
                    source.ok_or("dispatch source watermark unavailable")?;
                let at = state.journal.mark(seq, snapshot, session, source_serial)?;
                state.record(json!({"sequence":seq,"mark":label,"session":session,"client_session":client_session,"tick":at.tick,"source_serial":at.source_serial,"application_serial":application_serial}))?;
            }
            "signal" => {
                let name = text(
                    positional(call.arguments, 1, 1)?
                        .pop()
                        .expect("validated arity"),
                )?;
                state.record(json!({"sequence":seq,"tick":tick,"session":session,"signal":name,"role":self.name}))?;
                let mut signals = run.signals.lock();
                if signals.len() >= super::MAX_SIGNALS && !signals.contains_key(&name) {
                    return Err("scenario signal capacity exhausted".into());
                }
                let actors = signals.entry(name).or_default();
                let actor = 1_u16 << self.id;
                if *actors & actor != 0 {
                    return Err("duplicate signal from the same role".into());
                }
                *actors |= actor;
            }
            "note" => {
                let mut values = positional(call.arguments, 2, 2)?.into_iter();
                let key = text(values.next().expect("validated arity"))?;
                let value = text(values.next().expect("validated arity"))?;
                if state.notes.len() == MAX_RECORDS {
                    return Err("script_notes capacity exhausted".into());
                }
                if state.notes.insert(key, value).is_some() {
                    return Err("duplicate script note".into());
                }
            }
            "shot" => {
                let mut args = named(call.arguments)?;
                let label = grammar::take_string(&mut args, "label")?;
                if optional_text(&mut args, "settle")?.is_some_and(|kind| kind != "frame") {
                    return Err("shot.settle: expected frame".into());
                }
                let options = ShotOptions::parse(&mut args)?;
                grammar::empty(&args)?;
                self.request_shot(run, snapshot, Some(seq), label, options)?;
            }
            "seal" => {
                positional(call.arguments, 0, 0)?;
                self.dock
                    .seal(state.binding.ok_or("unbound seal")?)
                    .map_err(|err| format!("seal: {err:?}"))?;
                state.sealed = true;
                state.active_step = None;
            }
            "finish" => {
                let mut args = named(call.arguments)?;
                state.logout = match args.remove("logout") {
                    Some(Value::Bool(value)) => value,
                    None => false,
                    _ => return Err("finish.logout: expected Boolean".into()),
                };
                grammar::empty(&args)?;
                state.sealed = true;
                state.active_step = None;
                state
                    .record(json!({"sequence":seq,"tick":tick,"finish":{"logout":state.logout}}))?;
            }
            _ => return Err(format!("unsupported scenario capability {name:?}")),
        }
        Ok(())
    }

    fn complete(&self, state: &State, seq: u64, payload: ExtensionPayload) -> Result<(), String> {
        self.dock
            .complete(state.binding.ok_or("unbound completion")?, seq, payload)
            .map_err(|err| format!("{err:?}"))
    }

    fn evaluate_checks(
        &self,
        run: &Run,
        state: &mut State,
        snapshot: &GameSnapshot,
        selected: &api::game_data::SelectedGameData,
    ) -> Result<(), String> {
        let mut finished = Vec::new();
        let mut closed_terminal = None;
        for check in state
            .checks
            .iter_mut()
            .filter(|check| check.outcome.is_none())
        {
            let session = snapshot.gens().session;
            let (proof, signal_actor) = match &check.check {
                CheckKind::Proof(proof) => (Some(proof), None),
                CheckKind::Signal(name) => (
                    None,
                    run.signals
                        .lock()
                        .get(name)
                        .map(|actors| actors & !(1_u16 << self.id))
                        .filter(|actors| *actors != 0)
                        .map(u16::trailing_zeros),
                ),
            };
            let mut holds = match proof {
                Some(proof) => proof.check(snapshot, selected, &state.journal, session)?,
                None => signal_actor.is_some(),
            };
            let tick = u64::from(snapshot.tick());
            // A matching checked tile can close only this root's last actual movement aim,
            // after the owner exhausted the route and a new PLAYER_INFO confirms arrival.
            if holds && !check.probe {
                if let Some(root) = state.root.as_ref().filter(|root| {
                    let target = root.target;
                    proof.is_some_and(|proof| {
                        proof
                            .checks_tile((target.x, target.z, target.level), i32::from(root.radius))
                    })
                }) {
                    if let Some(send) = root.closure(snapshot) {
                        finished.push(json!({"root":root.request,"disposition":"EffectClosed","route":"AdmittedCheckMatched","case":"ground-walk-r289","send_commit":send.commit,"sent_from":send.sent_from,"aim":send.aim,"send_player_info":send.raw_player_info,"terminal_player_info":snapshot.gens().player_info,"check":check.seq}));
                        state.root = None;
                    } else {
                        holds = false;
                    }
                }
            }
            // A non-walk root closes only from a check admitted while it was
            // open that observes the family's own domain, with the contract's
            // fresh post-send terminal record present (LD2 §5.1 route 1).
            if holds && !check.probe {
                if let Some(root) = state
                    .action
                    .as_mut()
                    .filter(|root| check.root == Some(root.request) && root.sent())
                {
                    use super::contracts::Closure;
                    match &root.contract.closure {
                        Closure::Terminal(_) if TERMINAL_CHECKS.contains(&check.name.as_str()) => {
                            if let Some(evidence) = root.terminal_evidence(snapshot) {
                                closed_terminal = Some((root.request, check.seq, evidence));
                            } else {
                                holds = false;
                            }
                        }
                        Closure::SilentGuard { .. } if check.name == "silent" => {
                            root.window_held = true;
                        }
                        _ => {}
                    }
                }
            }
            if holds || (check.probe && tick >= check.last_tick) {
                check.outcome = Some(if check.probe {
                    ExtensionPayload::Boolean(holds)
                } else {
                    ExtensionPayload::Unit
                });
                if !check.probe && state.product_phase && proof.is_some_and(FileCheck::is_gameplay)
                {
                    state.gameplay_checks += 1;
                    self.gameplay_matched.store(true, Ordering::Release);
                }
                finished.push(json!({"sequence":check.seq,"label":check.label,"tick":tick,"session":session,"matched":holds,"probe":check.probe,"signal_actor":signal_actor}));
            } else if tick >= check.last_tick {
                return Err(format!(
                    "check {:?} failed at its last admitted tick {tick}",
                    check.label
                ));
            }
        }
        if let Some((request, check, evidence)) = closed_terminal {
            let root = state.action.take().ok_or("closed root disappeared")?;
            debug_assert_eq!(root.request, request);
            state.record(json!({"root":request,"disposition":"EffectClosed","route":"AdmittedCheckMatched",
                "contract":root.contract.id,"citations":root.contract.citations,"check":check,"terminal_record":evidence}))?;
            if let super::contracts::FileAction::UseOnHeld { source, target } = root.action {
                let discharged = state.paired.iter().position(|owed| {
                    (owed.item == source || owed.item == target)
                        && root
                            .stat_at_send(owed.skill)
                            .is_some_and(|level| level >= owed.level)
                });
                if let Some(index) = discharged {
                    let owed = state.paired.remove(index);
                    state.record(json!({"paired_level_control":{"silent_root":owed.silent_root,"control_root":request,
                        "item":owed.item,"skill":owed.skill,"level":owed.level,
                        "attribution":"separate positive branch evidence; the silent root stays Unattributed"}}))?;
                }
            }
        }
        for result in finished {
            state.record(result)?;
        }
        let binding = state.binding.ok_or("unbound checker")?;
        if let Some(sequence) = self.dock.pending_await(binding) {
            if let Some(payload) = state
                .checks
                .iter()
                .find(|check| check.seq == sequence)
                .and_then(|check| check.outcome)
            {
                self.complete(state, sequence, payload)?;
            }
        }
        Ok(())
    }

    pub(crate) fn admit_walk(
        &self,
        run: &super::FileRun,
        snapshot: &GameSnapshot,
        target: WorldTile,
        radius: u16,
        id: u64,
    ) -> bool {
        if self.fenced() || !run.0.check_deadline() {
            return false;
        }
        let mut state = self.state.lock();
        let valid = id != 0
            && state.root.is_none()
            && state.action.is_none()
            && state.session == Some(snapshot.gens().session)
            && run
                .0
                .plan
                .provenance
                .walk_contract()
                .is_some_and(|contract| contract.admits(snapshot, target, radius));
        if !valid {
            run.0.fail(format!(
                "{} unsupported File walk/effect-closure contract or conflicting open root",
                self.name
            ));
            return false;
        }
        if let Err(error) = state
            .journal
            .record_action(snapshot.gens().session, id, false)
        {
            run.0.fail(error);
            return false;
        }
        if let Err(error) = state.record(
            json!({"root":id,"disposition":"Admitted","session":snapshot.gens().session,
            "tick":snapshot.tick(),"target":target,"radius":radius}),
        ) {
            run.0.fail(error);
            return false;
        }
        state.root = Some(WalkRoot {
            request: id,
            route_pending: true,
            session: snapshot.gens().session,
            target,
            radius,
            send: None,
        });
        self.ready.store(false, Ordering::Release);
        true
    }

    /// Admit one non-walk normal action as an open File root, or fail the run.
    pub(crate) fn admit_action(
        &self,
        run: &super::FileRun,
        snapshot: &GameSnapshot,
        request: &script::interact::InteractReq,
    ) -> Option<u64> {
        if self.fenced() || !run.0.check_deadline() {
            return None;
        }
        let mut state = self.state.lock();
        match Self::open_action(&run.0, &mut state, snapshot, request) {
            Ok(id) => {
                self.ready.store(false, Ordering::Release);
                Some(id)
            }
            Err(error) => {
                run.0.fail(format!("{}: {error}", self.name));
                None
            }
        }
    }

    fn open_action(
        run: &Run,
        state: &mut State,
        snapshot: &GameSnapshot,
        request: &script::interact::InteractReq,
    ) -> Result<u64, String> {
        let session = snapshot.gens().session;
        if state.root.is_some() || state.action.is_some() {
            return Err("conflicting open root".into());
        }
        if state.session != Some(session) {
            return Err("File action has a stale session".into());
        }
        let action = super::actions::resolve(request, snapshot)?;
        let contract = run.plan.provenance.action_contract(action, snapshot)?;
        state.next_root = state.next_root.checked_add(1).ok_or("root ids exhausted")?;
        let id = ACTION_ROOT_BASE | state.next_root;
        state.journal.record_action(session, id, false)?;
        state.record(
            json!({"root":id,"disposition":"Admitted","session":session,"tick":snapshot.tick(),
            "action":format!("{action:?}"),"contract":contract.id}),
        )?;
        state.action = Some(super::actions::ActionRoot::new(
            id, session, action, contract,
        ));
        Ok(id)
    }

    /// Record the native send result of the single root dispatched this frame.
    pub(crate) fn note_action_send(
        &self,
        run: &super::FileRun,
        snapshot: &GameSnapshot,
        id: u64,
        sent: bool,
    ) {
        let mut state = self.state.lock();
        let result = (|| -> Result<(), String> {
            let session = snapshot.gens().session;
            let root = state
                .action
                .as_mut()
                .filter(|root| root.request == id && !root.sent() && root.session == session)
                .ok_or("native send does not match the admitted action root")?;
            if sent {
                root.note_send(snapshot);
                state.journal.note_sent(session, id)?;
                state.record(json!({"root":id,"send":"sent","tick":snapshot.tick()}))
            } else {
                state.action = None;
                state.record(json!({"root":id,"disposition":"NeverSent"}))
            }
        })();
        if let Err(error) = result {
            run.0.fail(format!("{}: {error}", self.name));
        }
    }

    fn poll_action(
        &self,
        run: &Run,
        state: &mut State,
        client: &mut Client,
        snapshot: &GameSnapshot,
    ) -> Result<(), String> {
        let Some(root) = state.action.as_mut() else {
            return Ok(());
        };
        let varp = run
            .fixtures
            .get()
            .ok_or("missing bound fixture definitions")?
            .barrier_varp()?;
        if let Some(certificate) = root.poll_window(client, snapshot, varp)? {
            let root = state.action.take().ok_or("closed root disappeared")?;
            if let super::contracts::Closure::SilentGuard { skill, level, item } =
                root.contract.closure
            {
                state.paired.push(super::actions::PairedControl {
                    item,
                    skill,
                    level,
                    silent_root: root.request,
                });
            }
            state.record(certificate)?;
        }
        Ok(())
    }

    pub(crate) fn reject_effect(&self, run: &super::FileRun) {
        run.0.fail(format!(
            "{}: native action lacks an admitted File effect-closure contract",
            self.name
        ));
    }

    fn observe_walk(
        state: &mut State,
        snapshot: &GameSnapshot,
        walk: WalkObservation,
    ) -> Result<(), String> {
        let Some(root) = &mut state.root else {
            return Ok(());
        };
        if root.session != snapshot.gens().session {
            return Err("walk changed sessions before closure".into());
        }
        if walk.request != root.request {
            return Err("normal walk owner was replaced".into());
        }
        if walk.unsupported_continuation {
            return Err("File walk crossed into an unsupported continuation".into());
        }
        if !root.route_pending {
            if walk.failed
                || walk.pending
                || walk.send.map(|send| send.commit) != root.send.map(|send| send.commit)
            {
                return Err(
                    "normal walk produced new activity after its owner reached a terminal outcome"
                        .into(),
                );
            }
            return Ok(());
        }
        if let Some(send) = walk.send.filter(|send| send.nav_request == root.request) {
            if root.send.is_none() {
                state.journal.note_sent(root.session, root.request)?;
            }
            root.send = Some(send);
        }
        if walk.outcome != root.request {
            return Ok(());
        }
        if walk.failed {
            return Err("native walk did not reach the admitted goal".into());
        }
        if walk.pending {
            return Ok(());
        }
        if !snapshot.tile().is_some_and(|(x, z, l)| {
            l == root.target.level
                && (x - root.target.x).abs().max((z - root.target.z).abs())
                    <= i32::from(root.radius)
        }) {
            return Err("owner walk result does not match the admitted destination".into());
        }
        if root.send.is_some() {
            root.route_pending = false;
        } else {
            let request = root.request;
            state.root = None;
            state.record(json!({"root":request,"disposition":"NeverSent"}))?;
        }
        Ok(())
    }
}

pub(crate) struct OwnerInputs<'a> {
    pub(crate) held: bool,
    pub(crate) selected: Option<&'a Arc<api::game_data::SelectedGameData>>,
    pub(crate) named_banks: &'a Arc<api::named_banks::NamedBankFacts>,
    pub(crate) start_hold: &'a std::sync::Mutex<std::sync::Weak<String>>,
    pub(crate) nav: Option<WalkObservation>,
    pub(crate) arm: &'a SlotArm,
    pub(crate) statuses: &'a Arc<std::sync::Mutex<Vec<crate::SlotStatus>>>,
}

impl State {
    fn record(&mut self, record: Value) -> Result<(), String> {
        if self.records.len() == MAX_RECORDS {
            return Err("scenario receipt record capacity exhausted".into());
        }
        self.records.push(record);
        Ok(())
    }
}

fn binding_record(binding: RunBinding) -> Value {
    json!({"slot_incarnation":binding.slot_incarnation,"run_key":binding.run_key,"dock_id":binding.dock_id})
}

fn fault_record(fault: &ExtensionFault) -> Value {
    json!({"binding":binding_record(fault.binding),"sequence":fault.sequence,"entry":format!("{:?}",fault.entry),
        "code":format!("{:?}",fault.code),"detail":fault.detail.as_str()})
}

fn end_record(end: &ExtensionEnd) -> Value {
    json!({"binding":binding_record(end.binding),"cause":format!("{:?}",end.cause),"vm_dropped":end.vm_dropped,
        "hook_failure":end.hook_failure.as_ref().map(|failure| json!({"code":failure.code.as_str(),"detail":failure.detail.as_str()})),
        "first_fault":end.first_fault.as_ref().map(fault_record),"load_error":end.load_error.as_ref().map(|error| error.as_str()),
        "disposed":{"staged":end.disposal_counts.staged,"queued":end.disposal_counts.queued,"completion":end.disposal_counts.completion,"handles":end.disposal_counts.handles}})
}

fn named(value: ExtensionValue) -> Result<Map<String, Value>, String> {
    match value {
        ExtensionValue::List(mut values) if values.len() == 1 => {
            named(values.pop().expect("one value"))
        }
        ExtensionValue::List(values) if values.is_empty() => Ok(Map::new()),
        ExtensionValue::Named(fields) => fields
            .into_iter()
            .map(|(key, value)| Ok((key, json_value(value)?)))
            .collect(),
        _ => Err("expected one named table".into()),
    }
}

pub(super) fn json_value(value: ExtensionValue) -> Result<Value, String> {
    Ok(match value {
        ExtensionValue::Named(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| Ok((key, json_value(value)?)))
                .collect::<Result<_, String>>()?,
        ),
        ExtensionValue::List(values) => Value::Array(
            values
                .into_iter()
                .map(json_value)
                .collect::<Result<_, _>>()?,
        ),
        ExtensionValue::String(value) => Value::String(value),
        ExtensionValue::Boolean(value) => Value::Bool(value),
        ExtensionValue::Integer(value) => Value::from(value),
        ExtensionValue::Number(value) => {
            Value::Number(serde_json::Number::from_f64(value).ok_or("nonfinite scenario argument")?)
        }
    })
}

fn positional(value: ExtensionValue, lo: usize, hi: usize) -> Result<Vec<ExtensionValue>, String> {
    match value {
        ExtensionValue::List(values) if (lo..=hi).contains(&values.len()) => Ok(values),
        _ => Err(format!("expected {lo}–{hi} positional arguments")),
    }
}

fn text(value: ExtensionValue) -> Result<String, String> {
    match value {
        ExtensionValue::String(text)
            if (1..=160).contains(&text.len()) && !text.chars().any(char::is_control) =>
        {
            Ok(text)
        }
        _ => Err("expected 1–160 printable bytes of text".into()),
    }
}

fn take_optional_u64(
    args: &mut Map<String, Value>,
    field: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, String> {
    args.remove(field)
        .map(|value| {
            value
                .as_u64()
                .filter(|n| (min..=max).contains(n))
                .ok_or_else(|| format!("{field}: expected integer in {min}..={max}"))
        })
        .transpose()
}

fn optional_text(args: &mut Map<String, Value>, field: &str) -> Result<Option<String>, String> {
    if args.contains_key(field) {
        grammar::take_string(args, field).map(Some)
    } else {
        Ok(None)
    }
}

fn registry() -> &'static Arc<ExtensionRegistry> {
    use ExtensionMode::{Await, Finish, Request};
    static REGISTRY: std::sync::LazyLock<Arc<ExtensionRegistry>> = std::sync::LazyLock::new(|| {
        let mut functions = vec![
            ExtensionFunction {
                name: "step",
                mode: Request,
            },
            ExtensionFunction {
                name: "phase",
                mode: Request,
            },
            ExtensionFunction {
                name: "mark",
                mode: Request,
            },
            ExtensionFunction {
                name: "note",
                mode: Request,
            },
            ExtensionFunction {
                name: "seal",
                mode: Request,
            },
            ExtensionFunction {
                name: "finish",
                mode: Finish,
            },
            ExtensionFunction {
                name: "shot",
                mode: Await,
            },
            ExtensionFunction {
                name: "expect.server_var",
                mode: Await,
            },
            ExtensionFunction {
                name: "relog",
                mode: Await,
            },
            ExtensionFunction {
                name: "signal",
                mode: Request,
            },
            ExtensionFunction {
                name: "expect.signal",
                mode: Await,
            },
            ExtensionFunction {
                name: "watch.signal",
                mode: Request,
            },
            ExtensionFunction {
                name: "probe.signal",
                mode: Await,
            },
        ];
        for &(check, watch, probe) in grammar::FILE_CHECK_ENTRIES {
            functions.extend([
                ExtensionFunction {
                    name: check,
                    mode: Await,
                },
                ExtensionFunction {
                    name: watch,
                    mode: Request,
                },
                ExtensionFunction {
                    name: probe,
                    mode: Await,
                },
            ]);
        }
        functions.push(ExtensionFunction {
            name: "expect.silent",
            mode: Await,
        });
        functions.extend(
            super::fixtures::SEED_ENTRIES
                .iter()
                .map(|&(name, _)| ExtensionFunction::new(name, Await)),
        );
        // Path scenarios only: admission refuses it without a loaded pack.
        functions.push(ExtensionFunction::new("seed.ext", Await));
        Arc::new(
            ExtensionRegistry::new(functions, Vec::new())
                .expect("valid static File scenario registry"),
        )
    });
    &REGISTRY
}
