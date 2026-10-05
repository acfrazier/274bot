use super::area::{AreaError, WorkArea};
use super::card::Prepared;
use super::drop::{DropBatch, DropBatchArgs, DropEnd, DropResult};
use super::gather::{GatherEnd, GatherResult, GatherRun, GatherRunArgs, DEFAULT_STALL_TICKS};
use super::oneop::{OneOp, OneOpArgs};
use super::select::{
    resource_return_target, select, AvoidedTile, FishingSurvey, PlacementClass, ReturnObservation,
    SelectedTarget, Selection, TargetPlan, RESOURCE_APPROACH_RADIUS,
};
use super::settings::{has_members_requirement, method_level, GathererSettings, Skill};
use super::status::{self, StatusData};
use super::supply::{self, SupplyPlan, SupplyPlanResult};
use super::widen::{
    remember_group, site_anchor, SearchExclusions, SearchResult, TriedGroup, WidenCursor,
};
use super::{GatherRetained, RecoveryState};
use crate::bank::{self, PickKind, SelectedBank};
use crate::native::walk::Walk;
use crate::native::{
    ActionError, ActionHandle, ConfigError, Interrupt, NativePhase, NativeTick, PreparedConfig,
    Script, ScriptFailure, ScriptFlow, SettingsApply, StopReason, WalkBit, WalkEnd, WalkOptions,
    WalkReceipt, WalkRequest,
};
use api::gather_methods::known_rows;
use api::selected::{RunKey, Truth};
use api::snapshot::{ItemView, StatView, WorldTile};
use nav::arrival::ArrivalKind;
use std::sync::Arc;
use std::task::Poll;

#[path = "recover.rs"]
mod recover;

