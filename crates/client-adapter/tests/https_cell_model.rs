//! Exhaustive finite-state scheduler for the HTTPS fetch/result-cell contract.
//!
//! Each scheduler edge is one atomic HTTPS-lock transition or one operation
//! outside that lock. Socket calls and verifier calls have separate entry and
//! return edges. The verifier also has a branch that never returns. Time is a
//! small monotone integer clock; scheduler ticks advance toward recorded
//! deadlines only while timed work remains; a due deadline transition must be
//! resolved before another tick. Phase entry preserves the original queue deadline.
//!
//! The model has one worker and one logical fetch owner. The owner can have its
//! first ticket's result cell still occupied while it starts the second
//! identity. Each cell has a permanent identity. Socket custody is represented
//! by exactly one enum value per connection, so a close can occur only after
//! the worker, cell, or owner transfers custody to the closer.
//!
//! Liveness assumes fair scheduling of enabled worker, owner, wake, and close
//! steps. A finite connect call times out at its recorded deadline. A late
//! success return is also modeled and must close its socket without publication.
//! The OS verifier is opaque and may never return, so it has no return-liveness
//! obligation. The owner's fixed blocking-call deadline and logical phase
//! expiry remain schedulable while that verifier is opaque.

use std::collections::{HashMap, VecDeque};

