//! One bounded reactor/worker custody cell for the initial TLS/OS handshake.
//!
//! The worker owns the submitted resource only while driving the initial
//! handshake. Every mio Registry operation is performed by the reactor and
//! outside this cell's lock; the original registered socket may remain
//! registered while its coupled resource is owned by the worker.
//!
//! A successful worker result remains in the single result cell until
//! `take_return` accepts it. Waker notifications are only hints: coalescing
//! them cannot erase the published result. Pre-accept revocation publishes a
//! stable `QueueReleaseProof` before waking either side. A running worker keeps
//! its resource, worker/lane charge, and physical retirement unacknowledged
//! until the resource is actually dropped or returned to the reactor.
//! On rejected submit the caller keeps the returned resource. A pending
//! revocation drops it on the revoking path and acknowledges only after drop;
//! a running worker drops it on failure or late success and then acknowledges.
//! After acceptance, the reactor owns registration/commit or drop and
//! acknowledges only after that operation is complete.

use std::io;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use mio::Waker;

use super::cancel::{AttemptRetire, CancelToken};
use super::control::BindingId;

const MAX_HANDOFF_WAIT: Duration = Duration::from_millis(100);

/// Attempt identity includes the cancellation generation and all stable
/// binding identities used by the reactor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandoffIdentity {
    pub generation: u64,
    pub binding: BindingId,
}

/// Exact sequence of one ownership transfer through the handoff cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandoffKey {
    pub identity: HandoffIdentity,
    pub transfer_seq: u64,
}

/// Stable, lock-free queue evidence for one pre-accept logical release.
///
/// The queue may retain an `Arc<QueueReleaseProof>` from admission and inspect
/// it while holding K1; `snapshot` never takes the handoff lock or another
/// mutex. The first exact `publish` wins and its immutable timestamp becomes
/// visible through `OnceLock::get` synchronization.
#[derive(Debug)]
pub struct QueueReleaseProof {
    identity: BindingId,
    transfer_seq: u64,
    published_at: OnceLock<Instant>,
}

impl QueueReleaseProof {
    pub fn new(identity: BindingId, transfer_seq: u64) -> Self {
        Self {
            identity,
            transfer_seq,
            published_at: OnceLock::new(),
        }
    }
    pub fn identity(&self) -> BindingId {
        self.identity
    }

    pub fn transfer_seq(&self) -> u64 {
        self.transfer_seq
    }

    /// Publish the event time once, and only for the exact bound identity.
    fn publish(&self, identity: BindingId, transfer_seq: u64, at: Instant) -> bool {
        if self.identity != identity || self.transfer_seq != transfer_seq {
            return false;
        }
        self.published_at.set(at).is_ok()
    }

