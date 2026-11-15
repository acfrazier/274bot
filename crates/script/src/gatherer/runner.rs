use super::area::{AreaError, WorkArea};
use super::card::Prepared;
use super::drop::{DropBatch, DropBatchArgs, DropEnd, DropResult};
use super::gather::{
    observe_target, GatherEnd, GatherResult, GatherRun, GatherRunArgs, TargetObservation,
    DEFAULT_STALL_TICKS,
};
use super::oneop::{OneOp, OneOpArgs, OneOpKind};
use super::select::{
    resource_return_target, select, tile_loaded, AvoidedTile, FishingSurvey, PlacementClass,
    ResourceReturn, ReturnObservation, SelectedTarget, Selection, TargetPlan,
    RESOURCE_APPROACH_RADIUS,
};
use super::settings::{has_members_requirement, method_level, GathererSettings, Skill};
use super::status::{self, StatusData};
use super::supply::{self, SupplyPlan};
use super::widen::{
    remember_group, site_anchor, SearchExclusions, SearchResult, TriedGroup, WidenCursor,
};
use super::{GatherRetained, RecoveryState};
use crate::acquire::run::AcquireRun;
use crate::acquire::{
    AcquireBlock, AcquireOutcome, AcquirePlan, AcquireRequest, AcquireVerdict, BankChoice,
    BankStage, CloseBank, CoinPolicy, DepositSpec, Need, OpenGate, SourceSet, VendorChoice,
};
use crate::native::walk::Walk;
use crate::native::{
    ActionError, ActionHandle, ConfigError, Interrupt, NativePhase, NativeTick, PreparedConfig,
    Script, ScriptFailure, ScriptFlow, SettingsApply, StopReason, WalkBit, WalkEnd, WalkOptions,
    WalkReceipt, WalkRefusal, WalkRequest, WalkRetryState, GATHER_WALK_MAX_ATTEMPTS,
};
use crate::quester::prereq::walk::PrereqWalk;
use api::game_data::GatherForbiddenState;
use api::gather_methods::known_rows;
use api::selected::{EntityId, RunKey, Truth};
use api::snapshot::{ItemView, StatView, WorldTile};
use nav_planner::contract::ArrivalKind;
use std::sync::Arc;
use std::task::Poll;

#[path = "park.rs"]
mod park;
#[path = "recover.rs"]
mod recover;
use park::Park;

#[path = "runner_timing.rs"]
mod timing;

const MAX_AVOID: usize = 8;
/// Session changes (relogs) one bank trip restarts across before it reports a
/// failure. This is a policy constant, not an engine fact. Each restart is a
/// fresh session with a fresh plan, and a bot that loses its session this many
/// times inside one trip is not in a passing state, so the run stops with the
/// last reason. It is the bound the open attempts used before the trip moved
/// into the acquisition core (`MAX_OPEN_ATTEMPTS`); a choice, not a derivation.
const MAX_TRIP_SESSION_RESTARTS: u8 = 3;

