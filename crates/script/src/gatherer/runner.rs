use super::area::{AreaError, WorkArea};
use super::card::Prepared;
use super::drop::{DropBatch, DropBatchArgs, DropEnd, DropResult};
use super::gather::{GatherEnd, GatherResult, GatherRun, GatherRunArgs, DEFAULT_STALL_TICKS};
use super::oneop::{OneOp, OneOpArgs};
use super::select::{select, AvoidedTile, PlacementClass, SelectedTarget, Selection, TargetPlan};
use super::settings::{has_members_requirement, method_level, GathererSettings, Skill};
use super::status::{self, StatusData};
use super::widen::{remember_group, SearchExclusions, SearchResult, TriedGroup, WidenCursor};
use super::{GatherRetained, RecoveryState};
use crate::native::walk::Walk;
use crate::native::{
    ActionError, ActionHandle, ConfigError, Interrupt, NativePhase, NativeTick, PreparedConfig,
    Script, ScriptFailure, ScriptFlow, SettingsApply, StopReason, WalkEnd, WalkReceipt,
    WalkRequest,
};
use crate::FindOptions;
use api::gather_methods::{known_rows, ToolUse};
use api::selected::{Knowledge, RunKey, Truth};
use api::snapshot::{ItemView, StatView, WorldTile};
use std::sync::Arc;
use std::task::Poll;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Validation {
    Ready,
    Pending,
    Wear(ToolState),
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
    wait_until: Option<u64>,
    widen: WidenCursor,
    tried_groups: [TriedGroup; 4],
    hazard_escape: Option<WorldTile>,
    last_gameplay_tick: u64,
    last_paint_tick: u64,
    fence: TickPacketFence,
    death: crate::quester::death::DeathLatch,
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
        let death = crate::quester::death::DeathLatch::from_watermark(retained.death_seq);
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
            wait_until: None,
            widen: WidenCursor::default(),
            tried_groups: [TriedGroup::default(); 4],
            hazard_escape: None,
            last_gameplay_tick: 0,
            last_paint_tick: 0,
            fence: TickPacketFence::default(),
            death,
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

    fn fail(&mut self, code: &'static str, message: impl Into<Arc<str>>, retryable: bool) {
        self.failure = Some(ScriptFailure {
            code: Arc::from(code),
            message: message.into(),
            retryable,
        });
        self.dirty = true;
    }

    fn clear_failure(&mut self) {
        if self.failure.take().is_some() {
            self.dirty = true;
        }
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

    fn apply_pending(&mut self) -> Option<u64> {
        let config = self.pending.take()?;
        let Some(prepared) = config.get::<Arc<Prepared>>() else {
            self.fail(
                "config-identity",
                "pending Gatherer settings were malformed",
                false,
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
                self.fail(
                    "level-too-low",
                    format!("level-too-low:{}", method.id.0),
                    false,
                );
                return Validation::Pending;
            }
            if !world.value.members && has_members_requirement(method) {
                self.fail(
                    "members-world",
                    "selected method requires a members world",
                    false,
                );
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
                        true,
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
                self.fail("area-invalid", "the configured work area is invalid", false);
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
                false,
            );
            return Validation::Pending;
        }

        for &index in self.prepared.methods.iter() {
            for consume in known_rows(&self.prepared.catalog.methods()[index].consumes) {
                if !inventory
                    .value
                    .iter()
                    .any(|row| row.def.id == consume.item && row.count > 0)
                {
                    let name = self
                        .prepared
                        .catalog
                        .alias(api::selected::EntityId::Obj(consume.item))
                        .unwrap_or("bait");
                    self.fail("supply-missing", format!("supply-missing:{name}"), true);
                    return Validation::Pending;
                }
            }
        }
        let Some(tool) = self.derive_tool(skill, skill_stat, inventory.value, equipment.value)
        else {
            self.fail(
                "tool-missing",
                "no usable selected gathering tool is held",
                true,
            );
            return Validation::Pending;
        };
        self.tool = tool;
        self.area = Some(area);
        self.dirty = true;
        if skill != Skill::Fishing
            && !tool.worn
            && self.can_wield(tool, inventory.value, stats.value)
        {
            return Validation::Wear(tool);
        }
        Validation::Ready
    }

    fn sync_retained_from_context(&mut self, cx: &mut crate::native::ActionContext<'_>) {
        *cx.retained().gather() = self.retained;
    }

    fn derive_tool(
        &self,
        skill: Skill,
        skill_stat: &StatView,
        inventory: &[ItemView],
        equipment: &[ItemView],
    ) -> Option<ToolState> {
        let mut best: Option<(ToolState, i32)> = None;
        for &index in self.prepared.methods.iter() {
            let Some(method) = self.prepared.catalog.methods().get(index) else {
                continue;
            };
            let Knowledge::Known(tools) = &method.tools else {
                continue;
            };
            for tool in tools.iter() {
                if !tool_gate_met(tool, skill, skill_stat) {
                    continue;
                }
                let worn = equipment.iter().any(|row| row.def.id == tool.item);
                let held = inventory.iter().any(|row| row.def.id == tool.item);
                if !worn && !held {
                    continue;
                }
                let level = tool.use_gate.map_or(0, |gate| i32::from(gate.level));
                let state = ToolState {
                    id: tool.item,
                    worn,
                };
                if best.as_ref().is_none_or(|(_, current)| level > *current) {
                    best = Some((state, level));
                }
            }
        }
        best.map(|(tool, _)| tool)
    }

    fn can_wield(&self, tool: ToolState, inventory: &[ItemView], stats: &[StatView]) -> bool {
        if tool.worn || !inventory.iter().any(|row| row.def.id == tool.id) {
            return false;
        }
        self.prepared.methods.iter().any(|&index| {
            known_rows(&self.prepared.catalog.methods()[index].tools)
                .iter()
                .any(|candidate| {
                    candidate.item == tool.id
                        && candidate.wield_gate.is_none_or(|gate| {
                            stats.iter().any(|stat| {
                                stat.index == i32::from(gate.skill)
                                    && stat.base >= i32::from(gate.level)
                            })
                        })
                })
        })
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
            self.fail("tool-missing", "the selected tool left the inventory", true);
            return;
        };
        let Some(name) = row.def.name.as_deref() else {
            self.fail(
                "tool-missing",
                "the selected tool has no observed name",
                true,
            );
            return;
        };
        let Some(action) = row.actions.iter().flatten().find(|action| {
            action.eq_ignore_ascii_case("Wield") || action.eq_ignore_ascii_case("Wear")
        }) else {
            self.fail(
                "tool-unusable",
                "the selected tool has no observed equip action",
                false,
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
        let retryable = !matches!(error, ActionError::Stale | ActionError::Cancelled);
        self.fail(
            "action-error",
            format!("native action failed: {error:?}"),
            retryable,
        );
    }

    fn start_drop(&mut self, tick: &mut NativeTick<'_>) {
        let snapshot = tick.cx.snapshot();
        let Some(inventory) = snapshot.inventory() else {
            return;
        };
        let products = Arc::clone(&self.prepared.products);
        let mut protected = [0; 8];
        let mut protected_len = if self.tool.id >= 0 {
            protected[0] = self.tool.id;
            1
        } else {
            0
        };
        for &index in self.prepared.methods.iter() {
            for consume in known_rows(&self.prepared.catalog.methods()[index].consumes) {
                if !protected[..protected_len].contains(&consume.item)
                    && protected_len < protected.len()
                {
                    protected[protected_len] = consume.item;
                    protected_len += 1;
                }
            }
        }
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
                true,
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
        // Observed NPC ops own their client-side approach. Their occupied water
        // tile is not a navigation destination (and can move before arrival).
        if selected.class == PlacementClass::Unloaded
            || (selected.plan.npc_index < 0
                && !adjacent(
                    tick.cx.snapshot().here().map(|here| here.value),
                    selected.plan.tile,
                ))
        {
            let request = WalkRequest {
                target: selected.plan.tile,
                radius: 1,
                options: FindOptions {
                    allow_teleports: self.settings().allow_teleports,
                    allow_wilderness: self.settings().allow_wilderness,
                    allow_bank_fetch: false,
                },
                required_after: tick.cx.evidence(),
                evidence: None,
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
        if let (Some((products, xp, tile)), Some((old_products, old_xp, old_tile))) =
            (current, previous)
        {
            let gained = products.saturating_sub(old_products);
            let xp = xp.saturating_sub(old_xp).max(0);
            if tile != old_tile || xp != 0 {
                self.last_gameplay_tick = tick.cx.evidence().tick;
            }
            if gained != 0 || xp != 0 {
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
        if result.gained > 0 || result.xp > 0 {
            self.last_gameplay_tick = tick.cx.evidence().tick;
            self.wait_until = None;
        }
        let target = self.target.take();
        match result.end {
            GatherEnd::Full => {
                self.set_event("inventory full");
                self.start_drop(tick);
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
                    true,
                );
            }
            DropEnd::Cleared => self.set_event("drop confirmed by empty slots"),
            DropEnd::Blocked { remaining } => {
                self.fail(
                    "inventory-blocked",
                    format!("inventory-blocked: {remaining} product slots remain"),
                    true,
                );
            }
        }
        self.dirty = true;
    }

    fn handle_walk(&mut self, result: WalkReceipt) {
        if result.end != WalkEnd::Arrived {
            self.target = None;
            self.fail(
                "walk-failed",
                format!("walk ended with {:?}", result.end),
                true,
            );
            return;
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
                    self.fence.seal();
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
                    self.fence.seal();
                    self.handle_walk(result);
                }
                Poll::Ready(Err(error)) => {
                    self.fence.seal();
                    self.action_failure(error);
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
                        self.fail("oneop-failed", "one operation did not settle", true);
                    }
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

    fn begin_idle(&mut self, tick: &mut NativeTick<'_>) {
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
            self.start_drop(tick);
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
        let selected = select(
            &self.prepared.catalog,
            &self.prepared.methods,
            self.settings(),
            area,
            &self.avoid,
            super::select::SelectionObservation {
                world: world.value,
                locs: locs.value,
                npcs: npcs.value,
                here: here.value,
                now: tick.cx.evidence().tick,
                skill_stat: self.skill_stat(snapshot.stats()),
            },
        );
        self.zone_gated = selected.zone_gated;
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
                        true,
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
                            self.fail("resource-unavailable", "resource-unavailable", true);
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
                true,
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
            radius: 0,
            options: FindOptions {
                allow_teleports: false,
                allow_wilderness: self.settings().allow_wilderness,
                allow_bank_fetch: false,
            },
            required_after: tick.cx.evidence(),
            evidence: None,
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
        if let Some(entry) = self.avoid.iter_mut().find(|entry| entry.tile == tile) {
            entry.until = until;
            return;
        }
        if let Some(entry) = self
            .avoid
            .iter_mut()
            .find(|entry| entry.until <= self.last_gameplay_tick)
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
        }
    }

    fn status_data(&self) -> StatusData {
        let (phase, _) = self.phase();
        StatusData {
            skill: self.settings().skill_kind().name(),
            method: Arc::clone(&self.method),
            phase,
            area: Arc::<str>::from(self.area_label()),
            target: self
                .target
                .as_ref()
                .map_or_else(|| Arc::from("—"), |target| Arc::clone(&target.alias)),
            tool: if self.tool.id >= 0 {
                Arc::from(format!(
                    "{}{}",
                    self.tool.id,
                    if self.tool.worn { " (worn)" } else { "" }
                ))
            } else {
                Arc::from("—")
            },
            bait: -1,
            food: -1,
            coins: -1,
            yielded: self.yielded,
            dropped: self.dropped,
            deposited: self.retained.deposited,
            trips: 0,
            xp: self.xp,
            xp_per_hour: None,
            bank: Arc::from("—"),
            last_progress: self.last_gameplay_tick,
            deaths: self.retained.deaths,
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
        let data = self.status_data();
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
            self.death = crate::quester::death::DeathLatch::from_watermark(self.retained.death_seq);
            self.set_event("run key changed; revalidating");
        }
        if !tick.cx.eligible || self.paused {
            self.publish(tick);
            return Ok(ScriptFlow::Continue);
        }
        if self.observe_death(tick) {
            self.cancel_active();
            self.retained.deaths = self.retained.deaths.saturating_add(1);
            self.retained.recovery = RecoveryState::Pending { step: 1 };
            self.sync_retained(tick);
            self.fail("died", "died", false);
            self.set_event("death observed");
        }
        if self.failure.is_some() {
            self.publish(tick);
            return Ok(ScriptFlow::Blocked(self.failure.clone().unwrap()));
        }
        self.observe_progress(tick);
        if self.active_matches_none() {
            if let Some(revision) = self.apply_pending() {
                tick.output.settings_applied(revision);
            }
        }
        if self.active_matches_none() && self.needs_validate {
            match self.validate(&mut tick.cx) {
                Validation::Pending => {
                    self.publish(tick);
                    return Ok(ScriptFlow::Continue);
                }
                Validation::Wear(tool) => {
                    self.tool = tool;
                    self.begin_wear(tick);
                    self.publish(tick);
                    return Ok(ScriptFlow::Continue);
                }
                Validation::Ready => self.needs_validate = false,
            }
        }
        if !self.active_matches_none() {
            self.poll_active(tick);
        } else {
            // Even at expiry, re-observe first: a freshly regrown resource
            // takes precedence over waiting or widening.
            self.begin_idle(tick);
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
        self.pending = Some(next);
        self.needs_validate = true;
        self.dirty = true;
        Ok(SettingsApply::Applied)
    }

    fn retry(&mut self) -> Result<(), ScriptFailure> {
        let Some(failure) = self.failure.as_ref() else {
            return Ok(());
        };
        if !failure.retryable {
            return Err(failure.clone());
        }
        let exhausted_search =
            failure.code.as_ref() == "resource-unavailable" && self.retained.anchor.is_none();
        if self.settings().location.eq_ignore_ascii_case("auto")
            && (failure.code.as_ref() == "widen-limit" || exhausted_search)
        {
            self.tried_groups = [TriedGroup::default(); 4];
            self.widen.reset();
            self.retained.anchor = None;
            self.area = None;
        }
        self.clear_failure();
        self.wait_until = None;
        self.needs_validate = true;
        self.rebaseline_gameplay = true;
        self.dirty = true;
        Ok(())
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
        self.retained.anchor
    }
}

impl Gatherer {
    fn active_matches_none(&self) -> bool {
        matches!(self.active, Active::None)
    }
}

fn tool_gate_met(tool: &ToolUse, skill: Skill, stat: &StatView) -> bool {
    if let Some(gate) = tool.use_gate {
        i32::from(gate.skill) == stat.index && stat.base >= i32::from(gate.level)
    } else {
        match skill {
            Skill::Woodcutting | Skill::Mining | Skill::Fishing => true,
        }
    }
}

fn adjacent(here: Option<WorldTile>, target: WorldTile) -> bool {
    here.is_some_and(|here| {
        here.level == target.level && (here.x - target.x).abs().max((here.z - target.z).abs()) <= 1
    })
}

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
    fn auto_retry_preserves_groups_unless_search_is_exhausted() {
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
        let anchor = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        for (code, current_group, reset) in [
            ("walk-failed", Some(anchor), false),
            ("inventory-blocked", Some(anchor), false),
            ("resource-unavailable", Some(anchor), false),
            ("widen-limit", Some(anchor), true),
            ("resource-unavailable", None, true),
        ] {
            let mut runner = Gatherer::new(
                RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                Arc::clone(&config),
                Arc::clone(config.get::<Arc<Prepared>>().unwrap()),
                GatherRetained::default(),
            );
            runner.retained.anchor = current_group;
            runner.area = current_group.map(|anchor| WorkArea {
                mode: super::super::area::AreaMode::Auto,
                anchor,
                radius: 12,
            });
            assert!(remember_group(&mut runner.tried_groups, anchor, 1, 1000));
            runner.fail(code, code, true);

            runner.retry().unwrap();

            assert!(runner.failure.is_none(), "{code}");
            assert!(runner.needs_validate, "{code}");
            assert_eq!(
                runner.retained.anchor,
                if reset { None } else { current_group },
                "{code}"
            );
            assert_eq!(runner.area.is_none(), reset, "{code}");
            assert_eq!(
                runner.tried_groups[0].until,
                if reset { 0 } else { 1000 },
                "{code}"
            );
        }
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
}