    /// Queue reducer unit-test fixture only. Product publication is private
    /// to the handoff reducer; integration proofs must exercise `revoke`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn publish_for_test(&self, identity: BindingId, transfer_seq: u64, at: Instant) -> bool {
        self.publish(identity, transfer_seq, at)
    }

    /// Read the release timestamp without taking K-handoff or K1's other
    /// locks. `OnceLock::get` provides the acquire synchronization for publish.
    pub fn snapshot(&self) -> Option<Instant> {
        self.published_at.get().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffPhase {
    Empty,
    Pending,
    Waiting,
    Running,
    Returned,
    Accepted,
    Released,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadyDirection {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCall {
    Progress,
    WouldBlock,
    SyntheticBudget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevocationReason {
    Cancelled,
    Deadline,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerFailure {
    Handshake,
    Verification,
    Io,
    Shutdown,
    Abandoned,
}

/// The worker's only published result. Success carries custody back to the
/// reactor; failure carries no resource because the worker dropped it first.
#[derive(Debug)]
pub enum WorkerOutcome<T> {
    Success(T),
    Failure(WorkerFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerYield {
    Continue,
    Ready,
    SyntheticBudget,
    Revoked,
    Stopped,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerFinish {
    Returned,
    Revoked,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitErrorKind {
    SequenceExhausted,
    WrongTransferSequence,
    IdentityMismatch,
    DeadlineExpired,
    ProofAlreadyPublished,
    Cancelled,
    WorkerStopped,
    Busy,
}

/// Rejected submission returns the original resource without dropping it.
pub struct SubmitError<T> {
    kind: SubmitErrorKind,
    resource: T,
}

impl<T> SubmitError<T> {
    pub fn kind(&self) -> SubmitErrorKind {
        self.kind
    }

    pub fn into_resource(self) -> T {
        self.resource
    }
}

impl<T> std::fmt::Debug for SubmitError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubmitError")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum TakeReturnError {
    Stale,
    NotReturned,
    AlreadyAccepted,
    Revoked(Option<Arc<QueueReleaseProof>>),
}

/// A result accepted exactly once by the reactor. The reactor moves a success
/// resource out of this result and completes any ownership barrier before it
/// calls `finish_accept`. The retirement cell remains with the reactor so its
/// acknowledgement can wait for the later committed-detachment or close point.
#[derive(Debug)]
pub struct Accepted<T> {
    pub key: HandoffKey,
    pub outcome: WorkerOutcome<T>,
    pub retirement: Arc<AttemptRetire>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishAcceptError {
    Stale,
    NotAccepted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReadyCounter {
    seq: u64,
    exhausted: bool,
}

impl ReadyCounter {
    const fn new() -> Self {
        Self {
            seq: 0,
            exhausted: false,
        }
    }

    fn publish(&mut self) {
        if let Some(next) = self.seq.checked_add(1) {
            self.seq = next;
        } else {
            // Once the finite sequence is exhausted, stay permanently dirty.
            // This sacrifices sleeping rather than permitting an ABA clear.
            self.exhausted = true;
        }
    }

    fn differs_from(self, seq: u64, exhausted: bool) -> bool {
        self.exhausted || exhausted || self.seq != seq
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadySnapshot {
    read: ReadyCounter,
    write: ReadyCounter,
}

impl ReadySnapshot {
    fn new(read: ReadyCounter, write: ReadyCounter) -> Self {
        Self { read, write }
    }

    pub fn read_seq(self) -> u64 {
        self.read.seq
    }

    pub fn write_seq(self) -> u64 {
        self.write.seq
    }
}

struct Attempt<T> {
    key: HandoffKey,
    token: CancelToken,
    deadline: Instant,
    retirement: Arc<AttemptRetire>,
    proof: Arc<QueueReleaseProof>,
    phase: HandoffPhase,
    ready: [ReadyCounter; 2],
    runnable: bool,
    worker_active: bool,
    revocation: Option<RevocationReason>,
    release_proof_issued: bool,
    pending: Option<T>,
    returned: Option<WorkerOutcome<T>>,
}

impl<T> Attempt<T> {
    fn snapshot(&self) -> ReadySnapshot {
        ReadySnapshot::new(self.ready[0], self.ready[1])
    }

    fn revocation_due(&self, now: Instant) -> Option<RevocationReason> {
        self.revocation.or_else(|| {
            if self.token.is_cancelled() {
                Some(RevocationReason::Cancelled)
            } else if now >= self.deadline {
                Some(RevocationReason::Deadline)
            } else {
                None
            }
        })
    }

    fn readiness_changed(&self, snapshot: ReadySnapshot) -> bool {
        self.ready[0].differs_from(snapshot.read.seq, snapshot.read.exhausted)
            || self.ready[1].differs_from(snapshot.write.seq, snapshot.write.exhausted)
    }

    fn consume_runnable(&mut self) {
        // An exhausted counter remains dirty forever, forcing a safe retry.
        self.runnable = self.ready.iter().any(|counter| counter.exhausted);
    }

    fn issue_release_proof(
        &mut self,
        reason: RevocationReason,
        at: Instant,
    ) -> Option<Arc<QueueReleaseProof>> {
        if self.revocation.is_none() {
            self.revocation = Some(reason);
        }
        if self.release_proof_issued {
            return None;
        }
        self.release_proof_issued = true;
        let _ = self
            .proof
            .publish(self.key.identity.binding, self.key.transfer_seq, at);
        Some(Arc::clone(&self.proof))
    }
}

struct HandoffState<T> {
    next_transfer_seq: Option<u64>,
    attempt: Option<Attempt<T>>,
    worker_stopped: bool,
}

/// A single bounded handoff cell shared by the reactor and one handshake
/// worker. `T` owns every resource that must move together (for example the
/// TLS connection and its registered `mio::TcpStream`).
pub struct HandoffCell<T: Send> {
    state: Mutex<HandoffState<T>>,
    worker_wake: Condvar,
    reactor_waker: Arc<Waker>,
    #[cfg(test)]
    wait_interlock: Mutex<Option<Arc<(std::sync::Barrier, std::sync::Barrier)>>>,
}

impl<T: Send> HandoffCell<T> {
    pub fn new(reactor_waker: Arc<Waker>) -> Self {
        Self {
            state: Mutex::new(HandoffState {
                next_transfer_seq: Some(1),
                attempt: None,
                worker_stopped: false,
            }),
            worker_wake: Condvar::new(),
            reactor_waker,
            #[cfg(test)]
            wait_interlock: Mutex::new(None),
        }
    }

    /// Observe the next admissible sequence. `submit` checks it again under K.
    pub fn next_transfer_seq(&self) -> Option<u64> {
        self.state.lock().unwrap().next_transfer_seq
    }

    pub fn phase(&self) -> HandoffPhase {
        self.state
            .lock()
            .unwrap()
            .attempt
            .as_ref()
            .map_or(HandoffPhase::Empty, |attempt| attempt.phase)
    }

    /// A capacity waiter may be promoted only when a new exact transfer
    /// could enter the handoff cell. Released attempts retain their physical
    /// charge until their worker has stopped and retirement was acknowledged.
    pub fn can_admit(&self) -> bool {
        let state = self.state.lock().unwrap();
        !state.worker_stopped
            && state.next_transfer_seq.is_some()
            && state.attempt.as_ref().is_none_or(|attempt| {
                matches!(
                    attempt.phase,
                    HandoffPhase::Accepted | HandoffPhase::Released
                ) && !attempt.worker_active
                    && attempt.retirement.acknowledged()
            })
    }

    /// Transfer custody only if the caller's sequence is still current.
    ///
    /// The supplied proof must be the stable `Arc` retained by the queue. The
    /// sequence is checked before `resource` is placed in the cell; on any
    /// rejection the exact resource is returned in `SubmitError`.
    pub fn submit(
        &self,
        key: HandoffKey,
        proof: Arc<QueueReleaseProof>,
        token: CancelToken,
        retirement: Arc<AttemptRetire>,
        deadline: Instant,
        resource: T,
    ) -> Result<Arc<QueueReleaseProof>, SubmitError<T>> {
        let receipt = Arc::clone(&proof);
        let mut state = self.state.lock().unwrap();

        let kind = match state.next_transfer_seq {
            None => Some(SubmitErrorKind::SequenceExhausted),
            Some(expected) if expected != key.transfer_seq => {
                Some(SubmitErrorKind::WrongTransferSequence)
            }
            Some(_) => None,
        };
        if let Some(kind) = kind {
            return Err(SubmitError { kind, resource });
        }
        if key.identity.generation != token.generation()
            || key.identity.generation != retirement.generation()
            || proof.identity() != key.identity.binding
            || proof.transfer_seq() != key.transfer_seq
        {
            return Err(SubmitError {
                kind: SubmitErrorKind::IdentityMismatch,
                resource,
            });
        }
        if proof.snapshot().is_some() {
            return Err(SubmitError {
                kind: SubmitErrorKind::ProofAlreadyPublished,
                resource,
            });
        }
        if token.is_cancelled() {
            return Err(SubmitError {
                kind: SubmitErrorKind::Cancelled,
                resource,
            });
        }
        if Instant::now() >= deadline {
            return Err(SubmitError {
                kind: SubmitErrorKind::DeadlineExpired,
                resource,
            });
        }
        if state.worker_stopped {
            return Err(SubmitError {
                kind: SubmitErrorKind::WorkerStopped,
                resource,
            });
        }

        let recyclable = state.attempt.as_ref().is_some_and(|attempt| {
            (attempt.phase == HandoffPhase::Accepted
                || (attempt.phase == HandoffPhase::Released && !attempt.worker_active))
                && attempt.retirement.acknowledged()
        });
        if state.attempt.is_some() && !recyclable {
            return Err(SubmitError {
                kind: SubmitErrorKind::Busy,
                resource,
            });
        }
        if recyclable {
            state.attempt = None;
        }

        state.next_transfer_seq = key.transfer_seq.checked_add(1);
        state.attempt = Some(Attempt {
            key,
            token,
            retirement,
            proof,
            deadline,
            phase: HandoffPhase::Pending,
            ready: [ReadyCounter::new(), ReadyCounter::new()],
            runnable: true,
            worker_active: false,
            revocation: None,
            release_proof_issued: false,
            pending: Some(resource),
            returned: None,
        });
        drop(state);
        self.worker_wake.notify_one();
        Ok(receipt)
    }

    /// Publish one direction's edge. The sequence is changed under K before
    /// the worker is notified, so an edge cannot be cleared by an old syscall.
    pub fn publish_ready(&self, key: HandoffKey, direction: ReadyDirection) -> bool {
        let publish = {
            let mut state = self.state.lock().unwrap();
            let Some(attempt) = state.attempt.as_mut() else {
                return false;
            };
            if attempt.key != key
                || !matches!(
                    attempt.phase,
                    HandoffPhase::Pending | HandoffPhase::Running | HandoffPhase::Waiting
                )
            {
                return false;
            }
            let index = match direction {
                ReadyDirection::Read => 0,
                ReadyDirection::Write => 1,
            };
            attempt.ready[index].publish();
            attempt.runnable = true;
            true
        };
        if publish {
            self.worker_wake.notify_one();
        }
        publish
    }

    /// Revoke a pending, running, waiting, or returned attempt before reactor
    /// acceptance. The returned Arc is the same published proof retained by
    /// the queue. For Running/Waiting, only the worker can drop `T` and
    /// acknowledge retirement; the proof is nevertheless visible now.
    pub fn revoke(
        &self,
        key: HandoffKey,
        reason: RevocationReason,
    ) -> io::Result<Option<Arc<QueueReleaseProof>>> {
        self.revoke_matching(Some(key), reason, false)
    }

    /// Stop future claims and revoke the currently owned pre-accept attempt
    /// in one handoff-lock transition. A running native call keeps its resource
    /// until it returns; shutdown never waits for it.
    pub fn stop_and_revoke_current(&self) -> io::Result<Option<Arc<QueueReleaseProof>>> {
        self.revoke_matching(None, RevocationReason::Shutdown, true)
    }

    fn revoke_matching(
        &self,
        key: Option<HandoffKey>,
        reason: RevocationReason,
        stop_worker: bool,
    ) -> io::Result<Option<Arc<QueueReleaseProof>>> {
        let mut pending_drop = None;
        let mut retirement_to_ack = None;
        let mut notify_worker = stop_worker;
        let proof = {
            let mut state = self.state.lock().unwrap();
            if stop_worker {
                state.worker_stopped = true;
            }
            let Some(attempt) = state.attempt.as_mut() else {
                drop(state);
                if notify_worker {
                    self.worker_wake.notify_all();
                }
                return Ok(None);
            };
            if key.is_some_and(|key| attempt.key != key) || attempt.phase == HandoffPhase::Accepted
            {
                drop(state);
                if notify_worker {
                    self.worker_wake.notify_all();
                }
                return Ok(None);
            }

            let at = Instant::now();
            let proof = attempt.issue_release_proof(reason, at);

            match attempt.phase {
                HandoffPhase::Pending => {
                    pending_drop = attempt.pending.take();
                    retirement_to_ack = Some(Arc::clone(&attempt.retirement));
                    attempt.phase = HandoffPhase::Released;
                    attempt.worker_active = false;
                }
                HandoffPhase::Running | HandoffPhase::Waiting => {
                    attempt.phase = HandoffPhase::Released;
                    notify_worker = true;
                }
                HandoffPhase::Returned => {
                    if let Some(WorkerOutcome::Success(resource)) = attempt.returned.take() {
                        pending_drop = Some(resource);
                        retirement_to_ack = Some(Arc::clone(&attempt.retirement));
                    } else {
                        attempt.returned = None;
                    }
                    attempt.phase = HandoffPhase::Released;
                    attempt.worker_active = false;
                }
                HandoffPhase::Released => {}
                HandoffPhase::Empty | HandoffPhase::Accepted => unreachable!(),
            }
            proof
        };

        if notify_worker {
            self.worker_wake.notify_all();
        }
        // The proof is published before either wake. No handoff lock is held
        // across the mio call or the resource's destructor.
        let wake_result = if proof.is_some() {
            self.reactor_waker.wake()
        } else {
            Ok(())
        };
        if let Some(resource) = pending_drop {
            drop(resource);
        }
        if let Some(retirement) = retirement_to_ack {
            retirement.ack();
        }
        wake_result?;
        Ok(proof)
    }

    /// Claim the one submitted resource, checking cancellation and its absolute
    /// deadline on each wake. Returns `None` after shutdown revokes the current
    /// attempt and stops future worker claims.
    pub fn worker_claim(&self) -> Option<WorkerWork<'_, T>> {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.worker_stopped {
                return None;
            }
            let now = Instant::now();
            let expired = state.attempt.as_ref().and_then(|attempt| {
                if matches!(
                    attempt.phase,
                    HandoffPhase::Pending
                        | HandoffPhase::Running
                        | HandoffPhase::Waiting
                        | HandoffPhase::Returned
                ) {
                    attempt
                        .revocation_due(now)
                        .map(|reason| (attempt.key, reason))
                } else {
                    None
                }
            });
            if let Some((key, reason)) = expired {
                drop(state);
                let _ = self.revoke(key, reason);
                state = self.state.lock().unwrap();
                continue;
            }
            if let Some(attempt) = state.attempt.as_mut() {
                if attempt.phase == HandoffPhase::Pending {
                    let resource = attempt.pending.take().expect("Pending owns its resource");
                    attempt.phase = HandoffPhase::Running;
                    attempt.worker_active = true;
                    attempt.consume_runnable();
                    let work = WorkerWork {
                        cell: self,
                        key: attempt.key,
                        retirement: Arc::clone(&attempt.retirement),
                        resource: Some(resource),
                        snapshot: attempt.snapshot(),
                        armed: true,
                    };
                    return Some(work);
                }
            }
            let timeout = state
                .attempt
                .as_ref()
                .filter(|attempt| {
                    matches!(
                        attempt.phase,
                        HandoffPhase::Pending
                            | HandoffPhase::Running
                            | HandoffPhase::Waiting
                            | HandoffPhase::Returned
                    )
                })
                .map_or(MAX_HANDOFF_WAIT, |attempt| {
                    attempt
                        .deadline
                        .saturating_duration_since(Instant::now())
                        .min(MAX_HANDOFF_WAIT)
                });
            state = self.worker_wake.wait_timeout(state, timeout).unwrap().0;
        }
    }

    /// Complete one native call. This must be called after every native call,
    /// even a successful/progressing one. A real WouldBlock uses a bounded
    /// condvar wait after a locked readiness recheck; synthetic budget
    /// exhaustion requeues without waiting for a readiness edge.
    ///
    /// `Ready` and `SyntheticBudget` return custody to Pending and disarm
    /// `work`; discard that value and call `worker_claim` before another native
    /// call. `Revoked` leaves the resource with the worker until it is dropped
    /// or otherwise released through the completion path.
    pub fn worker_yield(&self, work: &mut WorkerWork<'_, T>, call: NativeCall) -> WorkerYield {
        if !std::ptr::eq(self, work.cell) {
            return WorkerYield::Stale;
        }
        let mut state = self.state.lock().unwrap();
        let mut proof_wake = false;
        let mut requeued = false;
        let first = {
            let worker_stopped = state.worker_stopped;
            let Some(attempt) = state.attempt.as_mut() else {
                return WorkerYield::Stale;
            };
            if attempt.key != work.key {
                return WorkerYield::Stale;
            }
            let now = Instant::now();
            let reason = attempt.revocation_due(now).or_else(|| {
                (attempt.phase == HandoffPhase::Released).then_some(RevocationReason::Cancelled)
            });
            if let Some(reason) = reason {
                proof_wake = attempt.issue_release_proof(reason, now).is_some();
                attempt.phase = HandoffPhase::Released;
                Some(WorkerYield::Revoked)
            } else if worker_stopped {
                Some(WorkerYield::Stopped)
            } else if attempt.phase != HandoffPhase::Running {
                Some(WorkerYield::Stale)
            } else if call == NativeCall::SyntheticBudget {
                attempt.pending = Some(work.resource.take().expect("worker owns its resource"));
                attempt.phase = HandoffPhase::Pending;
                attempt.worker_active = false;
                attempt.runnable = true;
                work.snapshot = attempt.snapshot();
                work.armed = false;
                requeued = true;
                Some(WorkerYield::SyntheticBudget)
            } else {
                let changed = attempt.readiness_changed(work.snapshot) || attempt.runnable;
                if changed {
                    attempt.pending = Some(work.resource.take().expect("worker owns its resource"));
                    attempt.phase = HandoffPhase::Pending;
                    attempt.worker_active = false;
                    // Preserve the edge a producer published during Running.
                    attempt.runnable = true;
                    work.snapshot = attempt.snapshot();
                    work.armed = false;
                    requeued = true;
                    Some(WorkerYield::Ready)
                } else if call == NativeCall::Progress {
                    work.snapshot = attempt.snapshot();
                    Some(WorkerYield::Continue)
                } else {
                    attempt.phase = HandoffPhase::Waiting;
                    None
                }
            }
        };
        if let Some(result) = first {
            drop(state);
            if requeued {
                self.worker_wake.notify_one();
            }
            if proof_wake {
                let _ = self.reactor_waker.wake();
            }
            return result;
        }

        #[cfg(test)]
        self.before_wait_interlock();

        loop {
            let timeout = state
                .attempt
                .as_ref()
                .filter(|attempt| attempt.key == work.key)
                .map_or(Duration::ZERO, |attempt| {
                    attempt
                        .deadline
                        .saturating_duration_since(Instant::now())
                        .min(MAX_HANDOFF_WAIT)
                });
            state = self.worker_wake.wait_timeout(state, timeout).unwrap().0;
            proof_wake = false;
            requeued = false;
            let result = {
                let worker_stopped = state.worker_stopped;
                let Some(attempt) = state.attempt.as_mut() else {
                    return WorkerYield::Stale;
                };
                if attempt.key != work.key {
                    return WorkerYield::Stale;
                }
                let now = Instant::now();
                let reason = attempt.revocation_due(now).or_else(|| {
                    (attempt.phase == HandoffPhase::Released).then_some(RevocationReason::Cancelled)
                });
                if let Some(reason) = reason {
                    proof_wake = attempt.issue_release_proof(reason, now).is_some();
                    attempt.phase = HandoffPhase::Released;
                    Some(WorkerYield::Revoked)
                } else if worker_stopped {
                    Some(WorkerYield::Stopped)
                } else if attempt.phase != HandoffPhase::Waiting {
                    Some(WorkerYield::Stale)
                } else if attempt.readiness_changed(work.snapshot) || attempt.runnable {
                    attempt.pending = Some(work.resource.take().expect("worker owns its resource"));
                    attempt.phase = HandoffPhase::Pending;
                    attempt.worker_active = false;
                    attempt.runnable = true;
                    work.snapshot = attempt.snapshot();
                    work.armed = false;
                    requeued = true;
                    Some(WorkerYield::Ready)
                } else {
                    None
                }
            };
            if let Some(result) = result {
                drop(state);
                if requeued {
                    self.worker_wake.notify_one();
                }
                if proof_wake {
                    let _ = self.reactor_waker.wake();
                }
                return result;
            }
        }
    }

    /// Return a successfully verified initial transport to the reactor. A
    /// revoked/late success is dropped and acknowledged, never published.
    pub fn worker_success(&self, mut work: WorkerWork<'_, T>) -> io::Result<WorkerFinish> {
        let mut resource = Some(work.resource.take().expect("worker owns its resource"));
        work.armed = false;
        let mut state = self.state.lock().unwrap();
        let valid = state.attempt.as_ref().is_some_and(|attempt| {
            attempt.key == work.key
                && attempt.worker_active
                && matches!(
                    attempt.phase,
                    HandoffPhase::Running | HandoffPhase::Waiting | HandoffPhase::Released
                )
        });
        if !valid {
            drop(state);
            drop(resource.take());
            work.retirement.ack();
            return self.wake_reactor(WorkerFinish::Stale);
        }

        let attempt = state.attempt.as_mut().expect("validated active attempt");
        let now = Instant::now();
        let reason = attempt.revocation_due(now).or_else(|| {
            (attempt.phase == HandoffPhase::Released).then_some(RevocationReason::Cancelled)
        });
        let finish = if let Some(reason) = reason {
            let _ = attempt.issue_release_proof(reason, now);
            attempt.phase = HandoffPhase::Released;
            WorkerFinish::Revoked
        } else {
            attempt.returned = Some(WorkerOutcome::Success(
                resource.take().expect("success retains worker resource"),
            ));
            attempt.phase = HandoffPhase::Returned;
            attempt.worker_active = false;
            WorkerFinish::Returned
        };
        drop(state);

        // A revoked worker still owns the physical resource until this drop.
        if let Some(resource) = resource.take() {
            drop(resource);
            work.retirement.ack();
            let mut state = self.state.lock().unwrap();
            if let Some(attempt) = state.attempt.as_mut() {
                if attempt.key == work.key {
                    attempt.worker_active = false;
                    attempt.phase = HandoffPhase::Released;
                }
            }
        }
        self.worker_wake.notify_all();
        self.wake_reactor(finish)
    }

    /// Drop a failed handshake resource before acknowledging physical
    /// retirement, then publish a failure cell unless revocation already won.
    pub fn worker_failure(
        &self,
        mut work: WorkerWork<'_, T>,
        failure: WorkerFailure,
    ) -> io::Result<WorkerFinish> {
        let mut proof_wake = false;
        {
            let mut state = self.state.lock().unwrap();
            if let Some(attempt) = state.attempt.as_mut() {
                if attempt.key == work.key
                    && attempt.worker_active
                    && matches!(
                        attempt.phase,
                        HandoffPhase::Running | HandoffPhase::Waiting | HandoffPhase::Released
                    )
                {
                    let now = Instant::now();
                    let reason = attempt.revocation_due(now).or_else(|| {
                        (attempt.phase == HandoffPhase::Released)
                            .then_some(RevocationReason::Cancelled)
                    });
                    if let Some(reason) = reason {
                        proof_wake = attempt.issue_release_proof(reason, now).is_some();
                        attempt.phase = HandoffPhase::Released;
                    }
                }
            }
        }
        if proof_wake {
            let _ = self.reactor_waker.wake();
        }
        let resource = work.resource.take().expect("worker owns its resource");
        work.armed = false;
        drop(resource);
        work.retirement.ack();
        let finish = {
            let mut state = self.state.lock().unwrap();
            match state.attempt.as_mut() {
                Some(attempt)
                    if attempt.key == work.key
                        && attempt.worker_active
                        && matches!(
                            attempt.phase,
                            HandoffPhase::Running | HandoffPhase::Waiting | HandoffPhase::Released
                        ) =>
                {
                    let now = Instant::now();
                    let reason = attempt.revocation_due(now).or_else(|| {
                        (attempt.phase == HandoffPhase::Released)
                            .then_some(RevocationReason::Cancelled)
                    });
                    attempt.worker_active = false;
                    if let Some(reason) = reason {
                        let _ = attempt.issue_release_proof(reason, now);
                        attempt.phase = HandoffPhase::Released;
                        WorkerFinish::Revoked
                    } else {
                        attempt.returned = Some(WorkerOutcome::Failure(failure));
                        attempt.phase = HandoffPhase::Returned;
                        WorkerFinish::Returned
                    }
                }
                _ => WorkerFinish::Stale,
            }
        };
        self.worker_wake.notify_all();
        self.wake_reactor(finish)
    }

    /// Accept the persistent result exactly once if the full key still matches
    /// and neither cancellation nor its absolute deadline has revoked it. A
    /// discovered revocation publishes proof and drops the returned resource
    /// outside K-handoff.
    pub fn take_return(&self, key: HandoffKey) -> Result<Accepted<T>, TakeReturnError> {
        let mut drop_resource = None;
        let mut retirement_to_ack = None;
        let mut revoked_proof = None;
        let result = {
            let mut state = self.state.lock().unwrap();
            let Some(attempt) = state.attempt.as_mut() else {
                return Err(TakeReturnError::Stale);
            };
            if attempt.key != key {
                return Err(TakeReturnError::Stale);
            }
            match attempt.phase {
                HandoffPhase::Accepted => return Err(TakeReturnError::AlreadyAccepted),
                HandoffPhase::Released => return Err(TakeReturnError::Revoked(None)),
                HandoffPhase::Returned => {}
                _ => return Err(TakeReturnError::NotReturned),
            }

            let now = Instant::now();
            if let Some(reason) = attempt.revocation_due(now) {
                revoked_proof = attempt.issue_release_proof(reason, now);
                if let Some(WorkerOutcome::Success(resource)) = attempt.returned.take() {
                    drop_resource = Some(resource);
                    retirement_to_ack = Some(Arc::clone(&attempt.retirement));
                } else {
                    attempt.returned = None;
                }
                attempt.phase = HandoffPhase::Released;
                attempt.worker_active = false;
                None
            } else {
                let outcome = attempt.returned.take().expect("Returned owns its result");
                attempt.phase = HandoffPhase::Accepted;
                Some(Accepted {
                    key,
                    outcome,
                    retirement: Arc::clone(&attempt.retirement),
                })
            }
        };

        if let Some(resource) = drop_resource {
            drop(resource);
        }
        if let Some(retirement) = retirement_to_ack {
            retirement.ack();
        }
        if revoked_proof.is_some() {
            let _ = self.reactor_waker.wake();
        }
        match result {
            Some(accepted) => Ok(accepted),
            None => Err(TakeReturnError::Revoked(revoked_proof)),
        }
    }

    /// Finish an accepted handoff after the reactor has moved its resource
    /// into the established connection or closed it. This frees the singleton
    /// lane immediately; the returned retirement cell is acknowledged
    /// separately after committed detachment or actual close.
    pub fn finish_accept(&self, key: HandoffKey) -> Result<(), FinishAcceptError> {
        let mut state = self.state.lock().unwrap();
        let Some(attempt) = state.attempt.as_ref() else {
            return Err(FinishAcceptError::Stale);
        };
        if attempt.key != key {
            return Err(FinishAcceptError::Stale);
        }
        if attempt.phase != HandoffPhase::Accepted {
            return Err(FinishAcceptError::NotAccepted);
        }
        state.attempt = None;
        Ok(())
    }

    fn wake_reactor(&self, finish: WorkerFinish) -> io::Result<WorkerFinish> {
        self.reactor_waker.wake()?;
        Ok(finish)
    }

    #[cfg(test)]
    fn before_wait_interlock(&self) {
        let gate = self.wait_interlock.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.0.wait();
            gate.1.wait();
        }
    }
}

impl<T: Send> Drop for HandoffCell<T> {
    fn drop(&mut self) {
        // `get_mut` does not acquire K-handoff; the cell is exclusively owned
        // here, so any resource destructor and waker remain outside the lock.
        let state = match self.state.get_mut() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let Some(mut attempt) = state.attempt.take() else {
            return;
        };
        if attempt.phase != HandoffPhase::Accepted {
            let _ = attempt.issue_release_proof(RevocationReason::Shutdown, Instant::now());
        }
        if attempt.phase != HandoffPhase::Accepted {
            let _ = self.reactor_waker.wake();
        }
        let resource = attempt
            .pending
            .take()
            .or_else(|| match attempt.returned.take() {
                Some(WorkerOutcome::Success(resource)) => Some(resource),
                Some(WorkerOutcome::Failure(_)) | None => None,
            });
        if let Some(resource) = resource {
            drop(resource);
            attempt.retirement.ack();
        }
    }
}

/// Resource custody retained by the worker between nonblocking native calls.
pub struct WorkerWork<'a, T: Send> {
    cell: &'a HandoffCell<T>,
    key: HandoffKey,
    retirement: Arc<AttemptRetire>,
    resource: Option<T>,
    snapshot: ReadySnapshot,
    armed: bool,
}

impl<T: Send> WorkerWork<'_, T> {
    pub fn key(&self) -> HandoffKey {
        self.key
    }

    /// Borrow the coupled TLS/socket resource only while making the initial
    /// handshake or OS-verification calls.
    pub fn resource_mut(&mut self) -> &mut T {
        self.resource.as_mut().expect("worker owns its resource")
    }

    /// Sequence captured under K-handoff before the next native call.
    pub fn ready_snapshot(&self) -> ReadySnapshot {
        self.snapshot
    }
}

impl<T: Send> Drop for WorkerWork<'_, T> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let cell = self.cell;
        let mut proof_wake = false;
        {
            let mut state = cell.state.lock().unwrap();
            if let Some(attempt) = state.attempt.as_mut() {
                if attempt.key == self.key && attempt.worker_active {
                    let now = Instant::now();
                    let reason = attempt.revocation_due(now).or_else(|| {
                        (attempt.phase == HandoffPhase::Released)
                            .then_some(RevocationReason::Cancelled)
                    });
                    if let Some(reason) = reason {
                        proof_wake = attempt.issue_release_proof(reason, now).is_some();
                        attempt.phase = HandoffPhase::Released;
                    }
                }
            }
        }
        if proof_wake {
            let _ = cell.reactor_waker.wake();
        }

        let resource = self.resource.take();
        drop(resource);
        self.retirement.ack();

        {
            let mut state = cell.state.lock().unwrap();
            if let Some(attempt) = state.attempt.as_mut() {
                if attempt.key == self.key && attempt.worker_active {
                    let now = Instant::now();
                    let reason = attempt.revocation_due(now).or_else(|| {
                        (attempt.phase == HandoffPhase::Released)
                            .then_some(RevocationReason::Cancelled)
                    });
                    if let Some(reason) = reason {
                        let _ = attempt.issue_release_proof(reason, now);
                        attempt.phase = HandoffPhase::Released;
                    } else {
                        attempt.returned = Some(WorkerOutcome::Failure(WorkerFailure::Abandoned));
                        attempt.phase = HandoffPhase::Returned;
                    }
                    attempt.worker_active = false;
                }
            }
        }
        self.armed = false;
        cell.worker_wake.notify_all();
        let _ = cell.reactor_waker.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cancel::CancelSource;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Barrier, TryLockError};
    use std::thread;

    struct Tracked(Arc<AtomicUsize>);

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn cell() -> (mio::Poll, HandoffCell<Tracked>) {
        let poll = mio::Poll::new().unwrap();
        let waker = Arc::new(Waker::new(poll.registry(), mio::Token(0)).unwrap());
        (poll, HandoffCell::new(waker))
    }

    fn binding(attempt: u64) -> BindingId {
        BindingId {
            incarnation: 4,
            connection: 9,
            binding: 12,
            attempt,
        }
    }

    fn admit(
        cell: &HandoffCell<Tracked>,
        token: CancelToken,
        retirement: Arc<AttemptRetire>,
        binding: BindingId,
        drops: Arc<AtomicUsize>,
    ) -> (HandoffKey, Arc<QueueReleaseProof>) {
        let transfer_seq = cell.next_transfer_seq().unwrap();
        let key = HandoffKey {
            identity: HandoffIdentity {
                generation: token.generation(),
                binding,
            },
            transfer_seq,
        };
        let proof = Arc::new(QueueReleaseProof::new(binding, transfer_seq));
        let receipt = cell
            .submit(
                key,
                Arc::clone(&proof),
                token,
                retirement,
                Instant::now() + Duration::from_secs(60),
                Tracked(drops),
            )
            .unwrap();
        assert!(Arc::ptr_eq(&proof, &receipt));
        (key, receipt)
    }

    fn token(source: &CancelSource) -> CancelToken {
        source.child().unwrap()
    }

    #[test]
    fn edge_before_claim_and_during_would_block_cannot_be_cleared() {
        let (_poll, cell) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let retirement = Arc::new(AttemptRetire::new(1));
        let (key, _) = admit(
            &cell,
            token(&source),
            Arc::clone(&retirement),
            binding(1),
            Arc::clone(&drops),
        );

        assert!(cell.publish_ready(key, ReadyDirection::Read));
        let mut work = cell.worker_claim().unwrap();
        assert_eq!(work.ready_snapshot().read_seq(), 1);

        // This edge lands after the captured sequence and before the native
        // operation reports WouldBlock. The locked recheck must retry, not sleep.
        assert!(cell.publish_ready(key, ReadyDirection::Write));
        assert_eq!(
            cell.worker_yield(&mut work, NativeCall::WouldBlock),
            WorkerYield::Ready
        );
        assert_eq!(work.ready_snapshot().write_seq(), 1);
        assert_eq!(cell.phase(), HandoffPhase::Pending);
        drop(work);
        let work = cell.worker_claim().unwrap();
        assert_eq!(
            cell.worker_failure(work, WorkerFailure::Handshake).unwrap(),
            WorkerFinish::Returned
        );

        let accepted = cell.take_return(key).unwrap();
        assert!(retirement.acknowledged());
        assert!(matches!(
            accepted.outcome,
            WorkerOutcome::Failure(WorkerFailure::Handshake)
        ));
        assert!(accepted.retirement.acknowledged());
        cell.finish_accept(key).unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn edge_between_locked_recheck_and_condvar_sleep_is_not_lost() {
        let (_poll, cell) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let retirement = Arc::new(AttemptRetire::new(1));
        let (key, _) = admit(
            &cell,
            token(&source),
            Arc::clone(&retirement),
            binding(1),
            Arc::clone(&drops),
        );
        let gate = Arc::new((Barrier::new(2), Barrier::new(2)));
        *cell.wait_interlock.lock().unwrap() = Some(Arc::clone(&gate));
        thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let mut work = cell.worker_claim().unwrap();
                let result = cell.worker_yield(&mut work, NativeCall::WouldBlock);
                assert_eq!(result, WorkerYield::Ready);
                drop(work);
                let work = cell.worker_claim().unwrap();
                cell.worker_failure(work, WorkerFailure::Handshake).unwrap()
            });

            // The shim pauses after the worker's locked sequence recheck while
            // K-handoff is still held. Publication blocks on K until Condvar
            // atomically releases it into sleep, then notifies the waiter.
            gate.0.wait();
            assert!(matches!(
                cell.state.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
            gate.1.wait();
            assert!(cell.publish_ready(key, ReadyDirection::Read));
            assert_eq!(worker.join().unwrap(), WorkerFinish::Returned);
        });

        let accepted = cell.take_return(key).unwrap();
        assert!(matches!(
            accepted.outcome,
            WorkerOutcome::Failure(WorkerFailure::Handshake)
        ));
        cell.finish_accept(key).unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn synthetic_budget_requeues_without_a_readiness_edge() {
        let (_poll, cell) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let retirement = Arc::new(AttemptRetire::new(1));
        let (key, _) = admit(
            &cell,
            token(&source),
            Arc::clone(&retirement),
            binding(1),
            Arc::clone(&drops),
        );

        let mut work = cell.worker_claim().unwrap();
        assert_eq!(
            cell.worker_yield(&mut work, NativeCall::SyntheticBudget),
            WorkerYield::SyntheticBudget
        );
        assert_eq!(cell.phase(), HandoffPhase::Pending);
        assert!(!retirement.acknowledged());
        drop(work);
        let work = cell.worker_claim().unwrap();
        assert_eq!(work.ready_snapshot().read_seq(), 0);
        assert_eq!(
            cell.worker_failure(work, WorkerFailure::Handshake).unwrap(),
            WorkerFinish::Returned
        );
        let accepted = cell.take_return(key).unwrap();
        assert!(matches!(
            accepted.outcome,
            WorkerOutcome::Failure(WorkerFailure::Handshake)
        ));
        cell.finish_accept(key).unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn blocked_native_call_publishes_deadline_proof_without_reactor_scan() {
        let (_poll, cell) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let retirement = Arc::new(AttemptRetire::new(1));
        let (key, proof) = admit(
            &cell,
            token(&source),
            Arc::clone(&retirement),
            binding(1),
            Arc::clone(&drops),
        );
        let mut work = cell.worker_claim().unwrap();
        {
            let mut state = cell.state.lock().unwrap();
            state.attempt.as_mut().unwrap().deadline = Instant::now() + Duration::from_millis(5);
        }

        assert_eq!(
            cell.worker_yield(&mut work, NativeCall::WouldBlock),
            WorkerYield::Revoked
        );
        assert!(proof.snapshot().is_some());
        assert!(!retirement.acknowledged());
        assert_eq!(
            cell.worker_failure(work, WorkerFailure::Handshake).unwrap(),
            WorkerFinish::Revoked
        );
        assert!(retirement.acknowledged());
        assert!(matches!(
            cell.take_return(key),
            Err(TakeReturnError::Revoked(_))
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn accepted_transfer_frees_lane_before_commit_acknowledgement() {
        let (_poll, cell) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let retirement = Arc::new(AttemptRetire::new(1));
        let (key, _) = admit(
            &cell,
            token(&source),
            Arc::clone(&retirement),
            binding(1),
            Arc::clone(&drops),
        );
        let work = cell.worker_claim().unwrap();
        assert_eq!(cell.worker_success(work).unwrap(), WorkerFinish::Returned);

        // Coalesced notifications cannot consume or replace the persistent result.
        cell.reactor_waker.wake().unwrap();
        cell.reactor_waker.wake().unwrap();
        let accepted = cell.take_return(key).unwrap();
        assert_eq!(cell.phase(), HandoffPhase::Accepted);
        assert!(matches!(
            cell.take_return(key),
            Err(TakeReturnError::AlreadyAccepted)
        ));
        let Accepted {
            outcome,
            retirement: ack,
            ..
        } = accepted;
        let WorkerOutcome::Success(session_resource) = outcome else {
            panic!("expected successful handoff");
        };

        // The reactor has moved the resource into session custody. It can
        // release the singleton worker lane before Phase C acknowledges the
        // attempt's retirement cell.
        cell.finish_accept(key).unwrap();
        assert_eq!(cell.phase(), HandoffPhase::Empty);
        assert!(!ack.acknowledged());

        let next_retirement = Arc::new(AttemptRetire::new(2));
        let (next_key, _) = admit(
            &cell,
            token(&source),
            Arc::clone(&next_retirement),
            binding(2),
            Arc::clone(&drops),
        );
        assert_eq!(next_key.transfer_seq, 2);
        assert!(!ack.acknowledged());

        drop(session_resource);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(ack.ack());
        assert!(!next_retirement.acknowledged());
        drop(cell);
        assert!(next_retirement.acknowledged());
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn shutdown_revokes_exact_running_attempt_without_early_physical_ack() {
        let (_poll, cell) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let retirement = Arc::new(AttemptRetire::new(1));
        let (key, proof) = admit(
            &cell,
            token(&source),
            Arc::clone(&retirement),
            binding(1),
            Arc::clone(&drops),
        );
        let work = cell.worker_claim().unwrap();

        let revoked = cell.stop_and_revoke_current().unwrap().unwrap();
        assert!(Arc::ptr_eq(&proof, &revoked));
        assert!(proof.snapshot().is_some());
        assert_eq!(cell.phase(), HandoffPhase::Released);
        assert!(!retirement.acknowledged());
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        assert_eq!(
            cell.worker_failure(work, WorkerFailure::Shutdown).unwrap(),
            WorkerFinish::Revoked
        );
        assert!(retirement.acknowledged());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(cell.worker_claim().is_none());
        assert!(matches!(
            cell.take_return(key),
            Err(TakeReturnError::Revoked(_))
        ));
    }

    #[test]
    fn running_revocation_publishes_immediate_proof_but_delays_physical_ack() {
        let (_poll, cell) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let retirement = Arc::new(AttemptRetire::new(1));
        let (key, proof) = admit(
            &cell,
            token(&source),
            Arc::clone(&retirement),
            binding(1),
            Arc::clone(&drops),
        );
        let work = cell.worker_claim().unwrap();

        let published = cell
            .revoke(key, RevocationReason::Deadline)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&proof, &published));
        assert_eq!(published.identity(), key.identity.binding);
        assert_eq!(published.transfer_seq(), key.transfer_seq);
        assert!(published.snapshot().is_some());
        assert!(cell
            .revoke(key, RevocationReason::Cancelled)
            .unwrap()
            .is_none());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(!retirement.acknowledged());
        assert_eq!(cell.phase(), HandoffPhase::Released);

        // A late success cannot revive the released attempt. Its actual return
        // drops the resource first and only then acknowledges physical release.
        assert_eq!(cell.worker_success(work).unwrap(), WorkerFinish::Revoked);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(retirement.acknowledged());
        assert!(matches!(
            cell.take_return(key),
            Err(TakeReturnError::Revoked(_))
        ));
    }

    #[test]
    fn stale_old_retirement_ack_does_not_touch_a_new_attempt_in_the_same_cell() {
        let (_poll, cell) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let first_retirement = Arc::new(AttemptRetire::new(1));
        let first_drops = Arc::new(AtomicUsize::new(0));
        let (first_key, _) = admit(
            &cell,
            token(&source),
            Arc::clone(&first_retirement),
            binding(1),
            Arc::clone(&first_drops),
        );
        let first_work = cell.worker_claim().unwrap();
        cell.revoke(first_key, RevocationReason::Cancelled).unwrap();
        assert_eq!(
            cell.worker_failure(first_work, WorkerFailure::Shutdown)
                .unwrap(),
            WorkerFinish::Revoked
        );

        let second_retirement = Arc::new(AttemptRetire::new(2));
        let second_drops = Arc::new(AtomicUsize::new(0));
        let (second_key, _) = admit(
            &cell,
            token(&source),
            Arc::clone(&second_retirement),
            binding(2),
            Arc::clone(&second_drops),
        );
        assert_eq!(cell.phase(), HandoffPhase::Pending);
        assert!(!second_retirement.acknowledged());
        assert!(!first_retirement.ack());
        assert!(!second_retirement.acknowledged());
        assert!(!cell.publish_ready(first_key, ReadyDirection::Read));
        assert!(cell.publish_ready(second_key, ReadyDirection::Read));
    }

    #[test]
    fn transfer_sequence_exhaustion_rejects_without_taking_resource_custody() {
        let (_poll, handoff) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let first_token = token(&source);
        let first_retirement = Arc::new(AttemptRetire::new(1));
        let first_drops = Arc::new(AtomicUsize::new(0));
        let first_key = HandoffKey {
            identity: HandoffIdentity {
                generation: 1,
                binding: binding(1),
            },
            transfer_seq: 1,
        };
        let first_proof = Arc::new(QueueReleaseProof::new(binding(1), 1));
        handoff
            .submit(
                first_key,
                first_proof,
                first_token,
                Arc::clone(&first_retirement),
                Instant::now() + Duration::from_secs(60),
                Tracked(Arc::clone(&first_drops)),
            )
            .unwrap();
        handoff
            .revoke(first_key, RevocationReason::Shutdown)
            .unwrap();
        assert_eq!(handoff.next_transfer_seq(), Some(2));
        assert_eq!(first_drops.load(Ordering::SeqCst), 1);

        // A fresh cell can be positioned at the last sequence without exposing
        // a production setter; submitting it exhausts future values.
        let (_poll2, exhausted) = cell();
        exhausted.state.lock().unwrap().next_transfer_seq = Some(u64::MAX);
        let last_drops = Arc::new(AtomicUsize::new(0));
        let last_token = token(&source);
        let last_retirement = Arc::new(AttemptRetire::new(2));
        let last_key = HandoffKey {
            identity: HandoffIdentity {
                generation: 2,
                binding: binding(2),
            },
            transfer_seq: u64::MAX,
        };
        exhausted
            .submit(
                last_key,
                Arc::new(QueueReleaseProof::new(binding(2), u64::MAX)),
                last_token,
                Arc::clone(&last_retirement),
                Instant::now() + Duration::from_secs(60),
                Tracked(Arc::clone(&last_drops)),
            )
            .unwrap();
        assert_eq!(exhausted.next_transfer_seq(), None);

        let rejected_drops = Arc::new(AtomicUsize::new(0));
        let third_token = token(&source);
        let third_retirement = Arc::new(AttemptRetire::new(3));
        let rejected = exhausted
            .submit(
                HandoffKey {
                    identity: HandoffIdentity {
                        generation: 3,
                        binding: binding(3),
                    },
                    transfer_seq: u64::MAX,
                },
                Arc::new(QueueReleaseProof::new(binding(3), u64::MAX)),
                third_token,
                third_retirement,
                Instant::now() + Duration::from_secs(60),
                Tracked(Arc::clone(&rejected_drops)),
            )
            .unwrap_err();
        assert_eq!(rejected.kind(), SubmitErrorKind::SequenceExhausted);
        assert_eq!(rejected_drops.load(Ordering::SeqCst), 0);
        drop(rejected.into_resource());
        assert_eq!(rejected_drops.load(Ordering::SeqCst), 1);
        drop(exhausted);
        assert!(last_retirement.acknowledged());
        assert_eq!(last_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn expired_submit_returns_resource_without_taking_custody() {
        let (_poll, handoff) = cell();
        let source = CancelSource::new(Arc::new(super::super::park::Park::new()));
        let token = token(&source);
        let retirement = Arc::new(AttemptRetire::new(1));
        let drops = Arc::new(AtomicUsize::new(0));
        let binding = binding(1);
        let key = HandoffKey {
            identity: HandoffIdentity {
                generation: token.generation(),
                binding,
            },
            transfer_seq: handoff.next_transfer_seq().unwrap(),
        };
        let rejected = handoff
            .submit(
                key,
                Arc::new(QueueReleaseProof::new(binding, key.transfer_seq)),
                token,
                retirement,
                Instant::now(),
                Tracked(Arc::clone(&drops)),
            )
            .unwrap_err();

        assert_eq!(rejected.kind(), SubmitErrorKind::DeadlineExpired);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(rejected.into_resource());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(handoff.next_transfer_seq(), Some(1));
    }
}
