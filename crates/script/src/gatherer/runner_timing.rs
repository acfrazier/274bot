use super::super::gather::GatherTiming;
use super::*;
use crate::native::tickcycle::{OffReason, TickCycle, Verdict as CycleVerdict};
use api::gather_methods::TickArmKind;
use api::selected::EntityId;

impl Gatherer {
    /// Copy the policy before dropping a live machine. Cancellation never resets its ring.
    pub(super) fn capture_live_timing(&mut self) {
        if let Active::Gather(handle) = &self.active {
            if let Some(Some(timing)) = handle.inspect(|machine| machine.timing()) {
                self.tick_policy = timing.policy;
            }
        }
    }

    pub(super) fn timing_for_target(
        &mut self,
        target: &TargetPlan,
        cx: &crate::native::ActionContext<'_>,
    ) -> Option<GatherTiming> {
        if self.settings().tick_method != "Knife" {
            self.parked_cycle = None;
            self.tick_applicability = None;
            return None;
        }
        let method = self
            .prepared
            .catalog
            .methods()
            .get(usize::from(target.method_index))?;
        let arm = &method.tick_arm;
        let tool = arm.for_tool(self.tool.id);
        let stock = cx.snapshot().stock();
        let kit_missing = stock.bank.is_some_and(|bank| {
            SupplyPlan::build(
                &self.prepared,
                cx.snapshot().stats().map_or(&[], |stats| stats.value),
                stock,
                |id| bank.count(id).unwrap_or(0),
            )
            .kit_missing()
        });
        let off = if method.id.0.as_ref() == "woodcutting.normal" {
            Some(OffReason::NotInRelease)
        } else if arm.kind == TickArmKind::None {
            Some(OffReason::NoDelayVariable)
        } else if tool.map(|row| row.no_gain).or(arm.no_gain).unwrap_or(true) {
            Some(OffReason::NoGain)
        } else {
            let kit = self.prepared.supply.tick_kit.as_ref()?;
            let held = cx.snapshot().inventory()?.value;
            (kit_missing
                || super::super::gather::timing_item_count(held, kit.knife.id) < 1
                || super::super::gather::timing_item_count(held, kit.logs.id) < 1)
                .then_some(OffReason::NoArmSupply)
        };
        self.tick_applicability = off;
        if off.is_some() {
            self.parked_cycle = None;
            return None;
        }
        let ordinary_npc = match target.entity {
            EntityId::Npc(id) => {
                let operation = known_rows(&method.targets)
                    .iter()
                    .find(|row| row.entity == target.entity)?
                    .op;
                self.prepared
                    .selected
                    .server_ordered_catalog(cx.pin())
                    .ok()?
                    .builtin_gatherer_fishing_operation(id, operation)?;
                Some(id)
            }
            EntityId::Loc(_) => None,
            EntityId::Obj(_) => return None,
        };
        let snapshot = cx.snapshot();
        let spec = GatherTiming::spec(method, self.tool.id)?;
        let kit = self.prepared.supply.tick_kit.as_ref()?;
        let cycle = self.parked_cycle.take().unwrap_or_else(|| {
            TickCycle::new(
                spec,
                cx.evidence().tick,
                snapshot
                    .local_player()
                    .and_then(|player| player.value.animation_update)
                    .map_or(0, |anim| anim.serial),
                snapshot
                    .chat_lines(0)
                    .and_then(|lines| lines.value.iter().map(|line| line.sequence).max())
                    .unwrap_or(0),
            )
        });
        Some(GatherTiming {
            cycle,
            policy: self.tick_policy,
            kit: [kit.knife.id, kit.logs.id],
            skill_anim: tool
                .map(|row| row.skill_anim)
                .or(arm.skill_anim)
                .unwrap_or(-2),
            ordinary_npc,
        })
    }

    /// Inspect before publish's dirty check, so counters are visible on this frame.
    pub(super) fn refresh_tick_status(&mut self) {
        let before = self.tick_policy;
        self.capture_live_timing();
        if self.tick_policy != before {
            self.dirty = true;
        }
        let reason = self.tick_policy.off_reason().or(self.tick_applicability);
        if reason != self.tick_last_off {
            self.tick_last_off = reason;
            let event = reason.map_or_else(
                || "tick: on".to_owned(),
                |reason| format!("tick: off ({})", reason.name()),
            );
            self.set_event(&event);
            api::host_log!(
                api::hostlog::Category::ScriptLifecycle,
                api::hostlog::Level::Info,
                "Gatherer {event}; cycles={} off_until={:?} counters={:?}",
                self.tick_policy.cycles(),
                self.tick_policy.off_until(),
                self.tick_policy.stats()
            );
        } else if self.tick_last_sequence != self.tick_policy.diagnostic().sequence {
            match self.tick_policy.diagnostic().verdict {
                Some(CycleVerdict::Verified) if self.tick_policy.stats().verified == 1 => {
                    self.set_event("tick: armed")
                }
                Some(CycleVerdict::Slip) => self.set_event("tick: slip"),
                Some(CycleVerdict::Lost) => self.set_event("tick: lost"),
                Some(CycleVerdict::Delayed) => self.set_event("tick: delayed"),
                Some(CycleVerdict::Paused) => self.set_event("tick: paused (eat)"),
                _ => {}
            }
        }
        self.tick_last_sequence = self.tick_policy.diagnostic().sequence;
    }

    pub(super) fn tick_method_status(&self) -> Arc<str> {
        if self.settings().tick_method != "Knife" {
            return Arc::from("Off");
        }
        match self.tick_policy.off_reason().or(self.tick_applicability) {
            Some(reason) => Arc::from(format!("Knife (off: {})", reason.name())),
            None => Arc::from("Knife"),
        }
    }
}

#[cfg(test)]
#[path = "runner_timing_tests.rs"]
mod tests;
