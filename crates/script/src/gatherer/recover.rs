//! Verified recovery sequencing; bank and walk work remains in the shared machines.
use super::*;
use std::time::Duration;

impl Gatherer {
    pub(super) fn advance_recovery(&mut self, state: RecoveryState, tick: &mut NativeTick<'_>) {
        self.retained.recovery = state;
        self.dirty = true;
        self.sync_retained(tick);
    }

    pub(super) fn latch_recovery(&mut self, tick: &mut NativeTick<'_>) {
        let exceeded = self.retained.deaths >= self.settings().max_deaths;
        let repeated = self.retained.deaths > 0
            && (self.retained.recovery != RecoveryState::Idle || !self.retained.haul_since_death);
        self.cancel_active();
        self.trip = TripStep::Idle;
        self.selected_bank = None;
        self.scratch.withdrawals = None;
        self.supply_missing = None;
        self.observed_progress = None;
        self.proof_evidence = 0;
        self.tool = ToolState {
            id: -1,
            worn: false,
        };
        self.respawn_deadline = Some(tick.cx.active_now() + Duration::from_secs(20));
        self.respawn_settle = None;
        self.proving_runs = 0;
        self.recovery_reprovisions = 0;
        self.needs_validate = true;
        self.retained.deaths = self.retained.deaths.saturating_add(1);
        self.retained.haul_since_death = false;
        self.advance_recovery(RecoveryState::Pending { step: 1 }, tick);
        self.set_event("death observed; inventory unknown");
        if exceeded {
            self.fail(
                "max-deaths",
                "maximum deaths exceeded; Stop/Start required",
                false,
            );
        } else if repeated {
            self.fail("died-again", "died before a recovered haul completed", true);
        } else if self.settings().death_policy.eq_ignore_ascii_case("Stop") {
            self.fail("died", "death observed; Retry consents to recovery", true);
        }
    }

    pub(super) fn poll_recovery(&mut self, tick: &mut NativeTick<'_>) {
        let RecoveryState::Pending { step } = self.retained.recovery else {
            return;
        };
        if step == 1 {
            let deadline = *self
                .respawn_deadline
                .get_or_insert_with(|| tick.cx.active_now() + Duration::from_secs(20));
            let snapshot = tick.cx.snapshot();
            let respawned = snapshot
                .here()
                .is_some_and(|here| crate::native::death::RESPAWN_SQUARE.contains(here.value))
                && snapshot.stats().is_some_and(|stats| {
                    stats.value.iter().any(|stat| {
                        stat.name.eq_ignore_ascii_case("hitpoints")
                            && stat.base > 0
                            && stat.effective == stat.base
                    })
                });
            if self.respawn_settle.is_none() && respawned {
                self.respawn_settle = Some(tick.cx.evidence().tick.saturating_add(3));
                self.set_event("respawn observed; settling");
            }
            if self
                .respawn_settle
                .is_some_and(|until| tick.cx.evidence().tick >= until)
            {
                // observe_death has examined every eligible frame, including
                // this one: retain its current watermark, never a fresh latch.
                self.advance_recovery(RecoveryState::Pending { step: 2 }, tick);
            } else if self.respawn_settle.is_none() && tick.cx.active_now() >= deadline {
                self.fail(
                    "respawn-not-observed",
                    "restored HP at the respawn square was not observed",
                    true,
                );
            }
            return;
        }
        if !self.active_matches_none() {
            self.poll_active(tick);
            return;
        }
        if self.fence.sealed {
            return;
        }
        if let Some(revision) = self.apply_pending(false) {
            tick.output.settings_applied(revision);
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
        if self.failure.is_some() {
            return;
        }
        if self.begin_eat(tick) {
            return;
        }
        match step {
            2 => self.advance_recovery(RecoveryState::Pending { step: 3 }, tick),
            3 => {
                if self.trip != TripStep::Idle {
                    self.begin_trip(tick);
                    return;
                }
                let snapshot = tick.cx.snapshot();
                let (Some(stats), Some(inventory), Some(equipment), Some(_)) = (
                    snapshot.stats(),
                    snapshot.inventory(),
                    snapshot.equipment(),
                    snapshot.inventory_capacity(),
                ) else {
                    return;
                };
                if SupplyPlan::due(
                    &self.prepared,
                    stats.value,
                    inventory.value,
                    equipment.value,
                ) {
                    self.start_trip();
                } else if snapshot
                    .bank_session()
                    .is_some_and(|session| session.value.open)
                {
                    // Recreation can observe a completed withdrawal before
                    // the previous instance closed its bank modal.
                    self.advance_trip(TripStep::Close);
                } else {
                    self.advance_recovery(RecoveryState::Pending { step: 4 }, tick);
                    self.needs_validate = true;
                }
            }
            4 => {
                if self.trip != TripStep::Idle {
                    if self.recovery_reprovisions != 0 {
                        self.fail(
                            "supply-missing",
                            "supply-missing:tool after reprovision",
                            true,
                        );
                        return;
                    }
                    self.recovery_reprovisions += 1;
                    // A supply deficit re-observed at validation retries the
                    // same shared supply sequence, not a normal return trip.
                    self.advance_recovery(RecoveryState::Pending { step: 3 }, tick);
                } else {
                    self.advance_recovery(RecoveryState::Pending { step: 5 }, tick);
                }
            }
            5 => {
                if let Some(area) = self.area {
                    self.trip_walk(area.anchor, area.radius, tick);
                    self.set_event("returning after death");
                }
            }
            _ => unreachable!("invalid retained Gatherer recovery step"),
        }
    }

    pub(super) fn finish_recovery_return(
        &mut self,
        result: WalkReceipt,
        tick: &mut NativeTick<'_>,
    ) {
        let arrived = result.end == WalkEnd::Arrived
            && self.area.is_some_and(|area| {
                tick.cx.snapshot().here().is_some_and(|here| {
                    here.value.level == area.anchor.level
                        && here
                            .value
                            .x
                            .abs_diff(area.anchor.x)
                            .max(here.value.z.abs_diff(area.anchor.z))
                            <= u32::from(area.radius)
                })
            });
        if !arrived {
            self.fail(
                "return-failed",
                format!("return-failed:{:?}; arrival must be observed", result.end),
                true,
            );
            return;
        }
        self.proving_runs = 0;
        self.proof_evidence = 0;
        self.advance_recovery(RecoveryState::Proving, tick);
        self.needs_validate = true;
        self.set_event("returned; proving fresh product and XP");
    }
}
