//! Exhaustive bounded model of the game-TLS worker/reactor handoff.
//!
//! This is a finite scheduler, not a timed OS test. Its three actors are one
//! producer, one handshake worker, and one reactor. Each successor edge is one
//! atomic scheduler transition. The WouldBlock readiness recheck and condvar
//! release share the handoff lock; resource drops, mio wake calls, and K2
//! notifications are separate transitions after their relevant lock releases.
//!
//! The scheduler is weakly fair for continuously enabled finite actions. A
//! returning handshake turn has one of four oracle outcomes. An opaque turn
//! has no return transition: revocation, deadline publication, and TCP queue
//! release must still progress, but physical close/ack cannot be required
//! until that verifier call returns. Poll has a bounded timeout in the correct
//! model. No wall-time or OS-scheduling claim follows from this model.

use std::collections::{HashMap, VecDeque};
use std::fmt;

const FIRST: usize = 0;
const SECOND: usize = 1;
const NO_ID: u8 = u8::MAX;
const NO_TIME: u8 = u8::MAX;
const MAX_EDGE_PUBLICATIONS_PER_DIRECTION: u8 = 2;
const MAX_TURN_ENTRIES_PER_IDENTITY: u8 = 2;
const HANDSHAKE_BUDGET: u8 = 5;
const MIN_WSS_REMAINING: u8 = 5;
const MAX_EXPLORED_STATES: usize = 400_000;

const OWNER_REACTOR: u8 = 1 << 0;
const OWNER_HANDOFF_CELL: u8 = 1 << 1;
const OWNER_WORKER: u8 = 1 << 2;
const OWNER_RESULT_CELL: u8 = 1 << 3;
const OWNER_CLOSED: u8 = 1 << 4;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TransferKey {
    generation: u8,
    binding: u8,
    transfer_seq: u8,
}

impl TransferKey {
    const NONE: Self = Self {
        generation: 0,
        binding: 0,
        transfer_seq: 0,
    };

    fn game(identity: usize) -> Self {
        Self {
            generation: (identity + 1) as u8,
            binding: 41,
            transfer_seq: (identity + 1) as u8,
        }
    }

    fn later_tcp() -> Self {
        Self {
            generation: 3,
            binding: 41,
            transfer_seq: 3,
        }
    }
}

