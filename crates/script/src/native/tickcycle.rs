//! Allocation-free state machine for one observed tick-timed gather cycle.

use std::num::NonZeroU64;

use api::interact::server_ordered::ServerOrderedDispatch;

const VERDICT_WINDOW: u8 = 12;
const VERDICT_MASK: u32 = (1 << (VERDICT_WINDOW * 2)) - 1;
const EVIDENCE_RETENTION: i8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Continuing,
    Ending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RollSignal {
    Always,
    Never,
    Sometimes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnimClass {
    Fletching,
    Skill,
    Roll,
    Cast,
    Null,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OffReason {
    UnstableTiming,
    ContentRefusal,
    MembersOnlyArm,
    NoArmSupply,
    NoGain,
    NoDelayVariable,
    NotInRelease,
}

impl OffReason {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::UnstableTiming => "unstable-timing",
            Self::ContentRefusal => "content-refusal",
            Self::MembersOnlyArm => "members-only-arm",
            Self::NoArmSupply => "no-arm-supply",
            Self::NoGain => "no-gain",
            Self::NoDelayVariable => "no-delay-variable",
            Self::NotInRelease => "not-in-release",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Verified,
    Slip,
    Lost,
    Unobserved,
    Delayed,
    Paused,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CycleReason {
    OnSchedule,
    LateBatch,
    LateAct,
    LateRoll,
    HostRefused,
    LoneOpener,
    NoLanding,
    ActFailed,
    Fletch,
    MissingRoll,
    SkippedEdge,
    Delayed,
    Paused,
    ContentRefusal,
    PredictedRoll,
    BoundedRoll,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CycleDiagnostic {
    pub(crate) sequence: u32,
    pub(crate) verdict: Option<Verdict>,
    pub(crate) reason: Option<CycleReason>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CycleStats {
    pub verified: u32,
    pub slipped: u32,
    pub lost: u32,
    pub unobserved: u32,
    pub delayed: u32,
    pub paused: u32,
    pub wire_events: u32,
    pub fallback_clicks: u32,
    pub fallback_failed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OffUntil {
    reason: OffReason,
    tick: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TickPolicy {
    verdicts: u32,
    verdict_count: u8,
    cycles: u32,
    trips: u8,
    off_until: Option<OffUntil>,
    stats: CycleStats,
    last_cycle_ticks: Option<u32>,
    diagnostic: CycleDiagnostic,
}

impl TickPolicy {
    pub(crate) fn stats(&self) -> CycleStats {
        self.stats
    }

    pub(crate) const fn last_cycle_ticks(&self) -> Option<u32> {
        self.last_cycle_ticks
    }

    pub(crate) const fn diagnostic(&self) -> CycleDiagnostic {
        self.diagnostic
    }

    pub(crate) const fn cycles(&self) -> u32 {
        self.cycles
    }

    #[cfg(test)]
    pub(crate) const fn verdict_count(&self) -> u8 {
        self.verdict_count
    }

    pub(crate) const fn off_reason(&self) -> Option<OffReason> {
        match self.off_until {
            Some(off) => Some(off.reason),
            None => None,
        }
    }

    pub(crate) const fn off_until(&self) -> Option<u64> {
        match self.off_until {
            Some(off) => Some(off.tick),
            None => None,
        }
    }

    /// Disable the technique for the rest of the run for an applicability or content reason.
    pub(crate) fn disable(&mut self, reason: OffReason) {
        self.off_until = Some(OffUntil {
            reason,
            tick: u64::MAX,
        });
    }

    fn refresh_off(&mut self, tick: u64) -> Option<OffReason> {
        let off = self.off_until?;
        if off.tick != u64::MAX && tick >= off.tick {
            self.off_until = None;
            None
        } else {
            Some(off.reason)
        }
    }

    fn add_wire_events(&mut self, events: u8) {
        self.stats.wire_events = self.stats.wire_events.saturating_add(u32::from(events));
    }

    fn record(
        &mut self,
        verdict: Verdict,
        reason: CycleReason,
        tick: u64,
        cycle_ticks: Option<u32>,
    ) {
        self.diagnostic.sequence = self.diagnostic.sequence.saturating_add(1);
        self.diagnostic.verdict = Some(verdict);
        self.diagnostic.reason = Some(reason);
        if matches!(verdict, Verdict::Verified | Verdict::Unobserved) {
            self.last_cycle_ticks = cycle_ticks;
        }

        match verdict {
            Verdict::Verified => {
                self.stats.verified = self.stats.verified.saturating_add(1);
                self.push_verdict(1, tick);
            }
            Verdict::Slip => {
                self.stats.slipped = self.stats.slipped.saturating_add(1);
                self.push_verdict(2, tick);
            }
            Verdict::Lost => {
                self.stats.lost = self.stats.lost.saturating_add(1);
                self.push_verdict(3, tick);
            }
            Verdict::Unobserved => {
                self.stats.unobserved = self.stats.unobserved.saturating_add(1);
            }
            Verdict::Delayed => {
                self.stats.delayed = self.stats.delayed.saturating_add(1);
            }
            Verdict::Paused => {
                self.stats.paused = self.stats.paused.saturating_add(1);
            }
        }
    }

    fn push_verdict(&mut self, code: u32, tick: u64) {
        self.verdicts = ((self.verdicts << 2) | code) & VERDICT_MASK;
        self.verdict_count = self.verdict_count.saturating_add(1).min(VERDICT_WINDOW);

        let mut slips = 0;
        let mut lost = 0;
        for index in 0..self.verdict_count {
            match (self.verdicts >> (u32::from(index) * 2)) & 0b11 {
                2 => slips += 1,
                3 => lost += 1,
                _ => {}
            }
        }
        if slips < 3 && lost < 2 {
            return;
        }

        let trip = self.trips.saturating_add(1);
        self.trips = trip;
        let cooldown = match trip {
            1 => tick.saturating_add(80),
            2 => tick.saturating_add(160),
            _ => u64::MAX,
        };
        self.off_until = Some(OffUntil {
            reason: OffReason::UnstableTiming,
            tick: cooldown,
        });
        self.verdicts = 0;
        self.verdict_count = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CycleSpec {
    pub(crate) shape: Shape,
    pub(crate) roll_signal: RollSignal,
    pub(crate) delay_drop: bool,
    pub(crate) depletes: bool,
    pub(crate) native_arm: u8,
    pub(crate) act_min_d: u8,
    pub(crate) act_silent: bool,
}

impl CycleSpec {
    pub(crate) const fn new(
        shape: Shape,
        roll_signal: RollSignal,
        delay_drop: bool,
        depletes: bool,
        native_arm: u8,
        act_min_d: u8,
        act_silent: bool,
    ) -> Self {
        Self {
            shape,
            roll_signal,
            delay_drop,
            depletes,
            native_arm,
            act_min_d,
            act_silent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnimInstruction {
    pub(crate) serial: u32,
    pub(crate) class: AnimClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CycleStep {
    Wait,
    Batch,
    Click,
    Native(OffReason),
}

/// What the consumer observed during one poll. Bounded booleans mean newly seen at this poll.
pub(crate) struct CycleSignals<'a> {
    pub(crate) tick: u64,
    pub(crate) anim: Option<AnimInstruction>,
    pub(crate) act_line: bool,
    pub(crate) success_line: bool,
    pub(crate) fletch: bool,
    pub(crate) content_refusal: bool,
    pub(crate) menu_open: bool,
    pub(crate) product_rose: bool,
    pub(crate) target_ready: bool,
    /// The consumer can admit a complete batch now, without spending its budget first.
    pub(crate) batch_permitted: bool,
    /// Native completion tick and the unchanged API packet-clock receipt.
    pub(crate) receipt: Option<(u64, &'a ServerOrderedDispatch)>,
    pub(crate) checked_receipt: Option<(u64, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    AfterPlayerInfo,
    BeforePlayerInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Bound {
    lo: i8,
    hi: i8,
}

impl Bound {
    /// Label a first-seen packet relative to this poll's observed tick.
    fn label(position: Position, edge: bool) -> Self {
        match (position, edge) {
            (Position::AfterPlayerInfo, true) => Self { lo: -1, hi: 0 },
            (Position::AfterPlayerInfo, false) => Self { lo: 0, hi: 0 },
            (Position::BeforePlayerInfo, true) => Self { lo: 0, hi: 1 },
            (Position::BeforePlayerInfo, false) => Self { lo: 1, hi: 1 },
        }
    }

    /// `fletch` combines chat (before) and inventory/statistics (after) observations.
    fn label_fletch(edge: bool) -> Self {
        if edge {
            Self { lo: -1, hi: 1 }
        } else {
            Self { lo: 0, hi: 1 }
        }
    }

    fn contains_tick(&self, target: u64, current: u64) -> bool {
        let offset = i128::from(target) - i128::from(current);
        offset >= i128::from(self.lo) && offset <= i128::from(self.hi)
    }

    fn rebase(&mut self, delta: u64) -> bool {
        if delta > 127 {
            return false;
        }
        let delta = delta as i16;
        self.lo = (i16::from(self.lo) - delta).clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8;
        self.hi = (i16::from(self.hi) - delta).clamp(i16::from(i8::MIN), i16::from(i8::MAX)) as i8;
        self.hi >= -EVIDENCE_RETENTION
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Evidence {
    act_line: Option<Bound>,
    success_line: Option<Bound>,
    product: Option<Bound>,
    fletch: Option<Bound>,
}

impl Evidence {
    fn rebase(&mut self, delta: u64) {
        for bound in [
            &mut self.act_line,
            &mut self.success_line,
            &mut self.product,
            &mut self.fletch,
        ] {
            if let Some(value) = bound {
                if !value.rebase(delta) {
                    *bound = None;
                }
            }
        }
    }

    fn observe(&mut self, signals: &CycleSignals<'_>, edge: bool) {
        if signals.act_line {
            self.act_line = Some(Bound::label(Position::BeforePlayerInfo, edge));
        }
        if signals.success_line {
            self.success_line = Some(Bound::label(Position::BeforePlayerInfo, edge));
        }
        if signals.product_rose {
            self.product = Some(Bound::label(Position::AfterPlayerInfo, edge));
        }
        if signals.fletch {
            self.fletch = Some(Bound::label_fletch(edge));
        }
    }

    fn contains_act(&self, tick: u64, current: u64) -> bool {
        self.act_line
            .is_some_and(|bound| bound.contains_tick(tick, current))
    }

    fn success(&self, tick: u64, current: u64) -> bool {
        self.success_line
            .is_some_and(|bound| bound.contains_tick(tick, current))
            || self
                .product
                .is_some_and(|bound| bound.contains_tick(tick, current))
    }

    fn product_contains(&self, tick: u64, current: u64) -> bool {
        self.product
            .is_some_and(|bound| bound.contains_tick(tick, current))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObligationKind {
    Act,
    EndingClick,
    Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Obligation {
    kind: ObligationKind,
    not_before: i8,
    deadline: i8,
    request_id: Option<NonZeroU64>,
}

impl Obligation {
    const fn due(kind: ObligationKind, not_before: i8, deadline: i8) -> Self {
        Self {
            kind,
            not_before,
            deadline,
            request_id: None,
        }
    }

    fn rebase(&mut self) {
        self.not_before = self.not_before.saturating_sub(1);
        self.deadline = self.deadline.saturating_sub(1);
    }

    fn asked(&self) -> bool {
        self.request_id.is_some()
    }

    fn is_due_now(&self) -> bool {
        self.request_id.is_none() && self.not_before <= 0 && self.deadline >= 0
    }

    fn expired(&self) -> bool {
        self.request_id.is_none() && self.deadline < 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActState {
    Done,
    Owed(Obligation),
    Assumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Native {
        roll_due: Option<u64>,
        fallback: Option<Obligation>,
        pending: Option<Obligation>,
    },
    Idle {
        since: u64,
        fallback: Option<Obligation>,
        pending: Option<Obligation>,
    },
    Sent {
        sent: u64,
        blind: bool,
        null_edge: bool,
    },
    Armed {
        armed: u64,
        roll_due: u64,
        act: ActState,
        late: bool,
        null_edge: bool,
    },
}

/// A single timing cycle. The consumer parks this value between target machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TickCycle {
    spec: CycleSpec,
    phase: Phase,
    last_tick: u64,
    last_emit_tick: u64,
    cycle_started_tick: Option<u64>,
    anim_serial: u32,
    chat_cursor: i32,
    evidence: Evidence,
    pending_batch_id: Option<NonZeroU64>,
}

impl TickCycle {
    /// The current animation is a baseline, not a fresh event.
    pub(crate) const fn new(
        spec: CycleSpec,
        tick: u64,
        anim_serial: u32,
        chat_cursor: i32,
    ) -> Self {
        Self {
            spec,
            phase: Phase::Native {
                roll_due: None,
                fallback: None,
                pending: None,
            },
            last_tick: tick,
            last_emit_tick: tick,
            cycle_started_tick: None,
            anim_serial,
            chat_cursor,
            evidence: Evidence {
                act_line: None,
                success_line: None,
                product: None,
                fletch: None,
            },
            pending_batch_id: None,
        }
    }

    /// Advance the consumer's chat cursor after it has read all newer chat rows.
    pub(crate) const fn set_chat_cursor(&mut self, cursor: i32) {
        self.chat_cursor = cursor;
    }

    pub(crate) const fn chat_cursor(&self) -> i32 {
        self.chat_cursor
    }

    pub(crate) fn fresh_animation_at(&self, tick: u64, serial: u32) -> bool {
        tick == self.last_tick.saturating_add(1) && serial != self.anim_serial
    }

    pub(crate) fn allows_native_begin(&self) -> bool {
        matches!(
            self.phase,
            Phase::Native {
                fallback: None,
                pending: None,
                ..
            }
        ) && self.pending_batch_id.is_none()
    }

    /// A new native click starts a new cadence; retain the policy, not the parked baseline.
    pub(crate) fn begin_native(&mut self, tick: u64, anim_serial: u32, chat_cursor: i32) {
        debug_assert!(self.allows_native_begin());
        *self = Self::new(self.spec, tick, anim_serial, chat_cursor);
    }

    pub(crate) fn allows_stray_tend(&self) -> bool {
        matches!(self.phase, Phase::Native { .. } | Phase::Idle { .. })
    }

    pub(crate) fn batch_request_id(&self) -> Option<u64> {
        self.pending_batch_id.map(NonZeroU64::get)
    }

    pub(crate) fn checked_request_id(&self) -> Option<u64> {
        match self.phase {
            Phase::Armed {
                act: ActState::Owed(obligation),
                ..
            } => obligation.request_id.map(NonZeroU64::get),
            Phase::Native {
                fallback, pending, ..
            }
            | Phase::Idle {
                fallback, pending, ..
            } => match pending.and_then(|obligation| obligation.request_id) {
                Some(request_id) => Some(request_id.get()),
                None => fallback
                    .and_then(|obligation| obligation.request_id)
                    .map(NonZeroU64::get),
            },
            _ => None,
        }
    }

    pub(crate) fn click_kind(&self) -> Option<&'static str> {
        let kind = match self.phase {
            Phase::Armed {
                act: ActState::Owed(obligation),
                ..
            } => obligation.kind,
            Phase::Native {
                fallback, pending, ..
            }
            | Phase::Idle {
                fallback, pending, ..
            } => pending.or(fallback)?.kind,
            _ => return None,
        };
        Some(match kind {
            ObligationKind::Act => "Act",
            ObligationKind::EndingClick => "EndingClick",
            ObligationKind::Fallback => "Fallback",
        })
    }

    pub(crate) fn has_unresolved_requests(&self) -> bool {
        self.pending_batch_id.is_some() || self.checked_request_id().is_some()
    }

    #[cfg(test)]
    fn phase(&self) -> Phase {
        self.phase
    }

    /// Drive one edge or wake without allocation.
    pub(crate) fn drive(
        &mut self,
        signals: &CycleSignals<'_>,
        policy: &mut TickPolicy,
    ) -> CycleStep {
        let previous_tick = self.last_tick;
        let edge = previous_tick.checked_add(1) == Some(signals.tick);
        let skipped = signals.tick > previous_tick && !edge;
        let delta = signals.tick.saturating_sub(previous_tick);
        if delta != 0 {
            self.evidence.rebase(delta);
            self.rebase_obligations(delta);
            self.last_tick = signals.tick;
        }

        let fresh = if edge {
            match signals.anim {
                Some(animation) if animation.serial != self.anim_serial => {
                    self.anim_serial = animation.serial;
                    Some(animation.class)
                }
                _ => None,
            }
        } else if skipped {
            if let Some(animation) = signals.anim {
                self.anim_serial = animation.serial;
            }
            None
        } else {
            None
        };
        self.evidence.observe(signals, edge);

        if let Some(step) = self.consume_dispatch_receipt(signals.receipt, policy) {
            if signals.content_refusal {
                policy.disable(OffReason::ContentRefusal);
                return self.off_step(signals.tick, OffReason::ContentRefusal, policy);
            }
            return step;
        }
        if let Some(step) = self.consume_checked_receipt(signals.checked_receipt, policy) {
            if signals.content_refusal {
                policy.disable(OffReason::ContentRefusal);
                return self.off_step(signals.tick, OffReason::ContentRefusal, policy);
            }
            return step;
        }

        if signals.content_refusal {
            return self.handle_content_refusal(policy);
        }

        if skipped {
            return self.handle_skipped_edge(signals, policy);
        }

        let off_reason = policy.refresh_off(signals.tick);
        if signals.fletch {
            let live = matches!(self.phase, Phase::Sent { .. } | Phase::Armed { .. });
            if live {
                self.end_cycle(Verdict::Lost, CycleReason::Fletch, signals.tick, policy);
                self.start_fallback(signals.tick);
                return self.fallback_step(signals.tick);
            }
            if matches!(self.phase, Phase::Native { .. } | Phase::Idle { .. })
                && off_reason.is_none()
            {
                self.start_fallback(signals.tick);
            }
        }

        if let Some(reason) = off_reason {
            return self.off_step(signals.tick, reason, policy);
        }

        self.drive_phase(signals, fresh, edge, policy)
    }

    /// The consumer calls this only after its emission succeeds.
    pub(crate) fn note_emit(&mut self, tick: u64, step: CycleStep, request_id: u64) {
        let Some(request_id) = NonZeroU64::new(request_id) else {
            return;
        };
        self.last_emit_tick = tick;
        match (step, &mut self.phase) {
            (CycleStep::Batch, Phase::Sent { .. }) => {
                self.pending_batch_id = Some(request_id);
                self.cycle_started_tick = Some(tick);
            }
            (
                CycleStep::Click,
                Phase::Armed {
                    act: ActState::Owed(obligation),
                    ..
                },
            ) if obligation.request_id.is_none() => {
                obligation.request_id = Some(request_id);
            }
            (
                CycleStep::Click,
                Phase::Native {
                    fallback: Some(obligation),
                    ..
                },
            ) if obligation.request_id.is_none() => {
                obligation.request_id = Some(request_id);
            }
            _ => {}
        }
    }

    /// Pause a live cycle for the consumer's existing eat path.
    pub(crate) fn pause(&mut self, policy: &mut TickPolicy) {
        let old = self.phase;
        let (mut pending, fallback) = match old {
            Phase::Native {
                pending, fallback, ..
            }
            | Phase::Idle {
                pending, fallback, ..
            } => (pending, fallback.filter(Obligation::asked)),
            _ => (None, None),
        };
        if matches!(old, Phase::Sent { .. } | Phase::Armed { .. }) {
            self.record_cycle(policy, Verdict::Paused, CycleReason::Paused, self.last_tick);
            if let Phase::Armed {
                act: ActState::Owed(obligation),
                ..
            } = old
            {
                if obligation.asked() {
                    pending = Some(obligation);
                }
            }
        }
        self.phase = Phase::Native {
            roll_due: None,
            fallback,
            pending,
        };
    }

    fn rebase_obligations(&mut self, delta: u64) {
        if delta > 127 {
            return;
        }
        let count = delta as u8;
        match &mut self.phase {
            Phase::Native {
                fallback, pending, ..
            }
            | Phase::Idle {
                fallback, pending, ..
            } => {
                for obligation in [fallback, pending].into_iter().flatten() {
                    for _ in 0..count {
                        obligation.rebase();
                    }
                }
            }
            Phase::Armed {
                act: ActState::Owed(obligation),
                ..
            } => {
                for _ in 0..count {
                    obligation.rebase();
                }
            }
            _ => {}
        }
    }

    fn consume_dispatch_receipt(
        &mut self,
        receipt: Option<(u64, &ServerOrderedDispatch)>,
        policy: &mut TickPolicy,
    ) -> Option<CycleStep> {
        let (dispatch_tick, receipt) = receipt?;
        if self.pending_batch_id != NonZeroU64::new(receipt.request_id) {
            return None;
        }
        self.pending_batch_id = None;
        policy.add_wire_events(receipt.wire_events);
        if matches!(self.phase, Phase::Sent { .. }) && receipt.dispatched_prefix <= 1 {
            let reason = if receipt.dispatched_prefix == 0 {
                CycleReason::HostRefused
            } else {
                CycleReason::LoneOpener
            };
            self.end_cycle(Verdict::Lost, reason, dispatch_tick, policy);
            self.start_fallback(self.last_tick);
            return Some(self.fallback_step(self.last_tick));
        }
        None
    }

    fn consume_checked_receipt(
        &mut self,
        checked: Option<(u64, bool)>,
        policy: &mut TickPolicy,
    ) -> Option<CycleStep> {
        let (request_id, accepted) = checked?;
        let request_id = NonZeroU64::new(request_id)?;
        match self.phase {
            Phase::Armed {
                armed,
                roll_due,
                act: ActState::Owed(obligation),
                late,
                null_edge,
            } if obligation.request_id == Some(request_id) => {
                if accepted {
                    policy.add_wire_events(2);
                    // A continuing Act is only a recovered host click: its receipt
                    // cannot prove the original terminal row executed in time.
                    let recovered_act = self.spec.shape == Shape::Continuing
                        && obligation.kind == ObligationKind::Act;
                    self.phase = Phase::Armed {
                        armed,
                        roll_due,
                        act: ActState::Done,
                        late: late || recovered_act,
                        null_edge,
                    };
                    None
                } else {
                    self.end_cycle(
                        Verdict::Lost,
                        CycleReason::ActFailed,
                        self.last_tick,
                        policy,
                    );
                    if let Phase::Native { pending, .. } = &mut self.phase {
                        *pending = None;
                    }
                    self.start_fallback(self.last_tick);
                    Some(self.fallback_step(self.last_tick))
                }
            }
            Phase::Native {
                fallback: Some(obligation),
                ..
            }
            | Phase::Idle {
                fallback: Some(obligation),
                ..
            } if obligation.request_id == Some(request_id) => {
                if obligation.kind == ObligationKind::Fallback {
                    if accepted {
                        policy.add_wire_events(2);
                        policy.stats.fallback_clicks =
                            policy.stats.fallback_clicks.saturating_add(1);
                    } else {
                        policy.stats.fallback_failed =
                            policy.stats.fallback_failed.saturating_add(1);
                    }
                }
                match &mut self.phase {
                    Phase::Native { fallback, .. } | Phase::Idle { fallback, .. } => {
                        *fallback = None
                    }
                    _ => unreachable!(),
                }
                None
            }
            Phase::Native {
                pending: Some(obligation),
                ..
            }
            | Phase::Idle {
                pending: Some(obligation),
                ..
            } if obligation.request_id == Some(request_id) => {
                if accepted {
                    policy.add_wire_events(2);
                }
                match &mut self.phase {
                    Phase::Native { pending, .. } | Phase::Idle { pending, .. } => *pending = None,
                    _ => unreachable!(),
                }
                None
            }
            _ => None,
        }
    }

    fn handle_content_refusal(&mut self, policy: &mut TickPolicy) -> CycleStep {
        let tick = self.last_tick;
        let live = matches!(self.phase, Phase::Sent { .. } | Phase::Armed { .. });
        if live {
            self.end_cycle(Verdict::Lost, CycleReason::ContentRefusal, tick, policy);
            self.start_fallback(tick);
        } else if let Phase::Native {
            roll_due, fallback, ..
        } = &mut self.phase
        {
            *roll_due = None;
            *fallback = fallback.filter(Obligation::asked);
        }
        policy.disable(OffReason::ContentRefusal);
        self.off_step(tick, OffReason::ContentRefusal, policy)
    }

    fn handle_skipped_edge(
        &mut self,
        signals: &CycleSignals<'_>,
        policy: &mut TickPolicy,
    ) -> CycleStep {
        let tick = signals.tick;
        match self.phase {
            Phase::Sent { .. } => {
                self.phase = Phase::Native {
                    roll_due: None,
                    fallback: None,
                    pending: None,
                };
                self.record_cycle(policy, Verdict::Unobserved, CycleReason::SkippedEdge, tick);
            }
            Phase::Armed { act, .. } => {
                let pending = match act {
                    ActState::Owed(obligation) if obligation.asked() => Some(obligation),
                    _ => None,
                };
                self.phase = Phase::Native {
                    roll_due: None,
                    fallback: None,
                    pending,
                };
                self.record_cycle(policy, Verdict::Unobserved, CycleReason::SkippedEdge, tick);
            }
            Phase::Native {
                fallback, pending, ..
            } => {
                let fallback = match fallback {
                    Some(obligation) if obligation.asked() => Some(obligation),
                    Some(_) => {
                        policy.stats.fallback_failed =
                            policy.stats.fallback_failed.saturating_add(1);
                        None
                    }
                    None => None,
                };
                self.phase = Phase::Native {
                    roll_due: None,
                    fallback,
                    pending,
                };
            }
            Phase::Idle { .. } => {
                // A proven depleting outcome stays idle while the runner walks
                // to a replacement; only a live cycle is unobserved on a skip.
            }
        }

        // The higher-priority skipped-edge disposition does not hide a fletch
        // from an unresolved Asked action. Preserve it and create one recovery.
        if signals.fletch && self.has_asked_obligation() {
            self.start_fallback(tick);
            return self.fallback_step(tick);
        }
        CycleStep::Wait
    }

    fn has_asked_obligation(&self) -> bool {
        self.checked_request_id().is_some()
    }

    fn off_step(&mut self, tick: u64, reason: OffReason, policy: &mut TickPolicy) -> CycleStep {
        if let Some(step) = self.service_fallback(tick, policy) {
            return step;
        }
        CycleStep::Native(reason)
    }

    fn drive_phase(
        &mut self,
        signals: &CycleSignals<'_>,
        fresh: Option<AnimClass>,
        edge: bool,
        policy: &mut TickPolicy,
    ) -> CycleStep {
        let tick = signals.tick;
        match self.phase {
            Phase::Native {
                roll_due,
                fallback,
                pending,
            } => self.drive_native(signals, fresh, edge, roll_due, fallback, pending, policy),
            Phase::Idle { .. } => self.drive_idle(signals, policy),
            Phase::Sent {
                sent,
                blind,
                mut null_edge,
            } => {
                if !edge
                    && tick == sent.saturating_add(1)
                    && self.spec.delay_drop
                    && null_edge
                    && self.evidence.success(tick, tick)
                {
                    self.record_cycle(policy, Verdict::Delayed, CycleReason::Delayed, tick);
                    self.phase = Phase::Idle {
                        since: tick,
                        fallback: None,
                        pending: None,
                    };
                    return self.drive_idle(signals, policy);
                }
                if edge && tick == sent.saturating_add(1) {
                    let act_evidence =
                        self.spec.act_silent || self.evidence.contains_act(tick, tick);
                    if is_landing(fresh, act_evidence) {
                        let act = match self.spec.shape {
                            Shape::Ending => {
                                ActState::Owed(Obligation::due(ObligationKind::EndingClick, 1, 1))
                            }
                            Shape::Continuing if act_evidence => {
                                if self.spec.act_silent {
                                    ActState::Assumed
                                } else {
                                    ActState::Done
                                }
                            }
                            Shape::Continuing => ActState::Owed(Obligation::due(
                                ObligationKind::Act,
                                0,
                                1i8.saturating_sub(self.spec.act_min_d.min(i8::MAX as u8) as i8),
                            )),
                        };
                        self.phase = Phase::Armed {
                            armed: tick,
                            roll_due: tick.saturating_add(2),
                            act,
                            late: false,
                            null_edge: false,
                        };
                        return self.service_act(tick, edge, policy);
                    }
                    if self.spec.delay_drop && fresh == Some(AnimClass::Null) {
                        if self.evidence.success(tick, tick) {
                            self.record_cycle(policy, Verdict::Delayed, CycleReason::Delayed, tick);
                            self.phase = Phase::Idle {
                                since: tick,
                                fallback: None,
                                pending: None,
                            };
                            return self.drive_idle(signals, policy);
                        }
                        null_edge = true;
                    }
                    self.phase = Phase::Sent {
                        sent,
                        blind,
                        null_edge,
                    };
                    return CycleStep::Wait;
                }

                if edge && tick == sent.saturating_add(2) {
                    if is_late_landing(
                        self.spec.shape,
                        fresh,
                        self.evidence.contains_act(tick, tick),
                    ) {
                        if self.spec.shape == Shape::Continuing {
                            self.end_cycle(Verdict::Slip, CycleReason::LateBatch, tick, policy);
                            self.start_fallback(tick);
                            return self.fallback_step(tick);
                        }
                        self.phase = Phase::Armed {
                            armed: tick,
                            roll_due: tick.saturating_add(2),
                            act: ActState::Owed(Obligation::due(ObligationKind::EndingClick, 1, 1)),
                            late: true,
                            null_edge: false,
                        };
                        return CycleStep::Wait;
                    }
                    self.end_cycle(Verdict::Lost, CycleReason::NoLanding, tick, policy);
                    self.start_fallback(tick);
                    return self.fallback_step(tick);
                }
                CycleStep::Wait
            }
            Phase::Armed {
                armed,
                roll_due,
                act,
                late,
                null_edge,
            } => self.drive_armed(
                signals, fresh, edge, armed, roll_due, act, late, null_edge, policy,
            ),
        }
    }

    fn drive_idle(&mut self, signals: &CycleSignals<'_>, policy: &mut TickPolicy) -> CycleStep {
        if signals.target_ready && !signals.menu_open && self.checked_request_id().is_none() {
            self.start_batch(signals, false, policy)
        } else {
            CycleStep::Wait
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the Native phase fields stay explicit at this allocation-free transition boundary"
    )]
    fn drive_native(
        &mut self,
        signals: &CycleSignals<'_>,
        fresh: Option<AnimClass>,
        edge: bool,
        mut roll_due: Option<u64>,
        mut fallback: Option<Obligation>,
        pending: Option<Obligation>,
        policy: &mut TickPolicy,
    ) -> CycleStep {
        let tick = signals.tick;
        let pending_asked = pending.is_some_and(|obligation| obligation.asked())
            || fallback.is_some_and(|obligation| obligation.asked());
        let success = fresh == Some(AnimClass::Null)
            && self.spec.depletes
            && self.evidence.success(tick, tick);
        let rolled = is_rolled(fresh, self.evidence.contains_act(tick, tick), success);

        if fallback.is_some_and(|obligation| obligation.is_due_now()) && edge && rolled {
            fallback = None;
        }

        if edge {
            if let Some(due) = roll_due {
                if rolled && tick <= due {
                    if self.spec.depletes && success {
                        self.phase = Phase::Idle {
                            since: tick,
                            fallback: fallback.filter(Obligation::asked),
                            pending,
                        };
                        return self.drive_idle(signals, policy);
                    }
                    if !signals.menu_open && !pending_asked {
                        self.phase = Phase::Native {
                            roll_due: None,
                            fallback,
                            pending,
                        };
                        if fallback.is_some_and(|obligation| obligation.is_due_now()) {
                            return self.fallback_step(tick);
                        }
                        return self.start_batch(signals, false, policy);
                    }
                    self.phase = Phase::Native {
                        roll_due: None,
                        fallback,
                        pending,
                    };
                    return self
                        .service_fallback(tick, policy)
                        .unwrap_or(CycleStep::Wait);
                }
                if tick == due
                    && !rolled
                    && self.spec.roll_signal != RollSignal::Always
                    && !signals.menu_open
                    && !pending_asked
                {
                    self.phase = Phase::Native {
                        roll_due: None,
                        fallback,
                        pending,
                    };
                    if fallback.is_some_and(|obligation| obligation.is_due_now()) {
                        return self.fallback_step(tick);
                    }
                    return self.start_batch(signals, true, policy);
                }
                if tick == due && !rolled && self.spec.roll_signal == RollSignal::Always {
                    roll_due = None;
                }
                if tick > due && !rolled {
                    roll_due = None;
                }
            } else if rolled {
                if self.spec.depletes && success {
                    self.phase = Phase::Idle {
                        since: tick,
                        fallback: fallback.filter(Obligation::asked),
                        pending,
                    };
                    return self.drive_idle(signals, policy);
                }
                if !signals.menu_open && !pending_asked {
                    self.phase = Phase::Native {
                        roll_due: None,
                        fallback,
                        pending,
                    };
                    if fallback.is_some_and(|obligation| obligation.is_due_now()) {
                        return self.fallback_step(tick);
                    }
                    return self.start_batch(signals, false, policy);
                }
            }

            if fresh == Some(AnimClass::Cast) {
                roll_due =
                    Some(tick.saturating_add(u64::from(self.spec.native_arm.saturating_sub(1))));
            }
        }

        self.phase = Phase::Native {
            roll_due,
            fallback,
            pending,
        };
        if let Some(step) = self.service_fallback(tick, policy) {
            return step;
        }
        CycleStep::Wait
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the Armed phase fields stay explicit at this allocation-free transition boundary"
    )]
    fn drive_armed(
        &mut self,
        signals: &CycleSignals<'_>,
        fresh: Option<AnimClass>,
        edge: bool,
        armed: u64,
        roll_due: u64,
        mut act: ActState,
        late: bool,
        mut null_edge: bool,
        policy: &mut TickPolicy,
    ) -> CycleStep {
        let tick = signals.tick;

        // A continuing act first observed after its landing edge is ambiguous.
        // If a recovered Act receipt already resolved the one click obligation,
        // that late line cannot justify asking for another recovery click.
        if self.spec.shape == Shape::Continuing
            && !late
            && tick > armed
            && self.evidence.contains_act(tick, tick)
            && !self.evidence.contains_act(armed, tick)
        {
            self.end_cycle(Verdict::Slip, CycleReason::LateAct, tick, policy);
            self.start_fallback(tick);
            return self.fallback_step(tick);
        }

        if edge && tick == roll_due {
            let success = self.spec.depletes
                && fresh == Some(AnimClass::Null)
                && self.evidence.success(tick, tick);
            if is_rolled(fresh, self.evidence.contains_act(tick, tick), success) {
                return self.finish_armed_roll(
                    signals,
                    tick,
                    late,
                    success,
                    if late {
                        Self::late_slip_reason(self.spec.shape)
                    } else {
                        CycleReason::OnSchedule
                    },
                    policy,
                );
            }
            if fresh == Some(AnimClass::Null) && !success {
                null_edge = true;
                self.phase = Phase::Armed {
                    armed,
                    roll_due,
                    act,
                    late,
                    null_edge,
                };
                return CycleStep::Wait;
            }
            if self.spec.roll_signal != RollSignal::Always && !signals.menu_open {
                let (verdict, reason) = if late {
                    (Verdict::Slip, Self::late_slip_reason(self.spec.shape))
                } else {
                    (Verdict::Unobserved, CycleReason::PredictedRoll)
                };
                self.record_cycle(policy, verdict, reason, tick);
                if policy.refresh_off(tick).is_some() {
                    self.phase = Phase::Native {
                        roll_due: None,
                        fallback: None,
                        pending: self.asked_act(act),
                    };
                    return CycleStep::Native(OffReason::UnstableTiming);
                }
                self.phase = Phase::Native {
                    roll_due: None,
                    fallback: None,
                    pending: self.asked_act(act),
                };
                if late && self.spec.shape == Shape::Continuing {
                    return CycleStep::Wait;
                }
                return self.start_batch(signals, true, policy);
            }
        }

        if !edge
            && tick == roll_due
            && (null_edge || self.evidence.product_contains(roll_due, tick))
            && (self.evidence.success(roll_due, tick)
                || self.evidence.product_contains(roll_due, tick))
        {
            return self.finish_unobserved_roll(signals, tick, late, act, policy);
        }

        if edge && tick == roll_due.saturating_add(1) {
            let success = self.spec.depletes
                && fresh == Some(AnimClass::Null)
                && self.evidence.success(tick, tick);
            if is_rolled(fresh, self.evidence.contains_act(tick, tick), success) {
                return self.finish_armed_roll(
                    signals,
                    tick,
                    true,
                    success,
                    CycleReason::LateRoll,
                    policy,
                );
            }
            if self.evidence.product_contains(roll_due, tick) {
                let (verdict, reason) = if late {
                    (Verdict::Slip, Self::late_slip_reason(self.spec.shape))
                } else {
                    (Verdict::Unobserved, CycleReason::BoundedRoll)
                };
                self.record_cycle(policy, verdict, reason, tick);
                if late && self.spec.shape == Shape::Ending {
                    self.start_fallback(tick);
                    return self.fallback_step(tick);
                }
                self.phase = Phase::Native {
                    roll_due: None,
                    fallback: None,
                    pending: self.asked_act(act),
                };
                return CycleStep::Wait;
            }
            if self.spec.roll_signal == RollSignal::Always {
                self.end_cycle(Verdict::Lost, CycleReason::MissingRoll, tick, policy);
                self.start_fallback(tick);
                return self.fallback_step(tick);
            }
        }

        if let ActState::Owed(obligation) = act {
            if obligation.expired() && edge {
                self.end_cycle(Verdict::Lost, CycleReason::ActFailed, tick, policy);
                self.start_fallback(tick);
                return self.fallback_step(tick);
            }
            act = ActState::Owed(obligation);
        }
        self.phase = Phase::Armed {
            armed,
            roll_due,
            act,
            late,
            null_edge,
        };
        self.service_act(tick, edge, policy)
    }

    fn finish_unobserved_roll(
        &mut self,
        signals: &CycleSignals<'_>,
        tick: u64,
        late: bool,
        act: ActState,
        policy: &mut TickPolicy,
    ) -> CycleStep {
        self.record_cycle(
            policy,
            if late {
                Verdict::Slip
            } else {
                Verdict::Unobserved
            },
            if late {
                Self::late_slip_reason(self.spec.shape)
            } else {
                CycleReason::BoundedRoll
            },
            tick,
        );
        if policy.refresh_off(tick).is_some() {
            self.phase = Phase::Native {
                roll_due: None,
                fallback: None,
                pending: self.asked_act(act),
            };
            return CycleStep::Native(OffReason::UnstableTiming);
        }
        if self.spec.depletes && self.evidence.success(tick, tick) {
            self.phase = Phase::Idle {
                since: tick,
                fallback: None,
                pending: self.asked_act(act),
            };
            return self.drive_idle(signals, policy);
        }
        self.phase = Phase::Native {
            roll_due: None,
            fallback: None,
            pending: self.asked_act(act),
        };
        if late && self.spec.shape == Shape::Continuing {
            return CycleStep::Wait;
        }
        if !signals.menu_open {
            return self.start_batch(signals, true, policy);
        }
        CycleStep::Wait
    }

    fn finish_armed_roll(
        &mut self,
        signals: &CycleSignals<'_>,
        tick: u64,
        late: bool,
        success: bool,
        reason: CycleReason,
        policy: &mut TickPolicy,
    ) -> CycleStep {
        let pending = match self.phase {
            Phase::Armed { act, .. } => self.asked_act(act),
            _ => None,
        };
        self.record_cycle(
            policy,
            if late {
                Verdict::Slip
            } else {
                Verdict::Verified
            },
            reason,
            tick,
        );
        if let Some(reason) = policy.refresh_off(tick) {
            self.phase = Phase::Native {
                roll_due: None,
                fallback: None,
                pending,
            };
            return CycleStep::Native(reason);
        }
        if self.spec.depletes && success {
            self.phase = Phase::Idle {
                since: tick,
                fallback: None,
                pending,
            };
            return self.drive_idle(signals, policy);
        }
        self.phase = Phase::Native {
            roll_due: None,
            fallback: None,
            pending,
        };
        if signals.menu_open {
            CycleStep::Wait
        } else {
            self.start_batch(signals, false, policy)
        }
    }

    fn service_act(&mut self, tick: u64, edge: bool, _policy: &mut TickPolicy) -> CycleStep {
        match self.phase {
            Phase::Armed {
                act: ActState::Owed(obligation),
                ..
            } if obligation.is_due_now()
                && (obligation.kind != ObligationKind::EndingClick || edge)
                && self.last_emit_tick != tick =>
            {
                CycleStep::Click
            }
            _ => CycleStep::Wait,
        }
    }

    fn fallback_step(&mut self, tick: u64) -> CycleStep {
        match self.phase {
            Phase::Native {
                fallback: Some(obligation),
                ..
            } if obligation.is_due_now() && self.last_emit_tick != tick => CycleStep::Click,
            _ => CycleStep::Wait,
        }
    }

    fn service_fallback(&mut self, tick: u64, policy: &mut TickPolicy) -> Option<CycleStep> {
        let Phase::Native {
            fallback: Some(obligation),
            roll_due,
            pending,
        } = self.phase
        else {
            return None;
        };
        if obligation.expired() {
            policy.stats.fallback_failed = policy.stats.fallback_failed.saturating_add(1);
            self.phase = Phase::Native {
                roll_due,
                fallback: None,
                pending,
            };
            return Some(CycleStep::Wait);
        }
        if obligation.is_due_now() && self.last_emit_tick != tick {
            return Some(CycleStep::Click);
        }
        None
    }

    fn start_batch(
        &mut self,
        signals: &CycleSignals<'_>,
        blind: bool,
        policy: &mut TickPolicy,
    ) -> CycleStep {
        if !signals.batch_permitted || self.checked_request_id().is_some() {
            return CycleStep::Wait;
        }
        let tick = signals.tick;
        self.evidence = Evidence::default();
        self.cycle_started_tick = None;
        self.phase = Phase::Sent {
            sent: tick,
            blind,
            null_edge: false,
        };
        policy.cycles = policy.cycles.saturating_add(1);
        CycleStep::Batch
    }

    fn start_fallback(&mut self, tick: u64) {
        let (pending, existing) = match self.phase {
            Phase::Native {
                fallback, pending, ..
            }
            | Phase::Idle {
                fallback, pending, ..
            } => (pending, fallback),
            Phase::Armed {
                act: ActState::Owed(obligation),
                ..
            } if obligation.asked() => (Some(obligation), None),
            _ => (None, None),
        };
        let fallback = existing.or_else(|| Some(Obligation::due(ObligationKind::Fallback, 0, 1)));
        self.phase = Phase::Native {
            roll_due: None,
            fallback,
            pending,
        };
        let _ = tick;
    }

    fn end_cycle(
        &mut self,
        verdict: Verdict,
        reason: CycleReason,
        tick: u64,
        policy: &mut TickPolicy,
    ) {
        let pending = match self.phase {
            Phase::Armed {
                act: ActState::Owed(obligation),
                ..
            } if obligation.asked() => Some(obligation),
            Phase::Native { pending, .. } => pending,
            _ => None,
        };
        self.record_cycle(policy, verdict, reason, tick);
        self.phase = Phase::Native {
            roll_due: None,
            fallback: None,
            pending,
        };
    }

    fn record_cycle(
        &mut self,
        policy: &mut TickPolicy,
        verdict: Verdict,
        reason: CycleReason,
        tick: u64,
    ) {
        // The roll can arrive before a recovery Act's receipt. Keep the
        // existing pending-owner and emission rules, but never certify that
        // continuing cycle as if its original terminal row had been proved.
        let (verdict, reason) = if matches!(verdict, Verdict::Verified | Verdict::Unobserved)
            && self.spec.shape == Shape::Continuing
            && matches!(self.phase, Phase::Armed { act: ActState::Owed(obligation), .. }
                if obligation.kind == ObligationKind::Act)
        {
            (Verdict::Slip, CycleReason::LateAct)
        } else {
            (verdict, reason)
        };
        let cycle_ticks = if matches!(verdict, Verdict::Verified | Verdict::Unobserved) {
            self.cycle_started_tick
                .and_then(|started| tick.checked_sub(started))
                .and_then(|duration| (duration <= u64::from(u32::MAX)).then_some(duration as u32))
        } else {
            None
        };
        policy.record(verdict, reason, tick, cycle_ticks);
        self.cycle_started_tick = None;
    }

    fn late_slip_reason(shape: Shape) -> CycleReason {
        match shape {
            Shape::Continuing => CycleReason::LateAct,
            Shape::Ending => CycleReason::LateBatch,
        }
    }

    fn asked_act(&self, act: ActState) -> Option<Obligation> {
        match act {
            ActState::Owed(obligation) if obligation.asked() => Some(obligation),
            _ => None,
        }
    }
}

fn is_landing(fresh: Option<AnimClass>, act_evidence: bool) -> bool {
    matches!(fresh, Some(AnimClass::Fletching)) || (fresh == Some(AnimClass::Skill) && act_evidence)
}

fn is_late_landing(shape: Shape, fresh: Option<AnimClass>, act_evidence: bool) -> bool {
    match shape {
        Shape::Continuing => {
            matches!(fresh, Some(AnimClass::Fletching | AnimClass::Cast))
                || (fresh == Some(AnimClass::Skill) && act_evidence)
        }
        Shape::Ending => fresh == Some(AnimClass::Fletching),
    }
}

fn is_rolled(fresh: Option<AnimClass>, act_line: bool, success: bool) -> bool {
    fresh == Some(AnimClass::Roll) || (fresh == Some(AnimClass::Skill) && !act_line) || success
}

#[cfg(test)]
#[path = "tickcycle_tests.rs"]
mod tests;
