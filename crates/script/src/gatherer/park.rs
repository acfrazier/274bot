//! Evidence-only supply parking. The acquisition core remains the sole planner.

use super::*;

pub(super) struct Park {
    pub(super) code: &'static str,
    pub(super) request: AcquireRequest,
    pub(super) block: AcquireBlock,
    pub(super) since_tick: u64,
}

impl Gatherer {
    pub(super) fn park_supply(&mut self, request: AcquireRequest, block: AcquireBlock, now: u64) {
        let code = block.code();
        let since_tick = self
            .scratch
            .parked
            .as_ref()
            .filter(|park| {
                park.code == code
                    && park
                        .request
                        .needs
                        .get(usize::from(park.block.need))
                        .map(|need| need.item)
                        == request
                            .needs
                            .get(usize::from(block.need))
                            .map(|need| need.item)
            })
            .map_or(now, |park| park.since_tick);
        self.cancel_active();
        self.wait_until = None;
        self.scratch.needs = None;
        self.scratch.parked = Some(Box::new(Park {
            code,
            request,
            block,
            since_tick,
        }));
        self.scratch.replan_once = false;
        self.scratch.rebuild_park_request = false;
        self.set_event(code);
        self.dirty = true;
    }

    /// Return true while the park or its close operation owns this frame.
    pub(super) fn poll_park(&mut self, tick: &mut NativeTick<'_>) -> bool {
        self.refresh_park_bank_provenance(tick);
        if !self.active_matches_none() {
            self.poll_active(tick);
            if !self.active_matches_none() || self.failure.is_some() {
                return true;
            }
        }
        if tick
            .cx
            .snapshot()
            .bank_session()
            .is_some_and(|bank| bank.value.open)
        {
            if tick.cx.may_continue_this_tick() {
                let request = self.acquire_request(Arc::from([]), None, CloseBank::Close, &tick.cx);
                self.active = Active::ParkClose(Box::new(AcquireRun::new(
                    request,
                    AcquirePlan::empty(),
                    &self.prepared.banks,
                    &tick.cx,
                )));
                self.poll_active(tick);
            }
            return true;
        }
        if self.needs_validate {
            match self.validate(&mut tick.cx) {
                Validation::Pending => return true,
                Validation::Ready => self.needs_validate = false,
            }
        }
        let Some(park) = self.scratch.parked.as_ref() else {
            return false;
        };
        if !self.scratch.replan_once
            && !crate::acquire::run::resolved_current(&park.block, &park.request, &tick.cx)
        {
            return true;
        }
        if !tick.cx.may_continue_this_tick() {
            return true;
        }
        let request = if self.scratch.rebuild_park_request {
            let Some(stats) = tick.cx.snapshot().stats() else {
                return true;
            };
            let needs =
                match SupplyPlan::build(&self.prepared, stats.value, tick.cx.snapshot().stock()) {
                    Ok(needs) => needs,
                    Err(error) => {
                        self.action_failure(error);
                        return true;
                    }
                };
            Some(self.acquire_request(needs, None, CloseBank::Close, &tick.cx))
        } else {
            None
        };
        let mut park = self.scratch.parked.take().expect("observed park");
        let previous_item = park
            .request
            .needs
            .get(usize::from(park.block.need))
            .map(|need| need.item);
        if let Some(request) = request {
            park.request = request;
        }
        self.scratch.replan_once = false;
        self.scratch.rebuild_park_request = false;
        park.request.required_after = tick.cx.evidence();
        match crate::acquire::run::plan_current(&park.request, &tick.cx) {
            Ok(AcquireVerdict::Satisfied) => {
                if self.retained.recovery == (RecoveryState::Pending { step: 3 }) {
                    self.advance_trip(TripStep::Idle);
                    self.advance_recovery(RecoveryState::Pending { step: 4 }, tick);
                } else if matches!(
                    self.trip,
                    TripStep::Supply {
                        return_to_work: true,
                        ..
                    }
                ) {
                    self.advance_trip(TripStep::Validate);
                } else {
                    self.advance_trip(TripStep::Idle);
                }
                self.needs_validate = true;
                self.dirty = true;
                self.set_event("supplies observed; validating equipment");
                false
            }
            Ok(AcquireVerdict::Plan(plan)) => {
                self.start_supply_plan(park.request, plan, &tick.cx);
                self.dirty = true;
                self.set_event("supply evidence changed; acquiring");
                false
            }
            Ok(AcquireVerdict::Blocked(block)) => {
                if park.code != block.code()
                    || previous_item
                        != park
                            .request
                            .needs
                            .get(usize::from(block.need))
                            .map(|need| need.item)
                {
                    park.since_tick = tick.cx.evidence().tick;
                }
                park.code = block.code();
                park.block = block;
                self.set_event(park.code);
                self.scratch.parked = Some(park);
                self.dirty = true;
                true
            }
            Err(error) => {
                self.action_failure(error);
                true
            }
        }
    }

    /// A relog keeps the parked block but relabels the same bank observation a
    /// saved hint. Re-render its bank clause so the reason and `bank_seen`
    /// agree; this reads evidence only and never plans or walks.
    pub(super) fn refresh_park_bank_provenance(&mut self, tick: &NativeTick<'_>) {
        let Some(park) = self.scratch.parked.as_mut() else {
            return;
        };
        let Some(bank) = park.block.looked.bank else {
            return;
        };
        let Some(current) = crate::acquire::plan::Counts::bank_seen(&tick.cx.snapshot().stock())
        else {
            return;
        };
        if current.origin == bank.origin {
            return;
        }
        let refreshed = crate::acquire::BankSeen {
            origin: current.origin,
            ..bank
        };
        park.block.message = crate::acquire::plan::rebank_message(
            &park.block.message,
            refreshed,
            std::time::SystemTime::now(),
        );
        park.block.looked.bank = Some(refreshed);
    }
}