const MAX_AVOID: usize = 8;
const WAIT_GAMEPLAY_TICKS: u64 = 800; // Eight minutes at 600 ms per server tick.
const IDLE_AVOID_TICKS: u64 = 60;

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
    Walk(ActionHandle<Walk>),
    Tend(ActionHandle<OneOp>),
    Select(ActionHandle<bank::Select>),
    Open(ActionHandle<bank::Open>),
    Deposit(ActionHandle<bank::Deposit>),
    Withdraw(ActionHandle<bank::Withdraw>),
    Close(ActionHandle<bank::Close>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Validation {
    Ready,
    Pending,
    Wear(ToolState),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum TripStep {
    #[default]
    Idle,
    Select,
    Access,
    Open,
    Deposit,
    Withdraw,
    Close,
    Validate,
    Return,
}

fn walk_failure_message(receipt: &WalkReceipt) -> String {
    match receipt
        .detail
        .as_deref()
        .filter(|detail| !detail.is_empty())
    {
        Some(detail) => format!("walk ended with {:?}: {detail}", receipt.end),
        None => format!("walk ended with {:?}", receipt.end),
    }
}

/// Cold session collections share one allocation to retain the inline budget.
#[derive(Default)]
struct GatherScratch {
    fishing: FishingSurvey,
    withdrawals: Option<Arc<[bank::Withdrawal]>>,
}

pub struct Gatherer {
    run: RunKey,
    config: Arc<PreparedConfig>,
    prepared: Arc<Prepared>,
    pending: Option<Arc<PreparedConfig>>,
    area: Option<WorkArea>,
    target: Option<TargetPlan>,
    active: Active,
    tool: ToolState,
    avoid: [AvoidedTile; MAX_AVOID],
    scratch: Box<GatherScratch>,
    wait_until: Option<u64>,
    widen: WidenCursor,
    tried_groups: [TriedGroup; 4],
    hazard_escape: Option<WorldTile>,
    trip: TripStep,
    selected_bank: Option<SelectedBank>,
    supply_missing: Option<Arc<str>>,
    bank_label: Arc<str>,
    trips: u16,
    last_gameplay_tick: u64,
    last_paint_tick: u64,
    fence: TickPacketFence,
    death: crate::native::death::DeathLatch,
    respawn_deadline: Option<std::time::Duration>,
    respawn_settle: Option<u64>,
    proving_runs: u8,
    recovery_reprovisions: u8,
    proof_evidence: u8,
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
            selected_bank: None,
            supply_missing: None,
            bank_label: Arc::from("—"),
            trips: 0,
            last_gameplay_tick: 0,
            last_paint_tick: 0,
            fence: TickPacketFence::default(),
            death,
            respawn_deadline: None,
            respawn_settle: None,
            proving_runs: 0,
            recovery_reprovisions: 0,
            proof_evidence: 0,
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
        if let Some(selected) = &self.selected_bank {
            if let Some(bank) = self
                .prepared
                .banks
                .banks()
                .get(usize::from(selected.bank_index))
            {
                self.bank_label = Arc::from(format!(
                    "{}; {:?}; access:{},{},{}; {:?}",
                    bank.name,
                    selected.kind,
                    selected.access_tile.x,
                    selected.access_tile.z,
                    selected.access_tile.level,
                    step
                ));
            }
        }
        self.dirty = true;
    }

    fn start_trip(&mut self) {
        self.target = None;
        self.wait_until = None;
        self.selected_bank = None;
        self.scratch.withdrawals = None;
        self.supply_missing = None;
        self.advance_trip(TripStep::Select);
        self.set_event("bank trip due");
    }

    fn start_dispose(&mut self, tick: &mut NativeTick<'_>) {
        if self.settings().disposition.eq_ignore_ascii_case("Bank") {
            self.start_trip();
        } else {
            self.start_drop(tick);
        }
    }

    fn trip_walk(
        &mut self,
        target: WorldTile,
        radius: u16,
        arrival: ArrivalKind,
        tick: &mut NativeTick<'_>,
    ) {
        let request = WalkRequest {
            target,
            loc_id: None,
            radius,
            arrival,
            options: WalkOptions::default(),
            required_after: tick.cx.evidence(),
            evidence: None,
            cross: Box::default(),
            protect: false,
            allow: Default::default(),
        };
        match tick.actions.begin::<Walk>(request, &mut tick.cx) {
            Ok(handle) => self.active = Active::Walk(handle),
            Err(error) => self.action_failure(error),
        }
    }

    fn return_to_resources(&mut self, tick: &mut NativeTick<'_>) {
        let Some(area) = self.area else {
            self.fail("return-failed", "the resource work area is unavailable");
            return;
        };
        let snapshot = tick.cx.snapshot();
        let Some(here) = snapshot.here() else {
            return;
        };
        let Some(target) = resource_return_target(
            &self.prepared.catalog,
            &self.prepared.methods,
            self.settings(),
            area,
            ReturnObservation {
                here: here.value,
                now: tick.cx.evidence().tick,
                skill_stat: self.skill_stat(snapshot.stats()),
                avoided: &self.avoid,
            },
        ) else {
            self.fail(
                "return-failed",
                "no usable resource observation stand in the work area",
            );
            return;
        };
        let stand = target.tile;
        self.method = Arc::clone(&target.alias);
        self.target = Some(target);
        self.trip_walk(stand, RESOURCE_APPROACH_RADIUS, ArrivalKind::Area, tick);
        self.dirty = true;
    }

    fn resource_return_arrived(&self, tick: &NativeTick<'_>) -> bool {
        self.target.as_ref().is_some_and(|target| {
            tick.cx.snapshot().here().is_some_and(|here| {
                here.value.level == target.tile.level
                    && here
                        .value
                        .x
                        .abs_diff(target.tile.x)
                        .max(here.value.z.abs_diff(target.tile.z))
                        <= u32::from(RESOURCE_APPROACH_RADIUS)
            })
        })
    }

    fn begin_trip(&mut self, tick: &mut NativeTick<'_>) {
        if matches!(self.trip, TripStep::Deposit | TripStep::Withdraw)
            && tick
                .cx
                .snapshot()
                .bank_session()
                .is_some_and(|session| !session.value.open)
        {
            // A hold may dismiss the modal without consuming the trip step.
            self.advance_trip(TripStep::Open);
            return;
        }
        match self.trip {
            TripStep::Idle => {}
            TripStep::Select => {
                let Some(here) = tick.cx.snapshot().here() else {
                    return;
                };
                let args = bank::SelectArgs {
                    facts: Arc::clone(&self.prepared.banks),
                    from: here.value,
                    preferences: self.settings().bank_preferences(),
                    options: WalkOptions::default(),
                    explicit: (!self.settings().bank.eq_ignore_ascii_case("Nearest"))
                        .then(|| Arc::from(self.settings().bank.as_str())),
                };
                match tick.actions.begin::<bank::Select>(args, &mut tick.cx) {
                    Ok(handle) => self.active = Active::Select(handle),
                    Err(error) => self.action_failure(error),
                }
            }
            TripStep::Access => {
                if let Some(selected) = &self.selected_bank {
                    self.trip_walk(selected.access_tile, 1, ArrivalKind::Reach, tick);
                }
            }
            TripStep::Open => {
                let Some(selected) = &self.selected_bank else {
                    return;
                };
                let Some(access) = &selected.access else {
                    self.fail(
                        "bank-unavailable",
                        format!(
                            "bank-unavailable:{}: no packed stand or declared NPC access",
                            self.prepared
                                .banks
                                .banks()
                                .get(usize::from(selected.bank_index))
                                .map_or(self.settings().bank.as_str(), |bank| bank.name)
                        ),
                    );
                    return;
                };
                let args = bank::OpenArgs {
                    access: Arc::clone(access),
                };
                match tick.actions.begin::<bank::Open>(args, &mut tick.cx) {
                    Ok(handle) => self.active = Active::Open(handle),
                    Err(error) => self.action_failure(error),
                }
            }
            TripStep::Deposit => {
                let snapshot = tick.cx.snapshot();
                let Some(inventory) = snapshot.inventory() else {
                    return;
                };
                let args = bank::DepositArgs {
                    products: supply::bank_deposit_ids(
                        &self.prepared,
                        self.tool.id,
                        inventory.value,
                    ),
                    keep: Arc::from(supply::protected_ids(&self.prepared, self.tool.id).as_slice()),
                };
                match tick.actions.begin::<bank::Deposit>(args, &mut tick.cx) {
                    Ok(handle) => self.active = Active::Deposit(handle),
                    Err(error) => self.action_failure(error),
                }
            }
            TripStep::Withdraw => {
                if self.scratch.withdrawals.is_none() {
                    let snapshot = tick.cx.snapshot();
                    let (Some(stats), Some(inventory), Some(equipment)) =
                        (snapshot.stats(), snapshot.inventory(), snapshot.equipment())
                    else {
                        return;
                    };
                    match SupplyPlan::from_loaded_bank(
                        &self.prepared,
                        stats.value,
                        inventory.value,
                        equipment.value,
                        snapshot.bank().map(|bank| bank.value),
                    ) {
                        SupplyPlanResult::Pending => return,
                        SupplyPlanResult::Missing(item) => {
                            self.fail("supply-missing", format!("supply-missing:{item}"));
                            return;
                        }
                        SupplyPlanResult::Ready(plan) => {
                            self.supply_missing = plan.missing().cloned();
                            self.scratch.withdrawals = Some(plan.to_withdrawals());
                        }
                    }
                }
                let args = bank::WithdrawArgs {
                    withdrawals: Arc::clone(
                        self.scratch
                            .withdrawals
                            .as_ref()
                            .expect("latched supply plan"),
                    ),
                };
                match tick.actions.begin::<bank::Withdraw>(args, &mut tick.cx) {
                    Ok(handle) => self.active = Active::Withdraw(handle),
                    Err(error) => self.action_failure(error),
                }
            }
            TripStep::Close => match tick.actions.begin::<bank::Close>((), &mut tick.cx) {
                Ok(handle) => self.active = Active::Close(handle),
                Err(error) => self.action_failure(error),
            },
            TripStep::Validate => {
                // V-resume derives the observed tool and, if possible, waits
                // for its equipment settlement before this return boundary.
                if !self.needs_validate {
                    self.advance_trip(TripStep::Return);
                    self.set_event("returning to work area");
                }
            }
            TripStep::Return => self.return_to_resources(tick),
        }
    }

    fn fail(&mut self, code: &'static str, message: impl Into<Arc<str>>) {
        self.failure = Some(ScriptFailure {
            code: Arc::from(code),
            message: message.into(),
        });
        self.dirty = true;
    }

    fn cancel_active(&mut self) {
        self.active = Active::None;
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
        if self.scratch.withdrawals.is_some() {
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
        if changed_bank && self.trip != TripStep::Select && !next_disposal {
            return None;
        }
        if current.disposition != next.settings.disposition
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
        self.dirty = true;
        self.set_event("settings applied");
        Some(revision)
    }

    fn validate(&mut self, cx: &mut crate::native::ActionContext<'_>) -> Validation {
        let snapshot = cx.snapshot();
        let Some(_local) = snapshot.local_player() else {
            return Validation::Pending;
        };
        let Some(world) = snapshot.world() else {
            return Validation::Pending;
        };
        let Some(here) = snapshot.here() else {
            return Validation::Pending;
        };
        let Some(stats) = snapshot.stats() else {
            return Validation::Pending;
        };
        let Some(inventory) = snapshot.inventory() else {
            return Validation::Pending;
        };
        let Some(_capacity) = snapshot.inventory_capacity() else {
            return Validation::Pending;
        };
        let Some(equipment) = snapshot.equipment() else {
            return Validation::Pending;
        };
        let Some(_locs) = snapshot.locs() else {
            return Validation::Pending;
        };
        let skill = self.settings().skill_kind();
        if snapshot.npcs().is_none() {
            return Validation::Pending;
        }
        let skill_stat = stats
            .value
            .iter()
            .find(|stat| stat.name.eq_ignore_ascii_case(skill.stat_name()));
        let Some(skill_stat) = skill_stat else {
            return Validation::Pending;
        };
        for &index in self.prepared.methods.iter() {
            let Some(method) = self.prepared.catalog.methods().get(index) else {
                continue;
            };
            if skill_stat.base < method_level(method, skill_stat.index) {
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
            Err(AreaError::NotReady) => return Validation::Pending,
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
        if SupplyPlan::due(
            &self.prepared,
            stats.value,
            inventory.value,
            equipment.value,
        ) {
            self.start_trip();
            return Validation::Ready;
        }
        if let Some(tool) = tool {
            if skill != Skill::Fishing
                && !tool.worn
                && self.can_wield(tool, inventory.value, stats.value)
            {
                return Validation::Wear(tool);
            }
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

    fn can_wield(&self, tool: ToolState, inventory: &[ItemView], stats: &[StatView]) -> bool {
        !tool.worn && supply::can_wield(&self.prepared, tool.id, inventory, stats)
    }

    fn begin_wear(&mut self, tick: &mut NativeTick<'_>) {
        let snapshot = tick.cx.snapshot();
        let Some(inventory) = snapshot.inventory() else {
            return;
        };
        let Some(row) = inventory
            .value
            .iter()
            .find(|row| row.def.id == self.tool.id)
        else {
            self.fail("tool-missing", "the selected tool left the inventory");
            return;
        };
        let Some(name) = row.def.name.as_deref() else {
            self.fail("tool-missing", "the selected tool has no observed name");
            return;
        };
        let Some(action) = row.actions.iter().flatten().find(|action| {
            action.eq_ignore_ascii_case("Wield") || action.eq_ignore_ascii_case("Wear")
        }) else {
            self.fail(
                "tool-unusable",
                "the selected tool has no observed equip action",
            );
            return;
        };
        if !self.fence.reserve(1) {
            return;
        }
        match tick.actions.begin::<OneOp>(
            OneOpArgs::wear(Arc::from(name), self.tool.id, action),
            &mut tick.cx,
        ) {
            Ok(handle) => {
                self.active = Active::Tend(handle);
                self.set_event("wielding tool");
            }
            Err(error) => self.action_failure(error),
        }
    }

    fn action_failure(&mut self, error: ActionError) {
        if matches!(error, ActionError::Held) {
            return;
        }
        self.fail("action-error", format!("native action failed: {error:?}"));
    }

    fn start_drop(&mut self, tick: &mut NativeTick<'_>) {
        let snapshot = tick.cx.snapshot();
        let Some(inventory) = snapshot.inventory() else {
            return;
        };
        let products = Arc::clone(&self.prepared.products);
        let mut protected = [0; 8];
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
        self.method = Arc::clone(&selected.plan.alias);
        self.target = Some(selected.plan.clone());
        // Observed NPC ops own their client-side approach. Only their unloaded
        // observation stands use Area; locs keep loc-aware Reach settlement
        // as soon as their live footprint becomes available.
        let observation_approach = selected.class == PlacementClass::Unloaded
            && matches!(selected.plan.entity, api::selected::EntityId::Npc(_));
        let loc_id = match selected.plan.entity {
            api::selected::EntityId::Loc(id) => Some(id),
            _ => None,
        };
        let needs_walk = if selected.class == PlacementClass::Unloaded {
            true
        } else if selected.plan.npc_index >= 0 {
            false
        } else {
            let snapshot = tick.cx.snapshot();
            !snapshot.here().is_some_and(|here| match loc_id {
                Some(id) => snapshot.walk_loc_arrived(here.value, selected.plan.tile, 1, id),
                None => snapshot.walk_arrived(here.value, selected.plan.tile, 1),
            })
        };
        if needs_walk {
            let request = WalkRequest {
                target: selected.plan.tile,
                loc_id,
                radius: RESOURCE_APPROACH_RADIUS,
                arrival: if observation_approach {
                    ArrivalKind::Area
                } else {
                    ArrivalKind::Reach
                },
                options: WalkOptions::default(),
                required_after: tick.cx.evidence(),
                evidence: None,
                cross: Vec::new().into_boxed_slice(),
                protect: false,
                allow: Default::default(),
            };
            match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                Ok(handle) => {
                    self.active = Active::Walk(handle);
                    self.set_event("approaching target");
                }
                Err(error) => self.action_failure(error),
            }
            return;
        }
        let Some(plan) = self.target.clone() else {
            return;
        };
        match tick.actions.begin::<GatherRun>(
            GatherRunArgs {
                target: plan,
                stall_ticks: DEFAULT_STALL_TICKS,
                catalog: Arc::clone(&self.prepared.catalog),
                quest_owned: false,
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
        self.needs_validate = true;
        if result.gained > 0 || result.xp > 0 {
            self.last_gameplay_tick = tick.cx.evidence().tick;
            self.wait_until = None;
        }
        let target = self.target.take();
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
            DropEnd::Cleared
                if tick.cx.snapshot().inventory().is_some_and(|inventory| {
                    tick.cx
                        .snapshot()
                        .inventory_capacity()
                        .is_some_and(|capacity| {
                            inventory.value.len() >= usize::from(capacity.value)
                        })
                }) =>
            {
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
        if self.retained.recovery == (RecoveryState::Pending { step: 5 }) {
            self.finish_recovery_return(result, tick);
            return;
        }
        if result.end == WalkEnd::UserInput {
            self.target = None;
            self.fail("manual-movement", "cancelled by user input");
            return;
        }
        if result.end != WalkEnd::Arrived {
            self.target = None;
            self.fail(
                if self.trip == TripStep::Return {
                    "return-failed"
                } else if self.trip == TripStep::Access {
                    "bank-unavailable"
                } else {
                    "walk-failed"
                },
                walk_failure_message(&result),
            );
            return;
        }
        match self.trip {
            TripStep::Access => {
                if self.selected_bank.is_some() {
                    self.advance_trip(TripStep::Open);
                }
                return;
            }
            TripStep::Return => {
                let arrived = self.resource_return_arrived(tick);
                if !arrived {
                    self.fail("return-failed", "return receipt did not establish arrival at the resource observation stand");
                    return;
                }
                self.trips = self.trips.saturating_add(1);
                self.advance_trip(TripStep::Idle);
                self.needs_validate = true;
            }
            _ => {}
        }
        self.hazard_escape = None;
        // A walk receipt is not resource evidence: the target may have
        // depleted while travelling. Select again from the arrival frame.
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
                    if !matches!(result.end, GatherEnd::Depleted | GatherEnd::TargetGone) {
                        self.fence.seal();
                    }
                    self.handle_gather(result, tick);
                }
                Poll::Ready(Err(error)) => {
                    self.fence.seal();
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
                        self.fence.seal();
                        self.handle_drop(result, tick);
                    }
                    Poll::Ready(Err(error)) => {
                        self.fence.seal();
                        self.action_failure(error);
                    }
                }
            }
            Active::Walk(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Walk(handle),
                Poll::Ready(Ok(result)) => {
                    // Keep bank/recovery settlement fenced. A resource approach
                    // can reselect immediately from its fresh arrival frame.
                    if result.end != WalkEnd::Arrived
                        || self.trip != TripStep::Idle
                        || matches!(self.retained.recovery, RecoveryState::Pending { .. })
                    {
                        self.fence.seal();
                    }
                    self.handle_walk(result, tick);
                }
                Poll::Ready(Err(error)) => {
                    self.fence.seal();
                    if self.retained.recovery == (RecoveryState::Pending { step: 5 }) {
                        self.fail("return-failed", format!("return-failed:{error:?}"));
                    } else {
                        self.action_failure(error);
                    }
                }
            },
            Active::Tend(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Tend(handle),
                Poll::Ready(Ok(settled)) => {
                    self.fence.seal();
                    if settled {
                        self.needs_validate = true;
                        self.set_event("one operation settled");
                    } else {
                        self.fail("oneop-failed", "one operation did not settle");
                    }
                }
                Poll::Ready(Err(error)) => {
                    self.fence.seal();
                    self.action_failure(error);
                }
            },
            Active::Select(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Select(handle),
                Poll::Ready(Ok(selected)) => {
                    self.fence.seal();
                    if selected.kind == PickKind::NoCandidate || selected.bank_index == u16::MAX {
                        self.bank_label =
                            Arc::from(format!("{}; NoCandidate", self.settings().bank));
                        self.fail(
                            "bank-unavailable",
                            format!("bank-unavailable:{}", self.settings().bank),
                        );
                    } else {
                        self.selected_bank = Some(selected);
                        self.advance_trip(TripStep::Access);
                        self.set_event("bank selected");
                    }
                }
                Poll::Ready(Err(error)) => {
                    self.fence.seal();
                    self.fail(
                        "bank-unavailable",
                        format!("bank selection failed: {error:?}"),
                    );
                }
            },
            Active::Open(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Open(handle),
                Poll::Ready(Ok(())) => {
                    self.fence.seal();
                    self.advance_trip(TripStep::Deposit);
                }
                Poll::Ready(Err(error)) => {
                    self.fence.seal();
                    self.fail("bank-unavailable", format!("bank open failed: {error:?}"));
                }
            },
            Active::Deposit(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Deposit(handle),
                Poll::Ready(Ok(deposited)) => {
                    self.fence.seal();
                    self.retained.deposited = self.retained.deposited.saturating_add(deposited);
                    if deposited > 0 && self.retained.recovery == RecoveryState::Idle {
                        self.retained.haul_since_death = true;
                    }
                    self.sync_retained(tick);
                    self.advance_trip(TripStep::Withdraw);
                    self.set_event("deposit confirmed");
                }
                Poll::Ready(Err(error)) => {
                    self.fence.seal();
                    self.fail(
                        "bank-deposit-failed",
                        format!("bank deposit failed: {error:?}"),
                    );
                }
            },
            Active::Withdraw(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Withdraw(handle),
                Poll::Ready(Ok(true)) => {
                    self.fence.seal();
                    self.scratch.withdrawals = None;
                    if let Some(item) = self.supply_missing.take() {
                        self.fail("supply-missing", format!("supply-missing:{item}"));
                        return;
                    }
                    self.advance_trip(TripStep::Close);
                    self.set_event("withdrawal confirmed");
                }
                Poll::Ready(Ok(false)) => {
                    self.fence.seal();
                    self.fail("bank-withdraw-failed", "supply withdrawal was incomplete");
                }
                Poll::Ready(Err(error)) => {
                    self.fence.seal();
                    self.fail(
                        "bank-withdraw-failed",
                        format!("supply withdrawal failed: {error:?}"),
                    );
                }
            },
            Active::Close(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Close(handle),
                Poll::Ready(Ok(_)) => {
                    self.fence.seal();
                    if self.retained.recovery == (RecoveryState::Pending { step: 3 }) {
                        self.advance_trip(TripStep::Idle);
                        self.advance_recovery(RecoveryState::Pending { step: 4 }, tick);
                    } else {
                        self.advance_trip(TripStep::Validate);
                    }
                    self.needs_validate = true;
                    self.set_event("bank closed; validating equipment");
                }
                Poll::Ready(Err(error)) => {
                    self.fence.seal();
                    self.action_failure(error);
                }
            },
        }
    }

    fn begin_stray_modal(&mut self, tick: &mut NativeTick<'_>) -> bool {
        let snapshot = tick.cx.snapshot();
        let args = if snapshot
            .chat_continue()
            .is_some_and(|button| button.value >= 0)
        {
            OneOpArgs::continue_dialog()
        } else if snapshot
            .main_modal()
            .is_some_and(|modal| modal.value.root >= 0)
        {
            OneOpArgs::close_modal()
        } else {
            return false;
        };
        match tick.actions.begin::<OneOp>(args, &mut tick.cx) {
            Ok(handle) => self.active = Active::Tend(handle),
            Err(error) => self.action_failure(error),
        }
        true
    }

    fn begin_eat(&mut self, tick: &mut NativeTick<'_>) -> bool {
        let Some(food) = &self.prepared.supply.food else {
            return false;
        };
        let snapshot = tick.cx.snapshot();
        let (Some(stats), Some(inventory)) = (snapshot.stats(), snapshot.inventory()) else {
            return false;
        };
        let Some(hp) = stats.value.iter().find(|stat| stat.index == 3) else {
            return false;
        };
        let below = if self.settings().eat_below == 0 {
            hp.base.saturating_add(1) / 2
        } else {
            self.settings().eat_below
        };
        let held = inventory
            .value
            .iter()
            .filter(|row| row.def.id == food.id)
            .map(|row| row.count)
            .sum::<i32>();
        if hp.effective > below || held == 0 || !self.fence.reserve(1) {
            return false;
        }
        let args = OneOpArgs::eat(&food.name, food.id, held, hp.effective);
        match tick.actions.begin::<OneOp>(args, &mut tick.cx) {
            Ok(handle) => {
                self.active = Active::Tend(handle);
                self.set_event("eating at boundary");
            }
            Err(error) => self.action_failure(error),
        }
        true
    }

    fn begin_idle(&mut self, tick: &mut NativeTick<'_>, handoff: bool) {
        if self.fence.sealed {
            return;
        }
        if self.needs_validate {
            match self.validate(&mut tick.cx) {
                Validation::Pending => return,
                Validation::Wear(tool) => {
                    self.tool = tool;
                    self.begin_wear(tick);
                    return;
                }
                Validation::Ready => self.needs_validate = false,
            }
        }
        if self.begin_eat(tick) {
            return;
        }
        if self.trip != TripStep::Idle {
            self.begin_trip(tick);
            return;
        }
        let snapshot = tick.cx.snapshot();
        if let (Some(stats), Some(inventory), Some(equipment)) =
            (snapshot.stats(), snapshot.inventory(), snapshot.equipment())
        {
            if SupplyPlan::due(
                &self.prepared,
                stats.value,
                inventory.value,
                equipment.value,
            ) {
                self.start_trip();
                return;
            }
        }
        let Some(area) = self.area else {
            self.needs_validate = true;
            return;
        };
        let snapshot = tick.cx.snapshot();
        if snapshot
            .inventory()
            .zip(snapshot.inventory_capacity())
            .is_some_and(|(rows, capacity)| rows.value.len() >= usize::from(capacity.value))
        {
            self.start_dispose(tick);
            return;
        }
        if self.begin_stray_modal(tick) {
            return;
        }
        let (Some(world), Some(locs), Some(here)) =
            (snapshot.world(), snapshot.locs(), snapshot.here())
        else {
            self.needs_validate = true;
            return;
        };
        let Some(npcs) = snapshot.npcs() else {
            return;
        };
        let skill_stat = self.skill_stat(snapshot.stats());
        let selected = select(
            &self.prepared.catalog,
            &self.prepared.methods,
            &self.prepared.settings,
            area,
            &self.avoid,
            super::select::SelectionObservation {
                world: world.value,
                locs: locs.value,
                npcs: npcs.value,
                here: here.value,
                now: tick.cx.evidence().tick,
                skill_stat,
                fishing: &mut self.scratch.fishing,
            },
        );
        self.zone_gated = selected.zone_gated;
        // Completion can precede its inventory/XP packet. Only an available
        // target warrants a same-frame handoff; confirm terminal unavailability
        // on the next observation so the last yield is accounted before blocking.
        if handoff && selected.target.is_none() {
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
                Selection::Absent { absent } => {
                    self.absent = absent;
                    self.fail(
                        "resource-unavailable",
                        format!("resource-unavailable: absent: {absent}"),
                    );
                }
                Selection::Exhausted { wait_until, absent } => {
                    self.absent = absent;
                    let bounded = self.last_gameplay_tick.saturating_add(WAIT_GAMEPLAY_TICKS);
                    let deadline = *self.wait_until.get_or_insert(wait_until.min(bounded));
                    if tick.cx.evidence().tick >= deadline {
                        if self.settings().location.eq_ignore_ascii_case("auto") {
                            self.begin_widen(wait_until, tick);
                        } else {
                            self.fail("resource-unavailable", "resource-unavailable");
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
        let Some(here) = tick.cx.snapshot().here().map(|row| row.value) else {
            return;
        };
        let Some(hazard) = self.hazard_escape else {
            return;
        };
        let mut dx = here.x.saturating_sub(hazard.x).signum();
        let dz = here.z.saturating_sub(hazard.z).signum();
        if dx == 0 && dz == 0 {
            dx = 1;
        }
        let request = WalkRequest {
            target: WorldTile {
                x: here.x.saturating_add(dx),
                z: here.z.saturating_add(dz),
                level: here.level,
            },
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
            protect: false,
            allow: Default::default(),
        };
        match tick.actions.begin::<Walk>(request, &mut tick.cx) {
            Ok(handle) => {
                self.active = Active::Walk(handle);
                self.set_event("walking away from hazard");
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
        if matches!(self.retained.recovery, RecoveryState::Pending { .. }) {
            return ("recovering", NativePhase::Working);
        }
        if self.trip != TripStep::Idle && !matches!(self.active, Active::Walk(_)) {
            return ("banking", NativePhase::Working);
        }
        match self.active {
            Active::None => {
                if self.wait_until.is_some() {
                    ("waiting", NativePhase::Waiting)
                } else {
                    ("gathering", NativePhase::Working)
                }
            }
            Active::Gather(_) => ("gathering", NativePhase::Working),
            Active::Drop(_) => ("dropping", NativePhase::Working),
            Active::Walk(_) => ("walking", NativePhase::Working),
            Active::Tend(_) => ("tending", NativePhase::Working),
            Active::Select(_)
            | Active::Open(_)
            | Active::Deposit(_)
            | Active::Withdraw(_)
            | Active::Close(_) => ("banking", NativePhase::Working),
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
            bank: Arc::clone(&self.bank_label),
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
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let (_, native_phase) = self.phase();
        let data = self.status_data(tick.cx.snapshot().inventory().map(|rows| rows.value));
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
        self.fence.observe(tick.cx.evidence().tick);
        if self.run != tick.cx.run() {
            self.run = tick.cx.run();
            self.observed_progress = None;
            self.cancel_active();
            self.needs_validate = true;
            self.rebaseline_gameplay = true;
            self.death = crate::native::death::DeathLatch::from_watermark(self.retained.death_seq);
            self.set_event("run key changed; revalidating");
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
        if self.failure.is_some() {
            self.publish(tick);
            return Ok(ScriptFlow::Blocked(self.failure.clone().unwrap()));
        }
        let recoveries = self.retained.recoveries;
        self.observe_progress(tick);
        if matches!(self.retained.recovery, RecoveryState::Pending { .. }) {
            self.poll_recovery(tick);
            self.publish(tick);
            return Ok(match &self.failure {
                Some(failure) => ScriptFlow::Blocked(failure.clone()),
                None => ScriptFlow::Continue,
            });
        }
        let handoff = !self.active_matches_none();
        if handoff {
            self.poll_active(tick);
        }
        if self.active_matches_none() && !self.fence.sealed && self.failure.is_none() {
            let disposal_due = tick
                .cx
                .snapshot()
                .inventory()
                .zip(tick.cx.snapshot().inventory_capacity())
                .is_some_and(|(rows, capacity)| rows.value.len() >= usize::from(capacity.value));
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
        if self.prepared.settings.boundary_changed(&prepared.settings) {
            self.pending = Some(next);
            self.dirty = true;
            return Ok(SettingsApply::PendingBoundary);
        }
        self.prepared = Arc::clone(prepared);
        self.config = next;
        self.pending = None;
        self.needs_validate = true;
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
            Interrupt::Resume | Interrupt::Hold(false) | Interrupt::SessionReady => {
                self.paused = false;
                self.needs_validate = true;
                self.rebaseline_gameplay = true;
                self.dirty = true;
            }
        }
    }

    fn on_stop(&mut self, _reason: StopReason) {
        self.cancel_active();
        self.pending = None;
    }

    fn recovery_anchor(&self) -> Option<WorldTile> {
        (self.trip == TripStep::Idle && self.retained.recovery == RecoveryState::Idle)
            .then_some(self.retained.anchor)
            .flatten()
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
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn packet_fence_blocks_same_tick_reentry_and_resets_on_observation() {
        let mut fence = TickPacketFence::default();
        fence.observe(7);
        assert!(fence.reserve(2));
        fence.seal();
        assert!(!fence.reserve(1));
        fence.observe(8);
        assert!(fence.reserve(5));
        assert!(!fence.reserve(1));
    }

    #[test]
    fn instance_with_widening_stays_inside_the_inline_budget() {
        assert!(size_of::<Gatherer>() <= 1024);
        assert!(size_of::<GatherRetained>() <= 80);
        println!(
            "Gatherer={} Active={} GatherRun={} DropBatch={} OneOp={} GatherRetained={}",
            size_of::<Gatherer>(),
            size_of::<Active>(),
            size_of::<GatherRun>(),
            size_of::<DropBatch>(),
            size_of::<OneOp>(),
            size_of::<GatherRetained>()
        );
    }
    #[test]
    fn bank_inside_auto_radius_returns_to_resource_observation_stand() {
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
        gatherer.trip = TripStep::Return;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        }));
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));
        assert_eq!(gatherer.trip, TripStep::Return);
        assert_eq!(gatherer.trips, 0);
        let action = &ledger.as_ref().unwrap().outbox[0];
        let crate::native::HostEffect::Walk(request) = &action.effect else {
            panic!("the resource return must arm a walk");
        };
        assert_ne!(request.target, gatherer.area.unwrap().anchor);
        assert_eq!(
            request.radius, 1,
            "r40 bounds gathering, not resource observation"
        );
        assert_eq!(request.arrival, ArrivalKind::Area);
        assert!(gatherer
            .status_data(None)
            .target
            .contains("observation stand:"));
        with_tick(&snapshot, &mut ledger, 2, |tick| {
            gatherer.handle_walk(
                WalkReceipt {
                    request_id: 1,
                    evidence: tick.cx.evidence(),
                    end: WalkEnd::Arrived,
                    blocked: None,
                    detail: None,
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
    }

    #[test]
    fn bank_return_failure_preserves_walk_receipt_detail() {
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
                    request_id: 7,
                    evidence: tick.cx.evidence(),
                    end: WalkEnd::Failed,
                    blocked: None,
                    detail: Some(Arc::from(
                        "Dropped: stuck at (2809,3441,0), aiming (2848,3426,0)",
                    )),
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
                    request_id: 7,
                    evidence: api::quest_progress::EvidenceStamp {
                        run,
                        tick: 1,
                        sequence: 1,
                    },
                    end: WalkEnd::UserInput,
                    blocked: None,
                    detail: None,
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
}
