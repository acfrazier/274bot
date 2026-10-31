use super::*;
use crate::native::tickcycle::{
    AnimClass, AnimInstruction, CycleSignals, CycleSpec, CycleStep, OffReason, RollSignal, Shape,
    TickCycle, TickPolicy, Verdict,
};
use api::gather_methods::{TickArmKind, TickRollSignal};
use api::interact::server_ordered::{
    ExactLocOrNpcRef, ExactLocReference, ExactNpcReference, InventoryItemReference, KnifeArmShape,
    ServerOrderedRequest,
};
use api::snapshot::{ItemView, SnapshotView};

/// The live machine owns its cycle. The runner retains the run-level policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GatherTiming {
    pub(crate) cycle: TickCycle,
    pub(crate) policy: TickPolicy,
    pub(crate) kit: [i32; 2],
    pub(crate) skill_anim: i32,
    /// Selected static own-op proof, rechecked against the live type before every arm/click.
    pub(crate) ordinary_npc: Option<i32>,
}

impl GatherTiming {
    pub(crate) fn spec(method: &GatherMethod, tool: i32) -> Option<CycleSpec> {
        let arm = &method.tick_arm;
        let tool_fact = arm.for_tool(tool);
        Some(CycleSpec::new(
            match arm.kind {
                TickArmKind::Continuing => Shape::Continuing,
                TickArmKind::Ending => Shape::Ending,
                TickArmKind::None => return None,
            },
            match arm.roll_signal {
                TickRollSignal::Always => RollSignal::Always,
                TickRollSignal::Sometimes => RollSignal::Sometimes,
                TickRollSignal::Never => RollSignal::Never,
            },
            arm.delay_drop,
            arm.depletes,
            tool_fact.map(|row| row.native_arm).or(arm.native_arm)?,
            arm.act_min_d,
            arm.act_silent,
        ))
    }
}