// Fishing's maximum movement timer is 529 ticks (280 + random(250)):
// skill_fishing/scripts/fishing_movement.rs2:8-12. Add a 60-tick whirlpool
// (macro events/scripts/fishing/macro_event_whirlpool.rs2:7); 800 exceeds both.
const WAIT_GAMEPLAY_TICKS: u64 = 800; // Eight minutes at 600 ms per server tick.
const IDLE_AVOID_TICKS: u64 = 60;
// USER-0.2.3-GATHERER §4 requires a finite total action budget. Four failed
// walks allow two targets to use their initial request and one recovery each.
const MAX_FAILED_WALKS_PER_PHASE: u8 = 4;
const HAZARD_RETREAT_RADIUS: i32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ToolState {
    id: i32,
    worn: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TickPacketFence {
    tick: u64,
    packets: u8,
    sealed: bool,
}

impl Default for TickPacketFence {
    fn default() -> Self {
        Self {
            tick: u64::MAX,
            packets: 0,
            sealed: false,
        }
    }
}

impl TickPacketFence {
    fn observe(&mut self, tick: u64) {
        if self.tick != tick {
            self.tick = tick;
            self.packets = 0;
            self.sealed = false;
        }
    }

    fn reserve(&mut self, packets: u8) -> bool {
        if self.sealed || self.packets.saturating_add(packets) > 5 {
            return false;
        }
        self.packets += packets;
        true
    }

    fn seal(&mut self) {
        self.sealed = true;
    }
}

#[derive(Default)]
enum Active {
    #[default]
    None,
    Gather(ActionHandle<GatherRun>),
    Drop(ActionHandle<DropBatch>),
    Walk(PrereqWalk),
    EscapeWalk(ActionHandle<Walk>),
    Tend(ActionHandle<OneOp>),
    Acquire(Box<AcquireRun>),
    ParkClose(Box<AcquireRun>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Validation {
    Ready,
    Pending,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum TripStep {
    #[default]
    Idle,
    Deposit,
    Supply {
        return_to_work: bool,
        deposit_products: bool,
    },
    Validate,
    Return,
}

fn walk_failure_message(receipt: &WalkReceipt) -> String {
    if matches!(receipt.refusal, Some(WalkRefusal::SearchIncomplete { .. })) {
        return match receipt.failure_detail().filter(|detail| !detail.is_empty()) {
            Some(detail) => format!("walk search incomplete: {detail}"),
            None => "walk search incomplete".to_owned(),
        };
    }
    match receipt.failure_detail().filter(|detail| !detail.is_empty()) {
        Some(detail) => format!("walk ended with {:?}: {detail}", receipt.end),
        None => format!("walk ended with {:?}", receipt.end),
    }
}

/// Nearest stand within [`HAZARD_RETREAT_RADIUS`] of `here` that the
/// router can actually step to from `here` inside that bound, ranked
/// toward the away-from-hazard direction. Missing collision treats every
/// in-bound tile as reachable so the one-step away tile still wins.
fn hazard_retreat_stand(
    here: WorldTile,
    hazard: WorldTile,
    collision: Option<nav_planner::contract::CollisionView<'_>>,
) -> Option<WorldTile> {
    const SPAN: usize = (HAZARD_RETREAT_RADIUS as usize) * 2 + 1;
    let mut dx = here.x.saturating_sub(hazard.x).signum();
    let dz = here.z.saturating_sub(hazard.z).signum();
    if dx == 0 && dz == 0 {
        dx = 1;
    }
    let mut reachable = [[collision.is_none(); SPAN]; SPAN];
    if let Some(collision) = collision {
        let mut queue = [(0i32, 0i32); SPAN * SPAN];
        let mut head = 0usize;
        let mut tail = 1usize;
        let origin = HAZARD_RETREAT_RADIUS as usize;
        reachable[origin][origin] = true;
        queue[0] = (0, 0);
        while head < tail {
            let (ox, oz) = queue[head];
            head += 1;
            let cur = WorldTile {
                x: here.x.saturating_add(ox),
                z: here.z.saturating_add(oz),
                level: here.level,
            };
            for (step_x, step_z) in [
                (0, 1),
                (0, -1),
                (1, 0),
                (-1, 0),
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
            ] {
                let next_x = ox + step_x;
                let next_z = oz + step_z;
                if next_x.abs() > HAZARD_RETREAT_RADIUS || next_z.abs() > HAZARD_RETREAT_RADIUS {
                    continue;
                }
                let index_x = (next_x + HAZARD_RETREAT_RADIUS) as usize;
                let index_z = (next_z + HAZARD_RETREAT_RADIUS) as usize;
                if reachable[index_x][index_z] {
                    continue;
                }
                if !collision.step_ok(
                    cur,
                    WorldTile {
                        x: cur.x + step_x,
                        z: cur.z + step_z,
                        level: cur.level,
                    },
                ) {
                    continue;
                }
                reachable[index_x][index_z] = true;
                queue[tail] = (next_x, next_z);
                tail += 1;
            }
        }
    }
    let mut best = None;
    for ox in -HAZARD_RETREAT_RADIUS..=HAZARD_RETREAT_RADIUS {
        for oz in -HAZARD_RETREAT_RADIUS..=HAZARD_RETREAT_RADIUS {
            if ox == 0 && oz == 0 {
                continue;
            }
            let tile = WorldTile {
                x: here.x.saturating_add(ox),
                z: here.z.saturating_add(oz),
                level: here.level,
            };
            let index_x = (ox + HAZARD_RETREAT_RADIUS) as usize;
            let index_z = (oz + HAZARD_RETREAT_RADIUS) as usize;
            if tile == hazard || !reachable[index_x][index_z] {
                continue;
            }
            let away = i64::from(ox.signum() == dx) + i64::from(oz.signum() == dz);
            let key = (
                std::cmp::Reverse(away),
                ox.abs().max(oz.abs()),
                std::cmp::Reverse((tile.x - hazard.x).abs().max((tile.z - hazard.z).abs())),
                ox.abs(),
                oz.abs(),
            );
            if best.as_ref().is_none_or(|(_, current)| key < *current) {
                best = Some((tile, key));
            }
        }
    }
    best.map(|(tile, _)| tile)
}

/// The identity of the current walk plan, so retry state cannot leak to a
/// different resource, return stand, or hazard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalkRetryTarget {
    Resource(EntityId, WorldTile),
    /// A resource target held for another attempt after a host escape.
    EscapeResource(EntityId, WorldTile),
    Hazard(WorldTile),
}

/// The Gatherer's walk bookkeeping. `retry` is the shared core
/// ([`WalkRetryState`]): it counts requests, classifies receipts and holds a
/// retry to a later observed tick. The rest is the Gatherer's own target
/// policy, not a second retry counter.
struct GatherWalk {
    retry: WalkRetryState,
    /// The identity whose next action, if any, is the pending bounded retry.
    target: Option<WalkRetryTarget>,
    /// Failed return or approach walks across targets since the last progress.
    failures: u8,
    /// Detail from the newest failed walk, retained across target fallbacks.
    last_detail: Option<Arc<str>>,
}

impl Default for GatherWalk {
    fn default() -> Self {
        Self {
            retry: WalkRetryState::with_max_attempts(GATHER_WALK_MAX_ATTEMPTS),
            target: None,
            failures: 0,
            last_detail: None,
        }
    }
}

/// Cold session collections share one allocation to retain the inline budget.
#[derive(Default)]
struct GatherScratch {
    fishing: FishingSurvey,
    /// Retain the admitted full-batch reserve across a hold and re-plan.
    needs: Option<Arc<[Need]>>,
    /// Last tick a catch or XP was credited; bounds fishing surveys.
    last_yield_tick: u64,
    /// Walk retry and failure bookkeeping around the shared retry core.
    walk: GatherWalk,
    /// Whether this trip found an item at Deposit that engine content forbids banking.
    /// Retained across action restarts until the next trip begins.
    kept_unbankable: bool,
    /// A non-empty supply withdrawal completed during this trip.
    withdrew_supplies: bool,
    /// Only the in-flight observation request needs its selected-goal receipt.
    observation_walk: bool,
    parked: Option<Box<Park>>,
    replan_once: bool,
    rebuild_park_request: bool,
    /// Restarts of the current bank trip after a session change.
    session_restarts: u8,
    /// The last reason the trip's acquisition recorded before its session ended;
    /// kept for the terminal message when the restarts are spent.
    trip_reason: Option<Arc<str>>,
    /// The next acquisition starts from a fresh pick: the session just changed.
    fresh_access: bool,
}

pub struct Gatherer {
    run: RunKey,
    config: Arc<PreparedConfig>,
    prepared: Arc<Prepared>,
    pending: Option<Arc<PreparedConfig>>,
    area: Option<WorkArea>,
    target: Option<TargetPlan>,
    active: Active,
    tick_policy: crate::native::tickcycle::TickPolicy,
    parked_cycle: Option<crate::native::tickcycle::TickCycle>,
    tick_applicability: Option<crate::native::tickcycle::OffReason>,
    tick_last_off: Option<crate::native::tickcycle::OffReason>,
    tick_last_sequence: u32,
    tool: ToolState,
    avoid: [AvoidedTile; MAX_AVOID],
    scratch: Box<GatherScratch>,
    wait_until: Option<u64>,
    widen: WidenCursor,
    tried_groups: [TriedGroup; 4],
    hazard_escape: Option<WorldTile>,
    trip: TripStep,
    bank_label: Arc<str>,
    trips: u16,
    last_gameplay_tick: u64,
    last_paint_tick: u64,
    fence: TickPacketFence,
    death: crate::native::death::DeathLatch,
    survival: crate::survival::SurvivalStep,
    survival_deferred: bool,
    survival_interrupting: bool,
    /// The `Active::Tend` handle is a survival click (drink, fruit, bite or
    /// the close before one), not a wear or a stray-page op.
    survival_tend: bool,
    respawn_deadline: Option<std::time::Duration>,
    respawn_settle: Option<u64>,
    proving_runs: u8,
    recovery_reprovisions: u8,
    proof_evidence: u8,
    /// One bit per content-defined unbankable type, retained across trips.
    unbankable_logged: u8,
    retained: GatherRetained,
    failure: Option<ScriptFailure>,
    paused: bool,
    needs_validate: bool,
    rebaseline_gameplay: bool,
    dirty: bool,
    yielded: u32,
    dropped: u32,
    xp: i32,
    observed_progress: Option<(u32, i32, WorldTile)>,
    method: Arc<str>,
    last_event: Arc<str>,
    absent: u16,
    zone_gated: u16,
}

impl Gatherer {
    pub fn new(
        run: RunKey,
        config: Arc<PreparedConfig>,
        prepared: Arc<Prepared>,
        retained: GatherRetained,
    ) -> Self {
        let death = crate::native::death::DeathLatch::from_watermark(retained.death_seq);
        Self {
            run,
            config,
            prepared,
            pending: None,
            area: None,
            target: None,
            active: Active::None,
            tick_policy: Default::default(),
            parked_cycle: None,
            tick_applicability: None,
            tick_last_off: None,
            tick_last_sequence: 0,
            tool: ToolState {
                id: -1,
                worn: false,
            },
            avoid: [AvoidedTile::EMPTY; MAX_AVOID],
            scratch: Box::default(),
            wait_until: None,
            widen: WidenCursor::default(),
            tried_groups: [TriedGroup::default(); 4],
            hazard_escape: None,
            trip: TripStep::Idle,
            bank_label: Arc::from("—"),
            trips: 0,
            last_gameplay_tick: 0,
            last_paint_tick: 0,
            fence: TickPacketFence::default(),
            death,
            survival: crate::survival::SurvivalStep::default(),
            survival_deferred: false,
            survival_interrupting: false,
            survival_tend: false,
            respawn_deadline: None,
            respawn_settle: None,
            proving_runs: 0,
            recovery_reprovisions: 0,
            proof_evidence: 0,
            unbankable_logged: 0,
            retained,
            failure: None,
            paused: false,
            needs_validate: true,
            rebaseline_gameplay: true,
            dirty: true,
            yielded: retained.yielded,
            dropped: retained.dropped,
            xp: 0,
            observed_progress: None,
            method: Arc::from("—"),
            last_event: Arc::from("started"),
            absent: 0,
            zone_gated: 0,
        }
    }

    fn settings(&self) -> &GathererSettings {
        &self.prepared.settings
    }

    fn advance_trip(&mut self, step: TripStep) {
        self.trip = step;
        self.dirty = true;
    }

    fn start_trip(&mut self, step: TripStep) {
        self.capture_live_timing();
        self.parked_cycle = None;
        api::host_log!(
            api::hostlog::Category::NavTrace,
            api::hostlog::Level::Debug,
            "gather-approach clear reason=bank-trip target={:?}",
            self.target
                .as_ref()
                .map(|target| (target.entity, target.tile)),
        );
        self.target = None;
        self.wait_until = None;
        self.scratch.needs = None;
        self.scratch.kept_unbankable = false;
        self.scratch.withdrew_supplies = false;
        self.scratch.session_restarts = 0;
        self.scratch.fresh_access = false;
        self.scratch.trip_reason = None;
        self.reset_walk_retry();
        self.reset_phase_walk_budget();
        self.scratch.walk.last_detail = None;
        self.advance_trip(step);
        self.set_event("bank trip due");
    }

    fn acquire_request(
        &self,
        needs: Arc<[Need]>,
        deposit: Option<DepositSpec>,
        close: CloseBank,
        cx: &crate::native::ActionContext<'_>,
    ) -> AcquireRequest {
        AcquireRequest {
            needs,
            deposit,
            bank: BankChoice {
                bank: None,
                explicit: (!self.settings().bank.eq_ignore_ascii_case("Nearest"))
                    .then(|| Arc::from(self.settings().bank.as_str())),
                required: false,
                preferences: self.settings().bank_preferences(),
            },
            stage: BankStage::Planned,
            partial_ok: true,
            close,
            vendor: if self.settings().buy_tools {
                VendorChoice::Nearest
            } else {
                VendorChoice::None
            },
            coins: if self.settings().buy_tools {
                CoinPolicy::ForPurchase {
                    sources: SourceSet::PACK | SourceSet::BANK,
                }
            } else {
                CoinPolicy::None
            },
            walk: WalkOptions::default(),
            required_after: cx.evidence(),
        }
    }

    /// A new session inside a bank trip is information, not a failure
    /// (AGENTS.md: a walk failure is never terminal until its bounded
    /// retries are spent). The old session's acquisition is gone with its
    /// walk, modal and item table, and the reasons it recorded say nothing
    /// about the new session. The trip's acquisition starts again from the
    /// observed position, against the new session's inventory and bank table;
    /// its needs are held targets, so work the old session half applied is
    /// not repeated. Bounded by [`MAX_TRIP_SESSION_RESTARTS`]; once spent, the
    /// failure keeps the last reason the trip recorded.
    fn restart_trip_after_session(&mut self, session: u64) {
        let step = self.trip;
        if !matches!(step, TripStep::Deposit | TripStep::Supply { .. }) {
            return;
        }
        if self.scratch.session_restarts >= MAX_TRIP_SESSION_RESTARTS {
            let reason = self
                .scratch
                .trip_reason
                .as_deref()
                .unwrap_or("none recorded");
            self.fail(
                "bank-unavailable",
                format!(
                    "bank-unavailable: the session changed again at {step:?} after \
                     {MAX_TRIP_SESSION_RESTARTS} restarts of the bank trip; last reason: {reason}"
                ),
            );
            return;
        }
        self.scratch.session_restarts += 1;
        self.scratch.fresh_access = true;
        api::host_log!(
            api::hostlog::Category::BankOp,
            api::hostlog::Level::Info,
            "gatherer-bank-trip-restart session={session} step={step:?} attempt={}/{MAX_TRIP_SESSION_RESTARTS}",
            self.scratch.session_restarts,
        );
        self.target = None;
        self.wait_until = None;
        self.scratch.kept_unbankable = false;
        self.scratch.walk.retry.reset();
        self.set_event("session changed; restarting bank trip");
    }

    fn deposit_spec(&mut self, inventory: &[ItemView]) -> DepositSpec {
        for item in inventory.iter().filter(|item| item.count > 0) {
            let Some(index) = crate::acquire::unbankable_index(&self.prepared.selected, item)
            else {
                continue;
            };
            let mask = 1_u8 << index;
            if self.unbankable_logged & mask == 0 {
                self.unbankable_logged |= mask;
                api::host_log!(
                    api::hostlog::Category::BankOp,
                    api::hostlog::Level::Info,
                    "Gatherer keeps {} (id={}): engine content forbids banking this item",
                    item.def.name.as_deref().unwrap_or("unnamed item"),
                    item.def.id,
                );
            }
        }
        let mut keep = supply::protected_ids(&self.prepared, self.tool.id);
        keep.keep_unbankable(&self.prepared.selected, inventory);
        DepositSpec::Ids {
            products: supply::bank_deposit_ids(&self.prepared, self.tool.id, inventory),
            keep: Arc::from(keep.as_slice()),
        }
    }

    /// A known shortage refuses before a trip; hints earn the core's scan.
    fn admit_supply(&mut self, cx: &crate::native::ActionContext<'_>) -> bool {
        let snapshot = cx.snapshot();
        let Some(stats) = snapshot.stats() else {
            return false;
        };
        if !SupplyPlan::due(&self.prepared, stats.value, snapshot.stock()) {
            return false;
        }
        let needs = match SupplyPlan::build(&self.prepared, stats.value, snapshot.stock()) {
            Ok(needs) => needs,
            Err(error) => {
                self.action_failure(error);
                return true;
            }
        };
        let request = self.acquire_request(needs, None, CloseBank::Close, cx);
        match crate::acquire::run::plan_current(&request, cx) {
            Ok(AcquireVerdict::Satisfied) => false,
            Ok(AcquireVerdict::Plan(plan)) => {
                self.start_supply_plan(request, plan, cx);
                true
            }
            Ok(AcquireVerdict::Blocked(block)) => {
                self.park_supply(request, block, cx.evidence().tick);
                true
            }
            Err(error) => {
                self.action_failure(error);
                true
            }
        }
    }

    fn start_supply_plan(
        &mut self,
        mut request: AcquireRequest,
        plan: AcquirePlan,
        cx: &crate::native::ActionContext<'_>,
    ) {
        let bank_work = plan.scan || plan.withdrawal_count > 0;
        let continuing_return = self.trip == TripStep::Validate
            || matches!(
                self.trip,
                TripStep::Supply {
                    return_to_work: true,
                    ..
                }
            );
        if bank_work {
            if let Some(inventory) = cx.snapshot().inventory() {
                request.deposit = Some(self.deposit_spec(inventory.value));
            }
        }
        let step = TripStep::Supply {
            return_to_work: bank_work || plan.purchase_count > 0 || continuing_return,
            deposit_products: bank_work,
        };
        if continuing_return {
            self.advance_trip(step);
        } else {
            self.start_trip(step);
        }
        self.scratch.needs = Some(Arc::clone(&request.needs));
        self.active = Active::Acquire(Box::new(
            AcquireRun::new(request, plan, &self.prepared.banks, cx).with_access_recovery(),
        ));
    }

    fn start_dispose(&mut self, tick: &mut NativeTick<'_>) {
        if self.settings().disposition.eq_ignore_ascii_case("Bank") {
            self.start_trip(TripStep::Deposit);
        } else {
            self.start_drop(tick);
        }
    }

    fn trip_walk(
        &mut self,
        target: WorldTile,
        radius: u16,
        arrival: ArrivalKind,
        candidate_goals: Option<crate::native::WalkGoals>,
        tick: &mut NativeTick<'_>,
    ) -> bool {
        let observation_walk = candidate_goals.is_some();
        let request = WalkRequest {
            target,
            candidate_goals,
            loc_id: None,
            radius,
            arrival,
            options: WalkOptions::default(),
            required_after: tick.cx.evidence(),
            evidence: None,
            cross: Box::default(),
            avoid: Box::new([]),
            protect: false,
            food_guard: true,
            cure_guard: false,
            allow: crate::native::WalkAllow {
                food: true,
                ..Default::default()
            },
        };
        match PrereqWalk::begin(request, tick.actions, &mut tick.cx) {
            Ok(walk) => {
                self.scratch.observation_walk = observation_walk;
                self.active = Active::Walk(walk);
                true
            }
            Err(error) => {
                self.action_failure(error);
                false
            }
        }
    }

    /// The typed Traveller terminal includes its current server tile; rebuild
    /// the content-derived observation stand from the fresh snapshot, not from
    /// the route that stalled. Shared by the ordinary return trip and the
    /// death-recovery return.
    fn replan_return_after_stall(&mut self, tick: &mut NativeTick<'_>) {
        self.target = None;
        self.return_to_resources(tick);
    }

    fn return_to_resources(&mut self, tick: &mut NativeTick<'_>) {
        let now = tick.cx.evidence().tick;
        if self.wait_until.is_some_and(|until| now < until)
            && self
                .last_event
                .as_ref()
                .starts_with("waiting for excluded resource")
        {
            return;
        }
        let Some(area) = self.area else {
            self.fail("return-failed", "the resource work area is unavailable");
            return;
        };
        let snapshot = tick.cx.snapshot();
        let Some(here) = snapshot.here() else {
            return;
        };
        match resource_return_target(
            &self.prepared.catalog,
            &self.prepared.methods,
            self.settings(),
            area,
            ReturnObservation {
                walk_danger: tick.walk_danger,
                here: here.value,
                now: tick.cx.evidence().tick,
                skill_stat: self.skill_stat(snapshot.stats()),
                avoided: &self.avoid,
                collision: tick.frame.compiled.collision,
            },
        ) {
            ResourceReturn::Ready(selected) => {
                self.wait_until = None;
                let stand = selected.plan.tile;
                let goals = selected.observation_goals;
                let radius = if goals.is_some() {
                    0
                } else {
                    RESOURCE_APPROACH_RADIUS
                };
                self.method = Arc::clone(&selected.plan.alias);
                self.target = Some(selected.plan);
                let recovery = self.scratch.walk.retry.pending();
                let submitted = self.trip_walk(stand, radius, ArrivalKind::Area, goals, tick);
                if recovery && submitted {
                    self.scratch.walk.retry.action_submitted();
                    self.set_event("return recovery action submitted after transient Stalled");
                } else if !recovery {
                    self.set_event("returning to work area");
                }
                self.dirty = true;
            }
            ResourceReturn::ExcludedUntil { until } => {
                self.wait_until = Some(until);
                self.set_event(&self.block_context(
                    Some(here.value),
                    &format!("waiting for excluded resource until {until}"),
                ));
            }
            ResourceReturn::Unavailable { reason } => {
                self.fail(
                    "return-failed",
                    self.block_context(Some(here.value), reason),
                );
            }
            ResourceReturn::InvalidGoals(error) => {
                self.fail("return-failed", format!("observation goals: {error}"));
            }
            ResourceReturn::Danger(failure) => {
                self.fail(
                    "return-failed",
                    self.block_context(Some(here.value), &failure.to_string()),
                );
            }
        }
    }

    fn resource_return_arrived(&self, tick: &NativeTick<'_>) -> bool {
        self.target.as_ref().is_some_and(|target| {
            tick.cx.snapshot().local_player().is_some_and(|player| {
                // Native walk arrival follows the path head, not the rendered pose.
                let tile = player.value.player.network;
                tile.level == target.tile.level
                    && tile
                        .x
                        .abs_diff(target.tile.x)
                        .max(tile.z.abs_diff(target.tile.z))
                        <= u32::from(RESOURCE_APPROACH_RADIUS)
            })
        })
    }

    fn begin_trip(&mut self, tick: &mut NativeTick<'_>) {
        if matches!(self.active, Active::Acquire(_) | Active::ParkClose(_)) {
            self.poll_active(tick);
            return;
        }
        match self.trip {
            TripStep::Idle => {}

            TripStep::Deposit => {
                let snapshot = tick.cx.snapshot();
                let Some(inventory) = snapshot.inventory() else {
                    return;
                };
                let deposit = self.deposit_spec(inventory.value);
                let request = self.acquire_request(
                    Arc::from([]),
                    Some(deposit),
                    CloseBank::LeaveOpen,
                    &tick.cx,
                );
                self.begin_acquire(request, tick);
            }
            TripStep::Supply {
                deposit_products, ..
            } => {
                if self.scratch.needs.is_none() {
                    let snapshot = tick.cx.snapshot();
                    let Some(stats) = snapshot.stats() else {
                        return;
                    };
                    match SupplyPlan::build(&self.prepared, stats.value, snapshot.stock()) {
                        Ok(needs) => self.scratch.needs = Some(needs),
                        Err(error) => {
                            self.action_failure(error);
                            return;
                        }
                    }
                }
                let deposit = if deposit_products {
                    let snapshot = tick.cx.snapshot();
                    let Some(inventory) = snapshot.inventory() else {
                        return;
                    };
                    Some(self.deposit_spec(inventory.value))
                } else {
                    None
                };
                let request = self.acquire_request(
                    Arc::clone(self.scratch.needs.as_ref().expect("admitted supply needs")),
                    deposit,
                    CloseBank::Close,
                    &tick.cx,
                );
                self.begin_acquire(request, tick);
            }
            TripStep::Validate => {
                // V-resume derives the observed tool after the core's settled
                // Close and Wear steps, before the resource-return boundary.
                if !self.needs_validate {
                    if self.scratch.kept_unbankable
                        && tick.cx.snapshot().stock().pack_full() == Some(true)
                    {
                        if self.scratch.withdrew_supplies {
                            self.fail(
                                "inventory-blocked",
                                "supply withdrawals filled the inventory; no space remains for gathering",
                            );
                        } else {
                            self.deposit_failure(ActionError::BankDeposit(
                                crate::bank::ops::DepositFailure::Unbankable,
                            ));
                        }
                        return;
                    }
                    self.advance_trip(TripStep::Return);
                    self.set_event("returning to work area");
                    self.return_to_resources(tick);
                }
            }
            TripStep::Return => self.return_to_resources(tick),
        }
    }
    fn begin_acquire(&mut self, request: AcquireRequest, tick: &mut NativeTick<'_>) {
        let fresh = self.scratch.fresh_access;
        let planned = if fresh {
            crate::acquire::run::plan_fresh(&request, &tick.cx)
        } else {
            crate::acquire::run::plan_current(&request, &tick.cx)
        };
        let plan = match planned {
            // A chained supply request must close the already-open bank even
            // when no transfer or wear is left to do.
            Ok(AcquireVerdict::Satisfied) => AcquirePlan::empty(),
            Ok(AcquireVerdict::Plan(plan)) => plan,
            Ok(AcquireVerdict::Blocked(block)) => {
                self.park_supply(request, block, tick.cx.evidence().tick);
                return;
            }
            Err(error) => {
                self.action_failure(error);
                return;
            }
        };
        let mut run =
            AcquireRun::new(request, plan, &self.prepared.banks, &tick.cx).with_access_recovery();
        if fresh {
            self.scratch.fresh_access = false;
            run = run.with_fresh_access();
        }
        self.active = Active::Acquire(Box::new(run));
        self.poll_active(tick);
    }

    fn fail(&mut self, code: &'static str, message: impl Into<Arc<str>>) {
        self.scratch.parked = None;
        self.scratch.replan_once = false;
        self.scratch.rebuild_park_request = false;
        self.failure = Some(ScriptFailure {
            code: Arc::from(code),
            message: message.into(),
        });
        self.dirty = true;
    }
    /// Retry only after an observed game tick newer than the failed receipt.
    fn wait_for_walk_retry(&mut self, now: u64) -> bool {
        self.scratch.walk.retry.waiting_for_next_tick(now)
    }

    /// Start a retry sequence for one identity, resetting any previous budget.
    fn begin_walk_retry(&mut self, target: WalkRetryTarget) -> bool {
        let retrying =
            self.scratch.walk.target == Some(target) && self.scratch.walk.retry.pending();
        if !retrying {
            self.scratch.walk.retry.reset();
            self.scratch.walk.target = Some(target);
        }
        retrying
    }

    fn reset_walk_retry(&mut self) {
        self.scratch.walk.retry.reset();
        self.scratch.walk.target = None;
    }
    fn reset_phase_walk_budget(&mut self) {
        self.scratch.walk.failures = 0;
    }

    fn remember_walk_failure(&mut self, receipt: &WalkReceipt) {
        self.scratch.walk.last_detail = Some(receipt.failure_detail_or_fallback());
    }

    fn fail_last_walk(&mut self, code: &'static str, reason: &str) -> bool {
        let Some(detail) = self.scratch.walk.last_detail.as_ref() else {
            return false;
        };
        self.fail(code, format!("{reason}; last walk failure: {detail}"));
        true
    }

    fn block_context(&self, here: Option<WorldTile>, reason: &str) -> String {
        use std::fmt::Write;
        let mut out = String::from(reason);
        let mut methods = self.prepared.methods.iter().filter_map(|&index| {
            self.prepared
                .catalog
                .methods()
                .get(index)
                .map(|method| method.id.0.as_ref())
        });
        if let Some(first) = methods.next() {
            let _ = write!(out, "; method={first}");
            for id in methods {
                let _ = write!(out, ",{id}");
            }
        }
        if let Some(site) = self
            .prepared
            .site()
            .map(|site| site.id.as_str())
            .filter(|id| !id.is_empty())
        {
            let _ = write!(out, "; site={site}");
        }
        match self.area {
            Some(area) => {
                let _ = write!(out, "; area={}", area.label());
            }
            None => out.push_str("; area=unresolved"),
        }
        if let Some(tile) = here {
            let _ = write!(out, "; planning={},{},{}", tile.x, tile.z, tile.level);
        }
        out
    }

    fn cancel_active(&mut self) {
        self.capture_live_timing();
        self.parked_cycle = None;
        api::host_log!(
            api::hostlog::Category::NavTrace,
            api::hostlog::Level::Debug,
            "gather-approach clear reason=cancel-active target={:?}",
            self.target
                .as_ref()
                .map(|target| (target.entity, target.tile)),
        );
        if let Active::Walk(walk) = &mut self.active {
            walk.cancel();
        }
        self.active = Active::None;
        self.scratch.observation_walk = false;
        self.survival_deferred = false;
        self.survival_interrupting = false;
        self.survival_tend = false;
        self.target = None;
    }

    fn set_event(&mut self, event: &str) {
        if self.last_event.as_ref() != event {
            self.last_event = Arc::from(event);
            self.dirty = true;
        }
    }

    fn sync_retained(&mut self, tick: &mut NativeTick<'_>) {
        let cell = tick.cx.retained().gather();
        *cell = self.retained;
    }

    fn observe_death(&mut self, tick: &mut NativeTick<'_>) -> bool {
        let latched = self.death.observe(tick.cx.snapshot());
        self.retained.death_seq = self.death.watermark();
        self.sync_retained(tick);
        latched
    }

    fn apply_pending(&mut self, disposal_due: bool) -> Option<u64> {
        // A hold revokes the machine, not its full-batch reserve target.
        // Keep both the plan and its configuration stable until settlement.
        let parked = self.scratch.parked.is_some();
        if !parked && self.scratch.needs.is_some() {
            return None;
        }
        let next = self.pending.as_ref()?.get::<Arc<Prepared>>()?;
        let current = self.settings();
        let changed_bank = current.bank != next.settings.bank
            || current.use_mage_bank != next.settings.use_mage_bank
            || current.use_zanaris_bank != next.settings.use_zanaris_bank;
        // A bag is atomic: a disposition+bank edit is admitted together at
        // the disposal boundary that starts (or disables) the next bank trip.
        let next_disposal = disposal_due
            && self.trip == TripStep::Idle
            && current.disposition != next.settings.disposition;
        if !parked && changed_bank && self.trip != TripStep::Deposit && !next_disposal {
            return None;
        }
        if !parked
            && current.disposition != next.settings.disposition
            && (!disposal_due || self.trip != TripStep::Idle)
        {
            return None;
        }
        let config = self.pending.take()?;
        let Some(prepared) = config.get::<Arc<Prepared>>() else {
            self.fail(
                "config-identity",
                "pending Gatherer settings were malformed",
            );
            return None;
        };
        let revision = config.revision();
        if self.prepared.methods != prepared.methods {
            self.observed_progress = None;
        }
        self.prepared = Arc::clone(prepared);
        self.config = config;
        self.needs_validate = true;
        self.scratch.replan_once |= parked;
        self.scratch.rebuild_park_request |= parked;
        self.dirty = true;
        self.set_event("settings applied");
        Some(revision)
    }

    fn validate(&mut self, cx: &mut crate::native::ActionContext<'_>) -> Validation {
        let snapshot = cx.snapshot();
        let Some(_local) = snapshot.local_player() else {
            self.set_event("waiting for local player");
            return Validation::Pending;
        };
        let Some(world) = snapshot.world() else {
            self.set_event("waiting for world");
            return Validation::Pending;
        };
        let Some(here) = snapshot.here() else {
            self.set_event("waiting for player tile");
            return Validation::Pending;
        };
        let Some(stats) = snapshot.stats() else {
            self.set_event("waiting for stats");
            return Validation::Pending;
        };
        let Some(inventory) = snapshot.inventory() else {
            self.set_event("waiting for inventory");
            return Validation::Pending;
        };
        let Some(_capacity) = snapshot.inventory_capacity() else {
            self.set_event("waiting for inventory capacity");
            return Validation::Pending;
        };
        let Some(equipment) = snapshot.equipment() else {
            self.set_event("waiting for equipment");
            return Validation::Pending;
        };
        let Some(_locs) = snapshot.locs() else {
            self.set_event("waiting for locations");
            return Validation::Pending;
        };
        let skill = self.settings().skill_kind();
        if snapshot.npcs().is_none() {
            self.set_event("waiting for npcs");
            return Validation::Pending;
        }
        let skill_stat = stats
            .value
            .iter()
            .find(|stat| stat.name.eq_ignore_ascii_case(skill.stat_name()));
        let Some(skill_stat) = skill_stat else {
            self.set_event("waiting for skill stats");
            return Validation::Pending;
        };
        for &index in self.prepared.methods.iter() {
            let Some(method) = self.prepared.catalog.methods().get(index) else {
                continue;
            };
            if !supply::meets_gate(
                stats.value,
                api::selected::SkillMinimum {
                    skill: skill_stat.index as u8,
                    level: method_level(method, skill_stat.index) as u16,
                },
            ) {
                self.fail("level-too-low", format!("level-too-low:{}", method.id.0));
                return Validation::Pending;
            }
            if !world.value.members && has_members_requirement(method) {
                self.fail("members-world", "selected method requires a members world");
                return Validation::Pending;
            }
        }

        // Only watchdog-restamping admissions get a fresh gameplay baseline.
        // Widening and other revalidation must preserve the idle wait cap.
        // An existing wait deadline remains fixed, including across Resume.
        if self.rebaseline_gameplay {
            self.last_gameplay_tick = cx.evidence().tick;
            self.rebaseline_gameplay = false;
        }
        if self.retained.start_tile.is_none() {
            self.retained.start_tile = Some(here.value);
            self.sync_retained_from_context(cx);
        }
        if self.settings().location.eq_ignore_ascii_case("site") && self.retained.anchor.is_none() {
            let site = self.prepared.site().expect("admitted Site setting");
            let Some(anchor) = site_anchor(
                &self.prepared.catalog,
                &self.prepared.methods,
                &site.region,
                &self.avoid,
                cx.evidence().tick,
            ) else {
                self.fail(
                    "area-invalid",
                    format!("no selected resource is accessible at {}", site.label),
                );
                return Validation::Pending;
            };
            self.retained.anchor = Some(anchor);
            self.sync_retained_from_context(cx);
        }
        if self.settings().location.eq_ignore_ascii_case("auto") && self.retained.anchor.is_none() {
            match self.widen.poll(
                &self.prepared.catalog,
                &self.prepared.methods,
                self.retained.start_tile.unwrap_or(here.value),
                self.prepared.settings.radius,
                SearchExclusions {
                    avoided: &self.avoid,
                    tried: &self.tried_groups,
                },
                cx.evidence().tick,
            ) {
                SearchResult::Pending => {
                    self.set_event("searching Auto within 128 tiles");
                    return Validation::Pending;
                }
                SearchResult::Found(anchor) => {
                    self.retained.anchor = Some(anchor);
                    self.sync_retained_from_context(cx);
                }
                SearchResult::Exhausted => {
                    self.fail(
                        "resource-unavailable",
                        "Auto searched 128 tiles without a usable group",
                    );
                    return Validation::Pending;
                }
            }
        }
        let retained_anchor = self.retained.anchor;
        let area = match WorkArea::resolve(self.settings(), retained_anchor, Some(here.value)) {
            Ok(area) => area,
            Err(AreaError::NotReady) => {
                self.set_event("waiting for work-area anchor");
                return Validation::Pending;
            }
            Err(AreaError::Invalid) => {
                self.fail("area-invalid", "the configured work area is invalid");
                return Validation::Pending;
            }
        };
        if self.settings().location.eq_ignore_ascii_case("start") && self.retained.anchor.is_none()
        {
            self.retained.anchor = Some(area.anchor);
            self.retained.start_tile = Some(here.value);
            self.sync_retained_from_context(cx);
        }
        if !self
            .prepared
            .methods
            .iter()
            .filter_map(|index| self.prepared.catalog.methods().get(*index))
            .any(|method| {
                self.prepared
                    .catalog
                    .spots(method, &area.region())
                    .map(|spots| {
                        spots.into_iter().any(|spot| {
                            self.prepared
                                .catalog
                                .access(method, spot)
                                .unwrap_or(Truth::False)
                                != Truth::False
                        })
                    })
                    .unwrap_or(false)
            })
        {
            self.fail(
                "area-invalid",
                "no selected method has an accessible placement in the area",
            );
            return Validation::Pending;
        }

        self.area = Some(area);
        let tool = self.derive_tool(stats.value, inventory.value, equipment.value);
        self.tool = tool.unwrap_or(ToolState {
            id: -1,
            worn: false,
        });
        self.dirty = true;
        // Recovery owns supply/equip admission until its observed step 4.
        if matches!(
            self.retained.recovery,
            RecoveryState::Pending { step: 1..=3 | 5 }
        ) {
            return Validation::Ready;
        }
        // Resume revalidates the area and account facts, not the retained
        // trip's supply/equipment boundary. Never equip inside an open bank.
        if self.trip != TripStep::Idle && self.trip != TripStep::Validate {
            return Validation::Ready;
        }
        if self.scratch.parked.is_some() {
            return Validation::Ready;
        }
        // A full-pack haul is deposited before a known supply shortage can
        // refuse the second request at the open bank.
        if self.trip == TripStep::Idle
            && self.settings().disposition.eq_ignore_ascii_case("Bank")
            && snapshot.stock().pack_full() == Some(true)
        {
            return Validation::Ready;
        }
        if self.admit_supply(cx) {
            return if self.failure.is_some() || self.scratch.parked.is_some() {
                Validation::Pending
            } else {
                Validation::Ready
            };
        }
        Validation::Ready
    }

    fn sync_retained_from_context(&mut self, cx: &mut crate::native::ActionContext<'_>) {
        *cx.retained().gather() = self.retained;
    }

    fn derive_tool(
        &self,
        stats: &[StatView],
        inventory: &[ItemView],
        equipment: &[ItemView],
    ) -> Option<ToolState> {
        supply::best_tool(&self.prepared, stats, inventory, equipment).map(|tool| ToolState {
            id: tool.id,
            worn: tool.worn,
        })
    }

    fn action_failure(&mut self, error: ActionError) {
        match error {
            ActionError::Held | ActionError::BudgetExhausted => {}
            ActionError::BankDeposit(failure) => {
                self.fail("bank-deposit-failed", failure.message());
            }
            ActionError::WalkGoals(error) => {
                self.fail("action-error", format!("observation goals: {error}"));
            }
            error => self.fail("action-error", format!("native action failed: {error:?}")),
        }
    }

    fn deposit_failure(&mut self, error: ActionError) {
        match error {
            ActionError::BankDeposit(failure) => {
                self.fail("bank-deposit-failed", failure.message());
            }
            error => self.fail(
                "bank-deposit-failed",
                format!("bank deposit failed: {error:?}"),
            ),
        }
    }

    fn start_drop(&mut self, tick: &mut NativeTick<'_>) {
        self.parked_cycle = None;
        let snapshot = tick.cx.snapshot();
        let Some(inventory) = snapshot.inventory() else {
            return;
        };
        let products = Arc::clone(&self.prepared.products);
        let mut protected = [0; super::drop::MAX_PROTECTED];
        let ids = supply::protected_ids(&self.prepared, self.tool.id);
        let protected_len = ids.as_slice().len();
        protected[..protected_len].copy_from_slice(ids.as_slice());
        let args = DropBatchArgs {
            products: Arc::clone(&products),
            protected,
            protected_len: protected_len as u8,
        };
        if !inventory.value.iter().any(|row| {
            row.count > 0
                && row.def.name.is_some()
                && products.contains(&row.def.id)
                && !protected[..protected_len].contains(&row.def.id)
        }) {
            self.fail(
                "inventory-blocked",
                "no observed product rows can be dropped",
            );
            return;
        }
        match tick.actions.begin::<DropBatch>(args, &mut tick.cx) {
            Ok(handle) => {
                self.active = Active::Drop(handle);
                self.set_event("dropping observed products");
            }
            Err(error) => self.action_failure(error),
        }
    }

    fn start_target(&mut self, selected: SelectedTarget, tick: &mut NativeTick<'_>) {
        if api::hostlog::debug_enabled() {
            let loc = tick.cx.snapshot().locs().and_then(|locs| {
                locs.value.iter().find(|loc| {
                    selected.plan.entity == api::selected::EntityId::Loc(loc.id)
                        && selected.plan.tile == loc.tile
                })
            });
            api::host_log!(
                api::hostlog::Category::NavTrace,
                api::hostlog::Level::Debug,
                "gather-approach select tick={} id={:?} tile={:?} class={:?} footprint={:?} server={:?}",
                tick.cx.evidence().tick,
                selected.plan.entity,
                selected.plan.tile,
                selected.class,
                loc.map(|loc| (loc.footprint_width, loc.footprint_length, loc.shape, loc.angle, loc.force_approach)),
                tick.cx.snapshot().local_player().map(|player| player.value.player.network),
            );
        }
        let prepared = Arc::clone(&self.prepared);
        let Some(method) = prepared
            .catalog
            .methods()
            .get(usize::from(selected.plan.method_index))
        else {
            self.fail("gather-catalog", "selected target names no catalog method");
            return;
        };
        match prepared.catalog.forbidden_states(method) {
            Ok(states) if states.contains(&GatherForbiddenState::MonkeyForm) => {
                let Some(equipment) = tick.cx.snapshot().equipment() else {
                    // Worn items are unobserved: validation re-reads them before any walk starts.
                    self.needs_validate = true;
                    return;
                };
                // Content refuses this method while a greegree (`mm_greegree`) is worn.
                let greegree_worn = equipment.value.iter().any(|worn| {
                    prepared
                        .selected
                        .item_by_id(worn.def.id)
                        .and_then(|item| item.category.as_deref())
                        == Some("mm_greegree")
                });
                if greegree_worn {
                    self.fail(
                        "forbidden-state:monkey-form",
                        format!(
                            "{} refuses to start while a greegree is worn",
                            selected.plan.alias
                        ),
                    );
                    return;
                }
            }
            Ok(_) => {}
            Err(error) => {
                self.fail("gather-catalog", format!("selected facts: {error:?}"));
                return;
            }
        }
        self.method = Arc::clone(&selected.plan.alias);
        self.target = Some(selected.plan.clone());
        let approach =
            match selected.approach(tick.cx.snapshot(), tick.cx.evidence(), tick.walk_danger) {
                Ok(approach) => approach,
                Err(error) => {
                    let code = if self.trip == TripStep::Return {
                        "return-failed"
                    } else {
                        "resource-unavailable"
                    };
                    let context = format!("safe resource access goals: {error}");
                    if !self.fail_last_walk(code, &context) {
                        self.fail(code, context);
                    }
                    return;
                }
            };
        if let Some(request) = approach {
            let retrying = self.begin_walk_retry(WalkRetryTarget::Resource(
                selected.plan.entity,
                selected.plan.tile,
            ));
            match PrereqWalk::begin(request, tick.actions, &mut tick.cx) {
                Ok(walk) => {
                    if retrying {
                        self.scratch.walk.retry.action_submitted();
                    }
                    self.scratch.observation_walk = selected.observation_goals.is_some();
                    self.active = Active::Walk(walk);
                    self.set_event(if retrying {
                        "resource approach retry submitted from the observed position"
                    } else {
                        "approaching target"
                    });
                }
                Err(error) => self.action_failure(error),
            }
            return;
        }
        self.reset_walk_retry();
        self.reset_phase_walk_budget();
        self.scratch.walk.last_detail = None;
        let Some(plan) = self.target.clone() else {
            return;
        };
        let timing = self.timing_for_target(&plan, &tick.cx);
        self.begin_gather_target(plan, timing, tick);
    }

    fn begin_gather_target(
        &mut self,
        plan: TargetPlan,
        timing: Option<super::gather::GatherTiming>,
        tick: &mut NativeTick<'_>,
    ) {
        match tick.actions.begin::<GatherRun>(
            GatherRunArgs {
                target: plan,
                stall_ticks: DEFAULT_STALL_TICKS,
                catalog: Arc::clone(&self.prepared.catalog),
                quest_owned: false,
                timing,
            },
            &mut tick.cx,
        ) {
            Ok(handle) => {
                self.active = Active::Gather(handle);
                self.set_event("gathering");
            }
            Err(error) => self.action_failure(error),
        }
    }

    fn observe_progress(&mut self, tick: &mut NativeTick<'_>) {
        let snapshot = tick.cx.snapshot();
        let current = snapshot
            .inventory()
            .zip(snapshot.stats())
            .and_then(|(inventory, stats)| {
                let xp = stats
                    .value
                    .iter()
                    .find(|stat| {
                        stat.name
                            .eq_ignore_ascii_case(self.settings().skill_kind().stat_name())
                    })?
                    .xp;
                let products = inventory
                    .value
                    .iter()
                    .filter(|row| self.prepared.products.contains(&row.def.id))
                    .fold(0u32, |total, row| {
                        total.saturating_add(row.count.max(0) as u32)
                    });
                Some((products, xp, snapshot.here()?.value))
            });
        // Inventory may arrive after the target's depleted scene row. The
        // runner owns this baseline across machine completion and reselection.
        let previous = self.observed_progress;
        self.observed_progress = current;
        if matches!(self.retained.recovery, RecoveryState::Pending { .. }) {
            return;
        }
        if let (Some((products, xp, tile)), Some((old_products, old_xp, old_tile))) =
            (current, previous)
        {
            let gained = products.saturating_sub(old_products);
            let xp = xp.saturating_sub(old_xp).max(0);
            if self.retained.recovery == RecoveryState::Proving {
                // Inventory and XP can be published on adjacent frames.
                // Count positive deltas, not a total that disposal can lower.
                self.proof_evidence |= u8::from(gained > 0) | (u8::from(xp > 0) << 1);
                if self.proof_evidence == 3 {
                    self.retained.recovery = RecoveryState::Idle;
                    self.retained.recoveries = self.retained.recoveries.saturating_add(1);
                    self.set_event(&format!("recovered after death {}", self.retained.deaths));
                }
            }
            if tile != old_tile || xp != 0 {
                self.last_gameplay_tick = tick.cx.evidence().tick;
            }
            if gained != 0 || xp != 0 {
                self.reset_phase_walk_budget();
                self.scratch.last_yield_tick = tick.cx.evidence().tick;
                self.scratch.fishing.progress();
                self.yielded = self.yielded.saturating_add(gained);
                self.retained.yielded = self.yielded;
                self.xp = self.xp.saturating_add(xp);
                self.wait_until = None;
                self.dirty = true;
                self.sync_retained(tick);
            }
        }
    }

    fn handle_gather(&mut self, result: GatherResult, tick: &mut NativeTick<'_>) {
        if let Some(timing) = result.timing {
            self.tick_policy = timing.policy;
            self.parked_cycle = Some(timing.cycle);
        }
        self.needs_validate = true;
        if result.gained > 0 || result.xp > 0 {
            self.reset_phase_walk_budget();
            self.last_gameplay_tick = tick.cx.evidence().tick;
            self.scratch.last_yield_tick = tick.cx.evidence().tick;
            self.wait_until = None;
        }
        let target = self.target.take();
        api::host_log!(
            api::hostlog::Category::NavTrace,
            api::hostlog::Level::Debug,
            "gather-approach clear reason=gather-end end={:?} target={:?}",
            result.end,
            target.as_ref().map(|target| (target.entity, target.tile)),
        );
        match result.end {
            GatherEnd::Full => {
                self.set_event("inventory full");
            }
            GatherEnd::TargetGone => self.set_event("target gone"),
            GatherEnd::Depleted => {
                // Re-observe every tick rather than suppressing an early
                // respawn behind an avoidance timer.
                self.set_event("resource depleted");
            }
            GatherEnd::Hazard => {
                if let Some(target) = &target {
                    self.avoid(target.tile, tick.cx.evidence().tick.saturating_add(60));
                }
                self.hazard_escape = target.map(|target| target.tile);
                self.set_event("hazard observed; cancelling queued gather");
            }
            GatherEnd::Idle => {
                if let Some(target) = target {
                    self.avoid(
                        target.tile,
                        tick.cx.evidence().tick.saturating_add(IDLE_AVOID_TICKS),
                    );
                }
                self.set_event("target stalled");
            }
            GatherEnd::Refused => {
                if !self.begin_stray_modal(tick) {
                    self.set_event("server refused gather");
                }
                self.needs_validate = true;
            }
        }
        self.dirty = true;
        if self.retained.recovery == RecoveryState::Proving {
            self.proving_runs = self.proving_runs.saturating_add(1);
            if self.proving_runs >= 3 {
                self.fail(
                    "recovery-no-yield",
                    "three gather runs without product and XP",
                );
            }
        }
    }

    fn handle_drop(&mut self, result: DropResult, tick: &mut NativeTick<'_>) {
        match result.end {
            DropEnd::Cleared if tick.cx.snapshot().stock().pack_full() == Some(true) => {
                self.fail(
                    "inventory-blocked",
                    "full inventory has nothing left to drop",
                );
            }
            DropEnd::Cleared => {
                if self.retained.recovery == RecoveryState::Idle {
                    self.retained.haul_since_death = true;
                    self.sync_retained(tick);
                }
                self.set_event("drop confirmed by empty slots");
            }
            DropEnd::Blocked { remaining } => {
                self.fail(
                    "inventory-blocked",
                    format!("inventory-blocked: {remaining} product slots remain"),
                );
            }
        }
        self.dirty = true;
    }

    fn handle_walk(&mut self, result: WalkReceipt, tick: &mut NativeTick<'_>) {
        if result.end == WalkEnd::Arrived {
            self.reset_walk_retry();
            self.reset_phase_walk_budget();
            self.scratch.walk.last_detail = None;
        }
        if result.end == WalkEnd::Arrived && std::mem::take(&mut self.scratch.observation_walk) {
            let Some(goal) = result.goal else {
                self.fail(
                    if self.trip == TripStep::Return {
                        "return-failed"
                    } else {
                        "walk-failed"
                    },
                    "observation walk returned no selected goal",
                );
                return;
            };
            if let Some(target) = self.target.as_mut() {
                target.tile = goal;
            }
        }
        if api::hostlog::debug_enabled() {
            let server = tick
                .cx
                .snapshot()
                .local_player()
                .map(|player| player.value.player.network);
            let arrived_at = self.target.as_ref().and_then(|target| {
                let api::selected::EntityId::Loc(id) = target.entity else {
                    return None;
                };
                api::query::loc_approach::arrived_at(
                    tick.frame.snapshot?,
                    server?,
                    target.tile,
                    i32::from(RESOURCE_APPROACH_RADIUS),
                    id,
                )
            });
            api::host_log!(
                api::hostlog::Category::NavTrace,
                api::hostlog::Level::Debug,
                "gather-approach walk-end tick={} request={} end={:?} trip={:?} target={:?} server={server:?} arrived_at={arrived_at:?}",
                tick.cx.evidence().tick,
                result.request_id,
                result.end,
                self.trip,
                self.target.as_ref().map(|target| (target.entity, target.tile)),
            );
        }
        if matches!(
            result.refusal,
            Some(crate::native::WalkRefusal::EscapeInProgress)
        ) {
            // Host admission refuses every walk while its escape is live
            // (host-play/src/script_nav.rs:4599-4611). Recheck on the next
            // observed game tick; another refusal means the escape is live.
            if self.trip == TripStep::Idle {
                if let Some(target) = self.target.as_ref() {
                    self.scratch.walk.target =
                        Some(WalkRetryTarget::EscapeResource(target.entity, target.tile));
                }
            }
            self.scratch
                .walk
                .retry
                .hold_next_tick(tick.cx.evidence().tick);
            self.set_event("waiting for the active escape to end");
            return;
        }
        if self.retained.recovery == (RecoveryState::Pending { step: 5 }) {
            // The death-recovery return shares the ordinary return trip's one
            // bounded replan for a transient Traveller stall (AGENTS.md: a
            // walk failure is never terminal until its retries are spent).
            // The budget restarts with each latched death (recover.rs).
            if self
                .scratch
                .walk
                .retry
                .observe_return_stall(&result.end, tick.cx.evidence().tick)
            {
                self.replan_return_after_stall(tick);
                return;
            }
            self.finish_recovery_return(result, tick);
            return;
        }
        if result.end == WalkEnd::UserInput {
            api::host_log!(
                api::hostlog::Category::NavTrace,
                api::hostlog::Level::Debug,
                "gather-approach clear reason=user-input target={:?}",
                self.target
                    .as_ref()
                    .map(|target| (target.entity, target.tile)),
            );
            self.reset_walk_retry();
            self.target = None;
            self.fail("manual-movement", "cancelled by user input");
            return;
        }
        if result.end != WalkEnd::Arrived {
            self.remember_walk_failure(&result);
        }
        if self.trip == TripStep::Return
            && self
                .scratch
                .walk
                .retry
                .observe_return_stall(&result.end, tick.cx.evidence().tick)
        {
            self.replan_return_after_stall(tick);
            return;
        }
        if result.end == WalkEnd::RouteEnded && self.trip == TripStep::Idle {
            let snapshot = tick.cx.snapshot();
            if let Some(target) = self.target.as_ref().filter(|target| {
                matches!(target.entity, api::selected::EntityId::Loc(_))
                    && snapshot
                        .world()
                        .is_some_and(|world| tile_loaded(target.tile, world.value))
                    && matches!(
                        observe_target(snapshot, &self.prepared.catalog, target),
                        Some(TargetObservation::Gone | TargetObservation::Depleted)
                    )
            }) {
                // A loaded resource can disappear while walking, including
                // when another player depletes it. This is not failed reach
                // to a still-present Loc; re-observe before choosing again.
                api::host_log!(
                    api::hostlog::Category::NavTrace,
                    api::hostlog::Level::Debug,
                    "gather-approach clear reason=route-target-gone target={:?}",
                    (target.entity, target.tile),
                );
                self.reset_walk_retry();
                self.target = None;
                self.set_event("target gone during approach; observing resources");
                return;
            }
        }
        if result.end != WalkEnd::Arrived {
            api::host_log!(
                api::hostlog::Category::NavTrace,
                api::hostlog::Level::Debug,
                "gather-approach clear reason=walk-failed end={:?} target={:?}",
                result.end,
                self.target
                    .as_ref()
                    .map(|target| (target.entity, target.tile)),
            );
            // The return stand and the other trip steps keep the one bounded
            // replan for a transient stall above; any other failure there is
            // terminal. Only the resource approach (Idle) falls back across
            // targets, within the per-phase budget.
            if self.trip != TripStep::Idle {
                self.reset_walk_retry();
                self.target = None;
                self.fail(
                    if self.trip == TripStep::Return {
                        "return-failed"
                    } else {
                        "walk-failed"
                    },
                    walk_failure_message(&result),
                );
                return;
            }
            let now = tick.cx.evidence().tick;
            self.scratch.walk.failures = self.scratch.walk.failures.saturating_add(1);
            if self.scratch.walk.failures >= MAX_FAILED_WALKS_PER_PHASE {
                self.reset_walk_retry();
                if !self.fail_last_walk(
                    "walk-failed",
                    "total walk failure budget across targets was spent",
                ) {
                    self.fail("walk-failed", walk_failure_message(&result));
                }
                return;
            }
            if self.scratch.walk.retry.observe_gather_failure(&result, now) {
                if self.hazard_escape.is_some() {
                    self.target = None;
                }
                self.set_event(if self.hazard_escape.is_some() {
                    "retrying hazard escape from the observed position"
                } else {
                    "retrying resource approach from the observed position"
                });
                return;
            }
            let target = self.target.take();
            if let Some(target) = target {
                self.avoid(target.tile, u64::from(u32::MAX));
                self.reset_walk_retry();
                self.scratch.walk.retry.hold_next_tick(now);
                self.set_event("resource approach failed; selecting another target");
                return;
            }
            self.reset_walk_retry();
            if self.hazard_escape.is_some() {
                if !self.fail_last_walk("walk-failed", "hazard escape walk failed") {
                    self.fail("walk-failed", walk_failure_message(&result));
                }
            } else if !self.fail_last_walk("walk-failed", "walk retry budget was spent") {
                self.fail("walk-failed", walk_failure_message(&result));
            }
            return;
        }
        match self.trip {
            TripStep::Return => {
                let arrived = self.resource_return_arrived(tick);
                if !arrived {
                    self.fail("return-failed", "return receipt did not establish arrival at the resource observation stand");
                    return;
                }
                self.scratch.walk.retry.reset();
                self.trips = self.trips.saturating_add(1);
                self.advance_trip(TripStep::Idle);
                self.needs_validate = true;
            }
            TripStep::Idle if self.target.is_some() => {
                // Retain identity until begin_idle rechecks live resource and
                // footprint evidence, after the normal admission boundaries.
                self.hazard_escape = None;
                self.set_event("arrived; observing selected resource");
                return;
            }
            _ => {}
        }
        self.hazard_escape = None;
        // Bank returns and hazard escapes establish an observation stand,
        // not a selected resource identity.
        api::host_log!(
            api::hostlog::Category::NavTrace,
            api::hostlog::Level::Debug,
            "gather-approach clear reason=arrived-reselect target={:?}",
            self.target
                .as_ref()
                .map(|target| (target.entity, target.tile)),
        );
        self.target = None;
        self.set_event("arrived; observing resources");
    }

    fn poll_active(&mut self, tick: &mut NativeTick<'_>) {
        let active = std::mem::take(&mut self.active);
        match active {
            Active::None => {}
            Active::Gather(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Gather(handle),
                Poll::Ready(Ok(result)) => {
                    // These terminal observations emit no operation. Admit one
                    // new target from this same frame instead of idling a tick.
                    self.handle_gather(result, tick);
                }
                Poll::Ready(Err(error)) => {
                    self.action_failure(error);
                }
            },
            Active::Drop(handle) => {
                let result = tick.actions.poll(&handle, &mut tick.cx);
                // Credit observed empty slots on every poll, not just at batch
                // completion: cancellation must not discard confirmed progress.
                let dropped = tick.cx.retained().gather().dropped;
                if self.dropped != dropped {
                    self.dropped = dropped;
                    self.retained.dropped = dropped;
                    self.dirty = true;
                }
                match result {
                    Poll::Pending => self.active = Active::Drop(handle),
                    Poll::Ready(Ok(result)) => {
                        self.handle_drop(result, tick);
                    }
                    Poll::Ready(Err(error)) => {
                        self.action_failure(error);
                    }
                }
            }
            Active::Walk(mut walk) => match walk.poll(tick.actions, &mut tick.cx, tick.state) {
                Poll::Pending => self.active = Active::Walk(walk),
                Poll::Ready(Ok(result)) => {
                    self.handle_walk(result, tick);
                }
                Poll::Ready(Err(error)) => {
                    if self.retained.recovery == (RecoveryState::Pending { step: 5 }) {
                        self.fail("return-failed", format!("return-failed:{error:?}"));
                    } else {
                        self.action_failure(error);
                    }
                }
            },
            Active::EscapeWalk(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::EscapeWalk(handle),
                Poll::Ready(Ok(result)) => self.handle_walk(result, tick),
                Poll::Ready(Err(error)) => self.action_failure(error),
            },
            Active::Tend(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Tend(handle),
                Poll::Ready(Ok(settled)) if std::mem::take(&mut self.survival_tend) => {
                    self.needs_validate = true;
                    // Anchor the click's delays on the frame that showed its
                    // effect, then decide again in this tick with them: a late
                    // effect moves the drink lock past the send-frame clock.
                    self.survival.record_settled(settled, &tick.cx);
                    self.survival_deferred = true;
                    if settled {
                        self.set_event("survival operation settled");
                    } else {
                        // The engine drops a held click while the player is
                        // delayed (`OpHeldHandler.ts:16-19`), for example in
                        // the tick after a mining success's `p_delay(0)`
                        // (`mining.rs2:139`). That is a deferral: the poll
                        // after this one clicks again, and the interrupted
                        // work resumes when survival wants nothing more.
                        self.set_event("survival click not applied; clicking again");
                    }
                }
                Poll::Ready(Ok(settled)) => {
                    if settled {
                        self.needs_validate = true;
                        self.set_event("one operation settled");
                    } else {
                        self.fail("oneop-failed", "one operation did not settle");
                    }
                }
                Poll::Ready(Err(error)) => {
                    if std::mem::take(&mut self.survival_tend) {
                        self.survival.record_settled(false, &tick.cx);
                    }
                    self.action_failure(error);
                }
            },
            Active::Acquire(mut run) => {
                let (bank, access, stage, kind) = run.progress();
                let before = (bank.map(|bank| bank.tile), access.copied(), stage, kind);
                let result = run.poll(tick, OpenGate::Open);
                if self.survival_interrupting && run.take_interrupt_settled() {
                    run.cancel(tick.actions);
                    self.finish_survival_transfer();
                    return;
                }
                let (bank, access, stage, kind) = run.progress();
                let after = (bank.map(|bank| bank.tile), access.copied(), stage, kind);
                if before != after {
                    self.dirty = true;
                    if (before.0, before.1, before.3) != (after.0, after.1, after.3) {
                        if let (Some(bank), Some(access), Some(kind)) = (bank, access, kind) {
                            self.bank_label = Arc::from(format!(
                                "{}; {kind:?}; access:{},{},{}",
                                bank.name, access.x, access.z, access.level,
                            ));
                        }
                    }
                }
                match result {
                    Poll::Pending
                    | Poll::Ready(Err(ActionError::Held | ActionError::BudgetExhausted)) => {
                        self.active = Active::Acquire(run);
                    }
                    Poll::Ready(Ok(AcquireOutcome::Done(receipt))) => {
                        self.retained.deposited =
                            self.retained.deposited.saturating_add(receipt.deposited);
                        if receipt.deposited > 0 && self.retained.recovery == RecoveryState::Idle {
                            self.retained.haul_since_death = true;
                        }
                        self.scratch.kept_unbankable |= receipt.kept_unbankable;
                        // Retain the accepted attribution: any completed
                        // withdrawal makes a later full pack a supply block.
                        self.scratch.withdrew_supplies |= receipt.withdrew_any;
                        self.sync_retained(tick);
                        match self.trip {
                            TripStep::Deposit => {
                                if self.scratch.kept_unbankable
                                    && tick.cx.snapshot().stock().pack_full() != Some(false)
                                {
                                    self.deposit_failure(ActionError::BankDeposit(
                                        crate::bank::ops::DepositFailure::Unbankable,
                                    ));
                                    return;
                                }
                                self.advance_trip(TripStep::Supply {
                                    return_to_work: true,
                                    deposit_products: false,
                                });
                                self.set_event("deposit confirmed");
                            }
                            TripStep::Supply { return_to_work, .. } => {
                                self.scratch.needs = None;
                                if self.retained.recovery == (RecoveryState::Pending { step: 3 }) {
                                    self.advance_trip(TripStep::Idle);
                                    self.advance_recovery(RecoveryState::Pending { step: 4 }, tick);
                                } else if return_to_work {
                                    self.advance_trip(TripStep::Validate);
                                } else {
                                    self.advance_trip(TripStep::Idle);
                                }
                                self.needs_validate = true;
                                self.set_event("bank closed; validating equipment");
                            }
                            _ => unreachable!("an acquisition belongs to a deposit or supply trip"),
                        }
                    }
                    Poll::Ready(Ok(AcquireOutcome::ChildDue { .. })) => {
                        self.fail(
                            "action-error",
                            "a Gatherer bank-only supply request selected a child",
                        );
                    }
                    Poll::Ready(Err(error)) => {
                        self.acquire_failure(*run, error, tick);
                    }
                }
            }
            Active::ParkClose(mut run) => match run.poll(tick, OpenGate::Open) {
                Poll::Pending
                | Poll::Ready(Err(ActionError::Held | ActionError::BudgetExhausted)) => {
                    self.active = Active::ParkClose(run);
                }
                Poll::Ready(Ok(AcquireOutcome::Done(_))) => self.dirty = true,
                Poll::Ready(Ok(AcquireOutcome::ChildDue { .. })) => {
                    self.fail("action-error", "a park close selected a child");
                }
                Poll::Ready(Err(error)) => {
                    self.acquire_failure(*run, error, tick);
                }
            },
        }
        if tick.cx.interaction_event_spent() {
            self.fence.seal();
        }
    }

    fn acquire_failure(
        &mut self,
        mut run: AcquireRun,
        error: ActionError,
        tick: &mut NativeTick<'_>,
    ) {
        let stage = run.progress().2;
        // Cancellation moves the core to Done, so retain the failing stage
        // before releasing its native children.
        run.cancel(tick.actions);
        let cx = &tick.cx;
        if let Some((request, block)) = run.into_blocked() {
            self.park_supply(request, block, cx.evidence().tick);
            return;
        }
        if matches!(error, ActionError::BankDeposit(_)) {
            self.deposit_failure(error);
            return;
        }
        if let ActionError::Failed(message) = &error {
            for code in [
                "tool-missing",
                "tool-unusable",
                "bank-unavailable",
                "bank-close-failed",
            ] {
                if message
                    .strip_prefix(code)
                    .is_some_and(|tail| tail.starts_with(':'))
                {
                    let detail = if code.starts_with("tool-") {
                        message
                            .split_once(": ")
                            .map_or(message.as_ref(), |(_, detail)| detail)
                    } else {
                        message.as_ref()
                    };
                    self.fail(code, Arc::from(detail));
                    return;
                }
            }
        }
        if let ActionError::Blocked(message) = &error {
            if matches!(stage, "Select" | "Access") {
                // The pick's own detail and the last access-walk failure.
                self.fail("bank-unavailable", Arc::clone(message));
                return;
            }
        }
        let code = match stage {
            "Select" | "Access" | "Open" => "bank-unavailable",
            "Deposit" => "bank-deposit-failed",
            "Withdraw" => "bank-withdraw-failed",
            "Close" => "bank-close-failed",
            _ => {
                self.action_failure(error);
                return;
            }
        };
        self.fail(code, format!("{code}: {error:?}"));
    }

    fn begin_stray_modal(&mut self, tick: &mut NativeTick<'_>) -> bool {
        let Some(args) = OneOpArgs::stray_modal(tick.cx.snapshot()) else {
            return false;
        };
        match tick.actions.begin::<OneOp>(args, &mut tick.cx) {
            Ok(handle) => self.active = Active::Tend(handle),
            Err(error) => self.action_failure(error),
        }
        true
    }

    fn survival_eat_floor(&self, tick: &NativeTick<'_>) -> i32 {
        if self.settings().eat_below != 0 {
            return self.settings().eat_below;
        }
        tick.cx
            .snapshot()
            .stats()
            .and_then(|stats| stats.value.iter().find(|stat| stat.index == 3))
            .map_or(0, |hp| hp.base.saturating_add(1) / 2)
    }

    fn active_transfer_in_flight(&self) -> bool {
        match &self.active {
            Active::Acquire(run) => run.transfer_in_flight(),
            _ => false,
        }
    }

    fn interrupt_active_transfer(&mut self, tick: &mut NativeTick<'_>) {
        if self.survival_interrupting || !self.active_transfer_in_flight() {
            return;
        }
        match &mut self.active {
            Active::Acquire(run) => run.interrupt_transfer(tick.actions),
            _ => return,
        }
        self.survival_interrupting = true;
    }

    fn finish_survival_transfer(&mut self) {
        self.cancel_active();
        self.needs_validate = true;
        self.rebaseline_gameplay = true;
        self.survival_deferred = true;
        self.set_event("survival transfer interrupted at boundary");
    }

    fn begin_survival_click(&mut self, mut args: OneOpArgs, tick: &mut NativeTick<'_>) -> bool {
        if !tick.cx.may_continue_this_tick() || !self.fence.reserve(1) {
            self.survival_deferred = true;
            return false;
        }
        if let Some(close) = OneOpArgs::close_modal_if_open(tick.cx.snapshot()) {
            args = close;
        }
        let kind = args.kind;
        match tick.actions.begin::<OneOp>(args, &mut tick.cx) {
            Ok(handle) => {
                if let Some(cycle) = &mut self.parked_cycle {
                    cycle.pause(&mut self.tick_policy);
                }
                self.active = Active::Tend(handle);
                self.survival_tend = true;
                self.survival_deferred = false;
                self.survival.record_sent(kind, &tick.cx);
                self.set_event(match (kind, self.survival.report().decision) {
                    (OneOpKind::CloseModal, _) => "closing a page before survival",
                    (_, crate::survival::Decision::Drinking) => "drinking at boundary",
                    _ => "eating at boundary",
                });
            }
            Err(error) => self.action_failure(error),
        }
        true
    }

    fn poll_survival(&mut self, tick: &mut NativeTick<'_>) -> bool {
        let before = self.survival.report();
        let was_parked = self.survival.parked().is_some();
        let eat_floor = self.survival_eat_floor(tick);
        let want = self
            .survival
            .want(&tick.cx, crate::survival::step::Upkeep::floor(eat_floor));
        if was_parked && self.survival.parked().is_none() {
            // The un-park resumes work the way `Interrupt::Resume` does
            // (design A6.5 item 2). The park was not idle gathering, so the
            // no-yield window restarts too.
            self.needs_validate = true;
            self.rebaseline_gameplay = true;
            self.scratch.last_yield_tick = tick.cx.evidence().tick;
        }
        let wanted = match want {
            crate::survival::step::SurvivalWant::Nothing => {
                if self.active_matches_none() {
                    self.survival_deferred = false;
                }
                self.survival.parked().is_some()
            }
            crate::survival::step::SurvivalWant::Hold => {
                self.survival_deferred = true;
                true
            }
            crate::survival::step::SurvivalWant::Click(args) => {
                if matches!(self.active, Active::Tend(_)) {
                    self.survival_deferred = true;
                } else if matches!(self.active, Active::Walk(_)) {
                    self.survival.defer();
                    self.survival_deferred = true;
                } else if self.active_transfer_in_flight() {
                    self.interrupt_active_transfer(tick);
                    self.survival.defer();
                    self.survival_deferred = true;
                } else {
                    if !self.active_matches_none() {
                        self.cancel_active();
                        self.needs_validate = true;
                        self.rebaseline_gameplay = true;
                    }
                    self.begin_survival_click(args, tick);
                }
                true
            }
            crate::survival::step::SurvivalWant::Need(need) => {
                if matches!(self.active, Active::Walk(_) | Active::Tend(_)) {
                    self.survival_deferred = true;
                    true
                } else if self.active_transfer_in_flight() {
                    self.interrupt_active_transfer(tick);
                    self.survival_deferred = true;
                    true
                } else {
                    if !self.active_matches_none() {
                        self.cancel_active();
                        self.needs_validate = true;
                        self.rebaseline_gameplay = true;
                    }
                    self.survival.park(need, &tick.cx);
                    true
                }
            }
        };
        if self.survival.report() != before {
            self.dirty = true;
        }
        wanted
    }

    fn begin_idle(&mut self, tick: &mut NativeTick<'_>, handoff: bool) {
        if self.wait_for_walk_retry(tick.cx.evidence().tick) {
            return;
        }
        if self.needs_validate {
            match self.validate(&mut tick.cx) {
                Validation::Pending => return,
                Validation::Ready => self.needs_validate = false,
            }
        }
        if self.scratch.parked.is_some() {
            return;
        }
        if matches!(self.active, Active::Acquire(_) | Active::ParkClose(_)) {
            self.begin_trip(tick);
            return;
        }
        if self.trip != TripStep::Idle {
            self.begin_trip(tick);
            return;
        }
        let pack_full = tick.cx.snapshot().stock().pack_full() == Some(true);
        if pack_full && self.settings().disposition.eq_ignore_ascii_case("Bank") {
            self.start_dispose(tick);
            self.begin_trip(tick);
            return;
        }
        if self.admit_supply(&tick.cx) {
            if self.trip != TripStep::Idle {
                self.begin_trip(tick);
            }
            return;
        }
        let Some(area) = self.area else {
            self.needs_validate = true;
            return;
        };
        let snapshot = tick.cx.snapshot();
        if pack_full {
            self.start_dispose(tick);
            if self.trip != TripStep::Idle {
                self.begin_trip(tick);
            }
            return;
        }
        if self.begin_stray_modal(tick) {
            return;
        }
        let (Some(world), Some(locs), Some(here)) =
            (snapshot.world(), snapshot.locs(), snapshot.here())
        else {
            self.needs_validate = true;
            self.set_event(if snapshot.world().is_none() {
                "waiting for world"
            } else if snapshot.locs().is_none() {
                "waiting for locations"
            } else {
                "waiting for player tile"
            });
            return;
        };
        let Some(npcs) = snapshot.npcs() else {
            self.needs_validate = true;
            self.set_event("waiting for npcs");
            return;
        };
        if let Some(plan) = self.target.take() {
            // Hold this identity after an escape or bounded retry, before ranking.
            let retrying = self.scratch.walk.target
                == Some(WalkRetryTarget::EscapeResource(plan.entity, plan.tile))
                || (self.scratch.walk.retry.pending()
                    && self.scratch.walk.target
                        == Some(WalkRetryTarget::Resource(plan.entity, plan.tile)));
            let observation = observe_target(snapshot, &self.prepared.catalog, &plan);
            let enabled = self
                .prepared
                .methods
                .contains(&usize::from(plan.method_index));
            let selected = SelectedTarget {
                plan,
                class: PlacementClass::Live,
                observation_goals: None,
            };
            if enabled
                && observation == Some(TargetObservation::Active)
                && (retrying
                    || selected.plan.npc_index >= 0
                    || super::select::resource_arrived(
                        Some(snapshot),
                        selected.plan.entity,
                        selected.plan.tile,
                    ))
                && tick.walk_danger.is_none_or(|query| {
                    !matches!(
                        query.check(
                            selected.plan.tile,
                            match selected.plan.entity {
                                api::selected::EntityId::Loc(id) => Some(id),
                                _ => None,
                            },
                            super::select::RESOURCE_APPROACH_RADIUS,
                            ArrivalKind::Reach,
                        ),
                        crate::native::WalkDanger::Denied { .. }
                    )
                })
            {
                self.wait_until = None;
                self.absent = 0;
                self.start_target(selected, tick);
                return;
            }
            api::host_log!(
                api::hostlog::Category::NavTrace,
                api::hostlog::Level::Debug,
                "gather-approach clear reason=arrival-revalidation target={:?} observation={observation:?} enabled={enabled}",
                (selected.plan.entity, selected.plan.tile),
            );
        }
        let skill_stat = self.skill_stat(snapshot.stats());
        let selected = select(
            &self.prepared.catalog,
            &self.prepared.methods,
            &self.prepared.settings,
            area,
            &self.avoid,
            super::select::SelectionObservation {
                snapshot: Some(snapshot),
                walk_danger: tick.walk_danger,
                world: world.value,
                locs: locs.value,
                npcs: npcs.value,
                here: here.value,
                now: tick.cx.evidence().tick,
                skill_stat,
                fishing: &mut self.scratch.fishing,
                collision: tick.frame.compiled.collision,
            },
        );
        self.zone_gated = selected.zone_gated;
        // Completion can precede its inventory/XP packet. Only an available
        // target warrants a same-frame handoff; confirm terminal unavailability
        // on the next observation so the last yield is accounted before blocking.
        if handoff && selected.target.is_none() {
            return;
        }
        // Only permanent exclusions from failed walks spend the target set.
        // Temporary absence and finite survey waits keep their usual paths.
        if selected.target.is_none()
            && self.hazard_escape.is_none()
            && self.scratch.walk.last_detail.is_some()
            && matches!(
                &selected.outcome,
                Selection::Exhausted { wait_until, .. }
                    if *wait_until == u64::from(u32::MAX)
            )
        {
            let reason =
                self.block_context(Some(here.value), "no alternate resource target remains");
            if !self.fail_last_walk("walk-failed", &reason) {
                self.fail("walk-failed", reason);
            }
            return;
        }
        let fishing = self.settings().skill_kind() == Skill::Fishing;
        let progress_deadline = if fishing {
            self.scratch.last_yield_tick
        } else {
            self.last_gameplay_tick
        }
        .saturating_add(WAIT_GAMEPLAY_TICKS);
        // A live actor still takes precedence. Empty views, hazards and
        // unproductive observation walks cannot extend the no-yield bound.
        if fishing
            && tick.cx.evidence().tick >= progress_deadline
            && matches!(
                selected.outcome,
                Selection::Target(PlacementClass::Unloaded)
                    | Selection::Absent { .. }
                    | Selection::Exhausted { .. }
            )
        {
            self.fail(
                "resource-unavailable",
                self.block_context(Some(here.value), "no fishing spot observed for 800 ticks"),
            );
            return;
        }
        match selected.target {
            Some(target) => {
                self.hazard_escape = None;
                self.wait_until = None;
                self.absent = 0;
                self.start_target(target, tick);
            }
            None if self.hazard_escape.is_some() => self.escape_hazard(tick),
            None => match selected.outcome {
                Selection::NoStand => {
                    let place = self
                        .prepared
                        .site()
                        .map(|site| site.label.as_str())
                        .filter(|label| !label.is_empty())
                        .unwrap_or("the fishing area");
                    self.fail(
                        "resource-unavailable",
                        self.block_context(
                            Some(here.value),
                            &format!("no usable observation stand at {place}"),
                        ),
                    );
                }
                Selection::InvalidGoals(error) => {
                    self.fail(
                        "resource-unavailable",
                        format!("observation goals: {error}"),
                    );
                }
                Selection::Danger(failure) => {
                    self.fail(
                        "resource-unavailable",
                        self.block_context(Some(here.value), &failure.to_string()),
                    );
                }
                Selection::Absent { absent } => {
                    self.absent = absent;
                    if fishing {
                        // Fishing's authored NPCs move and temporarily change
                        // type; an empty actor view is not permanent absence.
                        self.scratch.fishing.progress();
                        self.wait_until = None;
                        self.set_event("looking for another fishing spot");
                    } else {
                        self.fail(
                            "resource-unavailable",
                            self.block_context(
                                Some(here.value),
                                &format!("resource-unavailable: absent: {absent}"),
                            ),
                        );
                    }
                }
                Selection::Exhausted { wait_until, absent } => {
                    self.absent = absent;
                    let next_deadline = wait_until.min(progress_deadline);
                    let deadline = *self.wait_until.get_or_insert(next_deadline);
                    if tick.cx.evidence().tick >= deadline {
                        if fishing {
                            // Expiry renews the observation search, not a claim
                            // that a content-authored fishing spot cannot exist.
                            self.scratch.fishing.progress();
                            self.wait_until = None;
                            self.set_event("looking for another fishing spot");
                        } else if self.settings().location.eq_ignore_ascii_case("auto") {
                            self.begin_widen(wait_until, tick);
                        } else {
                            self.fail(
                                "resource-unavailable",
                                self.block_context(Some(here.value), "resource-unavailable"),
                            );
                        }
                    } else {
                        self.set_event("waiting for a resource");
                    }
                }
                Selection::Target(_) => unreachable!(),
            },
        }
    }

    fn begin_widen(&mut self, exhausted_until: u64, tick: &mut NativeTick<'_>) {
        let now = tick.cx.evidence().tick;
        let Some(area) = self.area else { return };
        if !remember_group(&mut self.tried_groups, area.anchor, now, exhausted_until) {
            self.fail(
                "widen-limit",
                "Auto widening limit: tried: 4 (within 128 tiles)",
            );
            return;
        }
        self.retained.anchor = None;
        self.area = None;
        self.wait_until = None;
        self.widen.reset();
        self.needs_validate = true;
        self.sync_retained(tick);
        self.set_event("widening Auto within 128 tiles");
    }

    fn escape_hazard(&mut self, tick: &mut NativeTick<'_>) {
        let collision = tick.frame.compiled.collision;
        self.retreat_from_hazard(tick, collision);
    }

    fn retreat_from_hazard(
        &mut self,
        tick: &mut NativeTick<'_>,
        collision: Option<nav_planner::contract::CollisionView<'_>>,
    ) {
        let now = tick.cx.evidence().tick;
        if self
            .last_event
            .as_ref()
            .starts_with("waiting for hazard to clear")
            && self.avoid.iter().any(|entry| u64::from(entry.until) > now)
        {
            return;
        }
        let Some(here) = tick.cx.snapshot().here().map(|row| row.value) else {
            return;
        };
        let Some(hazard) = self.hazard_escape else {
            return;
        };
        let Some(target) = hazard_retreat_stand(here, hazard, collision) else {
            if self.avoid.iter().any(|entry| u64::from(entry.until) > now) {
                self.set_event(&self.block_context(Some(here), "waiting for hazard to clear"));
            } else {
                let context = self.block_context(Some(here), "no reachable stand away from hazard");
                if self.scratch.walk.last_detail.is_some() {
                    self.fail_last_walk("walk-failed", &context);
                } else {
                    self.hazard_escape = None;
                    self.set_event(&context);
                }
            }
            return;
        };
        let request = WalkRequest {
            target,
            candidate_goals: None,
            loc_id: None,
            radius: 0,
            arrival: ArrivalKind::Reach,
            options: WalkOptions {
                allow_teleports: WalkBit::Forbid,
                ..WalkOptions::default()
            },
            required_after: tick.cx.evidence(),
            evidence: None,
            cross: Vec::new().into_boxed_slice(),
            avoid: Box::new([]),
            protect: false,
            food_guard: true,
            cure_guard: false,
            allow: crate::native::WalkAllow {
                food: true,
                ..Default::default()
            },
        };
        let retrying = self.begin_walk_retry(WalkRetryTarget::Hazard(hazard));
        match tick.actions.begin::<Walk>(request, &mut tick.cx) {
            Ok(handle) => {
                if retrying {
                    self.scratch.walk.retry.action_submitted();
                }
                self.scratch.observation_walk = false;
                self.active = Active::EscapeWalk(handle);
                self.set_event(if retrying {
                    "hazard escape retry submitted from the observed position"
                } else {
                    "walking away from hazard"
                });
            }
            Err(error) => self.action_failure(error),
        }
    }

    fn skill_stat(&self, stats: Option<api::snapshot::Observed<&[StatView]>>) -> i32 {
        let Some(stats) = stats else {
            return -1;
        };
        stats
            .value
            .iter()
            .find(|stat| {
                stat.name
                    .eq_ignore_ascii_case(self.settings().skill_kind().stat_name())
            })
            .map_or(-1, |stat| stat.index)
    }

    fn avoid(&mut self, tile: WorldTile, until: u64) {
        let until = until.min(u64::from(u32::MAX)) as u32;
        if let Some(entry) = self.avoid.iter_mut().find(|entry| entry.tile == tile) {
            entry.until = until;
            return;
        }
        if let Some(entry) = self
            .avoid
            .iter_mut()
            .find(|entry| u64::from(entry.until) <= self.last_gameplay_tick)
        {
            *entry = AvoidedTile { tile, until };
            return;
        }
        if let Some((index, _)) = self
            .avoid
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| entry.until)
        {
            self.avoid[index] = AvoidedTile { tile, until };
        }
    }

    fn phase(&self) -> (&'static str, NativePhase) {
        if self.failure.is_some() {
            return ("blocked", NativePhase::Blocked);
        }
        if self.paused {
            return ("paused", NativePhase::Waiting);
        }
        if self.survival.parked().is_some() {
            return ("parked", NativePhase::Waiting);
        }
        if self.scratch.parked.is_some() {
            return ("blocked", NativePhase::Blocked);
        }
        if matches!(self.retained.recovery, RecoveryState::Pending { .. }) {
            return ("recovering", NativePhase::Working);
        }
        if self.trip != TripStep::Idle
            && !matches!(self.active, Active::Walk(_) | Active::EscapeWalk(_))
        {
            if self.wait_until.is_some() {
                return ("waiting", NativePhase::Waiting);
            }
            return ("banking", NativePhase::Working);
        }
        match self.active {
            Active::None => {
                if self.wait_until.is_some() {
                    ("waiting", NativePhase::Waiting)
                } else if self.settings().skill_kind() == Skill::Fishing {
                    ("surveying", NativePhase::Waiting)
                } else {
                    ("gathering", NativePhase::Working)
                }
            }
            Active::Gather(_) => ("gathering", NativePhase::Working),
            Active::Drop(_) => ("dropping", NativePhase::Working),
            Active::Walk(_) | Active::EscapeWalk(_)
                if self.settings().skill_kind() == Skill::Fishing
                    && self
                        .target
                        .as_ref()
                        .is_some_and(|target| target.npc_index < 0) =>
            {
                ("surveying", NativePhase::Waiting)
            }
            Active::Walk(_) | Active::EscapeWalk(_) => ("walking", NativePhase::Working),
            Active::Tend(_) => ("tending", NativePhase::Working),
            Active::Acquire(_) | Active::ParkClose(_) => ("banking", NativePhase::Working),
        }
    }

    fn status_data(&self, inventory: Option<&[ItemView]>) -> StatusData {
        let (phase, _) = self.phase();
        let inventory = if self.retained.recovery == (RecoveryState::Pending { step: 1 }) {
            None
        } else {
            inventory
        };
        let count = |id| {
            inventory.map_or(-1, |rows| {
                rows.iter()
                    .filter(|row| row.def.id == id)
                    .map(|row| i64::from(row.count))
                    .sum::<i64>()
            })
        };
        let bait = self
            .prepared
            .methods
            .iter()
            .flat_map(|&index| known_rows(&self.prepared.catalog.methods()[index].consumes))
            .next()
            .map_or(0, |item| count(item.item));
        StatusData {
            skill: self.settings().skill_kind().name(),
            method: Arc::clone(&self.method),
            survival: self.survival.report(),
            phase,
            area: Arc::<str>::from(self.area_label()),
            target: self.target.as_ref().map_or_else(
                || Arc::from("—"),
                |target| {
                    if self.trip == TripStep::Return
                        || self.retained.recovery == (RecoveryState::Pending { step: 5 })
                    {
                        Arc::from(format!(
                            "{}; observation stand:{},{},{}",
                            target.alias, target.tile.x, target.tile.z, target.tile.level,
                        ))
                    } else {
                        Arc::clone(&target.alias)
                    }
                },
            ),
            tool: if self.tool.id >= 0 {
                Arc::from(format!(
                    "{}{}",
                    self.tool.id,
                    if self.tool.worn { " (worn)" } else { "" }
                ))
            } else {
                Arc::from("—")
            },
            bait,
            food: self
                .prepared
                .supply
                .food
                .as_ref()
                .map_or(0, |food| count(food.id)),
            coins: count(995),
            yielded: self.yielded,
            dropped: self.dropped,
            deposited: self.retained.deposited,
            trips: u32::from(self.trips),
            xp: self.xp,
            xp_per_hour: None,
            bank: if self.bank_label.as_ref() == "—" {
                Arc::clone(&self.bank_label)
            } else {
                let stage = match &self.active {
                    Active::Acquire(run) | Active::ParkClose(run) => run.progress().2,
                    _ => match self.trip {
                        TripStep::Idle => "Idle",
                        TripStep::Deposit => "Deposit",
                        TripStep::Supply { .. } => "Supply",
                        TripStep::Validate => "Validate",
                        TripStep::Return => "Return",
                    },
                };
                Arc::from(format!("{}; {stage}", self.bank_label))
            },
            bank_seen: None,
            block_reason: self
                .scratch
                .parked
                .as_ref()
                .map_or_else(status::empty_block_reason, |park| {
                    Arc::clone(&park.block.message)
                }),
            last_progress: self.last_gameplay_tick,
            deaths: self.retained.deaths,
            recoveries: self.retained.recoveries,
            recovery_step: match self.retained.recovery {
                RecoveryState::Idle => 0,
                RecoveryState::Pending { step } => step,
                RecoveryState::Proving => 6,
            },
            absent: self.absent,
            zone_gated: self.zone_gated,
            excluded_targets: Arc::clone(&self.prepared.excluded_targets),
            last_event: Arc::clone(&self.last_event),
            tick_method: self.tick_method_status(),
            tick_cycle: self.tick_policy.last_cycle_ticks().map_or(-1, i64::from),
            tick_stats: self.tick_policy.stats(),
        }
    }

    fn area_label(&self) -> String {
        let mut label = self
            .area
            .map_or_else(|| "unresolved".to_owned(), WorkArea::label);
        if self.settings().location.eq_ignore_ascii_case("auto") {
            use std::fmt::Write;
            let tried = self
                .tried_groups
                .iter()
                .filter(|entry| u64::from(entry.until) > self.fence.tick)
                .count();
            let _ = write!(
                label,
                "; searched: {}; tried: {tried}; limit: 128",
                self.widen.searched
            );
        }
        if let Some(until) = self.wait_until {
            use std::fmt::Write;
            let _ = write!(label, "; wait_until: {until}");
        }
        label
    }

    fn publish(&mut self, tick: &mut NativeTick<'_>) {
        self.refresh_tick_status();
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.refresh_park_bank_provenance(tick);
        let (_, native_phase) = self.phase();
        let mut data = self.status_data(tick.cx.snapshot().inventory().map(|rows| rows.value));
        data.bank_seen = crate::acquire::plan::Counts::bank_seen(&tick.cx.snapshot().stock());
        status::publish(
            tick.output,
            self.run,
            self.config.revision(),
            self.pending.as_ref().map(|config| config.revision()),
            native_phase,
            self.failure.clone(),
            &data,
        );
        if self.last_paint_tick == 0
            || tick.cx.active_now().as_secs() >= self.last_paint_tick.saturating_add(2)
        {
            self.last_paint_tick = tick.cx.active_now().as_secs();
            tick.output.paint(status::paint(&data));
        }
    }
}

impl Script for Gatherer {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.fence.tick == u64::MAX {
            // Start the first no-yield window on the session clock, not tick 0.
            self.scratch.last_yield_tick = tick.cx.evidence().tick;
        }
        self.fence.observe(tick.cx.evidence().tick);
        if self.run != tick.cx.run() {
            let session_changed = self.run.session != tick.cx.run().session;
            self.run = tick.cx.run();
            self.observed_progress = None;
            if session_changed {
                // The restart message names the last access-walk failure the
                // cancelled run recorded.
                if let Active::Acquire(run) = &self.active {
                    if let Some(reason) = run.last_access_reason() {
                        self.scratch.trip_reason = Some(reason);
                    }
                }
            }
            self.cancel_active();
            self.tick_policy = Default::default();
            self.tick_applicability = None;
            self.tick_last_off = None;
            self.tick_last_sequence = 0;
            self.needs_validate = true;
            self.rebaseline_gameplay = true;
            self.death = crate::native::death::DeathLatch::from_watermark(self.retained.death_seq);
            self.survival = crate::survival::SurvivalStep::default();
            self.survival_deferred = false;
            self.survival_interrupting = false;
            self.set_event("run key changed; revalidating");
            if session_changed {
                self.restart_trip_after_session(self.run.session);
            }
        }
        if !tick.cx.eligible {
            // The host posts guardian holds through eligibility, including
            // frames with no explicit Interrupt::Hold callback.
            self.cancel_active();
            self.needs_validate = true;
            self.rebaseline_gameplay = true;
            self.set_event("held; revalidation required");
        }
        if !tick.cx.eligible || self.paused {
            self.publish(tick);
            return Ok(ScriptFlow::Continue);
        }
        if self.observe_death(tick) {
            self.latch_recovery(tick);
        }
        let recovering = matches!(self.retained.recovery, RecoveryState::Pending { .. });
        let survival_wanted = !recovering && self.poll_survival(tick);
        if self.failure.is_some() && !survival_wanted && self.survival.parked().is_none() {
            self.publish(tick);
            return Ok(ScriptFlow::Blocked(self.failure.clone().unwrap()));
        }
        let recoveries = self.retained.recoveries;
        self.observe_progress(tick);
        if self.scratch.parked.is_some() {
            if self.active_matches_none() {
                if let Some(revision) = self.apply_pending(false) {
                    tick.output.settings_applied(revision);
                }
            }
            if self.poll_park(tick) {
                self.publish(tick);
                return Ok(ScriptFlow::Continue);
            }
        }
        if matches!(self.retained.recovery, RecoveryState::Pending { .. }) {
            self.poll_recovery(tick);
            self.publish(tick);
            return Ok(match &self.failure {
                Some(failure) => ScriptFlow::Blocked(failure.clone()),
                None => ScriptFlow::Continue,
            });
        }
        let bank_close_before_poll =
            self.last_event.as_ref() == "bank closed; validating equipment";
        let handoff = !self.active_matches_none();
        if handoff {
            self.poll_active(tick);
        }
        let bank_closed_this_tick = !bank_close_before_poll
            && self.last_event.as_ref() == "bank closed; validating equipment";
        // Preserve the close observation unless survival starts a higher-priority operation.
        if bank_closed_this_tick {
            self.set_event("bank closed; validating equipment");
        }
        if handoff
            && self.active_matches_none()
            && self.survival_deferred
            && tick.cx.may_continue_this_tick()
        {
            self.survival_deferred = false;
            self.poll_survival(tick);
        }
        if self.survival.parked().is_some() {
            self.publish(tick);
            return Ok(ScriptFlow::Continue);
        }
        if self.active_matches_none()
            && !self.survival_deferred
            && tick.cx.may_continue_this_tick()
            && self.failure.is_none()
        {
            let disposal_due = tick.cx.snapshot().stock().pack_full() == Some(true);
            if let Some(revision) = self.apply_pending(disposal_due) {
                tick.output.settings_applied(revision);
            }
            // Even at expiry, re-observe first: a freshly regrown resource
            // takes precedence over waiting or widening.
            self.begin_idle(tick, handoff);
        }
        if self.retained.recoveries != recoveries {
            self.set_event(&format!("recovered after death {}", self.retained.deaths));
        }
        self.publish(tick);
        Ok(ScriptFlow::Continue)
    }

    fn configure(&mut self, next: Arc<PreparedConfig>) -> Result<SettingsApply, ConfigError> {
        let prepared = next
            .get::<Arc<Prepared>>()
            .ok_or_else(|| ConfigError::new("", "config-identity", "not Gatherer"))?;
        if self
            .prepared
            .settings
            .restart_required_changed(&prepared.settings)
        {
            // The slot replaces its pending revision with this edit. An older
            // boundary edit is no longer eligible to activate either.
            self.pending = None;
            return Ok(SettingsApply::RestartRequired);
        }
        if let Some(error) = &prepared.site_error {
            return Err(error.clone());
        }
        if self.prepared.settings.boundary_changed(&prepared.settings)
            && (self.scratch.parked.is_none() || !self.active_matches_none())
        {
            self.pending = Some(next);
            self.dirty = true;
            return Ok(SettingsApply::PendingBoundary);
        }
        self.prepared = Arc::clone(prepared);
        self.config = next;
        self.pending = None;
        self.needs_validate = true;
        self.scratch.replan_once |= self.scratch.parked.is_some();
        self.scratch.rebuild_park_request |= self.scratch.parked.is_some();
        self.dirty = true;
        Ok(SettingsApply::Applied)
    }

    fn interrupt(&mut self, event: Interrupt) {
        match event {
            Interrupt::Pause | Interrupt::Hold(true) | Interrupt::SessionEnded => {
                self.cancel_active();
                self.paused = true;
                self.dirty = true;
            }
            Interrupt::Resume => {
                self.paused = false;
                self.needs_validate = true;
                self.rebaseline_gameplay = true;
                self.scratch.replan_once = self.scratch.parked.is_some();
                self.dirty = true;
            }
            Interrupt::Hold(false) | Interrupt::SessionReady => {
                self.paused = false;
                self.needs_validate = true;
                self.rebaseline_gameplay = true;
                self.dirty = true;
            }
        }
    }

    fn user_input(&mut self) {
        // The same end as a walk cancelled by user input.
        self.cancel_active();
        if self.failure.is_none() {
            self.fail("manual-movement", "cancelled by user input");
        }
    }
    fn on_stop(&mut self, _reason: StopReason) {
        self.cancel_active();
        self.pending = None;
        self.scratch.parked = None;
    }

    fn recovery_anchor(&self) -> Option<WorldTile> {
        (self.trip == TripStep::Idle && self.retained.recovery == RecoveryState::Idle)
            .then_some(self.retained.anchor)
            .flatten()
    }

    fn waits_for_evidence(&self) -> bool {
        self.active_matches_none()
            && (self.survival.parked().is_some()
                || (self.scratch.parked.is_some()
                    && !self.scratch.replan_once
                    && self.failure.is_none()))
    }
}

impl Gatherer {
    fn active_matches_none(&self) -> bool {
        matches!(self.active, Active::None)
    }
}

#[cfg(test)]
#[path = "runner_switch_tests.rs"]
mod switch_tests;

#[cfg(test)]
#[path = "runner_park_tests.rs"]
mod park_tests;

#[cfg(test)]
#[path = "runner_trip_tests.rs"]
mod trip_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quester::families::tests::with_tick;
    use std::mem::size_of;

    pub(super) fn survival_condition(
        poison: api::condition::PoisonState,
        hp: u8,
        food: api::condition::FoodSummary,
    ) -> api::condition::BotCondition {
        api::condition::BotCondition {
            tick: 20,
            session: api::condition::SessionMark::Live,
            hp,
            hp_max: 99,
            hp_known: true,
            poison,
            poison_window: None,
            food,
            cures: api::condition::CureDoses::default(),
        }
    }

    pub(super) fn accept_last_interaction(tick: &mut NativeTick<'_>) {
        let evidence = tick.cx.evidence();
        let authority = tick
            .cx
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .authority();
        tick.cx.ledger.as_mut().unwrap().complete_interaction(
            &authority,
            crate::native::InteractionReceipt {
                request_id: authority.request_id().get(),
                evidence,
                accepted: true,
                chat_since: 0,
            },
        );
    }

    pub(super) fn antipoison_item(slot: i32) -> ItemView {
        use api::snapshot::{ItemActionFamily, ItemContainer};
        let id = crate::combat::guard::shared_tables()
            .unwrap()
            .potion(crate::combat::tables::PotionKind::Antipoison)
            .unwrap()
            .doses[0]
            .unwrap()
            .id;
        ItemView {
            def: crate::quester::families::tests::def(id, "Antipoison"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot,
            count: 1,
            actions: Vec::new(),
            component_id: 0,
        }
    }

    #[test]
    fn gatherer_names_incomplete_search_as_non_risk_failure() {
        let detail: Arc<str> = Arc::from(
            "search_result=BudgetExhausted nodes_settled=unavailable (incomplete search)",
        );
        let receipt = WalkReceipt {
            goal: None,
            request_id: 1,
            evidence: api::quest_progress::EvidenceStamp {
                run: RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 1,
                sequence: 1,
            },
            end: WalkEnd::Refused,
            blocked: None,
            detail: Some(Arc::clone(&detail)),
            refusal: Some(WalkRefusal::SearchIncomplete {
                nodes_settled: None,
            }),
            assessment: None,
            escape: None,
            prereq_allow: None,
        };
        assert_eq!(
            walk_failure_message(&receipt),
            format!("walk search incomplete: {detail}"),
        );
    }

    #[test]
    fn tick_kit_drop_copy_and_withdrawal_batch_hold_all_nine_supply_ids() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{def, with_tick};
        use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView};

        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let reserve = selected
            .teleports()
            .iter()
            .find(|spell| spell.available() && spell.runes.len() == 3)
            .unwrap();
        let mut settings = crate::native::SettingsBag::new();
        for (key, value) in [
            ("skill", serde_json::json!("Fishing")),
            ("fishingMethod", serde_json::json!("fishing.freshfish.op1")),
            ("food", serde_json::json!("Lobster")),
            ("foodTarget", serde_json::json!(1)),
            ("coinTarget", serde_json::json!(1)),
            ("reserveTeleport", serde_json::json!(reserve.name)),
            ("reserveCasts", serde_json::json!(1)),
            ("tickMethod", serde_json::json!("Knife")),
        ] {
            settings.insert(key.into(), value);
        }
        let config = api::selected::FamilyPreparation::run(move |families| {
            crate::slot::prepare_config(
                families,
                crate::CompiledId("Gatherer"),
                1,
                Arc::new(settings),
                selected,
                Arc::default(),
                None,
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        let prepared = Arc::clone(config.get::<Arc<Prepared>>().unwrap());
        let tool = known_rows(&prepared.catalog.methods()[prepared.methods[0]].tools)[0].item;
        let protected = supply::protected_ids(&prepared, tool);
        assert_eq!(protected.as_slice().len(), 9);
        let kit = prepared.supply.tick_kit.as_ref().unwrap();
        let ordered = prepared.selected.server_ordered_catalog().unwrap();
        assert_eq!(kit.knife.id, ordered.knife().knife().id());
        assert!(ordered
            .knife()
            .logs()
            .iter()
            .any(|logs| logs.id() == kit.logs.id));
        assert_eq!((kit.knife.id, kit.logs.id), (946, 1511));
        assert!(protected.as_slice().contains(&kit.knife.id));
        assert!(protected.as_slice().contains(&kit.logs.id));
        let row = |id, count, container| ItemView {
            def: def(
                id,
                prepared
                    .selected
                    .item_by_id(id)
                    .unwrap()
                    .name
                    .as_deref()
                    .unwrap(),
            ),
            container,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count,
            actions: Vec::new(),
            component_id: -1,
        };
        let bank: Vec<_> = protected
            .as_slice()
            .iter()
            .map(|&id| row(id, 10_000, ItemContainer::Bank))
            .collect();
        let stats = [StatView {
            index: 10,
            name: "fishing".into(),
            base: 99,
            effective: 99,
            xp: 0,
            used: true,
        }];
        let mut memory = api::bank_memory::BankMemory::default();
        memory.observe(
            &bank,
            64,
            api::bank_memory::ObservedAt {
                unix_secs: 1,
                tick: 1,
                bank_session: 1,
            },
        );
        let stock = api::stock::Stock {
            pack: Some(&[]),
            capacity: Some(28),
            worn: Some(&[]),
            bank: Some(&memory),
        };
        let needs = supply::SupplyPlan::build(&prepared, &stats, stock).unwrap();
        let needed: std::collections::BTreeSet<i32> = needs
            .iter()
            .flat_map(|need| std::iter::once(need.item).chain(need.alternatives.iter().copied()))
            .collect();
        for id in protected.as_slice() {
            assert!(
                needed.contains(id) || *id == tool,
                "the supply needs must cover protected item {id}"
            );
        }
        assert!(!supply::SupplyPlan::tick_kit_missing(&prepared, stock));
        let product = prepared.products[0];
        let mut inventory: Vec<_> = protected
            .as_slice()
            .iter()
            .map(|&id| row(id, 1, ItemContainer::Inventory))
            .collect();
        inventory.push(row(product, 1, ItemContainer::Inventory));
        assert_eq!(
            supply::bank_deposit_ids(&prepared, tool, &inventory).as_ref(),
            &[product]
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(inventory, 28);
        let mut gatherer = Gatherer::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            config,
            Arc::clone(&prepared),
            GatherRetained::default(),
        );
        gatherer.tool = ToolState {
            id: tool,
            worn: false,
        };
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.start_drop(tick));
        assert!(matches!(gatherer.active, Active::Drop(_)));
    }

    #[test]
    fn gatherer_resource_validation_uses_effective_level() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{local_player, with_tick};
        use api::snapshot::{GameSnapshot, WorldStateView};
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("skill".into(), serde_json::json!("Mining"));
        settings.insert("miningResources".into(), serde_json::json!(["iron"]));
        let config = api::selected::FamilyPreparation::run(move |families| {
            crate::slot::prepare_config(
                families,
                crate::CompiledId("Gatherer"),
                1,
                Arc::new(settings),
                selected,
                Arc::default(),
                None,
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        let prepared = Arc::clone(config.get::<Arc<Prepared>>().unwrap());
        let spot = known_rows(&prepared.catalog.methods()[prepared.methods[0]].spots)[0].origin;
        for (base, effective, refused) in [(14, 15, false), (15, 14, true)] {
            let mut gatherer = Gatherer::new(
                RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                Arc::clone(&config),
                Arc::clone(&prepared),
                GatherRetained::default(),
            );
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_local_player(local_player(spot));
            snapshot.seed_world(WorldStateView {
                map_base_x: spot.x - 50,
                map_base_z: spot.z - 50,
                level: spot.level,
                members: true,
                ..Default::default()
            });
            snapshot.seed_stats(vec![StatView {
                index: 14,
                name: "mining".into(),
                base,
                effective,
                xp: 0,
                used: true,
            }]);
            snapshot.seed_inventory(vec![], 28);
            snapshot.seed_equipment(vec![]);
            snapshot.seed_locs(vec![]);
            snapshot.seed_npcs(vec![]);
            let mut ledger = None;
            with_tick(&snapshot, &mut ledger, 1, |tick| {
                gatherer.validate(&mut tick.cx)
            });
            assert_eq!(
                gatherer.failure.is_some(),
                refused,
                "base={base}, effective={effective}"
            );
            if refused {
                assert_eq!(
                    gatherer.failure.as_ref().unwrap().code.as_ref(),
                    "level-too-low"
                );
            }
        }
    }

    #[test]
    fn packet_fence_caps_reserved_packets_and_resets_on_observation() {
        let mut fence = TickPacketFence::default();
        fence.observe(7);
        assert!(fence.reserve(5));
        assert!(!fence.reserve(1));
        fence.observe(8);
        assert!(fence.reserve(5));
        assert!(!fence.reserve(1));
    }

    #[test]
    fn instance_with_widening_stays_inside_the_inline_budget() {
        // Lazy acquisition owners shrink 1,432 B to 840 B; the restored 1,024-B cap leaves 184 B.
        assert_eq!(size_of::<Walk>(), 88, "Walk inline layout changed");
        // Measured 1,176 -> 1,584 B: parked_cycle 120 + tick_policy 80 +
        // sequence 4 + applicability 1 + last-off 1 = 206 B; Active 280 -> 480
        // adds 200 B when Gather 472 becomes its largest variant; padding 2 -> 4
        // adds 2 B. Main approves 1,592 B, leaving 8 B of headroom.
        // Follow-up question: if policy/parked state can stay cold for users
        // who never enable tick manipulation, Option<Box<...>> could save
        // about 200 B per runner, at the cost of an activation allocation.
        assert!(
            size_of::<Gatherer>() <= 1592,
            "Gatherer={} exceeds its inline budget",
            size_of::<Gatherer>()
        );
        // The stale-HP watch grew SurvivalStep from 192 B to 196 B; the
        // ceiling keeps 64 B of headroom.
        assert!(
            size_of::<crate::survival::SurvivalStep>() <= 260,
            "SurvivalStep={} exceeds its 260 B bound",
            size_of::<crate::survival::SurvivalStep>()
        );
        assert!(size_of::<GatherRetained>() <= 80);
        // One boxed `GatherScratch` per run: 56 bytes at the fix's base
        // (1bc5c3868), 80 at the 0.2.9 batch head (the selected bank moved in),
        // 104 with the bank relog fix, 144 with the supply park. The park moves
        // the selected bank, withdrawals, open attempts and access recovery out
        // of the scratch (the acquisition run owns them) and adds the held
        // needs, the park, its two flags, the walk bookkeeping
        // (`GatherWalk`: the shared retry core, the retry target, the
        // cross-target failure count and the last walk detail) and the trip's
        // session-restart count and last reason. Measured 144 B on macOS
        // aarch64; the pin is the reading.
        assert!(
            size_of::<GatherScratch>() <= 144,
            "GatherScratch={} exceeds 144 bytes",
            size_of::<GatherScratch>()
        );
        println!(
            "Gatherer={} Active={} GatherRun={} DropBatch={} OneOp={} GatherRetained={} GatherScratch={} RetryTargetOption={} AvoidedTile={} TargetPlan={} Walk={} WalkGoals={} WalkRequest={} WalkReceipt={}",
            size_of::<Gatherer>(),
            size_of::<Active>(),
            size_of::<GatherRun>(),
            size_of::<DropBatch>(),
            size_of::<OneOp>(),
            size_of::<GatherRetained>(),
            size_of::<GatherScratch>(),
            size_of::<Option<WalkRetryTarget>>(),
            size_of::<AvoidedTile>(),
            size_of::<TargetPlan>(),
            size_of::<Walk>(),
            size_of::<crate::native::WalkGoals>(),
            size_of::<crate::native::WalkRequest>(),
            size_of::<crate::native::WalkReceipt>()
        );
        println!(
            "TickPolicy={} ParkedCycle={} GatherTiming={} TickStats={}",
            size_of::<crate::native::tickcycle::TickPolicy>(),
            size_of::<Option<crate::native::tickcycle::TickCycle>>(),
            size_of::<crate::gatherer::gather::GatherTiming>(),
            size_of::<crate::gatherer::status::TickStats>(),
        );
    }
    #[test]
    fn gatherer_tickfix_validate_begins_resource_return_walk_same_tick() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{local_player, with_tick};
        use api::snapshot::GameSnapshot;

        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let mut bag = crate::native::SettingsBag::new();
        for (key, value) in [
            ("skill", serde_json::json!("Fishing")),
            ("fishingMethod", serde_json::json!("fishing.rarefish.op3")),
            ("location", serde_json::json!("Auto")),
            ("radius", serde_json::json!(40)),
            ("disposition", serde_json::json!("Bank")),
        ] {
            bag.insert(key.into(), value);
        }
        let config = api::selected::FamilyPreparation::run(move |families| {
            crate::slot::prepare_config(
                families,
                crate::CompiledId("Gatherer"),
                1,
                Arc::new(bag),
                selected,
                Arc::default(),
                None,
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        let mut gatherer = Gatherer::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            Arc::clone(&config),
            Arc::clone(config.get::<Arc<Prepared>>().unwrap()),
            GatherRetained::default(),
        );
        gatherer.area = Some(WorkArea {
            mode: super::super::area::AreaMode::Auto,
            anchor: WorldTile {
                x: 2848,
                z: 3426,
                level: 0,
            },
            radius: 40,
        });
        gatherer.trip = TripStep::Validate;
        gatherer.needs_validate = false;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        }));
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            gatherer.begin_idle(tick, false)
        });
        assert_eq!(gatherer.trip, TripStep::Return);
        assert_eq!(gatherer.trips, 0);
        let action = &ledger.as_ref().unwrap().outbox[0];
        let crate::native::HostEffect::Walk(request) = &action.effect else {
            panic!("the resource return must arm a walk");
        };
        assert_ne!(request.target, gatherer.area.unwrap().anchor);
        assert!(request.food_guard);
        assert!(request.allow.food);
        assert_eq!(
            request.radius, 0,
            "r40 bounds gathering; the router selects an exact observation goal"
        );
        assert_eq!(request.arrival, ArrivalKind::Area);
        let chosen_goal = request.candidate_goals.as_ref().unwrap().as_slice()[0];
        assert!(gatherer
            .status_data(None)
            .target
            .contains("observation stand:"));
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            gatherer.handle_walk(
                WalkReceipt {
                    goal: Some(chosen_goal),
                    request_id: 1,
                    evidence: tick.cx.evidence(),
                    end: WalkEnd::Arrived,
                    blocked: None,
                    detail: None,
                    refusal: None,
                    assessment: None,
                    escape: None,
                    prereq_allow: None,
                },
                tick,
            );
        });
        assert_eq!(
            gatherer.trips, 0,
            "bank membership is not resource observation"
        );
        assert_eq!(
            gatherer.failure.as_ref().unwrap().code.as_ref(),
            "return-failed"
        );

        let stand = gatherer.target.as_ref().expect("return target").tile;
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            assert!(
                !gatherer.resource_return_arrived(tick),
                "bank location is not the resource observation stand"
            );
        });

        // Reset the terminal failure for the independent successful-arrival case.
        gatherer.failure = None;

        // The path head is at the stand while the rendered actor still
        // trails at the bank tile.
        let rendered = snapshot.local_player().unwrap().player.actor.tile;
        let mut player = local_player(rendered);
        player.player.network = stand;
        snapshot.seed_local_player(player);
        assert_eq!(snapshot.local_player().unwrap().player.actor.tile, rendered);
        assert_eq!(snapshot.local_player().unwrap().player.network, stand);
        with_tick(&snapshot, &mut ledger, 4, |tick| {
            gatherer.handle_walk(
                WalkReceipt {
                    goal: None,
                    request_id: 1,
                    evidence: tick.cx.evidence(),
                    end: WalkEnd::Arrived,
                    blocked: None,
                    detail: None,
                    refusal: None,
                    assessment: None,
                    escape: None,
                    prereq_allow: None,
                },
                tick,
            );
        });
        assert_eq!(
            gatherer.trips, 1,
            "network arrival must count despite the lagging rendered tile"
        );
        assert_eq!(gatherer.trip, TripStep::Idle);
        assert!(gatherer.failure.is_none());
    }

    #[test]
    fn bank_return_failure_preserves_walk_receipt_detail() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;
        use api::snapshot::GameSnapshot;

        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let config = api::selected::FamilyPreparation::run(move |families| {
            crate::slot::prepare_config(
                families,
                crate::CompiledId("Gatherer"),
                1,
                Arc::new(crate::native::SettingsBag::new()),
                selected,
                Arc::default(),
                None,
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let mut gatherer = Gatherer::new(
            run,
            Arc::clone(&config),
            Arc::clone(config.get::<Arc<Prepared>>().unwrap()),
            GatherRetained::default(),
        );
        gatherer.trip = TripStep::Return;
        gatherer.set_event("returning to work area");
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            gatherer.handle_walk(
                WalkReceipt {
                    goal: None,
                    request_id: 7,
                    evidence: tick.cx.evidence(),
                    end: WalkEnd::Failed,
                    blocked: None,
                    detail: Some(Arc::from(
                        "Dropped: stuck at (2809,3441,0), aiming (2848,3426,0)",
                    )),
                    refusal: None,
                    assessment: None,
                    escape: None,
                    prereq_allow: None,
                },
                tick,
            );
        });
        let failure = gatherer.failure.as_ref().unwrap();
        assert_eq!(failure.code.as_ref(), "return-failed");
        assert!(failure.message.contains("Dropped"));
        assert!(failure.message.contains("2809,3441,0"));
        assert!(failure.message.contains("2848,3426,0"));
        assert_eq!(gatherer.phase(), ("blocked", NativePhase::Blocked));
        assert_eq!(gatherer.trips, 0);
    }

    #[test]
    fn user_input_walk_blocks_gatherer_without_rearming() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;
        use api::snapshot::GameSnapshot;

        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let mut bag = crate::native::SettingsBag::new();
        bag.insert("location".into(), serde_json::json!("Auto"));
        let config = api::selected::FamilyPreparation::run(move |families| {
            crate::slot::prepare_config(
                families,
                crate::CompiledId("Gatherer"),
                1,
                Arc::new(bag),
                selected,
                Arc::default(),
                None,
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let mut gatherer = Gatherer::new(
            run,
            Arc::clone(&config),
            Arc::clone(config.get::<Arc<Prepared>>().unwrap()),
            GatherRetained::default(),
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![], 28);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |native| {
            gatherer.handle_walk(
                WalkReceipt {
                    goal: None,
                    request_id: 7,
                    evidence: api::quest_progress::EvidenceStamp {
                        run,
                        tick: 1,
                        sequence: 1,
                    },
                    end: WalkEnd::UserInput,
                    blocked: None,
                    detail: None,
                    refusal: None,
                    assessment: None,
                    escape: None,
                    prereq_allow: None,
                },
                native,
            );
        });
        let failure = gatherer.failure.as_ref().unwrap();
        assert_eq!(failure.code.as_ref(), "manual-movement");
        assert_eq!(failure.message.as_ref(), "cancelled by user input");

        for tick in 1..=3 {
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, tick, |native| {
                    gatherer.tick(native).unwrap()
                }),
                ScriptFlow::Blocked(failure)
                    if failure.code.as_ref() == "manual-movement"
                        && failure.message.as_ref() == "cancelled by user input"
            ));
        }
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }

    /// A Draynor-style Bait fishing Gatherer at its spot holding only the
    /// rod, plus a `Session` memory of an empty bank and a `Hint` claiming
    /// 20 bait.
    pub(super) struct BaitFixture {
        pub(super) config: Arc<PreparedConfig>,
        pub(super) prepared: Arc<Prepared>,
        pub(super) snapshot: api::snapshot::GameSnapshot,
        pub(super) session: api::bank_memory::BankMemory,
        pub(super) hint: api::bank_memory::BankMemory,
    }

    impl BaitFixture {
        pub(super) fn new() -> Self {
            Self::with_banks(Self::test_banks())
        }

        fn test_banks() -> Arc<api::named_banks::NamedBankFacts> {
            Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![
                api::named_banks::NamedBank::new(
                    "Test bank",
                    WorldTile {
                        x: 3092,
                        z: 3243,
                        level: 0,
                    },
                ),
            ]))
        }

        pub(super) fn with_banks(banks: Arc<api::named_banks::NamedBankFacts>) -> Self {
            Self::with_eat_below_and_banks(0, banks)
        }

        fn with_eat_below(eat_below: i32) -> Self {
            Self::with_eat_below_and_banks(eat_below, Self::test_banks())
        }

        fn with_eat_below_and_banks(
            eat_below: i32,
            banks: Arc<api::named_banks::NamedBankFacts>,
        ) -> Self {
            use crate::quester::families::tests::{def, local_player};
            use api::bank_memory::{BankMemory, ObservedAt};
            use api::snapshot::{
                GameSnapshot, HitmarkView, HitmarksView, ItemActionFamily, ItemContainer,
                WorldStateView,
            };
            let selected =
                api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
            let mut bag = crate::native::SettingsBag::new();
            for (key, value) in [
                ("skill", serde_json::json!("Fishing")),
                ("fishingMethod", serde_json::json!("fishing.saltfish.op3")),
                ("baitTarget", serde_json::json!(10)),
                ("disposition", serde_json::json!("Bank")),
                ("buyTools", serde_json::json!(false)),
                ("eatBelow", serde_json::json!(eat_below)),
            ] {
                bag.insert(key.into(), value);
            }
            let config = api::selected::FamilyPreparation::run(move |families| {
                crate::slot::prepare_config(
                    families,
                    crate::CompiledId("Gatherer"),
                    1,
                    Arc::new(bag),
                    selected,
                    banks,
                    None,
                )
            })
            .unwrap()
            .join()
            .unwrap()
            .unwrap();
            let prepared = Arc::clone(config.get::<Arc<Prepared>>().unwrap());
            let spot = known_rows(&prepared.catalog.methods()[prepared.methods[0]].spots)[0].origin;
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);
            snapshot.seed_local_player(local_player(spot));
            snapshot.seed_world(WorldStateView {
                map_base_x: spot.x - 50,
                map_base_z: spot.z - 50,
                level: spot.level,
                ..Default::default()
            });
            snapshot.seed_stats(vec![StatView {
                index: 10,
                name: "fishing".into(),
                base: 99,
                effective: 99,
                xp: 0,
                used: true,
            }]);
            snapshot.seed_inventory(
                vec![ItemView {
                    def: def(307, "Fishing rod"),
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: 0,
                    count: 1,
                    actions: Vec::new(),
                    component_id: 3214,
                }],
                28,
            );
            snapshot.seed_equipment(vec![]);
            snapshot.seed_locs(vec![]);
            snapshot.seed_npcs(vec![]);
            snapshot.seed_players(Vec::new());
            snapshot.seed_projectiles(Vec::new());
            snapshot.seed_hitmarks(HitmarksView {
                marks: [HitmarkView {
                    value: 0,
                    kind: 0,
                    cycle: 0,
                }; 4],
                loop_cycle: 0,
            });
            let mut session = BankMemory::default();
            session.observe(
                &[],
                64,
                ObservedAt {
                    unix_secs: 1,
                    tick: 1,
                    bank_session: 1,
                },
            );
            let bait = prepared.supply.bait.as_ref().unwrap().id;
            let mut hint = BankMemory::default();
            hint.load_hint(vec![(bait, 20)], 1).unwrap();
            Self {
                config,
                prepared,
                snapshot,
                session,
                hint,
            }
        }

        pub(super) fn gatherer(&self, retained: GatherRetained) -> Gatherer {
            Gatherer::new(
                RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                Arc::clone(&self.config),
                Arc::clone(&self.prepared),
                retained,
            )
        }

        fn assert_missing(&self, block: &AcquireBlock) {
            let name = &self.prepared.supply.bait.as_ref().unwrap().name;
            assert_eq!(block.code(), "supply-missing");
            assert!(block.message.starts_with(&format!(
                "no {name} in the pack, worn, or in the bank as seen "
            )));
            assert!(block.message.ends_with(&format!(
                "(0 rows); nobody sells it; waiting: put a {name} in the pack"
            )));
        }
    }

    #[test]
    fn acquire_tool_failures_keep_the_gatherer_codes_and_detail() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;

        let fixture = BaitFixture::new();
        for code in ["tool-missing", "tool-unusable"] {
            let mut gatherer = fixture.gatherer(GatherRetained::default());
            let mut ledger = None;
            with_tick(&fixture.snapshot, &mut ledger, 1, |tick| {
                let request =
                    gatherer.acquire_request(Arc::from([]), None, CloseBank::Close, &tick.cx);
                let run = AcquireRun::new(
                    request,
                    AcquirePlan::empty(),
                    &gatherer.prepared.banks,
                    &tick.cx,
                );
                gatherer.acquire_failure(
                    run,
                    ActionError::Failed(Arc::from(format!("{code}: observed tool detail"))),
                    tick,
                );
            });
            let failure = gatherer
                .failure
                .expect("the core failure stops this request");
            assert_eq!(failure.code.as_ref(), code);
            assert_eq!(failure.message.as_ref(), "observed tool detail");
        }
    }

    #[test]
    fn a_partial_supply_hold_keeps_the_admitted_target_when_the_due_gate_clears() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{def, with_tick_bank};
        use api::snapshot::{ItemActionFamily, ItemContainer};

        let mut fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        let mut ledger = None;
        with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.hint),
            &mut ledger,
            1,
            |tick| {
                assert!(gatherer.admit_supply(&tick.cx));
            },
        );
        let admitted = Arc::clone(gatherer.scratch.needs.as_ref().unwrap());
        let bait = fixture.prepared.supply.bait.as_ref().unwrap();
        let mut inventory = fixture.snapshot.inventory().to_vec();
        inventory.push(ItemView {
            def: def(bait.id, &bait.name),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 1,
            count: 1,
            actions: Vec::new(),
            component_id: 3214,
        });
        fixture.snapshot.seed_inventory(inventory, 28);
        gatherer.interrupt(Interrupt::Hold(true));
        assert!(gatherer.active_matches_none());
        assert!(Arc::ptr_eq(
            gatherer.scratch.needs.as_ref().unwrap(),
            &admitted,
        ));
        gatherer.interrupt(Interrupt::Hold(false));
        with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.hint),
            &mut ledger,
            2,
            |tick| {
                let snapshot = tick.cx.snapshot();
                assert!(!SupplyPlan::due(
                    &fixture.prepared,
                    snapshot.stats().unwrap().value,
                    snapshot.stock(),
                ));
                gatherer.begin_trip(tick);
            },
        );
        assert!(gatherer.failure.is_none());
        assert!(matches!(gatherer.active, Active::Acquire(_)));
        assert!(Arc::ptr_eq(
            gatherer.scratch.needs.as_ref().unwrap(),
            &admitted,
        ));
        assert_eq!(
            admitted
                .iter()
                .find(|need| need.item == bait.id)
                .unwrap()
                .target,
            10,
        );
        assert!(matches!(
            ledger.as_ref().unwrap().outbox[0].effect,
            crate::native::HostEffect::BankPick(_),
        ));
    }

    #[test]
    fn unchanged_acquire_progress_does_not_republish_status() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{with_tick, with_tick_bank};

        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        let mut ledger = None;
        with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.hint),
            &mut ledger,
            1,
            |tick| {
                assert!(gatherer.admit_supply(&tick.cx));
                gatherer.poll_active(tick);
            },
        );
        gatherer.dirty = false;
        for now in 2..4 {
            with_tick(&fixture.snapshot, &mut ledger, now, |tick| {
                gatherer.poll_active(tick);
            });
            assert!(
                !gatherer.dirty,
                "the pending bank pick has not changed stage"
            );
        }
    }

    #[test]
    fn gatherer_eat_below_floor_unchanged() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{def, with_tick};
        use api::condition::{FoodSummary, PoisonState};
        use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, StatView};

        let fixture = BaitFixture::with_eat_below(1);
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        gatherer.needs_validate = false;
        gatherer.tool = ToolState {
            id: 307,
            worn: false,
        };
        let mut snapshot = fixture.snapshot;
        let food_id = api::game_data::for_revision(api::selected::ClientRevision::R289)
            .unwrap()
            .item_by_alias("lobster")
            .unwrap()
            .id;
        snapshot.seed_stats(vec![
            StatView {
                index: 10,
                name: "fishing".into(),
                base: 99,
                effective: 99,
                xp: 0,
                used: true,
            },
            StatView {
                index: 3,
                name: "hitpoints".into(),
                base: 99,
                effective: 1,
                xp: 0,
                used: true,
            },
        ]);
        snapshot.seed_inventory(
            vec![
                ItemView {
                    def: def(307, "Fishing rod"),
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: 0,
                    count: 1,
                    actions: Vec::new(),
                    component_id: 3214,
                },
                ItemView {
                    def: def(food_id, "Lobster"),
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: 1,
                    count: 1,
                    actions: vec![Some("Eat".into())],
                    component_id: 3214,
                },
            ],
            28,
        );
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.cx.condition = Some(survival_condition(
                PoisonState::Clear,
                1,
                FoodSummary {
                    ordinary_count: 1,
                    ordinary_heal_total: 12,
                    best_heal: 12,
                    smallest_heal: 12,
                    combo_count: 0,
                    more_kinds: false,
                },
            ));
            tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
            gatherer.tick(tick).unwrap();
            assert!(matches!(
                tick.cx.ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
                Some(crate::native::HostEffect::Interaction(
                    crate::interact::InteractReq::Held { action, .. }
                )) if action == "Eat"
            ));
        });
    }

    #[test]
    fn gatherer_passes_no_step_hazard() {
        if !api::test_support::game_data_available() {
            return;
        }
        use api::condition::{FoodSummary, PoisonState};
        let fixture = BaitFixture::with_eat_below(1);
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        let mut snapshot = fixture.snapshot;
        snapshot.seed_tick(20);
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 40,
            effective: 7,
            xp: 0,
            used: true,
        }]);
        let mut condition = survival_condition(PoisonState::Clear, 7, FoodSummary::default());
        condition.hp_max = 40;
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 20, |tick| {
            tick.cx.condition = Some(condition);
            assert_eq!(gatherer.survival_eat_floor(tick), 1);
            assert!(!gatherer.poll_survival(tick));
        });
        assert_eq!(
            gatherer.survival.report().decision,
            crate::survival::Decision::None
        );
        assert!(gatherer.survival.parked().is_none());
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }

    #[test]
    fn gatherer_reports_a_need_on_an_unattributed_hit_window_and_parks_at_the_next_edge() {
        if !api::test_support::game_data_available() {
            return;
        }
        use api::condition::{FoodSummary, PoisonState};
        use api::snapshot::{HitmarkView, HitmarksView};

        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        gatherer.needs_validate = false;
        let mut snapshot = fixture.snapshot;
        snapshot.seed_tick(20);
        snapshot.seed_stats(vec![StatView {
            index: 3,
            name: "hitpoints".into(),
            base: 40,
            effective: 19,
            xp: 0,
            used: true,
        }]);
        let bank_tile = WorldTile {
            x: 3245,
            z: 3423,
            level: 0,
        };
        let _ = bank_tile;
        gatherer.start_trip(TripStep::Deposit);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 19, |tick| {
            gatherer.begin_trip(tick);
        });
        assert!(matches!(gatherer.active, Active::Acquire(_)));
        let mut marks = [HitmarkView {
            value: 0,
            kind: 0,
            cycle: 0,
        }; 4];
        marks[0] = HitmarkView {
            value: 3,
            kind: 1,
            cycle: 630,
        };
        snapshot.seed_hitmarks(HitmarksView {
            marks,
            loop_cycle: 600,
        });
        let mut condition = survival_condition(PoisonState::Clear, 19, FoodSummary::default());
        condition.hp_max = 40;
        with_tick(&snapshot, &mut ledger, 20, |tick| {
            tick.cx.condition = Some(condition);
            assert!(!gatherer.poll_survival(tick));
        });
        assert_eq!(
            gatherer.survival.report().decision,
            crate::survival::Decision::Needed
        );
        assert!(gatherer.survival.parked().is_none());
        assert!(
            matches!(gatherer.active, Active::Acquire(_)),
            "the one-window need must not cancel the live machine"
        );
        assert!(!gatherer.needs_validate);

        snapshot.seed_tick(21);
        with_tick(&snapshot, &mut ledger, 21, |tick| {
            tick.cx.condition = Some(condition);
            assert!(gatherer.poll_survival(tick));
        });
        assert!(gatherer.survival.parked().is_some());
        assert!(gatherer.active_matches_none());
        assert!(gatherer.needs_validate);
    }

    /// A supply-only bait trip with a held Strange box stack and free slots
    /// deposits nothing and must continue to Withdraw, then withdraw the bait.
    #[test]
    fn supply_only_trip_with_strange_box_continues_to_withdraw() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{def, with_tick};
        use api::snapshot::{ItemActionFamily, ItemContainer};
        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        gatherer.needs_validate = false;
        gatherer.tool = ToolState {
            id: 307,
            worn: false,
        };
        let bait = fixture.prepared.supply.bait.clone().unwrap();
        let mut snapshot = fixture.snapshot;
        let mut pack = snapshot.inventory().to_vec();
        pack.push(ItemView {
            def: def(3062, "Strange box"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 1,
            count: 28,
            actions: Vec::new(),
            component_id: 3214,
        });
        snapshot.seed_inventory(pack, 28);
        let banked_bait = ItemView {
            def: def(bait.id, &bait.name),
            container: ItemContainer::Bank,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 20,
            actions: vec![Some("Withdraw-1".into())],
            component_id: 3214,
        };
        snapshot.seed_bank_observation(1, 1, Some(vec![banked_bait]), Vec::new());
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            assert!(gatherer.admit_supply(&tick.cx));
        });
        with_tick(&snapshot, &mut ledger, 2, |tick| gatherer.begin_trip(tick));
        with_tick(&snapshot, &mut ledger, 3, |tick| gatherer.poll_active(tick));
        assert!(gatherer.failure.is_none(), "{:?}", gatherer.failure);
        let Active::Acquire(run) = &gatherer.active else {
            panic!("the supply request owns the withdrawal");
        };
        assert_eq!(run.progress().2, "Withdraw");
        assert!(gatherer
            .scratch
            .needs
            .as_ref()
            .unwrap()
            .iter()
            .any(|need| need.item == bait.id));
        with_tick(&snapshot, &mut ledger, 4, |tick| gatherer.poll_active(tick));
        assert!(matches!(
            ledger.as_ref().unwrap().outbox[0].effect,
            crate::native::HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode {
                on: false
            })
        ));
    }

    #[test]
    fn gatherer_tickfix_admitted_bank_trip_begins_select_same_tick() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;
        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        gatherer.needs_validate = false;
        let mut ledger = None;

        with_tick(&fixture.snapshot, &mut ledger, 1, |tick| {
            gatherer.tick(tick).unwrap()
        });

        assert!(matches!(
            gatherer.trip,
            TripStep::Supply {
                return_to_work: true,
                deposit_products: true
            }
        ));
        assert!(matches!(gatherer.active, Active::Acquire(_)));
        assert!(
            matches!(
                ledger.as_ref().unwrap().outbox[0].effect,
                crate::native::HostEffect::BankPick(_)
            ),
            "supply admission must begin the bank selector immediately"
        );
    }

    #[test]
    fn gatherer_bank_status_preserves_selected_kind_and_access() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::bank::{AccessKind, BankPickReceipt, BankStandAccess, PickKind, SelectedBank};
        use crate::quester::families::tests::with_tick;
        use api::quest_progress::EvidenceStamp;

        for kind in [PickKind::Reachable, PickKind::AirFallback] {
            let fixture = BaitFixture::new();
            let mut gatherer = fixture.gatherer(GatherRetained::default());
            gatherer.needs_validate = false;
            let mut ledger = None;
            with_tick(&fixture.snapshot, &mut ledger, 1, |tick| {
                gatherer.tick(tick).unwrap()
            });
            let action = ledger.as_mut().unwrap().outbox.pop().unwrap();
            assert!(matches!(
                action.effect,
                crate::native::HostEffect::BankPick(_)
            ));
            let bank = api::named_banks::NamedBank::new(
                "Test bank",
                WorldTile {
                    x: 3092,
                    z: 3243,
                    level: 0,
                },
            );
            ledger.as_mut().unwrap().complete_bank_pick(
                &action.authority(),
                BankPickReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: action.authority().run(),
                        tick: 2,
                        sequence: 2,
                    },
                    outcome: crate::native_bank::BankPickOutcome::Selected(SelectedBank {
                        bank_index: 0,
                        access_tile: bank.tile,
                        kind,
                        access: Some(Arc::new(BankStandAccess {
                            bank,
                            stand_tile: bank.tile,
                            kind: AccessKind::Booth,
                            stand_op: 1,
                            name: None,
                            choose: None,
                        })),
                    }),
                },
            );
            with_tick(&fixture.snapshot, &mut ledger, 3, |tick| {
                gatherer.tick(tick).unwrap()
            });
            assert_eq!(
                gatherer.status_data(None).bank.as_ref(),
                format!("Test bank; {kind:?}; access:3092,3243,0; Access")
            );
        }
    }

    #[test]
    fn gatherer_tickfix_deposit_receipt_begins_supply_child_same_tick() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{def, with_tick};
        use api::snapshot::{ItemActionFamily, ItemContainer};
        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        gatherer.needs_validate = false;
        gatherer.trip = TripStep::Deposit;
        let bait = fixture.prepared.supply.bait.as_ref().unwrap();
        let mut snapshot = fixture.snapshot;
        snapshot.seed_bank_observation(
            1,
            1,
            Some(vec![ItemView {
                def: def(bait.id, &bait.name),
                container: ItemContainer::Bank,
                action_family: ItemActionFamily::Component,
                slot: 0,
                count: 20,
                actions: vec![Some("Withdraw-1".into())],
                component_id: 1,
            }]),
            Vec::new(),
        );
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            gatherer.tick(tick).unwrap()
        });
        assert!(gatherer.failure.is_none(), "{:?}", gatherer.failure);
        assert!(matches!(
            gatherer.trip,
            TripStep::Supply {
                return_to_work: true,
                deposit_products: false
            }
        ));
        let Active::Acquire(run) = &gatherer.active else {
            panic!("the deposit receipt starts the second request in the same frame");
        };
        assert_eq!(run.progress().2, "Withdraw");
        assert!(
            matches!(
                ledger.as_ref().unwrap().outbox.as_slice(),
                [action]
                    if matches!(
                        action.effect,
                        crate::native::HostEffect::Interaction(
                            crate::interact::InteractReq::SetNoteMode { on: false }
                        )
                    )
            ),
            "the second request immediately starts its one withdrawal child"
        );
    }

    #[test]
    fn gatherer_tickfix_empty_withdraw_begins_close_without_withdraw() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;
        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        gatherer.trip = TripStep::Supply {
            return_to_work: true,
            deposit_products: false,
        };
        gatherer.scratch.needs = Some(Arc::from([]));
        let mut snapshot = fixture.snapshot;
        snapshot.seed_bank_observation(1, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;

        with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));

        let Active::Acquire(run) = &gatherer.active else {
            panic!("the close-only request owns the modal");
        };
        assert_eq!(run.progress().2, "Close");
        assert!(
            matches!(
                ledger.as_ref().unwrap().outbox.as_slice(),
                [action]
                    if matches!(
                        action.effect,
                        crate::native::HostEffect::Interaction(crate::interact::InteractReq::Close)
                    )
            ),
            "an empty withdrawal starts only the core's close operation"
        );
    }

    #[test]
    fn full_pack_deposits_before_the_second_request_reports_a_session_shortage() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{def, with_tick_bank};
        use api::snapshot::{ItemActionFamily, ItemContainer};
        let mut fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        let rod = fixture.snapshot.inventory()[0].clone();
        let mut pack = vec![rod.clone()];
        for slot in 1..28 {
            pack.push(ItemView {
                def: def(317, "Raw shrimps"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot,
                count: 1,
                actions: Vec::new(),
                component_id: 3214,
            });
        }
        let side = pack
            .iter()
            .map(|row| ItemView {
                container: ItemContainer::BankSide,
                action_family: ItemActionFamily::Component,
                actions: vec![Some("Deposit-All".into())],
                ..row.clone()
            })
            .collect();
        fixture.snapshot.seed_inventory(pack, 28);
        fixture.snapshot.seed_main_modal(10, Vec::new());
        fixture
            .snapshot
            .seed_bank_observation(10, 1, Some(Vec::new()), side);
        let mut ledger = None;
        with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.session),
            &mut ledger,
            1,
            |tick| gatherer.tick(tick).unwrap(),
        );
        assert!(
            gatherer.failure.is_none(),
            "the first request has no supply needs"
        );
        assert_eq!(gatherer.trip, TripStep::Deposit);
        assert!(gatherer.scratch.needs.is_none());
        with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.session),
            &mut ledger,
            2,
            |tick| gatherer.tick(tick).unwrap(),
        );
        let deposit = ledger.as_mut().unwrap().outbox.remove(0);
        assert!(matches!(
            deposit.effect,
            crate::native::HostEffect::Interaction(crate::interact::InteractReq::InvButton {
                id: 317,
                ..
            })
        ));
        assert!(
            gatherer.failure.is_none(),
            "a known shortage cannot preempt the haul"
        );
        ledger.as_mut().unwrap().complete_interaction(
            &deposit.authority(),
            crate::native::InteractionReceipt {
                request_id: deposit.request_id.get(),
                evidence: api::quest_progress::EvidenceStamp {
                    run: deposit.run(),
                    tick: 2,
                    sequence: 2,
                },
                accepted: true,
                chat_since: 0,
            },
        );
        fixture.snapshot.seed_inventory(vec![rod], 28);
        fixture
            .snapshot
            .seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.session),
            &mut ledger,
            3,
            |tick| gatherer.tick(tick).unwrap(),
        );
        fixture.assert_missing(
            gatherer
                .scratch
                .parked
                .as_ref()
                .map(|park| &park.block)
                .expect("the second request parks on the observed empty bank"),
        );
        assert_eq!(gatherer.retained.deposited, 27);
        assert_eq!(fixture.snapshot.inventory().len(), 1);
        assert_eq!(fixture.snapshot.modals().main, 10);
        assert!(fixture.snapshot.bank_loaded());
        assert!(matches!(
            gatherer.trip,
            TripStep::Supply {
                return_to_work: true,
                deposit_products: false
            }
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    /// design-bank-snapshot §2.4 at the validate boundary: a shortage the
    /// bank showed this session parks in place with no trip and no action.
    /// A Hint or an unknown bank starts one verifying trip.
    #[test]
    fn validate_admits_supply_trips_from_the_bank_memory() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick_bank;
        let fixture = BaitFixture::new();
        for (bank, in_place) in [
            (Some(&fixture.session), true),
            (Some(&fixture.hint), false),
            (None, false),
        ] {
            let mut gatherer = fixture.gatherer(GatherRetained::default());
            let mut ledger = None;
            let validation = with_tick_bank(&fixture.snapshot, bank, &mut ledger, 1, |tick| {
                gatherer.validate(&mut tick.cx)
            });
            let origin = bank.map(api::bank_memory::BankMemory::origin);
            if in_place {
                assert_eq!(validation, Validation::Pending, "{origin:?}");
                assert!(gatherer.failure.is_none());
                let park = gatherer
                    .scratch
                    .parked
                    .as_ref()
                    .expect("a Session shortage parks");
                fixture.assert_missing(&park.block);
                assert_eq!(gatherer.trip, TripStep::Idle, "no walk is armed");
            } else {
                assert_eq!(validation, Validation::Ready, "{origin:?}");
                assert!(gatherer.failure.is_none(), "{origin:?}");
                assert!(matches!(
                    gatherer.trip,
                    TripStep::Supply {
                        return_to_work: true,
                        deposit_products: true
                    }
                ));
                assert_eq!(gatherer.last_event.as_ref(), "bank trip due");
            }
            assert!(ledger.is_none(), "admission emits no action");
        }
    }

    /// N7: death recovery's reprovision step takes the same verdict.
    #[test]
    fn recovery_reprovision_takes_the_same_verdict() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick_bank;
        let fixture = BaitFixture::new();
        for (bank, in_place) in [
            (Some(&fixture.session), true),
            (Some(&fixture.hint), false),
            (None, false),
        ] {
            let mut gatherer = fixture.gatherer(GatherRetained {
                recovery: RecoveryState::Pending { step: 3 },
                ..GatherRetained::default()
            });
            let mut ledger = None;
            with_tick_bank(&fixture.snapshot, bank, &mut ledger, 1, |tick| {
                gatherer.poll_recovery(tick)
            });
            let origin = bank.map(api::bank_memory::BankMemory::origin);
            if in_place {
                assert!(gatherer.failure.is_none());
                let park = gatherer
                    .scratch
                    .parked
                    .as_ref()
                    .expect("a Session shortage parks");
                fixture.assert_missing(&park.block);
                assert_eq!(gatherer.trip, TripStep::Idle, "no reprovision walk");
            } else {
                assert!(gatherer.failure.is_none(), "{origin:?}");
                assert!(matches!(
                    gatherer.trip,
                    TripStep::Supply {
                        return_to_work: true,
                        deposit_products: true
                    }
                ));
            }
            assert_eq!(
                gatherer.retained.recovery,
                RecoveryState::Pending { step: 3 },
                "{origin:?}"
            );
            assert!(ledger.is_none(), "admission emits no action");
        }
    }

    fn draynor_net_gatherer() -> Gatherer {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let mut bag = crate::native::SettingsBag::new();
        for (key, value) in [
            ("skill", serde_json::json!("Fishing")),
            ("fishingMethod", serde_json::json!("fishing.saltfish.op1")),
            ("location", serde_json::json!("Custom")),
            (
                "customTile",
                serde_json::json!({"x": 3086, "z": 3229, "level": 0}),
            ),
            ("radius", serde_json::json!(12)),
            ("disposition", serde_json::json!("Bank")),
        ] {
            bag.insert(key.into(), value);
        }
        let config = api::selected::FamilyPreparation::run(move |families| {
            crate::slot::prepare_config(
                families,
                crate::CompiledId("Gatherer"),
                1,
                Arc::new(bag),
                selected,
                Arc::default(),
                None,
            )
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        let mut gatherer = Gatherer::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            Arc::clone(&config),
            Arc::clone(config.get::<Arc<Prepared>>().unwrap()),
            GatherRetained::default(),
        );
        gatherer.area = Some(WorkArea {
            mode: super::super::area::AreaMode::Custom,
            anchor: WorldTile {
                x: 3086,
                z: 3229,
                level: 0,
            },
            radius: 12,
        });
        gatherer.needs_validate = false;
        gatherer
    }

    fn collision_grid(
        here: WorldTile,
        radius: i32,
        mut blocked: impl FnMut(WorldTile) -> bool,
    ) -> nav_planner::collision::WorldCollision {
        let origin = WorldTile {
            x: here.x - radius,
            z: here.z - radius,
            level: 0,
        };
        let size = usize::try_from(radius * 2 + 1).expect("retreat grid");
        let mut flags = vec![0u32; 4 * size * size];
        let block = client::dash3d::CollisionFlag::SQ_BLOCKED as u32;
        for dz in 0..size {
            for dx in 0..size {
                let tile = WorldTile {
                    x: origin.x + i32::try_from(dx).expect("dx"),
                    z: origin.z + i32::try_from(dz).expect("dz"),
                    level: 0,
                };
                if blocked(tile) {
                    flags[dz * size + dx] = block;
                }
            }
        }
        let (walk, packed) = nav_planner::collision::pack_walk(&flags);
        nav_planner::collision::WorldCollision {
            origin,
            width: size,
            height: size,
            walk,
            blocked: packed,
            flags: None,
        }
    }

    fn bank_snapshot(tile: WorldTile) -> api::snapshot::GameSnapshot {
        use crate::quester::families::tests::local_player;
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(tile));
        snapshot
    }

    #[test]
    fn draynor_small_net_exclusions_wait_then_return() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;

        let mut gatherer = draynor_net_gatherer();
        gatherer.trip = TripStep::Return;
        gatherer.avoid(
            WorldTile {
                x: 3085,
                z: 3231,
                level: 0,
            },
            167,
        );
        let bank = WorldTile {
            x: 3092,
            z: 3243,
            level: 0,
        };
        let snapshot = bank_snapshot(bank);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 117, |tick| {
            gatherer.return_to_resources(tick);
        });
        assert!(
            gatherer.failure.is_none(),
            "a temporary exclusion must not block: {:?}",
            gatherer.failure
        );
        assert_eq!(gatherer.wait_until, Some(167));
        assert!(
            gatherer
                .last_event
                .contains("waiting for excluded resource until 167"),
            "{}",
            gatherer.last_event
        );
        assert!(
            gatherer.last_event.contains("fishing.saltfish.op1"),
            "{}",
            gatherer.last_event
        );
        assert!(
            gatherer.last_event.contains("planning=3092,3243,0"),
            "{}",
            gatherer.last_event
        );
        assert!(ledger.is_none(), "waiting must not arm a walk");
        assert_eq!(gatherer.trip, TripStep::Return);
        assert_eq!(gatherer.phase(), ("waiting", NativePhase::Waiting));

        with_tick(&snapshot, &mut ledger, 118, |tick| {
            let allocations = allocation_counter::measure(|| {
                gatherer.return_to_resources(tick);
            });
            assert_eq!(
                allocations.count_total, 0,
                "waiting on a known exclusion deadline must not allocate: {allocations:?}"
            );
        });
        assert!(ledger.is_none(), "mid-wait ticks must not arm a walk");
        assert_eq!(gatherer.wait_until, Some(167));

        with_tick(&snapshot, &mut ledger, 167, |tick| {
            gatherer.return_to_resources(tick);
        });
        assert!(gatherer.failure.is_none());
        assert!(gatherer.wait_until.is_none());
        assert_eq!(gatherer.last_event.as_ref(), "returning to work area");
        let action = &ledger
            .as_ref()
            .expect("return walk after the deadline")
            .outbox[0];
        let crate::native::HostEffect::Walk(request) = &action.effect else {
            panic!("expiry must arm the resource return walk");
        };
        assert_ne!(request.target, bank);
        assert_eq!(request.radius, 0);
        assert_eq!(request.arrival, ArrivalKind::Area);
    }

    #[test]
    fn death_return_waits_out_an_active_exclusion() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;

        let mut gatherer = draynor_net_gatherer();
        gatherer.retained.recovery = RecoveryState::Pending { step: 5 };
        gatherer.avoid(
            WorldTile {
                x: 3085,
                z: 3231,
                level: 0,
            },
            167,
        );
        let bank = WorldTile {
            x: 3092,
            z: 3243,
            level: 0,
        };
        let snapshot = bank_snapshot(bank);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 117, |tick| {
            gatherer.poll_recovery(tick);
        });
        assert!(
            gatherer.failure.is_none(),
            "death return must wait out an exclusion: {:?}",
            gatherer.failure
        );
        assert!(
            ledger.is_none(),
            "step 5 must not arm a walk while excluded"
        );
        assert_eq!(
            gatherer.retained.recovery,
            RecoveryState::Pending { step: 5 }
        );
        assert_eq!(gatherer.wait_until, Some(167));
        assert!(
            gatherer
                .last_event
                .contains("waiting for excluded resource until 167"),
            "{}",
            gatherer.last_event
        );
        assert_eq!(gatherer.phase(), ("recovering", NativePhase::Working));

        with_tick(&snapshot, &mut ledger, 167, |tick| {
            gatherer.poll_recovery(tick);
        });
        assert!(gatherer.failure.is_none());
        assert!(gatherer.wait_until.is_none());
        assert_eq!(gatherer.last_event.as_ref(), "returning after death");
        assert_eq!(
            gatherer.retained.recovery,
            RecoveryState::Pending { step: 5 }
        );
        let action = &ledger
            .as_ref()
            .expect("death return walk after the deadline")
            .outbox[0];
        let crate::native::HostEffect::Walk(request) = &action.effect else {
            panic!("expiry must arm the death-return walk");
        };
        assert_ne!(request.target, bank);
        assert!(matches!(gatherer.active, Active::Walk(_)));
    }

    #[test]
    fn validate_pending_names_the_missing_observation() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;

        let mut gatherer = draynor_net_gatherer();
        gatherer.needs_validate = true;
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(2);
        let mut ledger = None;
        let validation = with_tick(&snapshot, &mut ledger, 1, |tick| {
            gatherer.validate(&mut tick.cx)
        });
        assert_eq!(validation, Validation::Pending);
        assert_eq!(gatherer.last_event.as_ref(), "waiting for local player");
    }

    #[test]
    fn idle_without_world_names_the_pending_reason() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;

        let mut gatherer = draynor_net_gatherer();
        let snapshot = bank_snapshot(WorldTile {
            x: 3092,
            z: 3243,
            level: 0,
        });
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            gatherer.tick(tick).unwrap();
        });
        assert_eq!(gatherer.last_event.as_ref(), "waiting for world");
    }

    #[test]
    fn hazard_retreat_uses_a_reachable_stand_when_the_away_tile_is_blocked() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;

        let here = WorldTile {
            x: 3087,
            z: 3228,
            level: 0,
        };
        let hazard = WorldTile {
            x: 3086,
            z: 3228,
            level: 0,
        };
        let away = WorldTile {
            x: 3088,
            z: 3228,
            level: 0,
        };
        let collision = collision_grid(here, HAZARD_RETREAT_RADIUS, |tile| tile == away);
        let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
        let stand = hazard_retreat_stand(here, hazard, Some(lease.collision()))
            .expect("another reachable stand");
        assert_ne!(stand, away);
        assert_ne!(stand, here);
        assert_ne!(stand, hazard);
        assert!(collision.walkable(stand));
        assert!(
            stand.x.abs_diff(here.x).max(stand.z.abs_diff(here.z)) <= HAZARD_RETREAT_RADIUS as u32
        );

        let mut gatherer = draynor_net_gatherer();
        gatherer.hazard_escape = Some(hazard);
        let snapshot = bank_snapshot(here);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            gatherer.retreat_from_hazard(tick, Some(lease.collision()));
        });
        assert!(gatherer.failure.is_none());
        let action = &ledger.as_ref().expect("hazard retreat walk").outbox[0];
        let crate::native::HostEffect::Walk(request) = &action.effect else {
            panic!("the retreat must arm a walk");
        };
        assert_eq!(request.target, stand);
        assert_eq!(request.radius, 0);
        assert_eq!(
            request.options.allow_danger_zones,
            crate::native::WalkBit::Inherit
        );
        assert_eq!(
            request.options.allow_teleports,
            crate::native::WalkBit::Forbid
        );
        assert_eq!(gatherer.last_event.as_ref(), "walking away from hazard");
    }

    #[test]
    fn cancelled_observation_walk_does_not_poison_hazard_retreat_arrival() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;

        for cancel in [true, false] {
            let here = WorldTile {
                x: 3087,
                z: 3228,
                level: 0,
            };
            let hazard = WorldTile { x: 3086, ..here };
            let collision = collision_grid(here, HAZARD_RETREAT_RADIUS, |_| false);
            let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
            let mut gatherer = draynor_net_gatherer();
            let snapshot = bank_snapshot(here);
            let mut ledger = None;
            with_tick(&snapshot, &mut ledger, 1, |tick| {
                let goals = crate::native::WalkGoals::new(vec![here]).unwrap();
                assert!(gatherer.trip_walk(here, 0, ArrivalKind::Area, Some(goals), tick));
            });
            assert!(gatherer.scratch.observation_walk);
            if cancel {
                gatherer.cancel_active();
                assert!(
                    !gatherer.scratch.observation_walk,
                    "cancellation must retire the goal contract"
                );
            } else {
                // Also protect the retreat begin site from a stale completed
                // observation, independently of the cancellation boundary.
                gatherer.active = Active::None;
            }
            gatherer.hazard_escape = Some(hazard);
            with_tick(&snapshot, &mut ledger, 2, |tick| {
                gatherer.retreat_from_hazard(tick, Some(lease.collision()));
                assert!(
                    !gatherer.scratch.observation_walk,
                    "Reach retreat has no candidate goal receipt"
                );
                gatherer.handle_walk(
                    WalkReceipt {
                        goal: None,
                        request_id: 2,
                        evidence: tick.cx.evidence(),
                        end: WalkEnd::Arrived,
                        blocked: None,
                        detail: None,
                        refusal: None,
                        assessment: None,
                        escape: None,
                        prereq_allow: None,
                    },
                    tick,
                );
            });
            assert!(gatherer.failure.is_none());
            assert_eq!(gatherer.last_event.as_ref(), "arrived; observing resources");
        }
    }

    #[test]
    fn temporary_hazard_with_no_reachable_stand_keeps_observing() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;

        let here = WorldTile {
            x: 3087,
            z: 3228,
            level: 0,
        };
        let hazard = WorldTile {
            x: 3086,
            z: 3228,
            level: 0,
        };
        let collision = collision_grid(here, HAZARD_RETREAT_RADIUS, |tile| tile != here);
        let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
        assert!(hazard_retreat_stand(here, hazard, Some(lease.collision())).is_none());

        let mut gatherer = draynor_net_gatherer();
        gatherer.hazard_escape = Some(hazard);
        gatherer.avoid(hazard, 180);
        let snapshot = bank_snapshot(here);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 120, |tick| {
            gatherer.retreat_from_hazard(tick, Some(lease.collision()));
        });
        assert!(
            gatherer.failure.is_none(),
            "a temporary hazard must not fail: {:?}",
            gatherer.failure
        );
        assert!(ledger.is_none(), "observe, do not walk onto a blocked tile");
        assert_eq!(gatherer.hazard_escape, Some(hazard));
        assert!(
            gatherer.last_event.contains("waiting for hazard to clear"),
            "{}",
            gatherer.last_event
        );
        assert!(
            gatherer.last_event.contains("planning=3087,3228,0"),
            "{}",
            gatherer.last_event
        );

        with_tick(&snapshot, &mut ledger, 121, |tick| {
            let allocations = allocation_counter::measure(|| {
                gatherer.retreat_from_hazard(tick, Some(lease.collision()));
            });
            assert_eq!(
                allocations.count_total, 0,
                "waiting on a known hazard deadline must not allocate: {allocations:?}"
            );
        });
        assert!(ledger.is_none(), "mid-wait ticks must not arm a walk");
        assert_eq!(gatherer.hazard_escape, Some(hazard));
    }

    #[test]
    fn hazard_retreat_does_not_pick_a_diagonal_only_cage() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;

        let here = WorldTile {
            x: 3087,
            z: 3228,
            level: 0,
        };
        let hazard = WorldTile {
            x: 3086,
            z: 3228,
            level: 0,
        };
        let collision = collision_grid(here, HAZARD_RETREAT_RADIUS, |tile| {
            let dx = (tile.x - here.x).abs();
            let dz = (tile.z - here.z).abs();
            (dx == 1 && dz == 0) || (dx == 0 && dz == 1)
        });
        let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
        assert_eq!(
            nav_planner::router::local_step_component(&collision, here, HAZARD_RETREAT_RADIUS)
                .len(),
            1,
            "the router cannot leave a four-neighbour cage"
        );
        assert!(
            hazard_retreat_stand(here, hazard, Some(lease.collision())).is_none(),
            "a diagonal-only stand is not a router step"
        );

        let mut gatherer = draynor_net_gatherer();
        gatherer.hazard_escape = Some(hazard);
        gatherer.avoid(hazard, 180);
        let snapshot = bank_snapshot(here);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 120, |tick| {
            gatherer.retreat_from_hazard(tick, Some(lease.collision()));
        });
        assert!(
            gatherer.failure.is_none(),
            "a temporary cage must keep observing: {:?}",
            gatherer.failure
        );
        assert!(
            ledger.is_none(),
            "do not walk a diagonal the router refuses"
        );
        assert_eq!(gatherer.hazard_escape, Some(hazard));
        assert!(
            gatherer.last_event.contains("waiting for hazard to clear"),
            "{}",
            gatherer.last_event
        );
    }

    #[test]
    fn hazard_retreat_ignores_walkable_tiles_in_another_component() {
        let here = WorldTile {
            x: 3087,
            z: 3228,
            level: 0,
        };
        let hazard = WorldTile {
            x: 3086,
            z: 3228,
            level: 0,
        };
        let island = WorldTile {
            x: 3089,
            z: 3228,
            level: 0,
        };
        let north = WorldTile {
            x: 3087,
            z: 3229,
            level: 0,
        };
        let south = WorldTile {
            x: 3087,
            z: 3227,
            level: 0,
        };
        // Wall at x=3088 with only diagonal gaps at here.z±1. Here's north and
        // south tiles are blocked so the router cannot step through those gaps;
        // an 8-way walkable flood still squeezes through and would pick the
        // island, which is also the nearest away tile under walkable-only ranking.
        let collision = collision_grid(here, HAZARD_RETREAT_RADIUS, |tile| {
            (tile.x == 3088 && tile.z != north.z && tile.z != south.z)
                || tile == north
                || tile == south
        });
        let lease = crate::gatherer::select::tests::test_nav_lease_for_collision(&collision);
        assert!(collision.walkable(island));
        assert!(!collision.walkable(WorldTile {
            x: 3088,
            z: 3228,
            level: 0,
        }));
        assert_eq!(
            nav_planner::paint::flood_component_id(&collision, &[here], island),
            None
        );
        let stand =
            hazard_retreat_stand(here, hazard, Some(lease.collision())).expect("a reachable stand");
        assert_ne!(stand, island);
        assert_eq!(
            nav_planner::paint::flood_component_id(&collision, &[here], stand),
            Some(0)
        );
    }

    #[test]
    fn hazard_retreat_uses_a_reachable_draynor_stand_on_the_real_pack() {
        let Some(_pack) = api::test_support::local_asset_path("NAV_PACK", "", "real 289 nav pack")
        else {
            return;
        };
        use crate::quester::families::tests::with_tick;

        let here = WorldTile {
            x: 3087,
            z: 3228,
            level: 0,
        };
        let hazard = WorldTile {
            x: 3086,
            z: 3228,
            level: 0,
        };
        let blocked_away = WorldTile {
            x: 3088,
            z: 3228,
            level: 0,
        };
        let blocked_north = WorldTile {
            x: 3088,
            z: 3231,
            level: 0,
        };
        let world = crate::gatherer::select::test_real_nav_world();
        let lease = crate::gatherer::select::tests::test_real_nav_lease();
        assert!(
            !world.collision.walkable(blocked_away),
            "S10 away tile must stay blocked on the reference pack"
        );
        assert!(!world.collision.walkable(blocked_north));
        let stand = hazard_retreat_stand(here, hazard, Some(lease.collision()))
            .expect("Draynor S10 must have a reachable stand");
        assert_ne!(stand, blocked_away);
        assert_ne!(stand, blocked_north);
        assert_ne!(stand, here);
        assert_ne!(stand, hazard);
        assert!(world.collision.walkable(stand));
        assert_eq!(
            nav_planner::paint::flood_component_id(&world.collision, &[here], stand),
            Some(0)
        );
        assert!(
            stand.x.abs_diff(here.x).max(stand.z.abs_diff(here.z)) <= HAZARD_RETREAT_RADIUS as u32
        );

        let mut gatherer = draynor_net_gatherer();
        gatherer.hazard_escape = Some(hazard);
        let snapshot = bank_snapshot(here);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            gatherer.retreat_from_hazard(tick, Some(lease.collision()));
        });
        assert!(gatherer.failure.is_none());
        let action = &ledger.as_ref().expect("hazard retreat walk").outbox[0];
        let crate::native::HostEffect::Walk(request) = &action.effect else {
            panic!("the retreat must arm a walk");
        };
        assert_eq!(request.target, stand);
        assert!(request.food_guard);
        assert!(request.allow.food);
        assert_eq!(
            request.options.allow_danger_zones,
            crate::native::WalkBit::Inherit
        );
        assert_eq!(
            request.options.allow_teleports,
            crate::native::WalkBit::Forbid
        );
    }

    #[test]
    fn parked_gatherer_publishes_waiting_continue_status_and_resumes_parked() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::native::{Interrupt, NativeOutput, NativePhase, ScriptFlow, ScriptStatus};
        use crate::quester::families::tests::with_tick_output;
        use api::condition::{FoodSummary, PoisonState};

        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
            fn log(&mut self, _: api::hostlog::Level, _: &str) {}
            fn settings_applied(&mut self, _: u64) {}
        }

        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        let snapshot = fixture.snapshot;
        let mut ledger = None;
        let mut output = Capture::default();
        let condition = survival_condition(
            PoisonState::Onset {
                since: 20,
                q: Some(11),
                hit_since_line: false,
            },
            5,
            FoodSummary::default(),
        );

        with_tick_output(&snapshot, &mut ledger, 1, &mut output, |tick| {
            tick.cx.condition = Some(condition);
            tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
            assert!(matches!(gatherer.tick(tick).unwrap(), ScriptFlow::Continue));
        });
        assert_eq!(gatherer.phase(), ("parked", NativePhase::Waiting));
        assert!(gatherer.waits_for_evidence());
        assert!(gatherer.survival.parked().is_some());
        let status = output.0.last().expect("parked status is published");
        assert_eq!(status.phase, NativePhase::Waiting);
        assert!(status.failure.is_none());
        assert_eq!(gatherer.status_data(None).phase, "parked");

        gatherer.interrupt(Interrupt::Pause);
        assert_eq!(gatherer.phase(), ("paused", NativePhase::Waiting));
        gatherer.interrupt(Interrupt::Resume);
        assert_eq!(gatherer.phase(), ("parked", NativePhase::Waiting));
        assert!(gatherer.survival.parked().is_some());

        with_tick_output(&snapshot, &mut ledger, 2, &mut output, |tick| {
            tick.cx.condition = Some(condition);
            tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
            assert!(matches!(gatherer.tick(tick).unwrap(), ScriptFlow::Continue));
        });
        assert_eq!(gatherer.phase(), ("parked", NativePhase::Waiting));
        assert!(gatherer.waits_for_evidence());
        let status = output
            .0
            .last()
            .expect("continued parked status is published");
        assert_eq!(status.phase, NativePhase::Waiting);
        assert!(status.failure.is_none());
        assert_eq!(gatherer.status_data(None).phase, "parked");
    }

    #[test]
    fn status_row_shows_needed_under_never() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::native::{NativeOutput, ScriptFlow, ScriptStatus};
        use crate::quester::families::tests::with_tick_output;
        use api::condition::PoisonState;

        #[derive(Default)]
        struct Capture(Vec<ScriptStatus>);
        impl NativeOutput for Capture {
            fn status(&mut self, status: ScriptStatus) {
                self.0.push(status);
            }
            fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
            fn log(&mut self, _: api::hostlog::Level, _: &str) {}
            fn settings_applied(&mut self, _: u64) {}
        }

        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        let snapshot = fixture.snapshot;
        let mut ledger = None;
        let mut output = Capture::default();
        let condition = survival_condition(
            PoisonState::Onset {
                since: 20,
                q: Some(11),
                hit_since_line: false,
            },
            5,
            api::condition::FoodSummary::default(),
        );

        with_tick_output(&snapshot, &mut ledger, 1, &mut output, |tick| {
            tick.cx.condition = Some(condition);
            tick.cx.cure_poison = crate::survival::CurePoison::Never;
            assert!(matches!(gatherer.tick(tick).unwrap(), ScriptFlow::Continue));
        });

        let status = output.0.last().expect("status is published");
        assert_eq!(
            status.survival.to_string(),
            "Needed: poisoned: would die; needs antipoison or 1 food"
        );
        assert!(gatherer.survival.parked().is_none());
    }
    #[test]
    fn runner_tick_poisoned_frame_allocates_nothing() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};

        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        gatherer.needs_validate = false;
        let snapshot = fixture.snapshot;
        let mut ledger = None;
        let condition = |tick: u64| BotCondition {
            tick: u32::try_from(tick).unwrap(),
            session: SessionMark::Live,
            hp: 99,
            hp_max: 99,
            hp_known: true,
            poison: PoisonState::Poisoned {
                per_tick: 1,
                last_mark: 1,
                hit_since_mark: false,
            },
            poison_window: None,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        };

        for tick in 1..=8 {
            with_tick(&snapshot, &mut ledger, tick, |native| {
                native.cx.condition = Some(condition(tick));
                native.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
                gatherer.tick(native).unwrap();
            });
        }
        let allocations = allocation_counter::measure(|| {
            for tick in 9..=1008 {
                with_tick(&snapshot, &mut ledger, tick, |native| {
                    native.cx.condition = Some(condition(tick));
                    native.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
                    gatherer.tick(native).unwrap();
                });
            }
        })
        .count_total;
        assert_eq!(allocations, 0, "poisoned runner ticks allocated");
        assert!(gatherer.survival.parked().is_none());
        assert_eq!(
            gatherer.survival.report().decision,
            crate::survival::Decision::None
        );
    }

    #[test]
    fn runner_tick_parked_frame_allocates_nothing() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::native::{NativePhase, ScriptFlow};
        use crate::quester::families::tests::with_tick;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};

        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        gatherer.needs_validate = false;
        let snapshot = fixture.snapshot;
        let mut ledger = None;
        let condition = |tick: u64| BotCondition {
            tick: u32::try_from(tick).unwrap(),
            session: SessionMark::Live,
            hp: 5,
            hp_max: 99,
            hp_known: true,
            poison: PoisonState::Onset {
                since: 1,
                q: Some(11),
                hit_since_line: false,
            },
            poison_window: None,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        };

        with_tick(&snapshot, &mut ledger, 1, |native| {
            native.cx.condition = Some(condition(1));
            native.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
            assert!(matches!(
                gatherer.tick(native).unwrap(),
                ScriptFlow::Continue
            ));
        });
        assert_eq!(gatherer.phase(), ("parked", NativePhase::Waiting));
        assert!(gatherer.waits_for_evidence());
        let allocations = allocation_counter::measure(|| {
            for tick in 2..=1001 {
                with_tick(&snapshot, &mut ledger, tick, |native| {
                    native.cx.condition = Some(condition(tick));
                    native.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
                    assert!(matches!(
                        gatherer.tick(native).unwrap(),
                        ScriptFlow::Continue
                    ));
                });
            }
        })
        .count_total;
        assert_eq!(allocations, 0, "parked runner ticks allocated");
        assert_eq!(gatherer.phase(), ("parked", NativePhase::Waiting));
        assert!(gatherer.waits_for_evidence());
        assert!(gatherer.survival.parked().is_some());
    }

    #[test]
    fn parked_runner_unparks_at_next_exact_fire_without_rescue() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::with_tick;
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};
        use api::snapshot::StatView;

        let fixture = BaitFixture::new();
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        let mut snapshot = fixture.snapshot;
        snapshot.seed_inventory(
            vec![ItemView {
                def: crate::quester::families::tests::def(307, "Fishing rod"),
                container: api::snapshot::ItemContainer::Inventory,
                action_family: api::snapshot::ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 3214,
            }],
            28,
        );
        let condition = |poison, hp, tick| BotCondition {
            tick,
            session: SessionMark::Live,
            hp,
            hp_max: 90,
            hp_known: true,
            poison,
            poison_window: None,
            food: FoodSummary::default(),
            cures: CureDoses::default(),
        };
        let set_hp = |snapshot: &mut api::snapshot::GameSnapshot, hp| {
            snapshot.seed_stats(vec![StatView {
                index: 3,
                name: "hitpoints".into(),
                base: 90,
                effective: hp,
                xp: 0,
                used: true,
            }]);
        };
        let mut ledger = None;

        set_hp(&mut snapshot, 90);
        with_tick(&snapshot, &mut ledger, 20, |tick| {
            tick.cx.condition = Some(condition(
                PoisonState::Onset {
                    since: 20,
                    q: Some(11),
                    hit_since_line: false,
                },
                90,
                20,
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        set_hp(&mut snapshot, 84);
        with_tick(&snapshot, &mut ledger, 50, |tick| {
            tick.cx.condition = Some(condition(
                PoisonState::Poisoned {
                    per_tick: 6,
                    last_mark: 50,
                    hit_since_mark: false,
                },
                84,
                50,
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        set_hp(&mut snapshot, 78);
        with_tick(&snapshot, &mut ledger, 80, |tick| {
            tick.cx.condition = Some(condition(
                PoisonState::Poisoned {
                    per_tick: 6,
                    last_mark: 80,
                    hit_since_mark: false,
                },
                78,
                80,
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        set_hp(&mut snapshot, 73);
        with_tick(&snapshot, &mut ledger, 110, |tick| {
            tick.cx.condition = Some(condition(
                PoisonState::Poisoned {
                    per_tick: 5,
                    last_mark: 110,
                    hit_since_mark: false,
                },
                73,
                110,
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_none());
        assert_eq!(
            gatherer.survival.report().decision,
            crate::survival::Decision::Needed
        );
        assert!(gatherer.survival.report().need.is_none());
        assert!(!gatherer.survival_deferred);
        // The un-park resumes as `Interrupt::Resume` does (design A6.5 item 2;
        // Argon review finding 10): the park did not count against the
        // fishing no-yield window (800 ticks, `WAIT_GAMEPLAY_TICKS`) or the
        // idle gameplay baseline.
        assert_eq!(gatherer.scratch.last_yield_tick, 110);
        assert!(gatherer.rebaseline_gameplay || gatherer.last_gameplay_tick == 110);
    }

    #[test]
    fn parked_runner_with_food_unparks_at_fire_eight() {
        if !api::test_support::game_data_available() {
            return;
        }
        use crate::quester::families::tests::{def, with_tick};
        use api::condition::{BotCondition, CureDoses, FoodSummary, PoisonState, SessionMark};
        use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, StatView};

        fn seed(snapshot: &mut GameSnapshot, lobster_id: i32, hp: u8, food_count: u8) {
            snapshot.seed_stats(vec![StatView {
                index: 3,
                name: "hitpoints".into(),
                base: 10,
                effective: i32::from(hp),
                xp: 0,
                used: true,
            }]);
            let mut inventory = vec![ItemView {
                def: def(307, "Fishing rod"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 3214,
            }];
            if food_count > 0 {
                inventory.push(ItemView {
                    def: def(lobster_id, "Lobster"),
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot: 1,
                    count: i32::from(food_count),
                    actions: vec![Some("Eat".into())],
                    component_id: 3214,
                });
            }
            snapshot.seed_inventory(inventory, 28);
        }
        fn condition(poison: PoisonState, hp: u8, food_count: u8) -> BotCondition {
            BotCondition {
                tick: 20,
                session: SessionMark::Live,
                hp,
                hp_max: 10,
                hp_known: true,
                poison,
                poison_window: None,
                food: FoodSummary {
                    ordinary_count: food_count,
                    ordinary_heal_total: u16::from(food_count) * 12,
                    best_heal: 12,
                    smallest_heal: 12,
                    combo_count: 0,
                    more_kinds: false,
                },
                cures: CureDoses::default(),
            }
        }

        let fixture = BaitFixture::with_eat_below(1);
        let mut gatherer = fixture.gatherer(GatherRetained::default());
        let mut snapshot = fixture.snapshot;
        let lobster_id = api::game_data::for_revision(api::selected::ClientRevision::R289)
            .unwrap()
            .item_by_alias("lobster")
            .unwrap()
            .id;
        let mut ledger = None;
        let onset = PoisonState::Onset {
            since: 20,
            q: Some(11),
            hit_since_line: false,
        };
        seed(&mut snapshot, lobster_id, 10, 9);
        with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.cx.condition = Some(condition(onset, 10, 9));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        let eat = |tick: &mut NativeTick<'_>, hp, count, per_tick, last_mark| {
            tick.cx.condition = Some(condition(
                PoisonState::Poisoned {
                    per_tick,
                    last_mark,
                    hit_since_mark: false,
                },
                hp,
                count,
            ));
            tick.cx.poison_unknown_pulse = 11;
            tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        };
        let expected_eat = |tick: &mut NativeTick<'_>| {
            assert!(matches!(
                tick.cx.ledger.as_ref().unwrap().outbox.last().map(|action| &action.effect),
                Some(crate::native::HostEffect::Interaction(
                    crate::interact::InteractReq::Held { action, .. }
                )) if action == "Eat"
            ));
            accept_last_interaction(tick);
        };

        seed(&mut snapshot, lobster_id, 5, 9);
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            eat(tick, 5, 9, 5, 50);
            gatherer.tick(tick).unwrap();
            expected_eat(tick);
        });
        assert!(gatherer.survival.parked().is_some());
        seed(&mut snapshot, lobster_id, 10, 8);
        with_tick(&snapshot, &mut ledger, 3, |tick| {
            eat(tick, 10, 8, 5, 50);
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        seed(&mut snapshot, lobster_id, 5, 8);
        with_tick(&snapshot, &mut ledger, 4, |tick| {
            eat(tick, 5, 8, 5, 80);
            gatherer.tick(tick).unwrap();
            expected_eat(tick);
        });
        seed(&mut snapshot, lobster_id, 10, 7);
        with_tick(&snapshot, &mut ledger, 5, |tick| {
            eat(tick, 10, 7, 5, 80);
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        seed(&mut snapshot, lobster_id, 6, 7);
        with_tick(&snapshot, &mut ledger, 6, |tick| {
            eat(tick, 6, 7, 4, 110);
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        seed(&mut snapshot, lobster_id, 2, 7);
        with_tick(&snapshot, &mut ledger, 7, |tick| {
            eat(tick, 2, 7, 4, 140);
            gatherer.tick(tick).unwrap();
            expected_eat(tick);
        });
        seed(&mut snapshot, lobster_id, 10, 6);
        with_tick(&snapshot, &mut ledger, 8, |tick| {
            eat(tick, 10, 6, 4, 140);
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        seed(&mut snapshot, lobster_id, 6, 6);
        with_tick(&snapshot, &mut ledger, 9, |tick| {
            eat(tick, 6, 6, 4, 170);
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        seed(&mut snapshot, lobster_id, 2, 6);
        with_tick(&snapshot, &mut ledger, 10, |tick| {
            eat(tick, 2, 6, 4, 200);
            gatherer.tick(tick).unwrap();
            expected_eat(tick);
        });
        seed(&mut snapshot, lobster_id, 10, 5);
        with_tick(&snapshot, &mut ledger, 11, |tick| {
            eat(tick, 10, 5, 4, 200);
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        seed(&mut snapshot, lobster_id, 6, 5);
        with_tick(&snapshot, &mut ledger, 12, |tick| {
            eat(tick, 6, 5, 4, 230);
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_some());

        seed(&mut snapshot, lobster_id, 3, 5);
        with_tick(&snapshot, &mut ledger, 13, |tick| {
            eat(tick, 3, 5, 3, 260);
            gatherer.tick(tick).unwrap();
            expected_eat(tick);
        });
        assert!(gatherer.survival.parked().is_some());

        seed(&mut snapshot, lobster_id, 10, 4);
        with_tick(&snapshot, &mut ledger, 14, |tick| {
            eat(tick, 10, 4, 3, 260);
            gatherer.tick(tick).unwrap();
        });
        assert!(gatherer.survival.parked().is_none());
        assert_eq!(
            snapshot
                .inventory()
                .iter()
                .find(|row| row.def.id == lobster_id)
                .unwrap()
                .count,
            4
        );
    }
}