impl fmt::Display for TransferKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "generation={},binding={},transfer_seq={}",
            self.generation, self.binding, self.transfer_seq
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Phase {
    Empty,
    Pending,
    Running,
    Waiting,
    Returned,
    Accepted,
    Released,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Revocation {
    Cancelled,
    Deadline,
    Shutdown,
}

impl fmt::Display for Revocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("cancelled"),
            Self::Deadline => f.write_str("deadline"),
            Self::Shutdown => f.write_str("shutdown"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Turn {
    Idle,
    Returning,
    Opaque,
    WouldBlock,
    SyntheticLimit,
    Success,
    Failure,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum CellOutcome {
    Empty,
    Success,
    Failure,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum CapacityPhase {
    Absent,
    Requested,
    Registered,
    Sleeping,
    Ready,
    Used,
    Refused,
    Expired,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum TcpPhase {
    Absent,
    Queued,
    Eligible,
    Pending,
    Accepted,
    Released,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct QueueProof {
    published: bool,
    key: TransferKey,
    published_at: u8,
    consumed: bool,
    consume_count: u8,
}

impl QueueProof {
    const EMPTY: Self = Self {
        published: false,
        key: TransferKey::NONE,
        published_at: NO_TIME,
        consumed: false,
        consume_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Attempt {
    key: TransferKey,
    phase: Phase,
    has_stream: bool,
    owner_mask: u8,
    ready_seq: [u8; 2],
    worker_snapshot: [u8; 2],
    call_snapshot: [u8; 2],
    runnable: bool,
    worker_active: bool,
    worker_sleeping: bool,
    worker_notified: bool,
    worker_epoch: u8,
    worker_sleep_epoch: u8,
    worker_timeout_scheduled: bool,
    worker_timeout_ready: bool,
    turn: Turn,
    turn_entries: u8,
    last_would_block: bool,
    start_time: u8,
    original_deadline: u8,
    revocation: Option<Revocation>,
    revoked_at: u8,
    proof: QueueProof,
    queue_credit: bool,
    queue_settlement_count: u8,
    queue_settlement_key: TransferKey,
    lane_credit: bool,
    physical_credit: bool,
    close_count: u8,
    drop_count: u8,
    physical_ack_count: u8,
    retirement_ack: bool,
    accepted_count: u8,
    accepted_key: TransferKey,
    accepted_at: u8,
    failure_ready: bool,
}

impl Attempt {
    const EMPTY: Self = Self {
        key: TransferKey::NONE,
        phase: Phase::Empty,
        has_stream: false,
        owner_mask: 0,
        ready_seq: [0; 2],
        worker_snapshot: [0; 2],
        call_snapshot: [0; 2],
        runnable: false,
        worker_active: false,
        worker_sleeping: false,
        worker_notified: false,
        worker_epoch: 0,
        worker_sleep_epoch: 0,
        worker_timeout_scheduled: false,
        worker_timeout_ready: false,
        turn: Turn::Idle,
        turn_entries: 0,
        last_would_block: false,
        start_time: 0,
        original_deadline: 0,
        revocation: None,
        revoked_at: NO_TIME,
        proof: QueueProof::EMPTY,
        queue_credit: false,
        queue_settlement_count: 0,
        queue_settlement_key: TransferKey::NONE,
        lane_credit: false,
        physical_credit: false,
        close_count: 0,
        drop_count: 0,
        physical_ack_count: 0,
        retirement_ack: false,
        accepted_count: 0,
        accepted_key: TransferKey::NONE,
        accepted_at: NO_TIME,
        failure_ready: false,
    };

    fn admitted(identity: usize, now: u8) -> Self {
        Self {
            key: TransferKey::game(identity),
            phase: Phase::Pending,
            has_stream: true,
            owner_mask: OWNER_HANDOFF_CELL,
            runnable: true,
            start_time: now,
            original_deadline: now
                .checked_add(2 * HANDSHAKE_BUDGET)
                .expect("finite model deadline must not wrap"),
            queue_credit: true,
            lane_credit: true,
            physical_credit: true,
            ..Self::EMPTY
        }
    }

    fn readiness_changed(self) -> bool {
        self.ready_seq != self.call_snapshot
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ResultCell {
    occupied: bool,
    identity: TransferKey,
    source: TransferKey,
    outcome: CellOutcome,
    read: bool,
}

impl ResultCell {
    const EMPTY: Self = Self {
        occupied: false,
        identity: TransferKey::NONE,
        source: TransferKey::NONE,
        outcome: CellOutcome::Empty,
        read: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct CapacityWaiter {
    phase: CapacityPhase,
    checked: bool,
    observed_free: bool,
    observed_epoch: u8,
    register_epoch: u8,
    recheck_epoch: u8,
    rechecked: bool,
    sleeping: bool,
    wake_pending: bool,
    budget_refusal_eligible: bool,
    deadline: u8,
    typed_refusal_count: u8,
    charged_start_count: u8,
    expiration_count: u8,
    cancellation_count: u8,
}

impl CapacityWaiter {
    const EMPTY: Self = Self {
        phase: CapacityPhase::Absent,
        checked: false,
        observed_free: false,
        observed_epoch: 0,
        register_epoch: 0,
        recheck_epoch: 0,
        rechecked: false,
        sleeping: false,
        wake_pending: false,
        budget_refusal_eligible: false,
        deadline: 0,
        typed_refusal_count: 0,
        charged_start_count: 0,
        expiration_count: 0,
        cancellation_count: 0,
    };

    fn registered(self) -> bool {
        matches!(self.phase, CapacityPhase::Registered | CapacityPhase::Sleeping)
    }

    fn live(self) -> bool {
        matches!(
            self.phase,
            CapacityPhase::Requested
                | CapacityPhase::Registered
                | CapacityPhase::Sleeping
                | CapacityPhase::Ready
                | CapacityPhase::Refused
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TcpRetirement {
    phase: TcpPhase,
    key: TransferKey,
    has_stream: bool,
    owner_mask: u8,
    close_count: u8,
    drop_count: u8,
    physical_ack_count: u8,
    acknowledgement_key: TransferKey,
    accepted_count: u8,
    accepted_key: TransferKey,
}

impl TcpRetirement {
    const EMPTY: Self = Self {
        phase: TcpPhase::Absent,
        key: TransferKey::NONE,
        has_stream: false,
        owner_mask: 0,
        close_count: 0,
        drop_count: 0,
        physical_ack_count: 0,
        acknowledgement_key: TransferKey::NONE,
        accepted_count: 0,
        accepted_key: TransferKey::NONE,
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct State {
    now: u8,
    next_transfer_seq: u8,
    authorized_count: u8,
    slot: u8,
    admission_closed: bool,
    attempts: [Attempt; 2],
    edge_publications: [u8; 2],
    result_cell: ResultCell,
    completion_hint: bool,
    wake_due: bool,
    wake_token: bool,
    reactor_sleeping: bool,
    poll_timeout_scheduled: bool,
    drained_wake: bool,
    k2_epoch: u8,
    capacity_notify_due: bool,
    capacity: CapacityWaiter,
    tcp: TcpRetirement,
    delayed_old_result: bool,
    stale_proof_delivered: bool,
}

impl State {
    const INITIAL: Self = Self {
        now: 0,
        next_transfer_seq: 1,
        authorized_count: 0,
        slot: NO_ID,
        admission_closed: false,
        attempts: [Attempt::EMPTY; 2],
        edge_publications: [0; 2],
        result_cell: ResultCell::EMPTY,
        completion_hint: false,
        wake_due: false,
        wake_token: false,
        reactor_sleeping: false,
        poll_timeout_scheduled: false,
        drained_wake: false,
        k2_epoch: 0,
        capacity_notify_due: false,
        capacity: CapacityWaiter::EMPTY,
        tcp: TcpRetirement::EMPTY,
        delayed_old_result: false,
        stale_proof_delivered: false,
    };

    fn lane_available(self) -> bool {
        self.slot == NO_ID && self.attempts.iter().all(|attempt| !attempt.lane_credit)
    }

    fn active_attempt(self) -> Option<usize> {
        (self.slot != NO_ID).then_some(self.slot as usize)
    }

    fn next_clock(self) -> Option<u8> {
        let mut next = None;
        let mut consider = |time: u8| {
            if time > self.now && next.is_none_or(|current| time < current) {
                next = Some(time);
            }
        };

        for attempt in self.attempts {
            if attempt.phase == Phase::Empty
                || matches!(attempt.phase, Phase::Accepted | Phase::Released)
            {
                continue;
            }
            consider(
                attempt
                    .start_time
                    .checked_add(HANDSHAKE_BUDGET)
                    .expect("finite model cancellation time must not wrap"),
            );
            consider(attempt.original_deadline);
        }

        if self.capacity.live() {
            if let Some(refusal_time) = self.capacity.deadline.checked_sub(MIN_WSS_REMAINING - 1) {
                consider(refusal_time);
            }
            consider(self.capacity.deadline);
        }
        next
    }

    fn settle_queue(&mut self, identity: usize, key: TransferKey) {
        let attempt = &mut self.attempts[identity];
        if attempt.queue_credit {
            attempt.queue_credit = false;
            attempt.queue_settlement_count = attempt
                .queue_settlement_count
                .checked_add(1)
                .expect("queue settlement counter must not wrap");
            attempt.queue_settlement_key = key;
        }
    }

    fn resolve_capacity(&mut self) {
        let available = self.lane_available();
        let now = self.now;
        let epoch = self.k2_epoch;
        let capacity = &mut self.capacity;
        capacity.sleeping = false;
        capacity.wake_pending = false;
        capacity.rechecked = true;
        capacity.recheck_epoch = epoch;
        if now >= capacity.deadline {
            capacity.phase = CapacityPhase::Expired;
            capacity.budget_refusal_eligible = false;
            capacity.expiration_count = 1;
        } else if available {
            if capacity.deadline - now < MIN_WSS_REMAINING {
                capacity.phase = CapacityPhase::Refused;
                capacity.budget_refusal_eligible = true;
                capacity.typed_refusal_count = 1;
            } else {
                capacity.phase = CapacityPhase::Ready;
                capacity.budget_refusal_eligible = false;
            }
        } else {
            capacity.phase = CapacityPhase::Registered;
        }
    }

    fn successors(self, mutation: Mutation) -> Vec<(Action, Self)> {
        let mut next = Vec::new();

        if !self.admission_closed && self.slot == NO_ID && self.lane_available() {
            if self.authorized_count == 0 && self.next_transfer_seq == 1 {
                let mut state = self;
                state.attempts[FIRST] = Attempt::admitted(FIRST, self.now);
                state.slot = FIRST as u8;
                state.authorized_count = 1;
                state.next_transfer_seq = state
                    .next_transfer_seq
                    .checked_add(1)
                    .expect("transfer sequence must not wrap");
                next.push((Action::Admit(FIRST as u8), state));
            }
            if self.authorized_count == 1
                && self.next_transfer_seq == 2
                && matches!(self.attempts[FIRST].phase, Phase::Accepted | Phase::Released)
                && (self.attempts[FIRST].phase == Phase::Accepted
                    || self.attempts[FIRST].retirement_ack)
                && matches!(self.capacity.phase, CapacityPhase::Absent | CapacityPhase::Ready)
            {
                if self.capacity.phase == CapacityPhase::Ready
                    && self.capacity.deadline.saturating_sub(self.now) < MIN_WSS_REMAINING
                {
                    let mut state = self;
                    state.capacity.phase = CapacityPhase::Refused;
                    state.capacity.typed_refusal_count = 1;
                    state.capacity.budget_refusal_eligible = true;
                    next.push((Action::RefuseWssAtGrant, state));
                } else {
                    let mut state = self;
                    state.attempts[SECOND] = Attempt::admitted(SECOND, self.now);
                    state.slot = SECOND as u8;
                    state.authorized_count = 2;
                    state.next_transfer_seq = state
                        .next_transfer_seq
                        .checked_add(1)
                        .expect("transfer sequence must not wrap");
                    if state.capacity.phase == CapacityPhase::Ready {
                        state.capacity.phase = CapacityPhase::Used;
                        state.capacity.charged_start_count = state
                            .capacity
                            .charged_start_count
                            .checked_add(1)
                            .expect("capacity charge counter must not wrap");
                        state.capacity.budget_refusal_eligible = false;
                    }
                    next.push((Action::Admit(SECOND as u8), state));
                }
            }
        }

        if let Some(time) = self.next_clock() {
            let mut state = self;
            state.now = time;
            next.push((Action::AdvanceTime(time), state));
        }

        if let Some(identity) = self.active_attempt() {
            let attempt = self.attempts[identity];

            if attempt.phase == Phase::Pending && !attempt.worker_active {
                let mut state = self;
                let worker = &mut state.attempts[identity];
                worker.phase = Phase::Running;
                worker.owner_mask = OWNER_WORKER;
                worker.worker_active = true;
                worker.worker_sleeping = false;
                worker.worker_notified = false;
                worker.worker_sleep_epoch = worker.worker_epoch;
                worker.worker_timeout_scheduled = false;
                worker.worker_timeout_ready = false;
                worker.runnable = false;
                worker.worker_snapshot = worker.ready_seq;
                next.push((Action::Claim(identity as u8), state));
            }

            if attempt.phase == Phase::Running
                && attempt.worker_active
                && attempt.turn == Turn::Idle
                && attempt.turn_entries < MAX_TURN_ENTRIES_PER_IDENTITY
            {
                for (opaque, turn) in [(false, Turn::Returning), (true, Turn::Opaque)] {
                    let mut state = self;
                    let worker = &mut state.attempts[identity];
                    worker.turn_entries = worker
                        .turn_entries
                        .checked_add(1)
                        .expect("turn entry counter must not wrap");
                    worker.turn = turn;
                    worker.call_snapshot = worker.ready_seq;
                    next.push((Action::EnterTurn(identity as u8, opaque), state));
                }
            }

            if attempt.turn == Turn::Returning {
                for (turn, action) in [
                    (Turn::WouldBlock, ReturnKind::WouldBlock),
                    (Turn::SyntheticLimit, ReturnKind::SyntheticLimit),
                    (Turn::Success, ReturnKind::Success),
                    (Turn::Failure, ReturnKind::Failure),
                ] {
                    let mut state = self;
                    state.attempts[identity].turn = turn;
                    next.push((Action::Return(identity as u8, action), state));
                }
            }

            if attempt.phase == Phase::Running
                && attempt.worker_active
                && attempt.turn == Turn::WouldBlock
            {
                let changed = attempt.readiness_changed() || attempt.runnable;
                let mut state = self;
                let worker = &mut state.attempts[identity];
                worker.last_would_block = true;
                if changed && mutation != Mutation::WrongClear {
                    worker.phase = Phase::Pending;
                    worker.owner_mask = OWNER_HANDOFF_CELL;
                    worker.worker_active = false;
                    worker.worker_sleeping = false;
                    worker.worker_notified = false;
                    worker.worker_timeout_scheduled = false;
                    worker.worker_timeout_ready = false;
                    worker.runnable = true;
                    worker.worker_snapshot = worker.ready_seq;
                    worker.worker_sleep_epoch = worker.worker_epoch;
                    worker.turn = Turn::Idle;
                } else if changed {
                    // Deliberately omit the sequence recheck, clear runnable,
                    // and atomically sleep. The old snapshot remains dirty.
                    worker.phase = Phase::Waiting;
                    worker.worker_sleeping = true;
                    worker.worker_notified = false;
                    worker.worker_timeout_scheduled = true;
                    worker.worker_timeout_ready = false;
                    worker.worker_sleep_epoch = worker.worker_epoch;
                    worker.runnable = false;
                    worker.turn = Turn::Idle;
                } else {
                    // The real condvar wait releases K-handoff atomically with
                    // the locked readiness recheck, so a producer cannot slip
                    // an edge into a check-then-sleep gap.
                    worker.phase = Phase::Waiting;
                    worker.worker_sleeping = true;
                    worker.worker_notified = false;
                    worker.worker_timeout_scheduled = true;
                    worker.worker_timeout_ready = false;
                    worker.worker_sleep_epoch = worker.worker_epoch;
                    worker.runnable = false;
                    worker.turn = Turn::Idle;
                }
                next.push((Action::LockedRecheckAndCondvarRelease(identity as u8), state));
            }

            if attempt.phase == Phase::Running
                && attempt.worker_active
                && attempt.turn == Turn::SyntheticLimit
                && attempt.revocation.is_none()
            {
                let mut state = self;
                let worker = &mut state.attempts[identity];
                worker.phase = Phase::Pending;
                worker.owner_mask = OWNER_HANDOFF_CELL;
                worker.worker_active = false;
                worker.worker_sleeping = false;
                worker.worker_notified = false;
                worker.runnable = true;
                worker.worker_snapshot = worker.ready_seq;
                worker.turn = Turn::Idle;
                next.push((Action::RequeueSyntheticLimit(identity as u8), state));
            }

            if attempt.phase == Phase::Waiting
                && attempt.worker_active
                && (attempt.worker_notified || attempt.worker_timeout_ready)
            {
                let mut state = self;
                let worker = &mut state.attempts[identity];
                if worker.revocation.is_some() {
                    worker.worker_sleeping = false;
                    worker.worker_notified = false;
                    worker.worker_timeout_scheduled = false;
                    worker.worker_timeout_ready = false;
                    worker.worker_sleep_epoch = worker.worker_epoch;
                    worker.turn = Turn::Idle;
                    worker.phase = Phase::Released;
                } else if worker.ready_seq != worker.worker_snapshot
                    || worker.runnable
                    || worker.worker_epoch != worker.worker_sleep_epoch
                {
                    worker.phase = Phase::Pending;
                    worker.owner_mask = OWNER_HANDOFF_CELL;
                    worker.worker_active = false;
                    worker.worker_sleeping = false;
                    worker.worker_notified = false;
                    worker.worker_timeout_scheduled = false;
                    worker.worker_timeout_ready = false;
                    worker.runnable = true;
                    worker.worker_snapshot = worker.ready_seq;
                    worker.worker_sleep_epoch = worker.worker_epoch;
                    worker.turn = Turn::Idle;
                } else {
                    worker.worker_sleeping = true;
                    worker.worker_notified = false;
                    worker.worker_timeout_scheduled = true;
                    worker.worker_timeout_ready = false;
                    worker.worker_sleep_epoch = worker.worker_epoch;
                }
                next.push((Action::WorkerWakeAndLockedRecheck(identity as u8), state));
            }

            if attempt.phase == Phase::Waiting
                && attempt.worker_active
                && attempt.worker_sleeping
                && attempt.worker_timeout_scheduled
            {
                let mut state = self;
                let worker = &mut state.attempts[identity];
                worker.worker_sleeping = false;
                worker.worker_timeout_scheduled = false;
                worker.worker_timeout_ready = true;
                next.push((Action::WorkerBoundedCondvarTimeout(identity as u8), state));
            }

            let can_observe_revocation = attempt.phase == Phase::Released
                && attempt.worker_active
                && attempt.turn != Turn::Opaque
                && attempt.turn != Turn::Returning
                && (!attempt.worker_sleeping || attempt.worker_notified);
            if can_observe_revocation {
                let mut state = self;
                let worker = &mut state.attempts[identity];
                worker.worker_sleeping = false;
                worker.worker_notified = false;
                worker.worker_timeout_scheduled = false;
                worker.worker_timeout_ready = false;
                worker.turn = Turn::Idle;
                next.push((Action::WorkerObservesRevocation(identity as u8), state));
            }

            if attempt.phase == Phase::Running
                && attempt.worker_active
                && attempt.turn == Turn::Failure
                && attempt.revocation.is_none()
            {
                let mut state = self;
                Self::close_and_drop(&mut state, identity);
                state.attempts[identity].failure_ready = true;
                state.attempts[identity].worker_active = false;
                state.attempts[identity].turn = Turn::Idle;
                next.push((Action::WorkerCloseDropFailure(identity as u8), state));
            }

            if attempt.phase == Phase::Released
                && attempt.has_stream
                && attempt.drop_count == 0
                && attempt.turn != Turn::Opaque
                && attempt.turn != Turn::Returning
                && (!attempt.worker_active
                    || (!attempt.worker_sleeping && attempt.turn != Turn::Idle)
                    || (!attempt.worker_sleeping && !attempt.worker_notified))
            {
                let mut state = self;
                Self::close_and_drop(&mut state, identity);
                state.attempts[identity].worker_active = false;
                state.attempts[identity].worker_sleeping = false;
                state.attempts[identity].worker_notified = false;
                state.attempts[identity].worker_timeout_scheduled = false;
                state.attempts[identity].worker_timeout_ready = false;
                state.attempts[identity].turn = Turn::Idle;
                next.push((Action::CloseDropRevokedStream(identity as u8), state));
            }

            if attempt.failure_ready
                && attempt.physical_ack_count == 1
                && attempt.phase == Phase::Running
                && !self.result_cell.occupied
            {
                let mut state = self;
                let worker = &mut state.attempts[identity];
                worker.phase = Phase::Returned;
                worker.worker_active = false;
                state.result_cell = ResultCell {
                    occupied: true,
                    identity: attempt.key,
                    source: attempt.key,
                    outcome: CellOutcome::Failure,
                    read: false,
                };
                state.completion_hint = true;
                state.wake_due = true;
                next.push((Action::PublishFailureResult(identity as u8), state));
            }

            if attempt.turn == Turn::Success
                && attempt.phase == Phase::Running
                && attempt.worker_active
                && attempt.revocation.is_none()
                && !self.result_cell.occupied
            {
                let mut state = self;
                let worker = &mut state.attempts[identity];
                worker.phase = Phase::Returned;
                worker.owner_mask = OWNER_RESULT_CELL;
                worker.worker_active = false;
                worker.worker_sleeping = false;
                worker.turn = Turn::Idle;
                state.result_cell = ResultCell {
                    occupied: true,
                    identity: attempt.key,
                    source: attempt.key,
                    outcome: CellOutcome::Success,
                    read: false,
                };
                state.completion_hint = true;
                state.wake_due = true;
                if identity == FIRST {
                    // A delayed old-key completion tuple has no stream custody.
                    // It tests that a reused result cell still checks the full
                    // binding and transfer sequence.
                    state.delayed_old_result = true;
                }
                next.push((Action::PublishSuccessResult(identity as u8), state));
            }

            for direction in 0..2 {
                if self.edge_publications[direction] < MAX_EDGE_PUBLICATIONS_PER_DIRECTION
                    && matches!(attempt.phase, Phase::Pending | Phase::Running | Phase::Waiting)
                {
                    let mut state = self;
                    let worker = &mut state.attempts[identity];
                    worker.ready_seq[direction] = worker.ready_seq[direction]
                        .checked_add(1)
                        .expect("bounded readiness sequence must not wrap");
                    worker.worker_epoch = worker
                        .worker_epoch
                        .checked_add(1)
                        .expect("bounded worker epoch must not wrap");
                    worker.runnable = true;
                    worker.worker_notified = true;
                    state.edge_publications[direction] = state.edge_publications[direction]
                        .checked_add(1)
                        .expect("bounded edge-publication count must not wrap");
                    next.push((Action::PublishEdge(identity as u8, direction as u8), state));
                }
            }

            if attempt.phase == Phase::Returned
                && attempt.revocation.is_none()
                && self.now >= attempt.original_deadline
            {
                let mut state = self;
                Self::revoke(&mut state, identity, Revocation::Deadline, mutation);
                next.push((Action::Revoke(identity as u8, Revocation::Deadline), state));
            }

            if attempt.phase == Phase::Returned
                && attempt.revocation.is_none()
                && attempt
                    .start_time
                    .checked_add(HANDSHAKE_BUDGET)
                    .is_some_and(|cancel_at| self.now >= cancel_at)
                && self.now < attempt.original_deadline
            {
                let mut state = self;
                Self::revoke(&mut state, identity, Revocation::Cancelled, mutation);
                next.push((Action::Revoke(identity as u8, Revocation::Cancelled), state));
            }

            if attempt.revocation.is_none()
                && matches!(attempt.phase, Phase::Pending | Phase::Running | Phase::Waiting)
            {
                if self.now >= attempt.original_deadline {
                    let mut state = self;
                    Self::revoke(&mut state, identity, Revocation::Deadline, mutation);
                    next.push((Action::Revoke(identity as u8, Revocation::Deadline), state));
                }
                if attempt
                    .start_time
                    .checked_add(HANDSHAKE_BUDGET)
                    .is_some_and(|cancel_at| self.now >= cancel_at)
                    && self.now < attempt.original_deadline
                {
                    let mut state = self;
                    Self::revoke(&mut state, identity, Revocation::Cancelled, mutation);
                    next.push((Action::Revoke(identity as u8, Revocation::Cancelled), state));
                }
            }

            if attempt.phase == Phase::Released
                && attempt.proof.published
                && !attempt.proof.consumed
            {
                let mut state = self;
                let fault_holds_queue = (matches!(
                    mutation,
                    Mutation::DeadlineQueueRelease
                        if attempt.revocation == Some(Revocation::Deadline)
                ) || matches!(
                    mutation,
                    Mutation::PreDeadlineQueueRelease(reason)
                        if attempt.revocation == Some(reason)
                            && attempt
                                .start_time
                                .checked_add(HANDSHAKE_BUDGET)
                                .is_some_and(|cancel_at| self.now >= cancel_at)
                            && self.now < attempt.original_deadline
                )) && self.tcp.phase != TcpPhase::Absent;
                state.attempts[identity].proof.consumed = true;
                state.attempts[identity].proof.consume_count = state.attempts[identity]
                    .proof
                    .consume_count
                    .checked_add(1)
                    .expect("proof-consumption counter must not wrap");
                if !fault_holds_queue {
                    state.settle_queue(identity, attempt.key);
                }
                next.push((Action::ConsumeQueueProof(identity as u8), state));
            }

            if (attempt.accepted_count == 1 || attempt.drop_count == 1)
                && attempt.physical_ack_count == 0
            {
                let mut state = self;
                let ack = &mut state.attempts[identity];
                ack.physical_ack_count = ack
                    .physical_ack_count
                    .checked_add(1)
                    .expect("physical acknowledgement counter must not wrap");
                ack.retirement_ack = true;
                if mutation == Mutation::WrongRetirementAck
                    && identity == FIRST
                    && state.tcp.phase == TcpPhase::Released
                    && state.tcp.physical_ack_count == 0
                {
                    state.tcp.physical_ack_count = 1;
                    state.tcp.acknowledgement_key = attempt.key;
                }
                next.push((Action::PhysicalAck(identity as u8), state));
            }

            if attempt.phase == Phase::Accepted
                && self.slot == identity as u8
                && !attempt.worker_active
            {
                let mut state = self;
                state.slot = NO_ID;
                state.capacity_notify_due = state.capacity.live();
                next.push((Action::FinishAcceptedSlot(identity as u8), state));
            }

            if attempt.phase == Phase::Released
                && self.slot == identity as u8
                && !attempt.worker_active
                && attempt.retirement_ack
                && !attempt.has_stream
            {
                let mut state = self;
                state.slot = NO_ID;
                state.capacity_notify_due = state.capacity.live();
                next.push((Action::FinishReleasedSlot(identity as u8), state));
            }
        }

        if !self.admission_closed {
            let mut state = self;
            // Shutdown closes admission before publishing its revocation proof.
            state.admission_closed = true;
            let mut changed = false;
            if state.active_attempt().is_some_and(|identity| {
                let attempt = state.attempts[identity];
                attempt.revocation.is_none()
                    && attempt.phase != Phase::Accepted
                    && attempt.phase != Phase::Released
            }) {
                let identity = state.active_attempt().expect("checked active attempt");
                Self::revoke(&mut state, identity, Revocation::Shutdown, mutation);
                changed = true;
            }
            if !matches!(
                state.capacity.phase,
                CapacityPhase::Absent | CapacityPhase::Used | CapacityPhase::Expired
            ) {
                state.capacity.phase = CapacityPhase::Cancelled;
                state.capacity.sleeping = false;
                state.capacity.wake_pending = true;
                state.capacity.cancellation_count = 1;
                changed = true;
            }
            if changed || !self.admission_closed {
                next.push((Action::Shutdown, state));
            }
        }

        if self.capacity.phase == CapacityPhase::Absent && self.slot != NO_ID {
            let mut state = self;
            state.capacity.phase = CapacityPhase::Requested;
            state.capacity.deadline = self
                .now
                .checked_add(2 * HANDSHAKE_BUDGET)
                .expect("capacity deadline must not wrap");
            state.capacity.budget_refusal_eligible = false;
            next.push((Action::RequestCapacity, state));
        }

        // K2 waiter steps are separate scheduler transitions: precheck,
        // register, locked recheck, then atomic K2 release/sleep. The producer
        // marks notify-due only after TLS unlock and later bumps/notifies K2.
        if self.capacity.phase == CapacityPhase::Requested && !self.capacity.checked {
            let mut state = self;
            state.capacity.checked = true;
            state.capacity.observed_free = self.lane_available();
            state.capacity.observed_epoch = self.k2_epoch;
            next.push((Action::CapacityCheckBeforeRegistration, state));
        }

        if self.capacity.phase == CapacityPhase::Requested && self.capacity.checked {
            let mut state = self;
            state.capacity.phase = CapacityPhase::Registered;
            state.capacity.register_epoch = self.k2_epoch;
            state.capacity.wake_pending = false;
            state.capacity.sleeping = false;
            next.push((Action::RegisterCapacityUnderK2, state));
        }

        if matches!(self.capacity.phase, CapacityPhase::Registered | CapacityPhase::Sleeping) {
            let mut state = self;
            state.resolve_capacity();
            next.push((Action::CapacityLockedRecheck, state));
        }

        if self.capacity.phase == CapacityPhase::Registered
            && self.capacity.rechecked
            && !self.capacity.wake_pending
            && !self.lane_available()
        {
            let mut state = self;
            state.capacity.phase = CapacityPhase::Sleeping;
            state.capacity.sleeping = true;
            next.push((Action::CapacitySleepUnderK2, state));
        }

        if mutation == Mutation::WrongCapacity
            && self.capacity.phase == CapacityPhase::Registered
            && self.capacity.checked
            && !self.capacity.observed_free
            && !self.capacity.rechecked
            && !self.capacity.wake_pending
            && !self.capacity_notify_due
            && self.lane_available()
        {
            let mut state = self;
            state.capacity.phase = CapacityPhase::Sleeping;
            state.capacity.sleeping = true;
            next.push((Action::WrongCapacitySleepWithoutRecheck, state));
        }

        if self.capacity_notify_due {
            let mut state = self;
            state.capacity_notify_due = false;
            state.k2_epoch = state
                .k2_epoch
                .checked_add(1)
                .expect("bounded K2 epoch must not wrap");
            if state.capacity.registered() {
                state.capacity.wake_pending = true;
                state.capacity.sleeping = false;
                state.capacity.phase = CapacityPhase::Registered;
                if state.lane_available() {
                    state.resolve_capacity();
                }
            }
            next.push((Action::ProducerUnlockThenK2Notify, state));
        }

        if self.capacity.phase == CapacityPhase::Ready
            && self.capacity.deadline.saturating_sub(self.now) < MIN_WSS_REMAINING
        {
            let mut state = self;
            state.capacity.phase = CapacityPhase::Refused;
            state.capacity.typed_refusal_count = 1;
            state.capacity.budget_refusal_eligible = true;
            next.push((Action::RefuseInsufficientWssTime, state));
        }

        if self.capacity.phase == CapacityPhase::Refused && self.now >= self.capacity.deadline {
            let mut state = self;
            state.capacity.phase = CapacityPhase::Expired;
            state.capacity.expiration_count = 1;
            state.capacity.budget_refusal_eligible = false;
            next.push((Action::ExpireRefusedWssAtOriginalDeadline, state));
        }

        if matches!(
            self.capacity.phase,
            CapacityPhase::Requested
                | CapacityPhase::Registered
                | CapacityPhase::Sleeping
                | CapacityPhase::Ready
                | CapacityPhase::Refused
        ) && self.capacity.cancellation_count == 0
        {
            let mut state = self;
            state.capacity.phase = CapacityPhase::Cancelled;
            state.capacity.sleeping = false;
            state.capacity.cancellation_count = 1;
            state.capacity.budget_refusal_eligible = false;
            next.push((Action::CancelCapacityWaiter, state));
        }

        if self.delayed_old_result
            && self.slot == SECOND as u8
            && self.attempts[SECOND].phase == Phase::Pending
            && !self.result_cell.occupied
        {
            let mut state = self;
            state.delayed_old_result = false;
            if mutation == Mutation::WrongResultIdentity {
                state.result_cell = ResultCell {
                    occupied: true,
                    identity: TransferKey::game(FIRST),
                    source: TransferKey::game(FIRST),
                    outcome: CellOutcome::Failure,
                    read: false,
                };
                state.completion_hint = true;
                state.wake_due = true;
            }
            next.push((Action::DeliverDelayedOldResult, state));
        }

        if self.attempts[FIRST].proof.published
            && !self.attempts[FIRST].proof.consumed
            && self.slot == SECOND as u8
            && !self.stale_proof_delivered
        {
            let mut state = self;
            state.stale_proof_delivered = true;
            if mutation == Mutation::WrongProofIdentity {
                let old_key = state.attempts[FIRST].proof.key;
                state.attempts[FIRST].proof.consumed = true;
                state.attempts[FIRST].proof.consume_count = 1;
                state.attempts[SECOND].queue_credit = false;
                state.attempts[SECOND].queue_settlement_count = 1;
                state.attempts[SECOND].queue_settlement_key = old_key;
            }
            next.push((Action::TryOldProofAgainstNewIdentity, state));
        }

        if self.result_cell.occupied
            && !self.result_cell.read
            && !self.reactor_sleeping
            && self.completion_hint
        {
            let mut state = self;
            state.result_cell.read = true;
            next.push((Action::ReactorReadsResultCell, state));
        }

        if self.result_cell.occupied
            && self.result_cell.read
            && self.slot != NO_ID
            && self.attempts[self.slot as usize].phase == Phase::Returned
            && self.attempts[self.slot as usize].key == self.result_cell.identity
            && self.result_cell.source == self.result_cell.identity
            && self.now < self.attempts[self.slot as usize].original_deadline
            && self.attempts[self.slot as usize].revocation.is_none()
        {
            let identity = self.slot as usize;
            let mut state = self;
            let source_key = self.result_cell.identity;
            let outcome = self.result_cell.outcome;
            let attempt = &mut state.attempts[identity];
            attempt.phase = Phase::Accepted;
            attempt.accepted_count = attempt
                .accepted_count
                .checked_add(1)
                .expect("acceptance count must not wrap");
            attempt.accepted_key = source_key;
            attempt.accepted_at = self.now;
            attempt.lane_credit = false;
            attempt.queue_settlement_count = attempt
                .queue_settlement_count
                .checked_add(1)
                .expect("queue settlement counter must not wrap");
            attempt.queue_settlement_key = attempt.key;
            attempt.queue_credit = false;
            if outcome == CellOutcome::Success {
                attempt.owner_mask = OWNER_REACTOR;
            } else {
                attempt.has_stream = false;
                attempt.owner_mask = OWNER_CLOSED;
            }
            state.result_cell = ResultCell::EMPTY;
            state.completion_hint = false;
            state.drained_wake = false;
            next.push((Action::TakeReturn(identity as u8), state));
        }

        if mutation == Mutation::WrongResultIdentity
            && self.result_cell.occupied
            && self.result_cell.read
            && self.slot == SECOND as u8
            && self.attempts[SECOND].phase == Phase::Pending
            && self.result_cell.identity != self.attempts[SECOND].key
        {
            let mut state = self;
            let source_key = self.result_cell.identity;
            let attempt = &mut state.attempts[SECOND];
            attempt.phase = Phase::Accepted;
            attempt.accepted_count = 1;
            attempt.accepted_key = source_key;
            attempt.accepted_at = self.now;
            state.result_cell = ResultCell::EMPTY;
            state.completion_hint = false;
            next.push((Action::WronglyAcceptOldResultForNewIdentity, state));
        }

        if self.result_cell.occupied
            && self.result_cell.read
            && self.slot != NO_ID
            && self.result_cell.identity != self.attempts[self.slot as usize].key
        {
            let mut state = self;
            state.result_cell = ResultCell::EMPTY;
            state.completion_hint = false;
            next.push((Action::RejectStaleResultIdentity, state));
        }

        if self.wake_due {
            let mut state = self;
            state.wake_due = false;
            // mio Waker tokens coalesce. The persistent cell/hint is not the
            // token and therefore survives an already-pending wake.
            state.wake_token = true;
            next.push((Action::MioWake, state));
        }

        if self.wake_token {
            let mut state = self;
            state.wake_token = false;
            state.reactor_sleeping = false;
            state.poll_timeout_scheduled = false;
            state.drained_wake = true;
            if mutation == Mutation::WrongCompletion {
                // Wrong completion mutant: it consumes the wake and drops the
                // only hint without reading the persistent result cell.
                state.completion_hint = false;
            }
            next.push((Action::PollDrainWake, state));
        }

        if !self.reactor_sleeping && !self.wake_token {
            let mut state = self;
            state.reactor_sleeping = true;
            state.poll_timeout_scheduled = mutation != Mutation::WrongCompletion;
            next.push((Action::PollSleep, state));
        }

        if self.reactor_sleeping && self.poll_timeout_scheduled {
            let mut state = self;
            state.reactor_sleeping = false;
            state.poll_timeout_scheduled = false;
            state.drained_wake = false;
            if state.result_cell.occupied {
                state.completion_hint = true;
            }
            next.push((Action::BoundedPollTimeoutAndDrain, state));
        }

        if self.tcp.phase == TcpPhase::Absent && self.active_attempt().is_some() {
            let mut state = self;
            state.tcp.phase = TcpPhase::Queued;
            state.tcp.key = TransferKey::later_tcp();
            next.push((Action::QueueLaterTcpAttempt, state));
        }

        if self.tcp.phase == TcpPhase::Queued
            && self.attempts[FIRST].phase != Phase::Empty
            && !self.attempts[FIRST].queue_credit
            && self.attempts[FIRST].queue_settlement_count == 1
        {
            let mut state = self;
            state.tcp.phase = TcpPhase::Eligible;
            next.push((Action::TcpDemandBecomesEligible, state));
        }

        if self.tcp.phase == TcpPhase::Eligible {
            let mut state = self;
            state.tcp.phase = TcpPhase::Pending;
            state.tcp.has_stream = true;
            state.tcp.owner_mask = OWNER_REACTOR;
            next.push((Action::StartEligibleTcpWithoutWaitingForWorker, state));
        }

        if self.tcp.phase == TcpPhase::Pending {
            let mut state = self;
            state.tcp.phase = TcpPhase::Accepted;
            state.tcp.accepted_count = 1;
            state.tcp.accepted_key = state.tcp.key;
            next.push((Action::AcceptTcp, state));

            let mut state = self;
            state.tcp.phase = TcpPhase::Released;
            state.tcp.has_stream = false;
            state.tcp.owner_mask = OWNER_CLOSED;
            state.tcp.close_count = 1;
            state.tcp.drop_count = 1;
            next.push((Action::CloseDropTcp, state));
        }

        if self.tcp.phase == TcpPhase::Accepted && self.tcp.physical_ack_count == 0 {
            let mut state = self;
            state.tcp.physical_ack_count = 1;
            state.tcp.acknowledgement_key = state.tcp.key;
            next.push((Action::TcpPhysicalAck, state));
        }

        if self.tcp.phase == TcpPhase::Released && self.tcp.physical_ack_count == 0 {
            let mut state = self;
            state.tcp.physical_ack_count = 1;
            state.tcp.acknowledgement_key = state.tcp.key;
            next.push((Action::TcpPhysicalAck, state));
        }

        if self.admission_closed
            && self.capacity.live()
            && self.capacity.cancellation_count == 0
        {
            let mut state = self;
            state.capacity.phase = CapacityPhase::Cancelled;
            state.capacity.sleeping = false;
            state.capacity.wake_pending = true;
            state.capacity.cancellation_count = 1;
            state.capacity.budget_refusal_eligible = false;
            next.push((Action::ShutdownCancelsCapacityWaiter, state));
        }

        next
    }

    fn revoke(state: &mut Self, identity: usize, reason: Revocation, mutation: Mutation) {
        let attempt = &mut state.attempts[identity];
        if attempt.revocation.is_some()
            || !matches!(
                attempt.phase,
                Phase::Pending | Phase::Running | Phase::Waiting | Phase::Returned
            )
        {
            return;
        }
        attempt.revocation = Some(reason);
        attempt.revoked_at = state.now;
        attempt.proof = QueueProof {
            published: true,
            key: attempt.key,
            published_at: state.now,
            consumed: false,
            consume_count: 0,
        };
        attempt.phase = Phase::Released;
        if state.result_cell.occupied && state.result_cell.identity == attempt.key {
            state.result_cell = ResultCell::EMPTY;
            state.completion_hint = false;
        }
        match attempt.owner_mask {
            OWNER_HANDOFF_CELL | OWNER_RESULT_CELL => {
                attempt.owner_mask = OWNER_REACTOR;
            }
            OWNER_WORKER => {
                if attempt.worker_sleeping {
                    attempt.worker_notified = true;
                }
            }
            _ => {}
        }
        state.wake_due = true;
        if reason == Revocation::Deadline && mutation == Mutation::DeadlinePhysicalRelease {
            // Mutant releases capacity at D while the opaque call still owns
            // the stream. Invariant 5 must reject this before a replacement.
            attempt.lane_credit = false;
            attempt.physical_credit = false;
        }
    }

    fn close_and_drop(state: &mut Self, identity: usize) {
        let attempt = &mut state.attempts[identity];
        attempt.close_count = attempt
            .close_count
            .checked_add(1)
            .expect("close counter must not wrap");
        attempt.drop_count = attempt
            .drop_count
            .checked_add(1)
            .expect("drop counter must not wrap");
        attempt.has_stream = false;
        attempt.owner_mask = OWNER_CLOSED;
        attempt.lane_credit = false;
        attempt.physical_credit = false;
        attempt.failure_ready = attempt.phase == Phase::Running && attempt.revocation.is_none();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mutation {
    None,
    WrongClear,
    WrongCompletion,
    WrongResultIdentity,
    WrongCapacity,
    DeadlineQueueRelease,
    DeadlinePhysicalRelease,
    PreDeadlineQueueRelease(Revocation),
    WrongProofIdentity,
    WrongRetirementAck,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ReturnKind {
    WouldBlock,
    SyntheticLimit,
    Success,
    Failure,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Action {
    Admit(u8),
    AdvanceTime(u8),
    Claim(u8),
    EnterTurn(u8, bool),
    Return(u8, ReturnKind),
    LockedRecheckAndCondvarRelease(u8),
    RequeueSyntheticLimit(u8),
    WorkerWakeAndLockedRecheck(u8),
    WorkerBoundedCondvarTimeout(u8),
    WorkerObservesRevocation(u8),
    WorkerCloseDropFailure(u8),
    CloseDropRevokedStream(u8),
    PublishFailureResult(u8),
    PublishSuccessResult(u8),
    PublishEdge(u8, u8),
    Revoke(u8, Revocation),
    ConsumeQueueProof(u8),
    PhysicalAck(u8),
    FinishAcceptedSlot(u8),
    FinishReleasedSlot(u8),
    Shutdown,
    RequestCapacity,
    CapacityCheckBeforeRegistration,
    RegisterCapacityUnderK2,
    CapacityLockedRecheck,
    CapacitySleepUnderK2,
    WrongCapacitySleepWithoutRecheck,
    ProducerUnlockThenK2Notify,
    RefuseWssAtGrant,
    RefuseInsufficientWssTime,
    ExpireRefusedWssAtOriginalDeadline,
    CancelCapacityWaiter,
    DeliverDelayedOldResult,
    TryOldProofAgainstNewIdentity,
    ReactorReadsResultCell,
    TakeReturn(u8),
    WronglyAcceptOldResultForNewIdentity,
    RejectStaleResultIdentity,
    MioWake,
    PollDrainWake,
    PollSleep,
    BoundedPollTimeoutAndDrain,
    QueueLaterTcpAttempt,
    TcpDemandBecomesEligible,
    StartEligibleTcpWithoutWaitingForWorker,
    AcceptTcp,
    CloseDropTcp,
    TcpPhysicalAck,
    ShutdownCancelsCapacityWaiter,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admit(id) => write!(f, "producer:submit transfer identity {}", id + 1),
            Self::AdvanceTime(time) => write!(f, "clock:advance to {time}s"),
            Self::Claim(id) => write!(f, "worker:claim transfer identity {}", id + 1),
            Self::EnterTurn(id, opaque) => write!(
                f,
                "worker:handshake-turn enter identity {} ({})",
                id + 1,
                if *opaque { "opaque nonreturning verifier" } else { "returning oracle" }
            ),
            Self::Return(id, kind) => write!(f, "worker:handshake-turn return identity {} ({kind:?})", id + 1),
            Self::LockedRecheckAndCondvarRelease(id) => write!(
                f,
                "worker:locked readiness recheck + atomic condvar release identity {}",
                id + 1
            ),
            Self::RequeueSyntheticLimit(id) => write!(f, "worker:requeue synthetic limit identity {}", id + 1),
            Self::WorkerWakeAndLockedRecheck(id) => write!(f, "worker:condvar wake + locked recheck identity {}", id + 1),
            Self::WorkerBoundedCondvarTimeout(id) => write!(f, "worker:bounded condvar timeout identity {}", id + 1),
            Self::WorkerObservesRevocation(id) => write!(f, "worker:observe revocation identity {}", id + 1),
            Self::WorkerCloseDropFailure(id) => write!(f, "worker:close/drop failed stream identity {}", id + 1),
            Self::CloseDropRevokedStream(id) => write!(f, "owner:close/drop revoked stream identity {}", id + 1),
            Self::PublishFailureResult(id) => write!(f, "worker:publish failure result identity {}", id + 1),
            Self::PublishSuccessResult(id) => write!(f, "worker:publish Returned result cell identity {}", id + 1),
            Self::PublishEdge(id, direction) => write!(
                f,
                "reactor:publish {} edge identity {}",
                if *direction == 0 { "read" } else { "write" },
                id + 1
            ),
            Self::Revoke(id, reason) => write!(f, "producer:revoke identity {} ({reason})", id + 1),
            Self::ConsumeQueueProof(id) => write!(f, "queue:consume exact TLS release proof identity {}", id + 1),
            Self::PhysicalAck(id) => write!(f, "owner:physical retirement ack identity {}", id + 1),
            Self::FinishAcceptedSlot(id) => write!(f, "reactor:finish accepted handoff slot identity {}", id + 1),
            Self::FinishReleasedSlot(id) => write!(f, "reactor:finish physically retired slot identity {}", id + 1),
            Self::Shutdown => f.write_str("producer:shutdown and close admission"),
            Self::RequestCapacity => f.write_str("producer:request WSS capacity"),
            Self::CapacityCheckBeforeRegistration => f.write_str("producer:precheck lane availability"),
            Self::RegisterCapacityUnderK2 => f.write_str("producer:register waiter under K2"),
            Self::CapacityLockedRecheck => f.write_str("producer:K2-locked lane recheck"),
            Self::CapacitySleepUnderK2 => f.write_str("producer:atomic K2 release and capacity sleep"),
            Self::WrongCapacitySleepWithoutRecheck => f.write_str("mutant:capacity sleep without K2 recheck"),
            Self::ProducerUnlockThenK2Notify => f.write_str("producer:after TLS unlock, bump K2 and notify"),
            Self::RefuseWssAtGrant => f.write_str("capacity:typed insufficient-time refusal without charge"),
            Self::RefuseInsufficientWssTime => f.write_str("capacity:typed insufficient-time refusal without charge"),
            Self::ExpireRefusedWssAtOriginalDeadline => f.write_str("capacity:expire once at immutable D"),
            Self::CancelCapacityWaiter => f.write_str("producer:stronger capacity-waiter cancellation"),
            Self::DeliverDelayedOldResult => f.write_str("reactor:deliver delayed old-key result tuple"),
            Self::TryOldProofAgainstNewIdentity => f.write_str("queue:try old release proof against next identity"),
            Self::ReactorReadsResultCell => f.write_str("reactor:read persistent result cell"),
            Self::TakeReturn(id) => write!(f, "reactor:take exact Returned identity {}", id + 1),
            Self::WronglyAcceptOldResultForNewIdentity => f.write_str("mutant:accept old result for new identity"),
            Self::RejectStaleResultIdentity => f.write_str("reactor:reject stale result identity"),
            Self::MioWake => f.write_str("worker/revoker:mio wake (coalescing token)"),
            Self::PollDrainWake => f.write_str("reactor:Poll drain wake token"),
            Self::PollSleep => f.write_str("reactor:Poll sleep"),
            Self::BoundedPollTimeoutAndDrain => f.write_str("reactor:bounded Poll timeout and result drain"),
            Self::QueueLaterTcpAttempt => f.write_str("producer:queue same-slot later TCP attempt"),
            Self::TcpDemandBecomesEligible => f.write_str("queue:make TCP demand eligible after receipt"),
            Self::StartEligibleTcpWithoutWaitingForWorker => f.write_str("reactor:start eligible TCP while TLS worker may remain occupied"),
            Self::AcceptTcp => f.write_str("reactor:accept later TCP attempt"),
            Self::CloseDropTcp => f.write_str("reactor:close/drop later TCP attempt"),
            Self::TcpPhysicalAck => f.write_str("reactor:ack later TCP attempt's own retirement cell"),
            Self::ShutdownCancelsCapacityWaiter => f.write_str("producer:shutdown revokes capacity waiter"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Violation {
    invariant: u8,
    detail: &'static str,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invariant {}: {}", self.invariant, self.detail)
    }
}

fn check_invariants(state: State) -> Result<(), Violation> {
    // I1: every live stream has one custody bit. Worker activity is singleton.
    let mut active_workers = 0u8;
    for attempt in state.attempts {
        if attempt.has_stream && attempt.owner_mask.count_ones() != 1 {
            return Err(Violation {
                invariant: 1,
                detail: "live stream does not have exactly one custodian",
            });
        }
        if !attempt.has_stream && attempt.owner_mask != 0 && attempt.owner_mask.count_ones() != 1 {
            return Err(Violation {
                invariant: 1,
                detail: "closed stream has multiple custody markers",
            });
        }
        if attempt.worker_active {
            active_workers = active_workers
                .checked_add(1)
                .expect("worker count is bounded");
            if attempt.owner_mask != OWNER_WORKER {
                return Err(Violation {
                    invariant: 1,
                    detail: "active worker does not own its stream",
                });
            }
        }
        if attempt.accepted_count > 1 {
            return Err(Violation {
                invariant: 1,
                detail: "one transfer identity was accepted more than once",
            });
        }
    }
    if active_workers > 1 {
        return Err(Violation {
            invariant: 1,
            detail: "more than one handshake worker is active",
        });
    }
    if state.result_cell.occupied
        && state.result_cell.outcome == CellOutcome::Success
        && state.result_cell.source == state.result_cell.identity
    {
        let source = state
            .attempts
            .iter()
            .find(|attempt| attempt.key == state.result_cell.source);
        if source.is_none_or(|attempt| attempt.owner_mask != OWNER_RESULT_CELL) {
            return Err(Violation {
                invariant: 1,
                detail: "successful result cell does not own its stream",
            });
        }
    }

    // I2: a take can grant authority only to the full current key and before
    // revocation/deadline. A stale tuple is harmless until a mutant takes it.
    for attempt in state.attempts {
        if attempt.accepted_count > 0
            && (attempt.accepted_key != attempt.key
                || attempt.revocation.is_some()
                || attempt.accepted_at >= attempt.original_deadline)
        {
            return Err(Violation {
                invariant: 2,
                detail: "revoked, stale, or late transfer gained reactor/session authority",
            });
        }
        if attempt.phase == Phase::Accepted && attempt.accepted_count != 1 {
            return Err(Violation {
                invariant: 2,
                detail: "Accepted phase has no unique matching acceptance",
            });
        }
    }

    // I3: an edge that differs from the worker's pre-call snapshot cannot be
    // hidden by runnable-clear followed by condvar sleep.
    for attempt in state.attempts {
        if attempt.phase == Phase::Waiting
            && attempt.worker_sleeping
            && (attempt.ready_seq != attempt.worker_snapshot
                || attempt.ready_seq != attempt.call_snapshot
                || attempt.worker_epoch != attempt.worker_sleep_epoch)
            && !attempt.runnable
            && !attempt.worker_notified
        {
            return Err(Violation {
                invariant: 3,
                detail: "unconsumed readiness is stranded behind worker sleep",
            });
        }
    }

    // I4: a persistent Returned cell must have a pending wake or a bounded Poll
    // timeout whenever the reactor sleeps.
    if state.result_cell.occupied
        && state.slot != NO_ID
        && state.attempts[state.slot as usize].phase == Phase::Returned
        && state.reactor_sleeping
        && !state.wake_token
        && !state.wake_due
        && !state.poll_timeout_scheduled
    {
        return Err(Violation {
            invariant: 4,
            detail: "Returned result is stranded behind a consumed wake",
        });
    }

    // I5: Abort retirement keeps lane/socket credit until acceptance or actual
    // close/drop. A physical acknowledgement belongs to the exact attempt.
    for attempt in state.attempts {
        if attempt.close_count > 1 || attempt.drop_count > 1 || attempt.physical_ack_count > 1 {
            return Err(Violation {
                invariant: 5,
                detail: "close/drop/physical-ack counter exceeded its one-shot bound",
            });
        }
        if attempt.drop_count == 1
            && (attempt.close_count != 1
                || attempt.has_stream
                || attempt.owner_mask != OWNER_CLOSED
                || attempt.lane_credit
                || attempt.physical_credit)
        {
            return Err(Violation {
                invariant: 5,
                detail: "drop proof did not close the stream before credit release",
            });
        }
        if attempt.revocation.is_some()
            && attempt.accepted_count == 0
            && attempt.has_stream
            && (!attempt.lane_credit || !attempt.physical_credit)
        {
            return Err(Violation {
                invariant: 5,
                detail: "revoked worker released lane/socket credit before close/drop",
            });
        }
        if !attempt.lane_credit
            && attempt.accepted_count == 0
            && attempt.drop_count == 0
            && attempt.phase != Phase::Empty
        {
            return Err(Violation {
                invariant: 5,
                detail: "lane retirement released before acceptance or close/drop proof",
            });
        }
        if !attempt.physical_credit
            && attempt.accepted_count == 0
            && attempt.drop_count == 0
            && attempt.phase != Phase::Empty
        {
            return Err(Violation {
                invariant: 5,
                detail: "physical retirement released before acceptance or close/drop proof",
            });
        }
        if attempt.physical_ack_count > 0
            && attempt.accepted_count == 0
            && attempt.drop_count == 0
        {
            return Err(Violation {
                invariant: 5,
                detail: "physical acknowledgement precedes acceptance or actual drop",
            });
        }
    }

    // I6: K2 registration/recheck closes the release-between-check-and-sleep
    // race. A release notification that is already scheduled is also usable.
    let capacity = state.capacity;
    let eligible_capacity_waiter = matches!(
        capacity.phase,
        CapacityPhase::Registered | CapacityPhase::Sleeping
    );
    if eligible_capacity_waiter
        && capacity.sleeping
        && state.lane_available()
        && !capacity.wake_pending
        && !state.capacity_notify_due
        && (!capacity.rechecked || capacity.recheck_epoch != state.k2_epoch)
    {
        return Err(Violation {
            invariant: 6,
            detail: "eligible capacity waiter sleeps on a free lane without a usable observation or wake",
        });
    }

    // I7: queue settlement and retirement are identity-scoped and one-shot.
    for attempt in state.attempts {
        if attempt.phase == Phase::Empty {
            continue;
        }
        if attempt.queue_settlement_count > 1
            || (attempt.queue_credit && attempt.queue_settlement_count != 0)
            || (!attempt.queue_credit && attempt.queue_settlement_count != 1)
        {
            return Err(Violation {
                invariant: 7,
                detail: "queue lease settlement is missing or duplicated",
            });
        }
        if !attempt.queue_credit && attempt.queue_settlement_key != attempt.key {
            return Err(Violation {
                invariant: 7,
                detail: "queue receipt settled a different transfer identity",
            });
        }
        if attempt.proof.consume_count > 1
            || attempt.proof.consumed != (attempt.proof.consume_count == 1)
        {
            return Err(Violation {
                invariant: 7,
                detail: "initial-TLS release proof was consumed more than once",
            });
        }
        if attempt.proof.consumed
            && (attempt.proof.key != attempt.key
                || attempt.accepted_count != 0
                || attempt.phase == Phase::Accepted)
        {
            return Err(Violation {
                invariant: 7,
                detail: "consumed initial-TLS proof did not irrevocably fence Accepted",
            });
        }
        if attempt.physical_ack_count > 0 && !attempt.retirement_ack {
            return Err(Violation {
                invariant: 7,
                detail: "physical acknowledgement did not update its own retirement cell",
            });
        }
    }
    if state.tcp.physical_ack_count > 0
        && (state.tcp.acknowledgement_key != state.tcp.key
            || (state.tcp.accepted_count == 0 && state.tcp.drop_count == 0))
    {
        return Err(Violation {
            invariant: 7,
            detail: "old physical acknowledgement released the later TCP retirement cell",
        });
    }
    if state.tcp.accepted_count > 1
        || (state.tcp.accepted_count == 1 && state.tcp.accepted_key != state.tcp.key)
    {
        return Err(Violation {
            invariant: 7,
            detail: "later TCP retirement cell has the wrong identity",
        });
    }

    // I8: every pre-accept revocation publishes its immutable queue receipt at
    // that event. Consuming it settles queue credit without waiting for an
    // opaque worker. WSS refusal preserves D and charges no start.
    for attempt in state.attempts {
        if let Some(_reason) = attempt.revocation {
            if !attempt.proof.published
                || attempt.proof.key != attempt.key
                || attempt.proof.published_at != attempt.revoked_at
                || attempt.proof.published_at > attempt.original_deadline
            {
                return Err(Violation {
                    invariant: 8,
                    detail: "revocation receipt is absent, stale, or later than original D",
                });
            }
            if attempt.proof.consumed && attempt.queue_credit {
                return Err(Violation {
                    invariant: 8,
                    detail: "consumed release receipt leaves TCP queue admission blocked",
                });
            }
        }
        if attempt.accepted_count > 0 && attempt.revocation.is_some() {
            return Err(Violation {
                invariant: 8,
                detail: "revocation incorrectly revoked established-session authority",
            });
        }
    }
    if capacity.phase == CapacityPhase::Refused
        && (capacity.typed_refusal_count != 1
            || capacity.charged_start_count != 0
            || !capacity.budget_refusal_eligible
            || capacity.deadline == 0)
    {
        return Err(Violation {
            invariant: 8,
            detail: "insufficient-time WSS demand was charged or lost its typed refusal",
        });
    }
    if capacity.phase == CapacityPhase::Expired && capacity.expiration_count != 1 {
        return Err(Violation {
            invariant: 8,
            detail: "refused WSS demand did not expire exactly once at immutable D",
        });
    }
    if capacity.phase == CapacityPhase::Cancelled && capacity.cancellation_count != 1 {
        return Err(Violation {
            invariant: 8,
            detail: "stronger WSS cancellation did not win exactly once",
        });
    }
    if capacity.charged_start_count > 1 || capacity.expiration_count > 1 {
        return Err(Violation {
            invariant: 8,
            detail: "WSS capacity start/expiry counter exceeded its bound",
        });
    }

    Ok(())
}

fn assert_fair_progress(state: State, mutation: Mutation) {
    let transitions = state.successors(mutation);
    if state.tcp.phase == TcpPhase::Eligible {
        assert!(
            transitions
                .iter()
                .any(|(action, _)| *action == Action::StartEligibleTcpWithoutWaitingForWorker),
            "eligible TCP demand must not depend on worker return"
        );
    }
    for (identity, attempt) in state.attempts.iter().copied().enumerate() {
        if attempt.phase == Phase::Pending && !attempt.worker_active {
            assert!(transitions.iter().any(|(action, _)| *action == Action::Claim(identity as u8)));
        }
        match attempt.turn {
            Turn::Returning => assert!(transitions.iter().any(|(action, _)| {
                matches!(action, Action::Return(id, _) if *id == identity as u8)
            })),
            Turn::WouldBlock if attempt.phase == Phase::Running => assert!(transitions
                .iter()
                .any(|(action, _)| *action == Action::LockedRecheckAndCondvarRelease(identity as u8))),
            Turn::SyntheticLimit if attempt.phase == Phase::Running => assert!(transitions
                .iter()
                .any(|(action, _)| *action == Action::RequeueSyntheticLimit(identity as u8))),
            Turn::Success if attempt.phase == Phase::Running => assert!(transitions
                .iter()
                .any(|(action, _)| *action == Action::PublishSuccessResult(identity as u8))),
            Turn::Failure if attempt.phase == Phase::Running => assert!(transitions
                .iter()
                .any(|(action, _)| *action == Action::WorkerCloseDropFailure(identity as u8))),
            Turn::Opaque => {
                // An opaque verifier has no return obligation. Logical revoke,
                // deadline, queue settlement, and independent TCP progress do.
            }
            _ => {}
        }
        if attempt.phase == Phase::Waiting && attempt.worker_timeout_scheduled {
            assert!(transitions.iter().any(|(action, _)| {
                *action == Action::WorkerBoundedCondvarTimeout(identity as u8)
            }));
        }
    }
    if state.result_cell.occupied && state.reactor_sleeping {
        assert!(state.wake_token || state.wake_due || state.poll_timeout_scheduled);
    }
    if state.capacity.phase == CapacityPhase::Ready
        && state.slot == NO_ID
        && state.lane_available()
        && !state.admission_closed
        && state.authorized_count == 1
    {
        assert!(transitions.iter().any(|(action, _)| {
            matches!(action, Action::Admit(id) if *id == SECOND as u8)
                || *action == Action::RefuseWssAtGrant
        }));
    }
}

#[derive(Clone, Debug)]
struct Node {
    state: State,
    parent: Option<usize>,
    via: Option<Action>,
    depth: usize,
}

#[derive(Debug)]
struct Counterexample {
    explored_states: usize,
    explored_transitions: usize,
    violation: Violation,
    trace: Vec<Action>,
}

#[derive(Debug)]
struct ExhaustiveReport {
    explored_states: usize,
    explored_transitions: usize,
    max_depth: usize,
    saw_edge_during_turn: bool,
    saw_edge_after_would_block: bool,
    saw_edge_before_and_after_would_block: bool,
    saw_would_block: bool,
    saw_synthetic_limit: bool,
    saw_success: bool,
    saw_failure: bool,
    saw_opaque_turn: bool,
    saw_second_identity: bool,
    saw_tcp_while_worker_active: bool,
}

fn reconstruct_trace(nodes: &[Node], mut index: usize) -> Vec<Action> {
    let mut trace = Vec::new();
    while let Some(parent) = nodes[index].parent {
        trace.push(nodes[index].via.expect("non-root node has an action"));
        index = parent;
    }
    trace.reverse();
    trace
}

fn find_counterexample(mutation: Mutation) -> Counterexample {
    let mut nodes = vec![Node {
        state: State::INITIAL,
        parent: None,
        via: None,
        depth: 0,
    }];
    let mut visited = HashMap::new();
    visited.insert(State::INITIAL, 0usize);
    let mut queue = VecDeque::from([0usize]);
    let mut transitions = 0usize;

    while let Some(index) = queue.pop_front() {
        let state = nodes[index].state;
        if let Err(violation) = check_invariants(state) {
            return Counterexample {
                explored_states: nodes.len(),
                explored_transitions: transitions,
                violation,
                trace: reconstruct_trace(&nodes, index),
            };
        }
        for (action, next) in state.successors(mutation) {
            transitions = transitions
                .checked_add(1)
                .expect("transition count must not wrap");
            if visited.contains_key(&next) {
                continue;
            }
            assert!(
                nodes.len() < MAX_EXPLORED_STATES,
                "mutant search exceeded bounded state count {MAX_EXPLORED_STATES}"
            );
            let child = nodes.len();
            visited.insert(next, child);
            nodes.push(Node {
                state: next,
                parent: Some(index),
                via: Some(action),
                depth: nodes[index].depth + 1,
            });
            queue.push_back(child);
        }
    }
    panic!("mutation {mutation:?} did not produce a counterexample")
}

fn explore_correct_model() -> ExhaustiveReport {
    let mut nodes = vec![Node {
        state: State::INITIAL,
        parent: None,
        via: None,
        depth: 0,
    }];
    let mut visited = HashMap::new();
    visited.insert(State::INITIAL, 0usize);
    let mut queue = VecDeque::from([0usize]);
    let mut transitions = 0usize;
    let mut report = ExhaustiveReport {
        explored_states: 0,
        explored_transitions: 0,
        max_depth: 0,
        saw_edge_during_turn: false,
        saw_edge_after_would_block: false,
        saw_edge_before_and_after_would_block: false,
        saw_would_block: false,
        saw_synthetic_limit: false,
        saw_success: false,
        saw_failure: false,
        saw_opaque_turn: false,
        saw_second_identity: false,
        saw_tcp_while_worker_active: false,
    };

    while let Some(index) = queue.pop_front() {
        let state = nodes[index].state;
        let depth = nodes[index].depth;
        report.max_depth = report.max_depth.max(depth);
        if let Err(violation) = check_invariants(state) {
            let trace = reconstruct_trace(&nodes, index);
            panic!("correct model violated {violation}\ntrace:\n{}", format_trace(&trace));
        }
        assert_fair_progress(state, Mutation::None);
        if state.authorized_count == 2 {
            report.saw_second_identity = true;
        }
        if state.tcp.phase == TcpPhase::Pending
            && state.attempts.iter().any(|attempt| attempt.worker_active)
        {
            report.saw_tcp_while_worker_active = true;
        }
        if state.attempts.iter().any(|attempt| attempt.turn == Turn::Opaque) {
            report.saw_opaque_turn = true;
        }

        let successors = state.successors(Mutation::None);
        for (action, next) in successors {
            transitions = transitions
                .checked_add(1)
                .expect("transition count must not wrap");
            match action {
                Action::PublishEdge(id, _) => {
                    let attempt = state.attempts[id as usize];
                    if attempt.turn == Turn::Returning {
                        report.saw_edge_during_turn = true;
                    }
                    if attempt.turn == Turn::WouldBlock
                        || (attempt.phase == Phase::Waiting && attempt.last_would_block)
                    {
                        report.saw_edge_after_would_block = true;
                    }
                    if attempt.turn == Turn::WouldBlock
                        && attempt.ready_seq != attempt.call_snapshot
                    {
                        report.saw_edge_before_and_after_would_block = true;
                    }
                }
                Action::Return(_, ReturnKind::WouldBlock) => report.saw_would_block = true,
                Action::Return(_, ReturnKind::SyntheticLimit) => report.saw_synthetic_limit = true,
                Action::Return(_, ReturnKind::Success) => report.saw_success = true,
                Action::Return(_, ReturnKind::Failure) => report.saw_failure = true,
                _ => {}
            }
            if visited.contains_key(&next) {
                continue;
            }
            assert!(
                nodes.len() < MAX_EXPLORED_STATES,
                "correct model exceeded bounded state count {MAX_EXPLORED_STATES}"
            );
            let child = nodes.len();
            visited.insert(next, child);
            nodes.push(Node {
                state: next,
                parent: Some(index),
                via: Some(action),
                depth: depth + 1,
            });
            queue.push_back(child);
        }
    }

    report.explored_states = nodes.len();
    report.explored_transitions = transitions;
    report
}

fn format_trace(trace: &[Action]) -> String {
    trace
        .iter()
        .enumerate()
        .map(|(index, action)| format!("  {:02}. {action}", index + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_shortest_mutant(mutation: Mutation, expected_invariant: u8, required: &[&str]) {
    let result = find_counterexample(mutation);
    assert_eq!(result.violation.invariant, expected_invariant, "{mutation:?}: {result:?}");
    assert!(!result.trace.is_empty(), "{mutation:?} needs a counterexample trace");
    let rendered = format_trace(&result.trace);
    for action in required {
        assert!(rendered.contains(action), "{mutation:?} trace lacks {action:?}:\n{rendered}");
    }
    assert!(result.explored_states <= MAX_EXPLORED_STATES);
    println!(
        "mutant={mutation:?} explored_states={} transitions={} shortest_trace_len={} violation={}\n{}",
        result.explored_states,
        result.explored_transitions,
        result.trace.len(),
        result.violation,
        rendered
    );
}

#[test]
fn game_tls_model_exhaustively_explores_bounded_interleavings_and_invariants() {
    let report = explore_correct_model();
    assert!(report.explored_states > 100, "state space is not an exhaustive scheduler");
    assert!(report.explored_states <= MAX_EXPLORED_STATES);
    assert!(report.explored_transitions >= report.explored_states);
    assert!(report.max_depth > 10);
    assert!(report.saw_edge_during_turn, "missing edge during an entered socket turn");
    assert!(report.saw_would_block);
    assert!(report.saw_edge_after_would_block, "missing edge after one WouldBlock return");
    assert!(
        report.saw_edge_before_and_after_would_block,
        "the two-edge bound must cover one edge before and another after WouldBlock"
    );
    assert!(report.saw_synthetic_limit);
    assert!(report.saw_success);
    assert!(report.saw_failure);
    assert!(report.saw_opaque_turn, "nonreturning verifier must be an explicit oracle branch");
    assert!(report.saw_second_identity, "the second transfer identity was not admitted");
    assert!(
        report.saw_tcp_while_worker_active,
        "queue release must permit eligible TCP while opaque TLS work still owns the stream"
    );
    println!(
        "correct_game_tls_model explored_states={} transitions={} max_depth={} assumptions=one producer/worker/reactor; two transfer identities; two edges per direction; counters checked; weak fairness; bounded Poll; opaque verifier has no return obligation",
        report.explored_states, report.explored_transitions, report.max_depth
    );
}

#[test]
fn mandatory_game_tls_mutants_have_shortest_counterexamples() {
    assert_shortest_mutant(
        Mutation::WrongClear,
        3,
        &[
            "worker:claim",
            "handshake-turn enter",
            "publish read edge",
            "WouldBlock",
            "locked readiness recheck",
        ],
    );
    assert_shortest_mutant(
        Mutation::WrongCompletion,
        4,
        &[
            "publish Returned result cell",
            "mio wake",
            "Poll drain wake token",
            "reactor:Poll sleep",
        ],
    );
    assert_shortest_mutant(
        Mutation::WrongResultIdentity,
        2,
        &[
            "submit transfer identity 2",
            "deliver delayed old-key result",
            "accept old result for new identity",
        ],
    );
    assert_shortest_mutant(
        Mutation::WrongCapacity,
        6,
        &[
            "precheck lane availability",
            "producer:after TLS unlock, bump K2 and notify",
            "register waiter under K2",
            "capacity sleep without K2 recheck",
        ],
    );
    assert_shortest_mutant(
        Mutation::DeadlineQueueRelease,
        8,
        &[
            "handshake-turn enter identity 1 (opaque nonreturning verifier)",
            "queue same-slot later TCP attempt",
            "deadline",
            "consume exact TLS release proof",
        ],
    );
    assert_shortest_mutant(
        Mutation::DeadlinePhysicalRelease,
        5,
        &[
            "opaque nonreturning verifier",
            "deadline",
        ],
    );

    // r4 keeps the deadline exception. Cancellation at t=5 and shutdown at
    // t=5 must each release queue admission at receipt consumption, even while
    // physical worker custody remains. The deadline case remains the control.
    assert_shortest_mutant(
        Mutation::PreDeadlineQueueRelease(Revocation::Cancelled),
        8,
        &[
            "clock:advance to",
            "cancelled",
            "queue same-slot later TCP attempt",
            "consume exact TLS release proof",
        ],
    );
    assert_shortest_mutant(
        Mutation::PreDeadlineQueueRelease(Revocation::Shutdown),
        8,
        &[
            "clock:advance to",
            "shutdown",
            "queue same-slot later TCP attempt",
            "consume exact TLS release proof",
        ],
    );

    // The identity guard also covers a delayed queue receipt and a late
    // physical acknowledgement. Neither may settle the next identity's cell.
    assert_shortest_mutant(
        Mutation::WrongProofIdentity,
        7,
        &[
            "submit transfer identity 2",
            "old release proof against next identity",
        ],
    );
    assert_shortest_mutant(
        Mutation::WrongRetirementAck,
        7,
        &[
            "take exact Returned identity 1",
            "same-slot later TCP attempt",
            "close/drop later TCP attempt",
            "physical retirement ack identity 1",
        ],
    );
}
