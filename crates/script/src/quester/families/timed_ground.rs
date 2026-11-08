//! Observed-tick bound for the existing ground Reach family.
//!
//! `drop tables/scripts/jailer.rs2:10` posts the jail key for 100 game ticks.
//! This adapter owns that authored bound, not a second pickup machine.
use super::super::compile::{FamilyReceipt, StepContext, StepOutcome, StepPlan, StepRun};
use super::reach::{Reach, ReachArgs, ReachKind};
use crate::native::{ActionError, ActionHandle, NativeActions};
use api::WorldTile;
use std::sync::Arc;
use std::task::Poll;

pub(super) fn plan(
    kind: ReachKind,
    anchor: Option<WorldTile>,
    radius: i32,
    max_ticks: u32,
) -> Arc<dyn StepPlan> {
    Arc::new(TimedGroundPlan {
        args: ReachArgs {
            kind,
            op: Arc::from("Take"),
            anchor,
            radius,
            wait_if_missing: true,
            target_tile: None,
            reachable_only: false,
        },
        max_ticks,
    })
}

struct TimedGroundPlan {
    args: ReachArgs,
    max_ticks: u32,
}

/// Inventory proof for a bounded ground-item acquisition.
#[derive(Debug)]
pub struct TimedGroundReceipt {
    /// Observed game ticks from step entry to inventory settlement.
    pub waited_ticks: u64,
    /// Held count after this acquisition.
    pub held: i32,
}

impl FamilyReceipt for TimedGroundReceipt {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StepPlan for TimedGroundPlan {
    fn anchor(&self) -> Option<WorldTile> {
        self.args.anchor
    }

    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let ReachKind::Ground { id, .. } = self.args.kind else {
            return Err(ActionError::Unavailable(Arc::from(
                "timed Take requires a ground item",
            )));
        };
        Ok(Box::new(TimedGroundRun {
            args: self.args.clone(),
            id,
            before: super::inventory_count(&cx.tick.cx, id),
            started_tick: cx.tick.cx.evidence().tick,
            max_ticks: self.max_ticks,
            reach: None,
        }))
    }
}

struct TimedGroundRun {
    args: ReachArgs,
    id: i32,
    before: Option<i32>,
    started_tick: u64,
    max_ticks: u32,
    reach: Option<ActionHandle<Reach>>,
}

impl StepRun for TimedGroundRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let elapsed = cx.tick.cx.evidence().tick.saturating_sub(self.started_tick);
        let limit = u64::from(self.max_ticks);
        if elapsed <= limit {
            if let (Some(before), Some(held)) =
                (self.before, super::inventory_count(&cx.tick.cx, self.id))
            {
                if held > before {
                    self.reach = None;
                    return Poll::Ready(Ok(StepOutcome {
                        progress: None,
                        evidence: cx.tick.cx.evidence(),
                        receipt: Some(Arc::new(TimedGroundReceipt {
                            waited_ticks: elapsed,
                            held,
                        })),
                    }));
                }
            }
        }
        if elapsed >= limit {
            self.reach = None;
            return Poll::Ready(Err(ActionError::Blocked(Arc::from(
                "ground pickup exceeded its tick budget",
            ))));
        }
        if self.before.is_none() {
            self.before = super::inventory_count(&cx.tick.cx, self.id);
            if self.before.is_none() {
                return Poll::Pending;
            }
        }
        if self.reach.is_none() {
            self.reach = Some(std::task::ready!(crate::native::defer_budget(
                cx.tick
                    .actions
                    .begin::<Reach>(self.args.clone(), &mut cx.tick.cx),
            ))?);
        }
        match cx
            .tick
            .actions
            .poll(self.reach.as_ref().unwrap(), &mut cx.tick.cx)
        {
            Poll::Pending | Poll::Ready(Ok(true)) => Poll::Pending,
            Poll::Ready(Ok(false)) => {
                self.reach = None;
                Poll::Pending
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self, _actions: &mut NativeActions) {
        self.reach = None;
    }

    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        let ReachKind::Ground { obj, .. } = &self.args.kind else {
            return None;
        };
        Some(("Waiting for timed ground pickup", obj))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_pickup_state_stays_bounded() {
        let plan = std::mem::size_of::<TimedGroundPlan>();
        let run = std::mem::size_of::<TimedGroundRun>();
        let receipt = std::mem::size_of::<TimedGroundReceipt>();
        eprintln!("TimedGroundPlan={plan} TimedGroundRun={run} TimedGroundReceipt={receipt}");
        assert!(run <= 1024, "timed pickup run is {run} bytes");
    }
}
