
use super::*;

fn continuing() -> CycleSpec {
    CycleSpec::new(
        Shape::Continuing,
        RollSignal::Always,
        false,
        false,
        4,
        1,
        false,
    )
}

fn ending() -> CycleSpec {
    CycleSpec::new(Shape::Ending, RollSignal::Always, false, false, 4, 1, false)
}

fn replay(spec: CycleSpec) -> Replay {
    Replay::new(spec, 100, 50)
}

struct Replay {
    cycle: TickCycle,
    policy: TickPolicy,
    next_serial: u32,
    next_request: u64,
}

impl Replay {
    fn new(spec: CycleSpec, tick: u64, serial: u32) -> Self {
        Self {
            cycle: TickCycle::new(spec, tick, serial, 7),
            policy: TickPolicy::default(),
            next_serial: serial + 1,
            next_request: 1,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "replay calls spell out each independently timed signal"
    )]
    fn poll(
        &mut self,
        tick: u64,
        class: Option<AnimClass>,
        act_line: bool,
        success_line: bool,
        fletch: bool,
        content_refusal: bool,
        menu_open: bool,
        product_rose: bool,
        target_ready: bool,
    ) -> CycleStep {
        self.poll_receipts(
            tick,
            class,
            act_line,
            success_line,
            fletch,
            content_refusal,
            menu_open,
            product_rose,
            target_ready,
            None,
            None,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "replay calls spell out each independently timed signal and both receipt kinds"
    )]
    fn poll_receipts(
        &mut self,
        tick: u64,
        class: Option<AnimClass>,
        act_line: bool,
        success_line: bool,
        fletch: bool,
        content_refusal: bool,
        menu_open: bool,
        product_rose: bool,
        target_ready: bool,
        receipt: Option<&ServerOrderedDispatch>,
        checked_receipt: Option<(u64, bool)>,
    ) -> CycleStep {
        let anim = class.map(|class| {
            let instruction = AnimInstruction {
                serial: self.next_serial,
                class,
            };
            self.next_serial = self.next_serial.wrapping_add(1);
            instruction
        });
        let signals = CycleSignals {
            tick,
            anim,
            act_line,
            success_line,
            fletch,
            content_refusal,
            menu_open,
            product_rose,
            target_ready,
            batch_permitted: true,
            receipt: receipt.map(|receipt| (receipt.dispatch_tick, receipt)),
            checked_receipt,
        };
        let step = self.cycle.drive(&signals, &mut self.policy);
        if matches!(step, CycleStep::Batch | CycleStep::Click) {
            self.cycle.note_emit(tick, step, self.next_request);
            self.next_request += 1;
        }
        step
    }

    fn batch(&mut self, tick: u64, class: AnimClass) -> u64 {
        assert_eq!(
            self.poll(
                tick,
                Some(class),
                false,
                false,
                false,
                false,
                false,
                false,
                false
            ),
            CycleStep::Batch
        );
        self.next_request - 1
    }

    fn ring(&self) -> u8 {
        self.policy.verdict_count()
    }

    fn accept_click(&mut self, tick: u64) {
        let request = self.next_request - 1;
        self.poll_receipts(
            tick,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            Some((request, true)),
        );
    }

    fn quiet_until(&mut self, tick: u64) {
        for observed in self.cycle.last_tick + 1..tick {
            assert_eq!(
                self.poll(observed, None, false, false, false, false, false, false, false),
                CycleStep::Wait,
                "quiet edge {observed}",
            );
        }
    }
}

fn dispatch_receipt(request_id: u64, tick: u64, shape: Shape, prefix: u8) -> ServerOrderedDispatch {
    use api::interact::server_ordered::{RowDispatch, ServerOrderedError};
    let row_events = match shape {
        Shape::Continuing => [1, 1, 2, 0, 0],
        Shape::Ending => [1, 1, 0, 0, 0],
    };
    let row_count = match shape {
        Shape::Continuing => 3,
        Shape::Ending => 2,
    };
    let dispatched_prefix = prefix.min(row_count);
    let mut rows = [RowDispatch::NotAttempted; 5];
    let mut wire_events = 0u8;
    for (index, row_wire_events) in row_events
        .iter()
        .copied()
        .take(usize::from(dispatched_prefix))
        .enumerate()
    {
        rows[index] = RowDispatch::HostQueued {
            wire_events: row_wire_events,
        };
        wire_events = wire_events.saturating_add(row_wire_events);
    }
    let terminal = if dispatched_prefix < row_count {
        let reason = ServerOrderedError::ContractUnavailable;
        let refused_row = usize::from(dispatched_prefix);
        rows[refused_row] = RowDispatch::Refused { reason };
        for row in rows
            .iter_mut()
            .take(usize::from(row_count))
            .skip(refused_row + 1)
        {
            *row = RowDispatch::SkippedDependency;
        }
        Some(reason)
    } else {
        None
    };
    ServerOrderedDispatch {
        request_id,
        dispatch_tick: tick,
        rows,
        row_count,
        dispatched_prefix,
        wire_events,
        terminal,
    }
}

fn assert_ring(replay: &Replay, entries: u8) {
    assert_eq!(replay.ring(), entries);
}

#[test]
fn three_verified_continuing_cycles_record_one_entry_each() {
    let mut run = replay(continuing());
    let mut tick = 101;
    let mut request = run.batch(tick, AnimClass::Skill);
    for cycle in 0..3 {
        let receipt = dispatch_receipt(request, tick, Shape::Continuing, 3);
        assert_eq!(
            run.poll_receipts(
                tick,
                None,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                Some(&receipt),
                None,
            ),
            CycleStep::Wait
        );
        assert_eq!(
            run.poll(
                tick + 1,
                Some(AnimClass::Fletching),
                true,
                false,
                false,
                false,
                false,
                false,
                false,
            ),
            CycleStep::Wait
        );
        assert_eq!(
            run.poll(
                tick + 2,
                None,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
            ),
            CycleStep::Wait
        );
        let step = run.poll(
            tick + 3,
            Some(AnimClass::Skill),
            false,
            false,
            false,
            false,
            cycle == 2,
            false,
            false,
        );
        assert_eq!(
            step,
            if cycle == 2 {
                CycleStep::Wait
            } else {
                CycleStep::Batch
            }
        );
        if cycle < 2 {
            request = run.next_request - 1;
        }
        tick += 3;
    }
    assert_ring(&run, 3);
    assert_eq!(run.policy.stats().verified, 3);
    assert_eq!(run.policy.stats().wire_events, 12);
    assert_eq!(run.policy.last_cycle_ticks(), Some(3));
    assert_eq!(
        run.policy.diagnostic(),
        CycleDiagnostic {
            sequence: 3,
            verdict: Some(Verdict::Verified),
            reason: Some(CycleReason::OnSchedule),
        }
    );
}