const CELLS: usize = 2;
const ADDRESSES: usize = 2;
const QUEUE_BUDGET: u8 = 2;
const CONNECT_BUDGET: u8 = 2;
const HANDSHAKE_BUDGET: u8 = 2;
const BLOCKING_OWNER_BUDGET: u8 = 1;
const MAX_TIME: u8 = 20;
const MAX_STATES: usize = 250_000;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Failure {
    ConnectRefused,
    TlsAlert,
    CertificateRejected,
    WorkerLimit,
    Cancelled,
    OwnerDropped,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Terminal {
    Success,
    Failure(Failure),
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Phase {
    Unused,
    Queued,
    Connecting(u8),
    Handshaking,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum AttemptState {
    NotStarted,
    Prepared,
    InFlight,
    Refused,
    TimedOut,
    Connected,
    ConnectedLate,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Attempt {
    state: AttemptState,
    started: u8,
    deadline: u8,
    connected_at: Option<u8>,
}

impl Attempt {
    const EMPTY: Self = Self {
        state: AttemptState::NotStarted,
        started: 0,
        deadline: 0,
        connected_at: None,
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Custody {
    None,
    Worker,
    Cell,
    Owner,
    Closer,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum CloseReason {
    None,
    WorkerFailure,
    WorkerRetired,
    LateConnect,
    CellRevoked,
    Owner,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ConnectResult {
    Refused,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Worker {
    Idle,
    ConnectReady,
    Connecting,
    ConnectReturned(ConnectResult),
    ConnectConnected,
    HandshakeReady,
    HandshakeRunning,
    VerifierOpaque,
    VerifierAccepted,
    SuccessReady,
    ClosingFailure(Failure),
    ClosingRetired,
    ClosingLateConnect,
    FailureReady(Failure),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct Cell {
    identity: u8,
    admitted_at: u8,
    phase: Phase,
    phase_started: u8,
    phase_deadline: u8,
    queue_deadline: u8,
    attempts: [Attempt; ADDRESSES],
    owner_alive: bool,
    terminal: Option<Terminal>,
    terminal_at: u8,
    terminal_deadline: u8,
    terminal_count: u8,
    result_occupied: bool,
    publication_count: u8,
    published_at: Option<u8>,
    taken_mask: u8,
    take_count: u8,
    revoked: bool,
    revoked_at: Option<u8>,
    revocation_reason: Option<Failure>,
    owner_dropped: bool,
    cancelled: bool,
    blocking_started: bool,
    blocking_started_at: u8,
    blocking_waiting: bool,
    blocking_returned: bool,
    blocking_saw_terminal: bool,
    blocking_deadline: u8,
    stepped_started: bool,
    stepped_due: bool,
    stepped_polls: u8,
    stepped_saw_terminal: bool,
    wake_pending: bool,
    wake_count: u8,
    failure_returned: bool,
}

impl Cell {
    const EMPTY: Self = Self {
        identity: 0,
        admitted_at: 0,
        phase: Phase::Unused,
        phase_started: 0,
        phase_deadline: 0,
        queue_deadline: 0,
        attempts: [Attempt::EMPTY; ADDRESSES],
        owner_alive: false,
        terminal: None,
        terminal_at: 0,
        terminal_deadline: 0,
        terminal_count: 0,
        result_occupied: false,
        publication_count: 0,
        published_at: None,
        taken_mask: 0,
        take_count: 0,
        revoked: false,
        revoked_at: None,
        revocation_reason: None,
        owner_dropped: false,
        cancelled: false,
        blocking_started: false,
        blocking_started_at: 0,
        blocking_waiting: false,
        blocking_returned: false,
        blocking_saw_terminal: false,
        blocking_deadline: 0,
        stepped_started: false,
        stepped_due: false,
        stepped_polls: 0,
        stepped_saw_terminal: false,
        wake_pending: false,
        wake_count: 0,
        failure_returned: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct State {
    now: u8,
    accepting: bool,
    next_identity: u8,
    active: Option<u8>,
    worker: Worker,
    address_index: u8,
    cells: [Cell; CELLS],
    custody: [Custody; CELLS],
    close_reason: [CloseReason; CELLS],
    close_count: [u8; CELLS],
    stale_owner_check: Option<u8>,
}

impl State {
    const fn initial() -> Self {
        Self {
            now: 0,
            accepting: true,
            next_identity: 1,
            active: None,
            worker: Worker::Idle,
            address_index: 0,
            cells: [Cell::EMPTY; CELLS],
            custody: [Custody::None; CELLS],
            close_reason: [CloseReason::None; CELLS],
            close_count: [0; CELLS],
            stale_owner_check: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Mutation {
    None,
    UnlockedCheckThenMove,
    OmitFailurePublication,
    DoublePublish,
    DoubleTake,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum HandshakeReturn {
    WouldBlock,
    TlsAlert,
    WorkerLimit,
    Success,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Action {
    Tick,
    Admit(u8),
    Claim(u8),
    StartConnect(u8, u8),
    ConnectRefused(u8, u8),
    ConnectSuccess(u8, u8),
    ConnectTimeout(u8, u8),
    ResolveConnect(u8, u8),
    BeginHandshake(u8),
    StartHandshakeTurn(u8),
    EnterVerifier(u8),
    VerifierAccept(u8),
    VerifierReject(u8),
    HandshakeReturn(u8, HandshakeReturn),
    PublishSuccess(u8),
    PublishFailure(u8, Failure),
    OmitFailurePublication(u8, Failure),
    SelectExpiry(u8),
    WorkerRetire(u8),
    CloseSocket(u8),
    PhysicalClose(u8),
    StartBlockingWait(u8),
    BlockingDeadline(u8),
    StartSteppedPoll(u8),
    SteppedPoll(u8),
    WakeOwner(u8),
    Take { cell: u8, contender: u8 },
    DropOwner(u8),
    Cancel(u8),
    Shutdown,
    OwnerClose(u8),
    CheckOwnerLive(u8),
    PublishAfterUncheckedCheck(u8),
    DuplicatePublish(u8),
}

#[derive(Clone, Copy, Debug, Default)]
struct Coverage {
    refused_address: bool,
    connect_refusal_terminal: bool,
    address_deadline_transition: bool,
    tls_alert: bool,
    tls_alert_terminal: bool,
    certificate_rejection: bool,
    certificate_rejection_terminal: bool,
    worker_limit_failure: bool,
    worker_limit_terminal: bool,
    success: bool,
    expiry_before_publication: bool,
    success_survived_deadline: bool,
    success_taken_after_deadline: bool,
    success_published_just_before_deadline: bool,
    second_take_attempt: bool,
    published_untaken_when_next_identity_starts: bool,
    owner_drop_while_worker_holds_socket: bool,
    publication_before_owner_drop: bool,
    owner_drop_drained_published_socket: bool,
    shutdown_drained_published_socket: bool,
    shutdown_woke_blocking_owner: bool,
    cancellation: bool,
    shutdown: bool,
    opaque_verifier_blocking_deadline: bool,
    opaque_verifier_expiry: bool,
    stepped_poll: bool,
    stepped_terminal_poll: bool,
    queued_expiry: bool,
}

impl Coverage {
    fn observe(&mut self, before: &State, action: Action, after: &State) {
        match action {
            Action::ConnectRefused(_, _) => self.refused_address = true,
            Action::ResolveConnect(index, address) if address == 0 => {
                if matches!(
                    before.worker,
                    Worker::ConnectReturned(ConnectResult::TimedOut)
                ) && after.cells[index as usize].phase == Phase::Connecting(1)
                    && after.cells[index as usize].terminal.is_none()
                {
                    self.address_deadline_transition = true;
                }
            }
            Action::HandshakeReturn(_, HandshakeReturn::TlsAlert) => self.tls_alert = true,
            Action::VerifierReject(_) => self.certificate_rejection = true,
            Action::HandshakeReturn(_, HandshakeReturn::WorkerLimit) => {
                self.worker_limit_failure = true
            }
            Action::PublishFailure(index, failure) => {
                if after.cells[index as usize].terminal == Some(Terminal::Failure(failure)) {
                    match failure {
                        Failure::ConnectRefused => self.connect_refusal_terminal = true,
                        Failure::TlsAlert => self.tls_alert_terminal = true,
                        Failure::CertificateRejected => {
                            self.certificate_rejection_terminal = true
                        }
                        Failure::WorkerLimit => self.worker_limit_terminal = true,
                        Failure::Cancelled | Failure::OwnerDropped | Failure::Shutdown => {}
                    }
                }
            }
            Action::PublishSuccess(index) => {
                let cell = after.cells[index as usize];
                if cell.terminal == Some(Terminal::Success) && cell.publication_count == 1 {
                    self.success = true;
                }
            }
            Action::Tick => {
                for cell in &before.cells {
                    if cell.terminal == Some(Terminal::Success)
                        && cell.result_occupied
                        && before.now < cell.phase_deadline
                        && after.now >= cell.phase_deadline
                    {
                        self.success_survived_deadline = true;
                    }
                }
            }
            Action::SelectExpiry(index) => {
                let cell = before.cells[index as usize];
                if before.active == Some(index) && before.worker == Worker::VerifierOpaque {
                    self.opaque_verifier_expiry = true;
                }
                if cell.publication_count == 0 {
                    self.expiry_before_publication = true;
                    if cell.phase == Phase::Queued {
                        self.queued_expiry = true;
                    }
                }
            }
            Action::Take { cell, .. } => {
                let index = cell as usize;
                let before_cell = before.cells[index];
                if before_cell.terminal == Some(Terminal::Success) && before_cell.take_count > 0 {
                    self.second_take_attempt = true;
                }
                if before_cell.terminal == Some(Terminal::Success)
                    && before_cell.phase_deadline < before.now
                    && after.cells[index].take_count > before_cell.take_count
                {
                    self.success_taken_after_deadline = true;
                    self.success_published_just_before_deadline |= before_cell
                        .published_at
                        .is_some_and(|published| published + 1 == before_cell.phase_deadline);
                }
            }
            Action::Admit(1) => {
                let first = before.cells[0];
                if first.terminal == Some(Terminal::Success)
                    && first.result_occupied
                    && first.take_count == 0
                {
                    self.published_untaken_when_next_identity_starts = true;
                }
            }
            Action::DropOwner(index) => {
                let cell = before.cells[index as usize];
                let after_cell = after.cells[index as usize];
                if before.custody[index as usize] == Custody::Worker {
                    self.owner_drop_while_worker_holds_socket = true;
                }
                if cell.terminal == Some(Terminal::Success)
                    && cell.publication_count == 1
                    && cell.result_occupied
                    && cell.take_count == 0
                {
                    self.publication_before_owner_drop = true;
                    if after.custody[index as usize] == Custody::Closer
                        && !after_cell.result_occupied
                        && after_cell.wake_pending
                    {
                        self.owner_drop_drained_published_socket = true;
                    }
                }
            }
            Action::Cancel(_) => self.cancellation = true,
            Action::Shutdown => {
                self.shutdown = true;
                for (index, cell) in before.cells.iter().enumerate() {
                    let after_cell = after.cells[index];
                    if cell.terminal == Some(Terminal::Success)
                        && cell.result_occupied
                        && after.custody[index] == Custody::Closer
                        && !after_cell.result_occupied
                        && after_cell.wake_pending
                    {
                        self.shutdown_drained_published_socket = true;
                    }
                    if cell.owner_alive && cell.blocking_waiting && after_cell.wake_pending {
                        self.shutdown_woke_blocking_owner = true;
                    }
                }
            }
            Action::BlockingDeadline(index) => {
                if before.active == Some(index)
                    && before.worker == Worker::VerifierOpaque
                {
                    self.opaque_verifier_blocking_deadline = true;
                }
            }
            Action::SteppedPoll(index) => {
                self.stepped_poll = true;
                self.stepped_terminal_poll |= after.cells[index as usize].stepped_saw_terminal;
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
struct Node {
    state: State,
    parent: Option<(usize, Action)>,
}

struct Exhausted {
    explored_states: usize,
    transitions: usize,
    coverage: Coverage,
}

struct Counterexample {
    explored_states: usize,
    trace: Vec<Action>,
    violation: String,
}

enum SearchResult {
    Exhausted(Exhausted),
    Counterexample(Counterexample),
}

// FIFO breadth-first search and first-visit parents make every mutant trace shortest.
fn explore(mutation: Mutation) -> SearchResult {
    let initial = State::initial();
    assert!(invariant(&initial, mutation).is_none());
    let mut nodes = vec![Node {
        state: initial,
        parent: None,
    }];
    let mut seen = HashMap::new();
    seen.insert(initial, 0usize);
    let mut queue = VecDeque::from([0usize]);
    let mut coverage = Coverage::default();
    let mut transitions = 0usize;

    while let Some(node_index) = queue.pop_front() {
        let state = nodes[node_index].state;
        assert!(
            state.now < MAX_TIME || !has_future_deadline(&state),
            "HTTPS model clock horizon truncated a recorded deadline"
        );
        for (action, next) in successors(&state, mutation) {
            transitions += 1;
            coverage.observe(&state, action, &next);
            if let Some(violation) = invariant(&next, mutation) {
                assert!(
                    nodes.len() < MAX_STATES,
                    "HTTPS counterexample exceeded its declared {MAX_STATES}-state bound"
                );
                let trace = shortest_trace(&nodes, node_index, action);
                return SearchResult::Counterexample(Counterexample {
                    explored_states: nodes.len() + 1,
                    trace,
                    violation,
                });
            }
            if let std::collections::hash_map::Entry::Vacant(entry) = seen.entry(next) {
                assert!(
                    nodes.len() < MAX_STATES,
                    "HTTPS model did not exhaust within its declared {MAX_STATES}-state bound"
                );
                let next_index = nodes.len();
                entry.insert(next_index);
                nodes.push(Node {
                    state: next,
                    parent: Some((node_index, action)),
                });
                queue.push_back(next_index);
            }
        }
    }

    SearchResult::Exhausted(Exhausted {
        explored_states: nodes.len(),
        transitions,
        coverage,
    })
}

fn shortest_trace(nodes: &[Node], parent: usize, final_action: Action) -> Vec<Action> {
    let mut trace = vec![final_action];
    let mut cursor = parent;
    while let Some((previous, action)) = nodes[cursor].parent {
        trace.push(action);
        cursor = previous;
    }
    trace.reverse();
    trace
}

fn has_future_deadline(state: &State) -> bool {
    state.cells.iter().any(|cell| {
        cell.identity != 0
            && ((cell.terminal.is_none() && state.now < cell.phase_deadline)
                || (cell.terminal == Some(Terminal::Success)
                    && cell.result_occupied
                    && state.now < cell.phase_deadline)
                || (cell.blocking_waiting && state.now < cell.blocking_deadline))
    })
}

fn has_due_deadline(state: &State) -> bool {
    state.cells.iter().any(|cell| {
        cell.identity != 0
            && ((cell.terminal.is_none() && state.now >= cell.phase_deadline)
                || (cell.blocking_waiting && state.now >= cell.blocking_deadline))
    })
}

fn invariant(state: &State, mutation: Mutation) -> Option<String> {
    if state.next_identity > (CELLS as u8 + 1) {
        return Some("identity allocator exceeded its two never-reused cells".into());
    }
    if state.active.is_none() != (state.worker == Worker::Idle) {
        return Some("worker activity and its single active identity disagree".into());
    }

    let mut workers_with_socket = 0usize;
    for (index, &cell) in state.cells.iter().enumerate() {
        let custody = state.custody[index];
        let id = index as u8;

        if cell.identity != 0 && cell.identity != id + 1 {
            return Some(format!("cell {id} changed its never-reused identity"));
        }
        if cell.identity == 0 {
            if cell.phase != Phase::Unused {
                return Some(format!("unadmitted cell {id} has a live phase"));
            }
            continue;
        }
        if cell.queue_deadline != cell.admitted_at + QUEUE_BUDGET {
            return Some(format!("cell {id} changed its original queue deadline"));
        }
        if cell.terminal_count != u8::from(cell.terminal.is_some()) {
            return Some(format!("cell {id} selected zero or multiple terminal states"));
        }
        if cell.terminal.is_none() && cell.publication_count != 0 {
            return Some(format!("cell {id} published before terminal selection"));
        }
        if cell.publication_count > 1 {
            return Some(format!("cell {id} published its result more than once"));
        }
        if cell.revoked != cell.revoked_at.is_some()
            || cell.revoked != cell.revocation_reason.is_some()
        {
            return Some(format!("cell {id} lost its first revocation reason or time"));
        }
        if let (Some(at), Some(reason)) = (cell.revoked_at, cell.revocation_reason) {
            if !matches!(
                reason,
                Failure::OwnerDropped | Failure::Cancelled | Failure::Shutdown
            ) || at > state.now
                || cell.terminal.is_none()
                || cell.terminal_at > at
            {
                return Some(format!("cell {id} recorded an invalid revocation clock"));
            }
        }
        if cell.result_occupied && cell.terminal.is_none() {
            return Some(format!("cell {id} is occupied without a terminal result"));
        }
        if cell.result_occupied && cell.publication_count != 1 {
            return Some(format!("cell {id} is occupied without one publication"));
        }
        if cell.publication_count == 0 && cell.published_at.is_some() {
            return Some(format!("cell {id} has a publication time without a publication"));
        }
        if cell.publication_count == 1 && cell.published_at.is_none() {
            return Some(format!("cell {id} lost its original publication time"));
        }
        if cell.take_count > 1 {
            return Some(format!("cell {id} result was taken more than once"));
        }
        if cell.take_count > 0 && cell.publication_count != 1 {
            return Some(format!("cell {id} was taken without a published result"));
        }
        if cell.taken_mask & !0b11 != 0 {
            return Some(format!("cell {id} has an unknown take contender"));
        }
        if cell.terminal.is_some() {
            if cell.terminal_at > cell.terminal_deadline {
                return Some(format!("cell {id} selected its terminal state after its recorded deadline"));
            }
            match cell.terminal {
                Some(Terminal::Success) if cell.terminal_at >= cell.terminal_deadline => {
                    return Some(format!("cell {id} selected success at or after its deadline"));
                }
                Some(Terminal::Expired) if cell.terminal_at != cell.terminal_deadline => {
                    return Some(format!("cell {id} did not retain the original expiry clock"));
                }
                _ => {}
            }
            if cell.wake_count == 0 && !cell.wake_pending && cell.owner_alive {
                return Some(format!("cell {id} terminal state has no owner wake"));
            }
        }
        if cell.blocking_started {
            if cell.blocking_deadline != cell.blocking_started_at + BLOCKING_OWNER_BUDGET {
                return Some(format!("cell {id} refreshed its blocking owner's original deadline"));
            }
        } else if cell.blocking_waiting || cell.blocking_returned || cell.blocking_saw_terminal {
            return Some(format!("cell {id} has a blocking wait without a start"));
        }
        if cell.blocking_returned && cell.blocking_waiting {
            return Some(format!("cell {id} both returned and still waits on its blocking call"));
        }
        if cell.blocking_saw_terminal && (!cell.blocking_returned || cell.terminal.is_none()) {
            return Some(format!("cell {id} blocking owner observed no terminal outcome"));
        }
        if cell.blocking_returned
            && !cell.blocking_saw_terminal
            && state.now < cell.blocking_deadline
        {
            return Some(format!("cell {id} returned its blocking call before its fixed deadline"));
        }
        if cell.stepped_saw_terminal && cell.terminal.is_none() {
            return Some(format!("cell {id} stepped poll saw no terminal result"));
        }
        if cell.stepped_polls > 2 {
            return Some(format!("cell {id} exceeded the finite stepped-poll bound"));
        }

        match cell.phase {
            Phase::Unused => return Some(format!("admitted cell {id} returned to Unused")),
            Phase::Queued => {
                if cell.phase_started != cell.admitted_at
                    || cell.phase_deadline != cell.queue_deadline
                {
                    return Some(format!("cell {id} changed its queued phase clock"));
                }
            }
            Phase::Connecting(address) => {
                let attempt = cell.attempts[address as usize];
                if attempt.state == AttemptState::NotStarted
                    || cell.phase_started != attempt.started
                    || cell.phase_deadline != attempt.deadline
                {
                    return Some(format!("cell {id} lost address {address}'s recorded clock"));
                }
            }
            Phase::Handshaking => {
                if cell.phase_deadline != cell.phase_started + HANDSHAKE_BUDGET
                    || !cell.attempts.iter().any(|attempt| {
                        attempt.state == AttemptState::Connected
                            && attempt.connected_at == Some(cell.phase_started)
                    })
                {
                    return Some(format!(
                        "cell {id} handshake deadline is not anchored to connect success"
                    ));
                }
            }
        }

        for (address, attempt) in cell.attempts.iter().enumerate() {
            let connected = matches!(
                attempt.state,
                AttemptState::Connected | AttemptState::ConnectedLate
            );
            if connected != attempt.connected_at.is_some() {
                return Some(format!("cell {id} lost address {address}'s connect-return time"));
            }
            if attempt.state == AttemptState::NotStarted {
                if attempt.started != 0 || attempt.deadline != 0 {
                    return Some(format!("cell {id} has clock data for unused address {address}"));
                }
            } else if attempt.deadline != attempt.started + CONNECT_BUDGET {
                return Some(format!("cell {id} changed address {address}'s connect deadline"));
            }
        }

        match custody {
            Custody::None => {
                if state.close_count[index] != 0 {
                    return Some(format!("cell {id} records a close without a socket"));
                }
            }
            Custody::Worker => {
                workers_with_socket += 1;
                if state.active != Some(id) {
                    return Some(format!("cell {id} worker socket has no worker custodian"));
                }
            }
            Custody::Cell => {
                if cell.terminal != Some(Terminal::Success)
                    || !cell.result_occupied
                    || cell.revoked
                {
                    return Some(format!("cell {id} holds a socket without valid success authority"));
                }
            }
            Custody::Owner => {
                if cell.terminal != Some(Terminal::Success) || cell.take_count != 1 {
                    return Some(format!("cell {id} owner socket lacks one successful take"));
                }
            }
            Custody::Closer => {
                if state.close_reason[index] == CloseReason::None || state.close_count[index] != 0 {
                    return Some(format!("cell {id} closer lacks a unique post-transfer close"));
                }
            }
            Custody::Closed => {
                if state.close_count[index] != 1 {
                    return Some(format!("cell {id} closed without exactly one close proof"));
                }
            }
        }

        if cell.terminal == Some(Terminal::Success)
            && cell.result_occupied
            && custody != Custody::Cell
            && !(mutation == Mutation::DoubleTake
                && cell.take_count == 1
                && custody == Custody::Owner)
        {
            return Some(format!("cell {id} has an occupied success outside cell custody"));
        }
        if cell.terminal != Some(Terminal::Success) && custody == Custody::Owner {
            return Some(format!("cell {id} transferred non-success authority to its owner"));
        }
        if cell.revoked && custody == Custody::Cell {
            return Some(format!("revoked cell {id} retained socket authority"));
        }
        if cell.blocking_waiting && cell.terminal.is_some()
            && !cell.wake_pending
            && cell.wake_count == 0
        {
            return Some(format!("cell {id} terminal selection stranded its blocking owner"));
        }
        if mutation == Mutation::OmitFailurePublication
            && cell.failure_returned
            && state.active != Some(id)
            && cell.terminal.is_none()
        {
            return Some(format!("worker closed cell {id}'s failed socket but published no result"));
        }
    }

    if workers_with_socket > 1 {
        return Some("more than one physical socket has the single worker as custodian".into());
    }
    for (index, (&custody, &reason)) in state
        .custody
        .iter()
        .zip(state.close_reason.iter())
        .enumerate()
    {
        if custody == Custody::Closer && reason == CloseReason::None {
            return Some(format!("cell {index} close has no ownership-transfer reason"));
        }
    }
    None
}

fn successors(state: &State, mutation: Mutation) -> Vec<(Action, State)> {
    let mut next = Vec::new();
    let mut add = |action, changed| next.push((action, changed));

    if state.now < MAX_TIME && has_future_deadline(state) && !has_due_deadline(state) {
        let mut changed = *state;
        changed.now += 1;
        add(Action::Tick, changed);
    }

    if state.accepting && state.next_identity == 1 {
        let mut changed = *state;
        admit(&mut changed, 0);
        add(Action::Admit(0), changed);
    }
    if state.accepting
        && state.next_identity == 2
        && state.cells[0].terminal.is_some()
        && state.cells[0].result_occupied
        && state.cells[0].publication_count == 1
        && state.cells[0].take_count == 0
        && !state.cells[0].revoked
    {
        let mut changed = *state;
        admit(&mut changed, 1);
        add(Action::Admit(1), changed);
    }

    if state.accepting && state.active.is_none() && state.worker == Worker::Idle {
        if let Some(index) = state.cells.iter().position(|cell| {
            cell.identity != 0
                && cell.phase == Phase::Queued
                && cell.terminal.is_none()
                && cell.owner_alive
                && state.now < cell.queue_deadline
        }) {
            let mut changed = *state;
            let cell = &mut changed.cells[index];
            cell.phase = Phase::Connecting(0);
            cell.phase_started = changed.now;
            cell.phase_deadline = changed.now + CONNECT_BUDGET;
            cell.attempts[0] = Attempt {
                state: AttemptState::Prepared,
                started: changed.now,
                deadline: changed.now + CONNECT_BUDGET,
                connected_at: None,
            };
            changed.active = Some(index as u8);
            changed.address_index = 0;
            changed.worker = Worker::ConnectReady;
            cell.wake_pending = true;
            add(Action::Claim(index as u8), changed);
        }
    }

    if let Some(active) = state.active {
        let index = active as usize;
        let cell = state.cells[index];
        match state.worker {
            Worker::ConnectReady => {
                let mut changed = *state;
                if changed.now >= cell.phase_deadline {
                    changed.cells[index].attempts[state.address_index as usize].state =
                        AttemptState::TimedOut;
                    changed.worker = Worker::ConnectReturned(ConnectResult::TimedOut);
                } else {
                    changed.cells[index].attempts[state.address_index as usize].state =
                        AttemptState::InFlight;
                    changed.worker = Worker::Connecting;
                }
                add(Action::StartConnect(active, state.address_index), changed);
            }
            Worker::Connecting => {
                if state.now < cell.phase_deadline {
                    let mut changed = *state;
                    changed.cells[index].attempts[state.address_index as usize].state =
                        AttemptState::Refused;
                    changed.worker = Worker::ConnectReturned(ConnectResult::Refused);
                    add(
                        Action::ConnectRefused(active, state.address_index),
                        changed,
                    );
                }
                let mut changed = *state;
                if state.now >= cell.phase_deadline {
                    changed.cells[index].attempts[state.address_index as usize].state =
                        AttemptState::TimedOut;
                    changed.worker = Worker::ConnectReturned(ConnectResult::TimedOut);
                    add(
                        Action::ConnectTimeout(active, state.address_index),
                        changed,
                    );
                }
                if state.now <= cell.phase_deadline {
                    let mut changed = *state;
                    let attempt =
                        &mut changed.cells[index].attempts[state.address_index as usize];
                    attempt.state = if state.now < cell.phase_deadline {
                        AttemptState::Connected
                    } else {
                        AttemptState::ConnectedLate
                    };
                    attempt.connected_at = Some(state.now);
                    changed.custody[index] = Custody::Worker;
                    if state.now < cell.phase_deadline {
                        changed.worker = Worker::ConnectConnected;
                    } else {
                        changed.worker = Worker::ClosingLateConnect;
                    }
                    add(
                        Action::ConnectSuccess(active, state.address_index),
                        changed,
                    );
                }
            }
            Worker::ConnectReturned(result) => {
                let mut changed = *state;
                let terminal = changed.cells[index].terminal;
                if terminal.is_some() || !changed.cells[index].owner_alive || !changed.accepting {
                    changed.active = None;
                    changed.worker = Worker::Idle;
                } else if state.address_index as usize + 1 < ADDRESSES {
                    let next_address = state.address_index + 1;
                    let cell = &mut changed.cells[index];
                    cell.phase = Phase::Connecting(next_address);
                    cell.phase_started = changed.now;
                    cell.phase_deadline = changed.now + CONNECT_BUDGET;
                    cell.attempts[next_address as usize] = Attempt {
                        state: AttemptState::Prepared,
                        started: changed.now,
                        deadline: changed.now + CONNECT_BUDGET,
                        connected_at: None,
                    };
                    cell.wake_pending = true;
                    changed.address_index = next_address;
                    changed.worker = Worker::ConnectReady;
                } else if result == ConnectResult::Refused
                    && state.now < cell.phase_deadline
                {
                    changed.cells[index].failure_returned = true;
                    changed.worker = Worker::FailureReady(Failure::ConnectRefused);
                } else {
                    select_expiry(&mut changed, index);
                    changed.active = None;
                    changed.worker = Worker::Idle;
                }
                add(Action::ResolveConnect(active, state.address_index), changed);
            }
            Worker::ConnectConnected => {
                let mut changed = *state;
                let cell = &mut changed.cells[index];
                if cell.terminal.is_some() || !cell.owner_alive || !changed.accepting {
                    changed.custody[index] = Custody::Closer;
                    changed.close_reason[index] = CloseReason::WorkerRetired;
                    changed.worker = Worker::ClosingRetired;
                } else {
                    let connected_at = cell.attempts[state.address_index as usize]
                        .connected_at
                        .expect("connect return omitted its timestamp");
                    cell.phase = Phase::Handshaking;
                    cell.phase_started = connected_at;
                    cell.phase_deadline = connected_at + HANDSHAKE_BUDGET;
                    cell.wake_pending = true;
                    changed.worker = Worker::HandshakeReady;
                }
                add(Action::BeginHandshake(active), changed);
            }
            Worker::HandshakeReady => {
                if cell.terminal.is_some() || !cell.owner_alive || !state.accepting {
                    let mut changed = *state;
                    changed.custody[index] = Custody::Closer;
                    changed.close_reason[index] = CloseReason::WorkerRetired;
                    changed.worker = Worker::ClosingRetired;
                    add(Action::WorkerRetire(active), changed);
                } else if state.now < cell.phase_deadline {
                    let mut changed = *state;
                    changed.worker = Worker::HandshakeRunning;
                    add(Action::StartHandshakeTurn(active), changed);
                }
            }
            Worker::HandshakeRunning => {
                let mut changed = *state;
                changed.worker = Worker::HandshakeReady;
                add(
                    Action::HandshakeReturn(active, HandshakeReturn::WouldBlock),
                    changed,
                );
                let mut changed = *state;
                changed.worker = Worker::ClosingFailure(Failure::TlsAlert);
                changed.cells[index].failure_returned = true;
                add(
                    Action::HandshakeReturn(active, HandshakeReturn::TlsAlert),
                    changed,
                );
                let mut changed = *state;
                changed.worker = Worker::ClosingFailure(Failure::WorkerLimit);
                changed.cells[index].failure_returned = true;
                add(
                    Action::HandshakeReturn(active, HandshakeReturn::WorkerLimit),
                    changed,
                );
                let mut changed = *state;
                changed.worker = Worker::VerifierOpaque;
                add(Action::EnterVerifier(active), changed);
            }
            Worker::VerifierOpaque => {
                let mut changed = *state;
                changed.worker = Worker::VerifierAccepted;
                add(Action::VerifierAccept(active), changed);
                let mut changed = *state;
                changed.worker = Worker::ClosingFailure(Failure::CertificateRejected);
                changed.cells[index].failure_returned = true;
                add(Action::VerifierReject(active), changed);
            }
            Worker::VerifierAccepted => {
                let mut changed = *state;
                if state.now >= cell.phase_deadline {
                    changed.cells[index].failure_returned = true;
                    changed.worker = Worker::ClosingFailure(Failure::WorkerLimit);
                } else {
                    changed.worker = Worker::SuccessReady;
                }
                add(
                    Action::HandshakeReturn(active, HandshakeReturn::Success),
                    changed,
                );
            }
            Worker::SuccessReady => match mutation {
                Mutation::UnlockedCheckThenMove => {
                    if cell.owner_alive
                        && cell.terminal.is_none()
                        && state.accepting
                        && state.now < cell.phase_deadline
                    {
                        let mut changed = *state;
                        changed.stale_owner_check = Some(active);
                        add(Action::CheckOwnerLive(active), changed);
                    }
                    if state.stale_owner_check == Some(active)
                        && cell.owner_dropped
                    {
                        let mut changed = *state;
                        if cell.terminal.is_none() {
                            let now = changed.now;
                            let deadline = cell.phase_deadline;
                            select_terminal(
                                &mut changed,
                                index,
                                Terminal::Success,
                                now,
                                deadline,
                                true,
                            );
                        } else if changed.cells[index].publication_count == 0 {
                            changed.cells[index].result_occupied = true;
                            changed.cells[index].publication_count = 1;
                            changed.cells[index].published_at = Some(changed.now);
                        } else {
                            changed.cells[index].result_occupied = true;
                            changed.cells[index].publication_count += 1;
                        }
                        changed.custody[index] = Custody::Cell;
                        changed.active = None;
                        changed.worker = Worker::Idle;
                        add(Action::PublishAfterUncheckedCheck(active), changed);
                    }
                    if cell.terminal.is_some() || !cell.owner_alive || !state.accepting {
                        let mut changed = *state;
                        changed.custody[index] = Custody::Closer;
                        changed.close_reason[index] = CloseReason::WorkerRetired;
                        changed.worker = Worker::ClosingRetired;
                        add(Action::WorkerRetire(active), changed);
                    }
                }
                _ => {
                    let mut changed = *state;
                    if cell.terminal.is_none()
                        && cell.owner_alive
                        && changed.accepting
                        && changed.now < cell.phase_deadline
                    {
                        let now = changed.now;
                        let deadline = cell.phase_deadline;
                        select_terminal(
                            &mut changed,
                            index,
                            Terminal::Success,
                            now,
                            deadline,
                            true,
                        );
                        changed.custody[index] = Custody::Cell;
                        changed.active = None;
                        changed.worker = Worker::Idle;
                    } else {
                        if cell.terminal.is_none() && changed.now >= cell.phase_deadline {
                            select_expiry(&mut changed, index);
                        }
                        changed.custody[index] = Custody::Closer;
                        changed.close_reason[index] = CloseReason::WorkerRetired;
                        changed.worker = Worker::ClosingRetired;
                    }
                    add(Action::PublishSuccess(active), changed);
                }
            },
            Worker::ClosingFailure(_) | Worker::ClosingRetired | Worker::ClosingLateConnect => {
                if state.custody[index] == Custody::Worker {
                    let mut changed = *state;
                    let reason = match state.worker {
                        Worker::ClosingFailure(_) => CloseReason::WorkerFailure,
                        Worker::ClosingRetired => CloseReason::WorkerRetired,
                        Worker::ClosingLateConnect => CloseReason::LateConnect,
                        _ => unreachable!(),
                    };
                    changed.custody[index] = Custody::Closer;
                    changed.close_reason[index] = reason;
                    add(Action::CloseSocket(active), changed);
                }
            }
            Worker::FailureReady(failure) => {
                let mut changed = *state;
                match mutation {
                    Mutation::OmitFailurePublication
                        if state.custody[index] == Custody::Closed
                            && state.close_count[index] == 1 =>
                    {
                        changed.active = None;
                        changed.worker = Worker::Idle;
                        add(Action::OmitFailurePublication(active, failure), changed);
                    }
                    _ => {
                        if changed.cells[index].terminal.is_none() {
                            if changed.now >= changed.cells[index].phase_deadline {
                                select_expiry(&mut changed, index);
                            } else {
                                let now = changed.now;
                                let cell = changed.cells[index];
                                select_terminal(
                                    &mut changed,
                                    index,
                                    Terminal::Failure(failure),
                                    now,
                                    cell.phase_deadline.max(now),
                                    cell.owner_alive,
                                );
                            }
                        }
                        changed.active = None;
                        changed.worker = Worker::Idle;
                        add(Action::PublishFailure(active, failure), changed);
                    }
                }
            }
            Worker::Idle => unreachable!(),
        }
    }

    for (index, &cell) in state.cells.iter().enumerate() {
        if cell.identity == 0 {
            continue;
        }

        let expiry_eligible = match cell.phase {
            Phase::Queued | Phase::Handshaking => state.now >= cell.phase_deadline,
            Phase::Connecting(address) => {
                state.now >= cell.phase_deadline
                    && address as usize + 1 == ADDRESSES
            }
            Phase::Unused => false,
        };
        if cell.terminal.is_none() && expiry_eligible {
            let mut changed = *state;
            select_expiry(&mut changed, index);
            add(Action::SelectExpiry(index as u8), changed);
        }

        // This edge models Condvar::wait_timeout's atomic unlock-and-park.
        if cell.owner_alive && !cell.blocking_started && cell.terminal.is_none() {
            let mut changed = *state;
            let entry = &mut changed.cells[index];
            entry.blocking_started = true;
            entry.blocking_started_at = changed.now;
            entry.blocking_deadline = changed.now + BLOCKING_OWNER_BUDGET;
            entry.blocking_waiting = true;
            add(Action::StartBlockingWait(index as u8), changed);
        }
        if cell.blocking_waiting && state.now >= cell.blocking_deadline {
            let mut changed = *state;
            let entry = &mut changed.cells[index];
            entry.blocking_waiting = false;
            entry.blocking_returned = true;
            entry.blocking_saw_terminal = entry.terminal.is_some();
            add(Action::BlockingDeadline(index as u8), changed);
        }
        if cell.owner_alive && !cell.stepped_started && cell.terminal.is_none() {
            let mut changed = *state;
            changed.cells[index].stepped_started = true;
            changed.cells[index].stepped_due = true;
            add(Action::StartSteppedPoll(index as u8), changed);
        }
        // The stepped owner reads the same cell but never changes socket custody.
        if cell.stepped_due {
            let mut changed = *state;
            changed.cells[index].stepped_due = false;
            changed.cells[index].stepped_polls =
                changed.cells[index].stepped_polls.saturating_add(1).min(2);
            if cell.terminal.is_some() {
                changed.cells[index].stepped_saw_terminal = true;
            }
            add(Action::SteppedPoll(index as u8), changed);
        }
        // Notification is outside the lock; the owner re-locks and rechecks here.
        if cell.wake_pending {
            let mut changed = *state;
            let entry = &mut changed.cells[index];
            entry.wake_pending = false;
            entry.wake_count = entry.wake_count.saturating_add(1).min(2);
            if entry.terminal.is_some() && entry.blocking_waiting {
                entry.blocking_waiting = false;
                entry.blocking_returned = true;
                entry.blocking_saw_terminal = true;
            }
            if entry.stepped_started {
                entry.stepped_due = true;
            }
            add(Action::WakeOwner(index as u8), changed);
        }

        if cell.terminal.is_some()
            && (cell.result_occupied || cell.take_count > 0)
        {
            // These are two competing calls by the same logical owner. The
            // HTTPS-cell lock serializes them; they are not two fetch owners.
            for contender in 0..2u8 {
                if cell.taken_mask & (1 << contender) == 0 {
                    let mut changed = *state;
                    let entry = &mut changed.cells[index];
                    entry.taken_mask |= 1 << contender;
                    if entry.result_occupied
                        && entry.owner_alive
                        && (!entry.revoked || entry.terminal != Some(Terminal::Success))
                    {
                        if mutation == Mutation::DoubleTake
                            && entry.terminal == Some(Terminal::Success)
                        {
                            // The mutant transfers the stream but leaves the cell occupied,
                            // allowing the competing take to consume it a second time.
                            entry.take_count += 1;
                            changed.custody[index] = Custody::Owner;
                        } else {
                            entry.result_occupied = false;
                            entry.owner_alive = false;
                            entry.take_count += 1;
                            if entry.terminal == Some(Terminal::Success) {
                                changed.custody[index] = Custody::Owner;
                            }
                        }
                    }
                    add(
                        Action::Take {
                            cell: index as u8,
                            contender,
                        },
                        changed,
                    );
                }
            }
        }

        if cell.owner_alive && !cell.owner_dropped {
            let mut changed = *state;
            revoke_cell(&mut changed, index, Failure::OwnerDropped, true);
            add(Action::DropOwner(index as u8), changed);
        }
        if cell.owner_alive && !cell.cancelled {
            let mut changed = *state;
            revoke_cell(&mut changed, index, Failure::Cancelled, false);
            changed.cells[index].cancelled = true;
            add(Action::Cancel(index as u8), changed);
        }
    }

    if state.accepting {
        let mut changed = *state;
        changed.accepting = false;
        let cells = changed.cells;
        for (index, cell) in cells.iter().enumerate() {
            let should_revoke = cell.identity != 0 && cell.owner_alive;
            if should_revoke {
                revoke_cell(&mut changed, index, Failure::Shutdown, false);
            }
        }
        add(Action::Shutdown, changed);
    }

    for (index, &custody) in state.custody.iter().enumerate() {
        if custody == Custody::Closer {
            let mut changed = *state;
            let reason = changed.close_reason[index];
            changed.custody[index] = Custody::Closed;
            changed.close_count[index] += 1;
            changed.close_reason[index] = CloseReason::None;
            match reason {
                CloseReason::WorkerFailure => {
                    let failure = match changed.worker {
                        Worker::ClosingFailure(failure) => failure,
                        _ => unreachable!(),
                    };
                    changed.worker = Worker::FailureReady(failure);
                }
                CloseReason::WorkerRetired => {
                    if changed.active == Some(index as u8) {
                        changed.active = None;
                        changed.worker = Worker::Idle;
                    }
                }
                CloseReason::LateConnect => {
                    changed.worker = Worker::ConnectReturned(ConnectResult::TimedOut);
                }
                CloseReason::CellRevoked | CloseReason::Owner | CloseReason::None => {}
            }
            add(Action::PhysicalClose(index as u8), changed);
        }
        if custody == Custody::Owner {
            let mut changed = *state;
            changed.custody[index] = Custody::Closer;
            changed.close_reason[index] = CloseReason::Owner;
            add(Action::OwnerClose(index as u8), changed);
        }
    }

    if mutation == Mutation::DoublePublish {
        for (index, &cell) in state.cells.iter().enumerate() {
            if cell.terminal == Some(Terminal::Success)
                && cell.result_occupied
                && state.custody[index] == Custody::Cell
                && cell.publication_count == 1
            {
                let mut changed = *state;
                changed.cells[index].publication_count += 1;
                add(Action::DuplicatePublish(index as u8), changed);
            }
        }
    }

    next
}
fn admit(state: &mut State, index: usize) {
    let identity = state.next_identity;
    debug_assert_eq!(identity as usize, index + 1);
    let deadline = state.now + QUEUE_BUDGET;
    state.cells[index] = Cell {
        identity,
        admitted_at: state.now,
        phase: Phase::Queued,
        phase_started: state.now,
        phase_deadline: deadline,
        queue_deadline: deadline,
        owner_alive: true,
        ..Cell::EMPTY
    };
    state.next_identity += 1;
}

/// Select and publish a terminal outcome in one lock transaction.
///
/// `publish` is false only when owner drop makes the receipt unobservable.
/// Notification is a later outside-lock scheduler step.
fn select_terminal(
    state: &mut State,
    index: usize,
    terminal: Terminal,
    at: u8,
    deadline: u8,
    publish: bool,
) {
    let cell = &mut state.cells[index];
    cell.terminal = Some(terminal);
    cell.terminal_at = at;
    cell.terminal_deadline = deadline;
    cell.terminal_count = cell.terminal_count.saturating_add(1);
    if publish {
        cell.result_occupied = true;
        cell.publication_count = 1;
        cell.published_at = Some(at);
    }
    cell.wake_pending = true;
    if cell.stepped_started {
        cell.stepped_due = true;
    }
}

fn select_expiry(state: &mut State, index: usize) {
    let deadline = state.cells[index].phase_deadline;
    let publish = state.cells[index].owner_alive;
    select_terminal(
        state,
        index,
        Terminal::Expired,
        deadline,
        deadline,
        publish,
    );
}

/// Revoke one ticket while holding the lane lock, then hand any cell socket
/// to the outside-lock closer. A worker-held socket remains worker-owned.
fn revoke_cell(state: &mut State, index: usize, reason: Failure, owner_drop: bool) {
    let cell = state.cells[index];
    if cell.identity == 0 {
        return;
    }
    let now = state.now;
    let expired = cell.terminal.is_none()
        && now >= cell.phase_deadline
        && !matches!(cell.phase, Phase::Connecting(address) if address as usize + 1 < ADDRESSES);
    if cell.terminal.is_none() {
        if expired {
            select_expiry(state, index);
        } else {
            let deadline = cell.phase_deadline.max(now);
            select_terminal(
                state,
                index,
                Terminal::Failure(reason),
                now,
                deadline,
                !owner_drop && cell.owner_alive,
            );
        }
    }

    if state.custody[index] == Custody::Cell {
        state.custody[index] = Custody::Closer;
        state.close_reason[index] = CloseReason::CellRevoked;
        state.cells[index].result_occupied = false;
    }

    let entry = &mut state.cells[index];
    if entry.revoked_at.is_none() {
        entry.revoked_at = Some(now);
        entry.revocation_reason = Some(reason);
    }
    entry.revoked = true;
    if owner_drop {
        entry.owner_dropped = true;
        entry.owner_alive = false;
        entry.result_occupied = false;
    }
    if entry.terminal == Some(Terminal::Success) {
        entry.result_occupied = false;
    }
    entry.wake_pending = true;
    if entry.stepped_started {
        entry.stepped_due = true;
    }
}

fn invariant_name(mutation: Mutation) -> &'static str {
    match mutation {
        Mutation::None => "correct HTTPS state machine",
        Mutation::UnlockedCheckThenMove => "unlocked check-then-move after owner drop",
        Mutation::OmitFailurePublication => "failure closes but does not publish",
        Mutation::DoublePublish => "double publish",
        Mutation::DoubleTake => "double take without occupied-cell transition",
    }
}

fn render_trace(trace: &[Action]) -> String {
    trace
        .iter()
        .map(|action| format!("{action:?}"))
        .collect::<Vec<_>>()
        .join(" -> ")
}

#[test]
fn https_cell_scheduler_exhausts_interleavings_and_finds_shortest_mutants() {
    let result = explore(Mutation::None);
    let exhausted = match result {
        SearchResult::Exhausted(exhausted) => exhausted,
        SearchResult::Counterexample(counterexample) => panic!(
            "correct HTTPS model invariant failed after {} explored states: {}\n{}",
            counterexample.explored_states,
            counterexample.violation,
            render_trace(&counterexample.trace)
        ),
    };
    assert!(exhausted.explored_states <= MAX_STATES);
    let coverage = exhausted.coverage;
    assert!(coverage.refused_address, "no refused connect address was explored");
    assert!(
        coverage.connect_refusal_terminal,
        "no typed refused-address result was published"
    );
    assert!(
        coverage.address_deadline_transition,
        "no nonfinal address deadline transition was explored"
    );
    assert!(coverage.tls_alert, "no TLS alert was explored");
    assert!(coverage.tls_alert_terminal, "no typed TLS alert result was published");
    assert!(
        coverage.certificate_rejection,
        "no certificate rejection was explored"
    );
    assert!(
        coverage.certificate_rejection_terminal,
        "no typed certificate-rejection result was published"
    );
    assert!(
        coverage.worker_limit_failure,
        "no worker-limit failure was explored"
    );
    assert!(coverage.worker_limit_terminal, "no typed worker-limit result was published");
    assert!(coverage.success, "no successful handshake publication was explored");
    assert!(
        coverage.expiry_before_publication,
        "no pre-publication expiry was explored"
    );
    assert!(
        coverage.success_taken_after_deadline,
        "no published success was taken after its deadline"
    );
    assert!(
        coverage.second_take_attempt,
        "no second competing take was explored"
    );
    assert!(
        coverage.success_survived_deadline,
        "published success did not remain valid past its original deadline"
    );
    assert!(
        coverage.success_published_just_before_deadline,
        "no result published just before its deadline was taken afterward"
    );
    assert!(
        coverage.published_untaken_when_next_identity_starts,
        "no second identity started with the first result cell occupied"
    );
    assert!(
        coverage.owner_drop_while_worker_holds_socket,
        "no owner drop raced a worker-held socket"
    );
    assert!(
        coverage.publication_before_owner_drop,
        "no published result preceded owner drop"
    );
    assert!(
        coverage.owner_drop_drained_published_socket,
        "owner drop did not drain the socket or leave the owner wake pending"
    );
    assert!(coverage.cancellation, "no cancellation was explored");
    assert!(coverage.shutdown, "no shutdown was explored");
    assert!(
        coverage.shutdown_drained_published_socket,
        "shutdown did not drain a published socket after ownership transfer"
    );
    assert!(
        coverage.shutdown_woke_blocking_owner,
        "shutdown did not wake a blocked owner"
    );
    assert!(
        coverage.opaque_verifier_blocking_deadline,
        "no blocking owner deadline elapsed during an opaque verifier call"
    );
    assert!(
        coverage.opaque_verifier_expiry,
        "no logical expiry was selected while the verifier remained opaque"
    );
    assert!(coverage.stepped_poll, "no stepped owner poll was explored");
    assert!(
        coverage.stepped_terminal_poll,
        "no stepped owner poll observed a terminal result"
    );
    assert!(coverage.queued_expiry, "no queued fetch expiry was explored");

    println!(
        "HTTPS model exhausted: explored_states={} transitions={} state_bound={} max_time={}",
        exhausted.explored_states, exhausted.transitions, MAX_STATES, MAX_TIME
    );
    println!(
        "HTTPS model assumptions: atomic lock transitions; one worker; one logical owner; two immutable identities and cells; FIFO BFS with state deduplication; fair scheduling of enabled finite steps; clock advances toward recorded deadlines and resolves due transitions before another tick; finite connect call times out at its recorded deadline; a late connect success is closed without publication; verifier may remain opaque forever; no wall-time liveness claim"
    );
    println!("HTTPS model coverage: {coverage:?}");

    for mutation in [
        Mutation::UnlockedCheckThenMove,
        Mutation::OmitFailurePublication,
        Mutation::DoublePublish,
        Mutation::DoubleTake,
    ] {
        match explore(mutation) {
            SearchResult::Counterexample(counterexample) => {
                assert!(counterexample.explored_states <= MAX_STATES);
                assert!(!counterexample.trace.is_empty());
                match mutation {
                    Mutation::UnlockedCheckThenMove => {
                        let checked = counterexample
                            .trace
                            .iter()
                            .position(|action| matches!(*action, Action::CheckOwnerLive(_)))
                            .expect("owner-drop mutant trace omitted its unlocked liveness check");
                        let dropped = counterexample
                            .trace
                            .iter()
                            .position(|action| matches!(*action, Action::DropOwner(_)))
                            .expect("owner-drop mutant trace omitted owner drop");
                        let published = counterexample
                            .trace
                            .iter()
                            .position(|action| {
                                matches!(*action, Action::PublishAfterUncheckedCheck(_))
                            })
                            .expect("owner-drop mutant trace omitted unchecked publication");
                        assert!(checked < dropped && dropped < published);
                    }
                    Mutation::OmitFailurePublication => {
                        let closed = counterexample
                            .trace
                            .iter()
                            .position(|action| matches!(*action, Action::PhysicalClose(_)))
                            .expect("failure mutant trace omitted physical close");
                        let omitted = counterexample
                            .trace
                            .iter()
                            .position(|action| {
                                matches!(*action, Action::OmitFailurePublication(_, _))
                            })
                            .expect("failure mutant trace omitted missing publication");
                        assert!(closed < omitted);
                    }
                    Mutation::DoublePublish => {
                        let published = counterexample
                            .trace
                            .iter()
                            .position(|action| matches!(*action, Action::PublishSuccess(_)))
                            .expect("double-publish mutant omitted initial publication");
                        let duplicate = counterexample
                            .trace
                            .iter()
                            .position(|action| matches!(*action, Action::DuplicatePublish(_)))
                            .expect("double-publish mutant omitted duplicate publication");
                        assert!(published < duplicate);
                    }
                    Mutation::DoubleTake => assert_eq!(
                        counterexample
                            .trace
                            .iter()
                            .filter(|action| matches!(**action, Action::Take { .. }))
                            .count(),
                        2,
                        "double-take mutant trace must contain both competing take calls"
                    ),
                    Mutation::None => panic!("correct model returned a mutant counterexample"),
                }
                println!(
                    "HTTPS mutant={} explored_states={} shortest_trace_len={} counterexample={} trace=[{}]",
                    invariant_name(mutation),
                    counterexample.explored_states,
                    counterexample.trace.len(),
                    counterexample.violation,
                    render_trace(&counterexample.trace)
                );
            }
            SearchResult::Exhausted(exhausted) => panic!(
                "mandatory HTTPS mutant escaped after {} explored states: {}",
                exhausted.explored_states,
                invariant_name(mutation)
            ),
        }
    }
}