impl GatherRun {
    /// Observe on every poll, including wakes. Only the core consumes animation edges.
    pub(super) fn drive_timing(
        &mut self,
        cx: &mut ActionContext<'_>,
        product_rose: bool,
    ) -> Result<bool, ActionError> {
        let Some(timing) = &mut self.args.timing else {
            return Ok(false);
        };
        let snapshot = cx.snapshot();
        let method = &self.args.catalog.methods()[usize::from(self.args.target.method_index)];
        let arm = &method.tick_arm;
        let before = timing.policy.diagnostic();
        let mut act_line = false;
        let mut success_line = false;
        let mut fletch = false;
        let mut content_refusal = false;
        let mut members_refusal = false;
        let mut cursor = timing.cycle.chat_cursor();
        if let Some(lines) = snapshot.chat_lines(cursor) {
            for line in lines
                .value
                .iter()
                .filter(|line| line.sequence > timing.cycle.chat_cursor())
            {
                cursor = cursor.max(line.sequence);
                act_line |= contains_any(
                    &line.text,
                    &[
                        "You swing your pick at the rock.",
                        "You swing your axe at the tree.",
                        "You attempt to catch a fish.",
                        "You attempt to catch an eel.",
                    ],
                );
                success_line |=
                    contains_any(&line.text, &["You manage to mine some", "You get some"]);
                fletch |= contains_any(
                    &line.text,
                    &[
                        "You carefully cut the wood into 15 arrow shafts.",
                        "You have run out of logs.",
                    ],
                );
                members_refusal |= contains_any(
                    &line.text,
                    &[
                        "To use this item please login to a members' server.",
                        "You need to be on a members' world to gain experience in Fletching.",
                    ],
                );
                content_refusal |= contains_any(
                    &line.text,
                    &[
                        "You need a Fletching level of at least",
                        "Nothing interesting happens.",
                    ],
                );
            }
        }
        timing.cycle.set_chat_cursor(cursor);
        let inventory = snapshot.inventory().map(|rows| rows.value);
        if let Some(rows) = inventory {
            let logs = item_count(rows, timing.kit[1]);
            fletch |= logs < self.previous_logs;
            self.previous_logs = logs;
        }
        if let Some(stats) = snapshot.stats() {
            let xp = stat_xp(stats.value, 9);
            fletch |= xp > self.previous_fletch_xp;
            self.previous_fletch_xp = xp;
        }
        let anim = snapshot
            .local_player()
            .and_then(|player| player.value.animation_update)
            .map(|instruction| {
                let sequence = instruction.sequence;
                let class = if sequence == -1 {
                    AnimClass::Null
                } else if Some(sequence) == arm.fletch_anim {
                    AnimClass::Fletching
                } else if sequence == timing.skill_anim {
                    AnimClass::Skill
                } else if Some(sequence) == arm.roll_anim {
                    AnimClass::Roll
                } else if Some(sequence) == arm.cast_anim {
                    AnimClass::Cast
                } else {
                    AnimClass::Other
                };
                AnimInstruction {
                    serial: instruction.serial,
                    class,
                }
            });
        if let Some(instruction) = anim.filter(|instruction| {
            timing
                .cycle
                .fresh_animation_at(cx.evidence().tick, instruction.serial)
        }) {
            if arm.depletes && instruction.class == AnimClass::Null {
                // A depletion's loc update can follow its null animation in a wake.
                // Only a replacement machine may use Idle to arm another target.
                self.waiting_target_change = true;
            } else if matches!(
                instruction.class,
                AnimClass::Skill | AnimClass::Roll | AnimClass::Cast
            ) {
                self.waiting_target_change = false;
            }
        }
        let active = observe_target(snapshot, &self.args.catalog, &self.args.target)
            == Some(TargetObservation::Active)
            && match self.args.target.entity {
                EntityId::Npc(id) => timing.ordinary_npc == Some(id),
                EntityId::Loc(_) => true,
                EntityId::Obj(_) => false,
            };
        let target_ready = active && !self.waiting_target_change;
        let request = target_ready
            .then(|| knife_request(snapshot, timing, method, &self.args.target))
            .flatten();
        let receipt = timing
            .cycle
            .batch_request_id()
            .and_then(|id| cx.server_ordered_completion(id));
        let checked_receipt = timing.cycle.checked_request_id().and_then(|id| {
            cx.interaction_receipt(id)
                .map(|receipt| (id, receipt.accepted))
        });
        let signals = CycleSignals {
            tick: cx.evidence().tick,
            anim,
            act_line,
            success_line,
            fletch,
            content_refusal: content_refusal || members_refusal,
            menu_open: snapshot
                .chat_modal()
                .is_some_and(|modal| modal.value.root == 8880),
            product_rose,
            target_ready,
            batch_permitted: request.is_some()
                && cx.server_ordered_available()
                && OneOpArgs::stray_modal(snapshot).is_none(),
            receipt: receipt.map(|completion| (completion.evidence.tick, &completion.dispatch)),
            checked_receipt,
        };
        let window = signals.tick;
        let prior_click_kind = timing.cycle.click_kind();
        let step = timing.cycle.drive(&signals, &mut timing.policy);
        if members_refusal {
            timing.policy.disable(OffReason::MembersOnlyArm);
        }
        let diagnostic = timing.policy.diagnostic();
        if diagnostic != before
            || receipt.is_some_and(|completion| completion.dispatch.terminal.is_some())
            || matches!(step, CycleStep::Batch | CycleStep::Click)
            || checked_receipt.is_some()
        {
            api::host_log!(
                api::hostlog::Category::ScriptLifecycle,
                api::hostlog::Level::Debug,
                "gatherer tick: verdict={:?} reason={:?} sequence={} window={} step={:?} click_kind={:?} request={:?} prefix={:?} terminal={:?} checked={:?} anim={:?} target_npc={:?} target_ready={} cycle={:?}",
                diagnostic.verdict, diagnostic.reason, diagnostic.sequence, window, step,
                timing.cycle.click_kind().or(prior_click_kind),
                receipt.map(|completion| completion.dispatch.request_id), receipt.map(|completion| completion.dispatch.dispatched_prefix),
                receipt.and_then(|completion| completion.dispatch.terminal), checked_receipt, anim,
                snapshot.npcs().and_then(|rows| rows.value.iter().find(|npc| npc.index == self.args.target.npc_index as usize))
                    .map(|npc| (npc.index, npc.r#type, snapshot.npc_info_generation())),
                target_ready, timing.cycle,
            );
        }
        match step {
            CycleStep::Batch => {
                if let Some(request) = request {
                    let id = cx.emit_server_ordered(request)?;
                    timing.cycle.note_emit(window, step, id);
                    self.last_emit_tick = window;
                }
            }
            CycleStep::Click if active => match cx.emit(click_request(&self.args.target)?) {
                Ok(id) => {
                    timing.cycle.note_emit(window, step, id);
                    self.last_emit_tick = window;
                }
                Err(ActionError::BudgetExhausted) => {}
                Err(error) => return Err(error),
            },
            CycleStep::Wait | CycleStep::Native(_) | CycleStep::Click => {}
        }
        Ok(diagnostic != before
            && matches!(
                diagnostic.verdict,
                Some(Verdict::Verified | Verdict::Delayed | Verdict::Unobserved)
            ))
    }
}

pub(in crate::gatherer) fn item_count(rows: &[ItemView], id: i32) -> i32 {
    rows.iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count.max(0))
        .sum()
}

fn knife_request(
    snapshot: SnapshotView<'_>,
    timing: &GatherTiming,
    method: &GatherMethod,
    target: &TargetPlan,
) -> Option<ServerOrderedRequest> {
    let inventory = snapshot.inventory()?.value;
    let generation = snapshot.inventory_packet_generation()?;
    let item_ref = |id| {
        let item = inventory
            .iter()
            .find(|row| row.def.id == id && row.count > 0)?;
        InventoryItemReference::new(id, item.slot, item.component_id, generation).ok()
    };
    let knife = item_ref(timing.kit[0])?;
    let logs = item_ref(timing.kit[1])?;
    let operation = known_rows(&method.targets)
        .iter()
        .find(|row| row.entity == target.entity && row.class == TargetClass::Resource)?
        .op;
    let shape = if method.tick_arm.kind == TickArmKind::Ending {
        KnifeArmShape::Ending
    } else {
        let click = match target.entity {
            EntityId::Loc(id) => {
                let loc = snapshot
                    .locs()?
                    .value
                    .iter()
                    .find(|loc| loc.id == id && loc.tile == target.tile)?;
                let (scene, static_locs, _) = snapshot.ground_collision_generations()?;
                ExactLocOrNpcRef::Loc(
                    ExactLocReference::new(
                        id,
                        loc.typecode,
                        loc.tile,
                        scene,
                        static_locs,
                        operation,
                    )
                    .ok()?,
                )
            }
            EntityId::Npc(id) => {
                let npc = snapshot.npcs()?.value.iter().find(|npc| {
                    npc.index == target.npc_index as usize
                        && npc.r#type == Some(id as usize)
                        && npc.tile == target.tile
                })?;
                ExactLocOrNpcRef::Npc(
                    ExactNpcReference::new(
                        npc.index,
                        id,
                        npc.network,
                        snapshot.npc_info_generation()?,
                        operation,
                    )
                    .ok()?,
                )
            }
            EntityId::Obj(_) => return None,
        };
        KnifeArmShape::Continuing { click }
    };
    Some(ServerOrderedRequest::KnifeArm { knife, logs, shape })
}