#[test]
fn packet_split_act_is_recovered_as_one_conservative_slip() {
    let mut run = replay(continuing());
    run.batch(101, AnimClass::Skill);
    assert_eq!(
        run.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    assert_eq!(
        run.poll(
            103,
            Some(AnimClass::Skill),
            true,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    assert_eq!(run.policy.stats().verified, 0);
    assert_eq!(run.policy.stats().slipped, 1);
    assert_ring(&run, 1);
}

#[test]
fn accepted_recovery_act_cannot_verify_the_on_time_roll() {
    let mut run = replay(continuing());
    let batch_request = run.batch(101, AnimClass::Skill);
    let receipt = dispatch_receipt(batch_request, 101, Shape::Continuing, 3);
    assert_eq!(
        run.poll_receipts(
            101,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some(&receipt),
            None,
        ),
        CycleStep::Wait
    );

    // The landing has no act line, so the core asks once for a checked
    // recovery act rather than treating the batch's receipt as execution.
    assert_eq!(
        run.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        ),
        CycleStep::Click
    );
    let recovery_request = run
        .cycle
        .checked_request_id()
        .expect("recovery act request is retained until its receipt");
    assert_eq!(run.cycle.click_kind(), Some("Act"));
    assert_eq!(
        run.poll_receipts(
            102,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            Some((recovery_request, true)),
        ),
        CycleStep::Wait
    );
    assert_eq!(run.cycle.checked_request_id(), None);
    assert_eq!(run.policy.stats().wire_events, 6);
    assert_eq!(run.cycle.click_kind(), None);
    assert!(!run.cycle.has_unresolved_requests());

    assert_eq!(
        run.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    // Fresh roll animation at the original deadline. It proves cadence,
    // but not that the original terminal row executed in time.
    assert_eq!(
        run.poll(
            104,
            Some(AnimClass::Skill),
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        ),
        CycleStep::Batch
    );
    assert_eq!(run.policy.stats().verified, 0);
    assert_eq!(run.policy.stats().unobserved, 0);
    assert_eq!(run.policy.stats().slipped, 1);
    assert_eq!(run.policy.stats().lost, 0);
    assert_eq!(run.policy.diagnostic().sequence, 1);
    assert_eq!(run.policy.diagnostic().reason, Some(CycleReason::LateAct));
    assert_ring(&run, 1);

    // The act line can first arrive in the roll's wake (bounded through
    // the next tick). It cannot reopen that cycle or ask another click.
    assert_eq!(
        run.poll(104, None, true, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(run.policy.stats().verified, 0);
    assert_eq!(run.policy.stats().unobserved, 0);
    assert_eq!(run.policy.stats().slipped, 1);
    assert_eq!(run.policy.diagnostic().sequence, 1);
    assert_ring(&run, 1);
}

#[test]
fn landing_survives_until_inventory_wake_without_an_extra_emission() {
    let mut run = replay(continuing());
    run.batch(101, AnimClass::Skill);
    assert_eq!(
        run.poll(
            102,
            Some(AnimClass::Fletching),
            true,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(102, None, false, false, false, false, false, true, false),
        CycleStep::Wait
    );
    assert!(matches!(run.cycle.phase(), Phase::Armed { .. }));
    assert_ring(&run, 0);
}

#[test]
fn retained_animation_is_not_a_landing_and_serial_is_the_freshness_key() {
    let mut run = Replay::new(continuing(), 100, 50);
    run.batch(101, AnimClass::Skill);
    let retained = AnimInstruction {
        serial: 51,
        class: AnimClass::Fletching,
    };
    let signals = CycleSignals {
        tick: 102,
        anim: Some(retained),
        act_line: false,
        success_line: false,
        fletch: false,
        content_refusal: false,
        menu_open: false,
        product_rose: false,
        target_ready: false,
        batch_permitted: true,
        receipt: None,
        checked_receipt: None,
    };
    assert_eq!(run.cycle.drive(&signals, &mut run.policy), CycleStep::Wait);
    let signals = CycleSignals {
        tick: 103,
        ..signals
    };
    assert_eq!(run.cycle.drive(&signals, &mut run.policy), CycleStep::Click);
    run.cycle.note_emit(103, CycleStep::Click, run.next_request);
    run.next_request += 1;
    assert_eq!(run.policy.stats().lost, 1);
    assert_ring(&run, 1);
    assert_eq!(
        run.poll(104, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );

    let mut fresh = replay(continuing());
    fresh.batch(101, AnimClass::Skill);
    let signals = CycleSignals {
        tick: 102,
        anim: Some(AnimInstruction {
            serial: 52,
            class: AnimClass::Fletching,
        }),
        act_line: true,
        success_line: false,
        fletch: false,
        content_refusal: false,
        menu_open: false,
        product_rose: false,
        target_ready: false,
        batch_permitted: true,
        receipt: None,
        checked_receipt: None,
    };
    assert_eq!(
        fresh.cycle.drive(&signals, &mut fresh.policy),
        CycleStep::Wait
    );
    assert!(matches!(fresh.cycle.phase(), Phase::Armed { .. }));

    let mut baseline = TickCycle::new(continuing(), 100, 50, 0);
    let mut policy = TickPolicy::default();
    let retained = CycleSignals {
        tick: 101,
        anim: Some(AnimInstruction {
            serial: 50,
            class: AnimClass::Fletching,
        }),
        act_line: false,
        ..signals
    };
    assert_eq!(baseline.drive(&retained, &mut policy), CycleStep::Wait);
    assert!(matches!(baseline.phase(), Phase::Native { .. }));
    assert_eq!(policy.verdict_count(), 0);
}

#[test]
fn refused_packet_clock_receipt_uses_native_completion_for_cooldown() {
    let mut run = replay(continuing());
    let batch = run.batch(101, AnimClass::Skill);
    run.policy
        .record(Verdict::Lost, CycleReason::HostRefused, 100, None);
    let receipt = dispatch_receipt(batch, 7, Shape::Continuing, 0);
    run.cycle
        .consume_dispatch_receipt(Some((101, &receipt)), &mut run.policy);
    assert_eq!(run.policy.stats().lost, 2);
    assert_eq!(
        run.policy.off_until,
        Some(OffUntil {
            reason: OffReason::UnstableTiming,
            tick: 181,
        })
    );
}

#[test]
fn ending_click_occurs_at_the_next_edge_and_roll_starts_next_cycle() {
    use api::interact::server_ordered::RowDispatch;
    let mut run = replay(ending());
    let batch = run.batch(101, AnimClass::Skill);
    let receipt = dispatch_receipt(batch, 101, Shape::Ending, 2);
    assert_eq!(receipt.row_count, 2);
    assert_eq!(receipt.wire_events, 2);
    assert_eq!(receipt.rows[0], RowDispatch::HostQueued { wire_events: 1 });
    assert_eq!(receipt.rows[1], RowDispatch::HostQueued { wire_events: 1 });
    assert_eq!(receipt.terminal, None);
    assert_eq!(
        run.poll_receipts(
            101,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some(&receipt),
            None,
        ),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(102, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    let request = run.next_request - 1;
    assert_eq!(
        run.poll_receipts(
            103,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            Some((request, true))
        ),
        CycleStep::Wait
    );
    assert_eq!(run.policy.stats().wire_events, 4);
    run.poll_receipts(
        103,
        None,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        None,
        Some((request, true)),
    );
    assert_eq!(
        run.policy.stats().wire_events,
        4,
        "duplicate receipts do not add packets"
    );
    assert_eq!(
        run.poll(
            104,
            Some(AnimClass::Roll),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );
    assert_eq!(run.policy.stats().verified, 1);
    assert_ring(&run, 1);
}

#[test]
fn late_ending_landing_records_one_slip_at_its_roll() {
    let mut run = replay(ending());
    run.batch(101, AnimClass::Skill);
    assert_eq!(
        run.poll(102, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(
            103,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(run.policy.stats().slipped, 0);
    assert_eq!(
        run.poll(104, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    run.accept_click(104);
    assert_eq!(
        run.poll(
            105,
            Some(AnimClass::Roll),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );
    assert_eq!(run.policy.stats().slipped, 1);
    assert_ring(&run, 1);

    let never = CycleSpec::new(Shape::Ending, RollSignal::Never, false, false, 4, 1, false);
    let mut blind = replay(never);
    blind.batch(101, AnimClass::Skill);
    blind.poll(102, None, false, false, false, false, false, false, false);
    blind.poll(
        103,
        Some(AnimClass::Fletching),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    blind.poll(104, None, false, false, false, false, false, false, false);
    blind.accept_click(104);
    assert_eq!(
        blind.poll(105, None, false, false, false, false, false, false, false),
        CycleStep::Batch
    );
    assert_eq!(blind.policy.stats().slipped, 1);
    assert_ring(&blind, 1);

    let mut refused = replay(ending());
    refused.batch(101, AnimClass::Skill);
    refused.poll(102, None, false, false, false, false, false, false, false);
    refused.poll(
        103,
        Some(AnimClass::Fletching),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    assert_eq!(
        refused.poll(104, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    let request = refused.next_request - 1;
    assert_eq!(
        refused.poll_receipts(
            104,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            Some((request, false))
        ),
        CycleStep::Wait
    );
    assert_eq!(
        refused.poll(105, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    assert_eq!(refused.policy.stats().lost, 1);
    assert_ring(&refused, 1);
}

#[test]
fn shifted_ending_cycles_enter_once_and_trip_only_on_the_third_roll() {
    for roll_signal in [RollSignal::Always, RollSignal::Never] {
        let mut spec = ending();
        spec.roll_signal = roll_signal;
        let mut run = replay(spec);
        let mut batch = run.batch(101, AnimClass::Skill);
        for cycle in 0..3 {
            let sent = 101 + cycle * 4;
            let receipt = dispatch_receipt(batch, sent, Shape::Ending, 2);
            run.poll_receipts(
                sent,
                None,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                Some(&receipt),
                None,
            );
            assert_eq!(
                run.poll(
                    sent + 1,
                    None,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false
                ),
                CycleStep::Wait
            );
            assert_eq!(
                run.poll(
                    sent + 2,
                    Some(AnimClass::Fletching),
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false
                ),
                CycleStep::Wait
            );
            assert_eq!(run.policy.stats().slipped, cycle as u32);
            assert_eq!(
                run.poll(
                    sent + 3,
                    None,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false
                ),
                CycleStep::Click
            );
            run.accept_click(sent + 3);
            let animation = (roll_signal == RollSignal::Always).then_some(AnimClass::Roll);
            let step = run.poll(
                sent + 4,
                animation,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
            );
            assert_eq!(run.policy.stats().slipped, cycle as u32 + 1);
            if cycle == 2 {
                assert_eq!(step, CycleStep::Native(OffReason::UnstableTiming));
                assert_ring(&run, 0);
                assert_eq!(run.policy.off_until(), Some(sent + 4 + 80));
            } else {
                assert_eq!(step, CycleStep::Batch);
                assert_ring(&run, cycle as u8 + 1);
                batch = run.next_request - 1;
            }
        }
    }

    let mut missing = replay(ending());
    missing.batch(101, AnimClass::Skill);
    missing.poll(102, None, false, false, false, false, false, false, false);
    missing.poll(
        103,
        Some(AnimClass::Fletching),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    assert_eq!(
        missing.poll(104, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    missing.accept_click(104);
    assert_eq!(
        missing.poll(105, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(
        missing.poll(106, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    let fallback = missing.next_request - 1;
    missing.poll_receipts(
        106,
        None,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        None,
        Some((fallback, false)),
    );
    assert_eq!(missing.policy.stats().slipped, 0);
    assert_eq!(missing.policy.stats().lost, 1);
    assert_eq!(missing.policy.stats().fallback_failed, 1);
    assert_ring(&missing, 1);
}

#[test]
fn normal_rock_null_and_bounded_success_is_delayed_not_refused() {
    let spec = CycleSpec::new(
        Shape::Continuing,
        RollSignal::Always,
        true,
        true,
        4,
        0,
        false,
    );
    let mut run = replay(spec);
    run.batch(101, AnimClass::Skill);
    assert_eq!(
        run.poll(101, None, false, true, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(
            102,
            Some(AnimClass::Null),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(102, None, false, false, false, false, false, false, true),
        CycleStep::Batch
    );
    assert_eq!(run.policy.stats().delayed, 1);
    assert_eq!(run.policy.stats().wire_events, 0);
    assert_ring(&run, 0);

    let mut line_at_edge = replay(spec);
    line_at_edge.batch(101, AnimClass::Skill);
    line_at_edge.poll(
        102,
        Some(AnimClass::Null),
        false,
        true,
        false,
        false,
        false,
        false,
        false,
    );
    assert_eq!(line_at_edge.policy.stats().delayed, 1);
    assert_ring(&line_at_edge, 0);

    let mut product = replay(spec);
    product.batch(101, AnimClass::Skill);
    product.poll(
        102,
        Some(AnimClass::Null),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    assert_eq!(
        product.poll(102, None, false, false, false, false, false, true, true),
        CycleStep::Batch
    );
    assert_eq!(product.policy.stats().delayed, 1);
    assert_eq!(
        product.poll(
            103,
            Some(AnimClass::Fletching),
            true,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(
        product.poll(104, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(
        product.poll(
            105,
            Some(AnimClass::Skill),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );
    assert_eq!(product.policy.stats().verified, 1);
    assert_eq!(product.policy.stats().delayed, 1);
    assert_ring(&product, 1);
}

#[test]
fn transit_batch_then_fletch_ends_once_without_acknowledgement_inference() {
    let spec = CycleSpec::new(
        Shape::Continuing,
        RollSignal::Always,
        true,
        false,
        4,
        1,
        false,
    );
    let mut run = replay(spec);
    run.batch(101, AnimClass::Skill);
    run.poll(101, None, false, true, false, false, false, false, false);
    run.poll(
        102,
        Some(AnimClass::Null),
        false,
        false,
        false,
        false,
        false,
        false,
        true,
    );
    let transit_batch = run.next_request - 1;
    assert_eq!(
        run.poll(
            103,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    let act_request = run.next_request - 1;
    assert_eq!(
        run.poll_receipts(
            103,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some(&dispatch_receipt(transit_batch, 102, Shape::Continuing, 3)),
            Some((act_request, true))
        ),
        CycleStep::Wait
    );
    run.poll(104, None, false, false, false, false, false, false, false);
    assert_eq!(
        run.poll(105, None, false, false, true, false, false, false, false),
        CycleStep::Click
    );
    assert_eq!(run.policy.stats().lost, 1);
    assert_ring(&run, 1);
}

#[test]
fn host_refusal_of_the_idle_replacement_batch_gets_one_fallback() {
    let spec = CycleSpec::new(
        Shape::Continuing,
        RollSignal::Always,
        true,
        true,
        4,
        0,
        false,
    );
    let mut run = replay(spec);
    run.batch(101, AnimClass::Skill);
    run.poll(101, None, false, true, false, false, false, false, false);
    assert_eq!(
        run.poll(
            102,
            Some(AnimClass::Null),
            false,
            false,
            false,
            false,
            false,
            false,
            true
        ),
        CycleStep::Batch
    );
    let replacement = run.next_request - 1;
    let receipt = dispatch_receipt(replacement, 102, Shape::Continuing, 0);
    assert_eq!(
        run.poll_receipts(
            102,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some(&receipt),
            None
        ),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    run.accept_click(103);
    assert_eq!(run.policy.stats().delayed, 1);
    assert_eq!(run.policy.stats().lost, 1);
    assert_eq!(run.policy.stats().fallback_clicks, 1);
    assert_eq!(run.policy.stats().wire_events, 2);
    assert_ring(&run, 1);
}

#[test]
fn bounded_intervals_expire_and_keep_their_packet_order() {
    assert_eq!(
        Bound::label(Position::AfterPlayerInfo, true),
        Bound { lo: -1, hi: 0 }
    );
    assert_eq!(
        Bound::label(Position::AfterPlayerInfo, false),
        Bound { lo: 0, hi: 0 }
    );
    assert_eq!(
        Bound::label(Position::BeforePlayerInfo, true),
        Bound { lo: 0, hi: 1 }
    );
    assert_eq!(
        Bound::label(Position::BeforePlayerInfo, false),
        Bound { lo: 1, hi: 1 }
    );

    let mut run = replay(continuing());
    run.poll(100, None, false, true, false, false, false, false, false);
    run.batch(101, AnimClass::Skill);
    run.poll(
        102,
        Some(AnimClass::Fletching),
        true,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    assert_eq!(
        run.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert!(!run.cycle.evidence.success(104, 103));

    let mut product = replay(CycleSpec::new(
        Shape::Continuing,
        RollSignal::Never,
        false,
        false,
        4,
        1,
        false,
    ));
    product.poll(
        101,
        Some(AnimClass::Cast),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    product.poll(102, None, false, false, false, false, false, true, false);
    product.poll(103, None, false, false, false, false, false, false, false);
    product.poll(104, None, false, false, false, false, false, false, false);
    assert!(!product.cycle.evidence.product_contains(106, 104));
    assert_ring(&run, 0);
}

#[test]
fn act_obligation_uses_prefix_independent_signal_and_receipt_id() {
    let mut run = replay(continuing());
    let batch_id = run.batch(101, AnimClass::Skill);
    let receipt = dispatch_receipt(batch_id, 101, Shape::Continuing, 2);
    assert_eq!(
        run.poll_receipts(
            101,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some(&receipt),
            None
        ),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    let act_id = run.next_request - 1;
    assert_eq!(
        run.poll_receipts(
            102,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            Some((act_id, true))
        ),
        CycleStep::Wait
    );
    assert_eq!(run.policy.stats().wire_events, 4);
    assert_eq!(
        run.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(
            104,
            Some(AnimClass::Skill),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );
    assert_eq!(run.policy.stats().verified, 0);
    assert_eq!(run.policy.stats().slipped, 1);
    assert_eq!(run.policy.stats().unobserved, 0);
    assert_eq!(run.policy.diagnostic().reason, Some(CycleReason::LateAct));
    assert_ring(&run, 1);

    let mut prefix3 = replay(continuing());
    let id = prefix3.batch(101, AnimClass::Skill);
    let receipt = dispatch_receipt(id, 101, Shape::Continuing, 3);
    prefix3.poll_receipts(
        101,
        None,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        Some(&receipt),
        None,
    );
    assert_eq!(
        prefix3.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    assert_eq!(prefix3.policy.stats().verified, 0);
    assert_ring(&prefix3, 0);
}

#[test]
fn act_deadline_and_late_receipts_never_create_a_second_act_request() {
    let mut mining = replay(CycleSpec::new(
        Shape::Continuing,
        RollSignal::Always,
        false,
        false,
        4,
        0,
        false,
    ));
    mining.batch(101, AnimClass::Skill);
    // Spending the landing window leaves the Act Due, not Asked.
    mining.cycle.last_emit_tick = 102;
    assert_eq!(
        mining.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(
        mining.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    assert_eq!(
        mining.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );

    let mut fishing = replay(continuing());
    fishing.batch(101, AnimClass::Skill);
    fishing.cycle.last_emit_tick = 102;
    assert_eq!(
        fishing.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(
        fishing.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    assert_eq!(fishing.policy.stats().lost, 1);
    assert_ring(&fishing, 1);

    let mut refused = replay(continuing());
    refused.batch(101, AnimClass::Skill);
    assert_eq!(
        refused.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    let request = refused.next_request - 1;
    assert_eq!(
        refused.poll_receipts(
            102,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            Some((request, false))
        ),
        CycleStep::Wait
    );
    assert_eq!(
        refused.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    assert_eq!(refused.policy.stats().lost, 1);
    assert_ring(&refused, 1);

    let mut late_receipt = replay(continuing());
    late_receipt.batch(101, AnimClass::Skill);
    assert_eq!(
        late_receipt.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    let request = late_receipt.next_request - 1;
    assert_eq!(
        late_receipt.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(
        late_receipt.poll_receipts(
            103,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            Some((request, false))
        ),
        CycleStep::Click
    );
    assert_eq!(late_receipt.policy.stats().lost, 1);
    assert_ring(&late_receipt, 1);
}

#[test]
fn lone_opener_gets_one_fallback_and_never_a_make_retry() {
    let mut run = replay(continuing());
    let batch = run.batch(101, AnimClass::Skill);
    let receipt = dispatch_receipt(batch, 101, Shape::Continuing, 1);
    assert_eq!(
        run.poll_receipts(
            101,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            Some(&receipt),
            None
        ),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(102, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    let fallback_id = run.next_request - 1;
    assert_eq!(
        run.poll_receipts(
            102,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            Some((fallback_id, false))
        ),
        CycleStep::Wait
    );
    assert_eq!(run.policy.stats().fallback_failed, 1);
    assert_eq!(
        run.poll(
            103,
            Some(AnimClass::Roll),
            false,
            false,
            false,
            false,
            true,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(
            104,
            Some(AnimClass::Roll),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );
    assert_eq!(run.policy.stats().lost, 1);
    assert_ring(&run, 1);
}

#[test]
fn continuing_late_batch_is_one_slip_for_all_observed_splits() {
    for (act_line, menu_wake, roll_tick) in [
        (false, false, 105),
        (false, true, 105),
        (true, false, 105),
        (false, false, 106),
    ] {
        let mut run = replay(continuing());
        run.batch(101, AnimClass::Skill);
        run.poll(
            102, None, false, false, false, false, menu_wake, false, false,
        );
        assert_eq!(
            run.poll(
                103,
                Some(AnimClass::Fletching),
                act_line,
                false,
                false,
                false,
                false,
                false,
                false
            ),
            CycleStep::Click
        );
        assert_eq!(run.policy.stats().slipped, 1);
        run.accept_click(103);
        assert_eq!(
            run.poll(104, None, false, false, false, false, false, false, false),
            CycleStep::Wait
        );
        run.quiet_until(roll_tick);
        assert_eq!(
            run.poll(
                roll_tick,
                Some(AnimClass::Skill),
                false,
                false,
                false,
                false,
                false,
                false,
                false
            ),
            CycleStep::Batch
        );
        assert_eq!(run.policy.stats().slipped, 1);
        assert_ring(&run, 1);
    }
}

#[test]
fn late_prefix_two_uses_one_slip_and_one_fallback() {
    let mut run = replay(continuing());
    let batch = run.batch(101, AnimClass::Skill);
    let receipt = dispatch_receipt(batch, 101, Shape::Continuing, 2);
    run.poll_receipts(
        101,
        None,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        Some(&receipt),
        None,
    );
    run.poll(102, None, false, false, false, false, false, false, false);
    assert_eq!(
        run.poll(
            103,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    assert_eq!(run.policy.stats().slipped, 1);
    assert_ring(&run, 1);
}

#[test]
fn late_fishing_fallback_waits_for_a_real_cast_and_never_invents_lava_cast() {
    let fly = CycleSpec::new(
        Shape::Continuing,
        RollSignal::Always,
        false,
        false,
        4,
        1,
        false,
    );
    let mut run = replay(fly);
    run.batch(101, AnimClass::Skill);
    run.poll(102, None, false, false, false, false, false, false, false);
    assert_eq!(
        run.poll(
            103,
            Some(AnimClass::Cast),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    run.accept_click(103);
    run.quiet_until(106);
    assert_eq!(
        run.poll(
            106,
            Some(AnimClass::Roll),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );

    let lava = CycleSpec::new(
        Shape::Continuing,
        RollSignal::Never,
        false,
        false,
        5,
        1,
        true,
    );
    let mut eel = replay(lava);
    eel.batch(101, AnimClass::Skill);
    eel.poll(102, None, false, false, false, false, false, false, false);
    assert_eq!(
        eel.poll(
            103,
            Some(AnimClass::Cast),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    eel.accept_click(103);
    eel.quiet_until(108);
    assert_eq!(
        eel.poll(108, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert!(matches!(
        eel.cycle.phase(),
        Phase::Native { roll_due: None, .. }
    ));
    eel.poll(
        109,
        Some(AnimClass::Cast),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    eel.quiet_until(113);
    assert_eq!(
        eel.poll(113, None, false, false, false, false, false, false, false),
        CycleStep::Batch
    );
}

#[test]
fn lost_continuing_cycle_recovers_only_from_a_fresh_cast() {
    let never = CycleSpec::new(
        Shape::Continuing,
        RollSignal::Never,
        false,
        false,
        4,
        1,
        false,
    );
    let mut run = replay(never);
    run.batch(101, AnimClass::Skill);
    run.poll(102, None, false, false, false, false, false, false, false);
    assert_eq!(
        run.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    run.accept_click(103);
    run.poll(104, None, false, false, false, false, false, false, false);
    run.poll(
        105,
        Some(AnimClass::Cast),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    run.quiet_until(108);
    assert_eq!(
        run.poll(108, None, false, false, false, false, false, false, false),
        CycleStep::Batch
    );
    assert_eq!(run.policy.stats().lost, 1);
    assert_ring(&run, 1);

    let always = CycleSpec::new(
        Shape::Continuing,
        RollSignal::Always,
        false,
        false,
        4,
        1,
        false,
    );
    let mut observed = replay(always);
    observed.batch(101, AnimClass::Skill);
    observed.poll(102, None, false, false, false, false, false, false, false);
    observed.poll(103, None, false, false, false, false, false, false, false);
    observed.accept_click(103);
    observed.poll(104, None, false, false, false, false, false, false, false);
    observed.poll(
        105,
        Some(AnimClass::Cast),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    observed.quiet_until(108);
    assert_eq!(
        observed.poll(108, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
}

#[test]
fn lost_ending_cycle_uses_cast_prediction_without_early_rebatch() {
    let spec = CycleSpec::new(Shape::Ending, RollSignal::Never, false, false, 4, 1, false);
    let mut run = replay(spec);
    run.batch(101, AnimClass::Skill);
    run.poll(102, None, false, false, false, false, false, false, false);
    assert_eq!(
        run.poll(103, None, false, false, false, false, false, false, false),
        CycleStep::Click
    );
    run.accept_click(103);
    assert_eq!(
        run.poll(104, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    run.poll(
        105,
        Some(AnimClass::Cast),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    run.quiet_until(108);
    assert_eq!(
        run.poll(108, None, false, false, false, false, false, false, false),
        CycleStep::Batch
    );
    assert_eq!(run.policy.stats().lost, 1);
    assert_ring(&run, 1);
}

#[test]
fn skipped_edges_consume_serials_preserve_asked_and_recover_fletch_once() {
    let mut sent = replay(continuing());
    sent.batch(101, AnimClass::Skill);
    let skipped = CycleSignals {
        tick: 104,
        anim: Some(AnimInstruction {
            serial: 55,
            class: AnimClass::Fletching,
        }),
        act_line: false,
        success_line: false,
        fletch: false,
        content_refusal: false,
        menu_open: false,
        product_rose: false,
        target_ready: false,
        batch_permitted: true,
        receipt: None,
        checked_receipt: None,
    };
    assert_eq!(
        sent.cycle.drive(&skipped, &mut sent.policy),
        CycleStep::Wait
    );
    assert_eq!(sent.cycle.anim_serial, 55);
    assert_eq!(sent.policy.stats().unobserved, 1);
    assert_eq!(
        sent.poll(
            105,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(
        sent.poll(
            106,
            Some(AnimClass::Roll),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );
    assert_ring(&sent, 0);

    let mut armed = replay(continuing());
    armed.batch(101, AnimClass::Skill);
    assert_eq!(
        armed.poll(
            102,
            Some(AnimClass::Fletching),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Click
    );
    let asked = armed.next_request - 1;
    let skip_fletch = CycleSignals {
        tick: 104,
        anim: None,
        act_line: false,
        success_line: false,
        fletch: true,
        content_refusal: false,
        menu_open: false,
        product_rose: false,
        target_ready: false,
        batch_permitted: true,
        receipt: None,
        checked_receipt: None,
    };
    assert_eq!(
        armed.cycle.drive(&skip_fletch, &mut armed.policy),
        CycleStep::Click
    );
    assert_eq!(armed.cycle.checked_request_id(), Some(asked));
    armed.poll_receipts(
        104,
        None,
        false,
        false,
        false,
        false,
        false,
        false,
        false,
        None,
        Some((asked, true)),
    );
    assert_eq!(
        armed.policy.stats().wire_events,
        2,
        "a retained Asked receipt still contributes its packets"
    );
    assert_eq!(
        armed.cycle.checked_request_id(),
        Some(asked + 1),
        "the old Asked receipt is consumed and the new fallback remains owned"
    );
    assert_eq!(armed.policy.stats().unobserved, 1);
    assert_eq!(armed.policy.stats().slipped, 0);
    assert_ring(&armed, 0);

    let mut due = replay(continuing());
    // Observe fletch without reporting a successful fallback emission.
    let un_emitted = CycleSignals {
        tick: 101,
        fletch: true,
        ..skip_fletch
    };
    assert_eq!(
        due.cycle.drive(&un_emitted, &mut due.policy),
        CycleStep::Click
    );
    assert_eq!(
        due.poll(104, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(due.policy.stats().fallback_failed, 1);
    assert_ring(&due, 0);
}

#[test]
fn host_refusal_returns_to_idle_and_slips_trip_the_policy() {
    let mut run = replay(continuing());
    let mut batch = run.batch(101, AnimClass::Skill);
    for tick in [101, 105] {
        let receipt = dispatch_receipt(batch, tick, Shape::Continuing, 0);
        assert_eq!(
            run.poll_receipts(
                tick,
                None,
                false,
                false,
                false,
                false,
                false,
                false,
                false,
                Some(&receipt),
                None
            ),
            CycleStep::Wait
        );
        assert_eq!(
            run.poll(
                tick + 1,
                None,
                false,
                false,
                false,
                false,
                false,
                false,
                false
            ),
            CycleStep::Click
        );
        run.accept_click(tick + 1);
        if tick == 101 {
            assert_ring(&run, 1);
            run.quiet_until(105);
            batch = run.batch(105, AnimClass::Skill);
        }
    }
    assert_eq!(run.policy.stats().lost, 2);
    assert_eq!(run.policy.off_reason(), Some(OffReason::UnstableTiming));
    assert_eq!(run.policy.off_until(), Some(185));
    assert_ring(&run, 0);
}

#[test]
fn slip_ring_slides_and_cooldown_ladder_is_run_scoped() {
    let mut policy = TickPolicy::default();
    policy.record(Verdict::Slip, CycleReason::LateBatch, 1, None);
    for tick in 2..=13 {
        policy.record(Verdict::Verified, CycleReason::OnSchedule, tick, None);
    }
    policy.record(Verdict::Slip, CycleReason::LateBatch, 14, None);
    assert_eq!(policy.verdict_count(), 12);
    assert_eq!(policy.off_reason(), None);
    policy.record(Verdict::Slip, CycleReason::LateBatch, 15, None);
    assert_eq!(
        policy.off_reason(),
        None,
        "the first slip has left the ring"
    );
    policy.record(Verdict::Slip, CycleReason::LateBatch, 16, None);
    assert_eq!(policy.off_until(), Some(96));
    assert_eq!(policy.verdict_count(), 0);
    policy.refresh_off(96);
    policy.record(Verdict::Slip, CycleReason::LateBatch, 97, None);
    assert_eq!(policy.off_reason(), None);
    policy.record(Verdict::Lost, CycleReason::NoLanding, 98, None);
    policy.record(Verdict::Lost, CycleReason::NoLanding, 99, None);
    assert_eq!(policy.off_until(), Some(259));
    policy.refresh_off(259);
    policy.record(Verdict::Lost, CycleReason::NoLanding, 260, None);
    policy.record(Verdict::Lost, CycleReason::NoLanding, 261, None);
    assert_eq!(policy.off_until(), Some(u64::MAX));
    assert_eq!(policy.off_reason(), Some(OffReason::UnstableTiming));

    let mut keep = TickPolicy::default();
    keep.record(Verdict::Slip, CycleReason::LateBatch, 1, None);
    let _cancelled_cycle = TickCycle::new(continuing(), 2, 55, 7);
    assert_eq!(keep.verdict_count(), 1);
    assert_eq!(keep.trips, 0);
}

#[test]
fn content_refusal_is_terminal_off_and_never_claims_server_ack() {
    let mut run = replay(continuing());
    run.batch(101, AnimClass::Skill);
    assert_eq!(
        run.poll(102, None, false, false, false, true, false, false, false),
        CycleStep::Click
    );
    assert_eq!(run.policy.off_reason(), Some(OffReason::ContentRefusal));
    assert_eq!(run.policy.stats().lost, 1);
    let request = run.next_request - 1;
    assert_eq!(
        run.poll_receipts(
            103,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            Some((request, true)),
        ),
        CycleStep::Native(OffReason::ContentRefusal)
    );
    assert_eq!(run.policy.stats().fallback_clicks, 1);
    assert_ring(&run, 1);

    let mut native = replay(continuing());
    assert_eq!(
        native.poll(101, None, false, false, false, true, false, false, false),
        CycleStep::Native(OffReason::ContentRefusal)
    );
    assert_eq!(native.policy.stats().lost, 0);
    assert_eq!(native.policy.stats().fallback_failed, 0);
    assert_ring(&native, 0);
}

#[test]
fn pause_counts_only_live_sent_or_armed_cycles() {
    let mut sent = replay(continuing());
    sent.batch(101, AnimClass::Skill);
    sent.cycle.pause(&mut sent.policy);
    assert_eq!(sent.policy.stats().paused, 1);
    assert_ring(&sent, 0);
    assert_eq!(
        sent.poll(
            102,
            Some(AnimClass::Roll),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );

    let mut armed = replay(continuing());
    armed.batch(101, AnimClass::Skill);
    armed.poll(
        102,
        Some(AnimClass::Fletching),
        true,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    armed.cycle.pause(&mut armed.policy);
    assert_eq!(armed.policy.stats().paused, 1);
    assert_ring(&armed, 0);

    let mut idle = replay(continuing());
    idle.cycle.phase = Phase::Idle {
        since: 100,
        fallback: None,
        pending: None,
    };
    idle.cycle.pause(&mut idle.policy);
    assert_eq!(idle.policy.stats().paused, 0);
    assert_ring(&idle, 0);

    let mut native = replay(continuing());
    native.cycle.pause(&mut native.policy);
    assert_eq!(native.policy.stats().paused, 0);
    assert_ring(&native, 0);
}

#[test]
fn bounded_only_roll_is_unobserved_and_never_a_verified_animation() {
    let lava = CycleSpec::new(
        Shape::Continuing,
        RollSignal::Never,
        false,
        false,
        5,
        1,
        true,
    );
    let mut wake = replay(lava);
    wake.cycle.phase = Phase::Armed {
        armed: 101,
        roll_due: 103,
        act: ActState::Assumed,
        late: false,
        null_edge: false,
    };
    wake.cycle.last_tick = 103;
    assert_eq!(
        wake.poll(103, None, false, false, false, false, false, true, false),
        CycleStep::Batch
    );
    assert_eq!(wake.policy.stats().unobserved, 1);
    assert_eq!(wake.policy.stats().verified, 0);
    assert_ring(&wake, 0);

    let mut late = replay(lava);
    late.cycle.phase = Phase::Armed {
        armed: 101,
        roll_due: 103,
        act: ActState::Assumed,
        late: false,
        null_edge: false,
    };
    late.cycle.last_tick = 103;
    assert_eq!(
        late.poll(104, None, false, false, false, false, false, true, false),
        CycleStep::Wait
    );
    assert_eq!(late.policy.stats().unobserved, 1);
    assert_eq!(late.policy.stats().verified, 0);
    assert_ring(&late, 0);
}

#[test]
fn fletch_during_live_cycle_falls_back_once_and_cast_resumes_from_observation() {
    let mut run = replay(continuing());
    run.batch(101, AnimClass::Skill);
    assert_eq!(
        run.poll(102, None, false, false, true, false, false, false, false),
        CycleStep::Click
    );
    assert_eq!(run.policy.stats().lost, 1);
    assert_ring(&run, 1);
    run.accept_click(102);
    run.poll(103, None, false, false, false, false, false, false, false);
    run.poll(
        104,
        Some(AnimClass::Cast),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    run.quiet_until(107);
    assert_eq!(
        run.poll(
            107,
            Some(AnimClass::Roll),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );
    assert_eq!(run.policy.stats().lost, 1);
    assert_ring(&run, 1);
}

#[test]
fn stale_menu_blocks_roll_emission_until_a_fresh_roll_after_close() {
    let mut run = replay(continuing());
    assert_eq!(
        run.poll(
            101,
            Some(AnimClass::Skill),
            false,
            false,
            false,
            false,
            true,
            false,
            false
        ),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(102, None, false, false, false, false, false, false, false),
        CycleStep::Wait
    );
    assert_eq!(
        run.poll(
            103,
            Some(AnimClass::Skill),
            false,
            false,
            false,
            false,
            false,
            false,
            false
        ),
        CycleStep::Batch
    );
    assert_eq!(run.policy.cycles(), 1);
    assert_ring(&run, 0);
}

#[test]
fn parked_cycle_keeps_asked_id_cursor_evidence_and_receipt_owner() {
    let mut run = replay(continuing());
    run.batch(101, AnimClass::Skill);
    run.poll(
        102,
        Some(AnimClass::Fletching),
        false,
        false,
        false,
        false,
        false,
        false,
        false,
    );
    let asked = run.next_request - 1;
    run.cycle.evidence.act_line = Some(Bound { lo: 0, hi: 1 });
    run.cycle.set_chat_cursor(27);
    let mut parked = run.cycle;
    let mut adopted = parked;
    assert_eq!(adopted.chat_cursor(), 27);
    assert_eq!(adopted.anim_serial, run.cycle.anim_serial);
    assert_eq!(adopted.evidence, run.cycle.evidence);
    assert_eq!(adopted.phase(), run.cycle.phase());
    adopted.last_tick = 102;
    let receipt = CycleSignals {
        tick: 103,
        anim: None,
        act_line: false,
        success_line: false,
        fletch: false,
        content_refusal: false,
        menu_open: false,
        product_rose: false,
        target_ready: false,
        batch_permitted: true,
        receipt: None,
        checked_receipt: Some((asked, false)),
    };
    assert_eq!(adopted.drive(&receipt, &mut run.policy), CycleStep::Click);
    assert_eq!(run.policy.stats().lost, 1);
    assert_ring(&run, 1);
    parked.set_chat_cursor(28);
    assert_eq!(parked.chat_cursor(), 28);
}

#[test]
fn drive_has_no_allocation_over_ten_thousand_polls() {
    let mut cycle = TickCycle::new(continuing(), 1, 1, 0);
    let mut policy = TickPolicy::default();
    let allocations = allocation_counter::measure(|| {
        for tick in 2..=10_001 {
            let signals = CycleSignals {
                tick,
                anim: None,
                act_line: false,
                success_line: false,
                fletch: false,
                content_refusal: false,
                menu_open: false,
                product_rose: false,
                target_ready: false,
                receipt: None,
                batch_permitted: true,
                checked_receipt: None,
            };
            std::hint::black_box(cycle.drive(&signals, &mut policy));
        }
    });
    assert_eq!(allocations, allocation_counter::AllocationInfo::default());
}

#[test]
fn unadmitted_batch_never_starts_or_spends_a_cycle() {
    let mut cycle = TickCycle::new(continuing(), 100, 50, 0);
    let mut policy = TickPolicy::default();
    let signals = CycleSignals {
        tick: 101,
        anim: Some(AnimInstruction {
            serial: 51,
            class: AnimClass::Roll,
        }),
        act_line: false,
        success_line: false,
        fletch: false,
        content_refusal: false,
        menu_open: false,
        product_rose: false,
        target_ready: true,
        batch_permitted: false,
        receipt: None,
        checked_receipt: None,
    };
    assert_eq!(cycle.drive(&signals, &mut policy), CycleStep::Wait);
    assert_eq!(policy.cycles(), 0);
    assert_eq!(cycle.batch_request_id(), None);
    assert!(cycle.allows_native_begin());
    let wake = CycleSignals {
        batch_permitted: true,
        ..signals
    };
    assert_eq!(cycle.drive(&wake, &mut policy), CycleStep::Wait);
    assert_eq!(policy.cycles(), 0, "a wake cannot reuse the consumed roll");
    let next = CycleSignals {
        tick: 102,
        anim: Some(AnimInstruction {
            serial: 52,
            class: AnimClass::Roll,
        }),
        ..wake
    };
    assert_eq!(cycle.drive(&next, &mut policy), CycleStep::Batch);
    assert_eq!(policy.cycles(), 1);
}

#[test]
fn armed_path_has_no_allocation_over_ten_thousand_polls() {
    let allocations = allocation_counter::measure(|| {
        for _ in 0..2_000 {
            let mut run = replay(continuing());
            let id = run.batch(101, AnimClass::Roll);
            let receipt = dispatch_receipt(id, 101, Shape::Continuing, 3);
            assert_eq!(
                run.poll_receipts(
                    101,
                    None,
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    true,
                    Some(&receipt),
                    None
                ),
                CycleStep::Wait
            );
            assert_eq!(
                run.poll(
                    102,
                    Some(AnimClass::Fletching),
                    true,
                    false,
                    false,
                    false,
                    false,
                    false,
                    true
                ),
                CycleStep::Wait
            );
            assert_eq!(
                run.poll(103, None, false, false, false, false, false, false, true),
                CycleStep::Wait
            );
            assert_eq!(
                run.poll(
                    104,
                    Some(AnimClass::Roll),
                    false,
                    false,
                    false,
                    false,
                    false,
                    false,
                    true
                ),
                CycleStep::Batch
            );
            assert_eq!(run.policy.stats().verified, 1);
        }
    });
    assert_eq!(allocations, allocation_counter::AllocationInfo::default());
}

#[test]
fn core_layout_stays_within_the_inline_budget() {
    assert!(std::mem::size_of::<TickCycle>() <= 128);
    assert!(std::mem::size_of::<TickPolicy>() <= 96);
}
