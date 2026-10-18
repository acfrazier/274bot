//! Single-owner cancellation of work that was already admitted.

use std::marker::PhantomData;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use api::identity::{Cancellation, WorkIdentity};
use api::work::{CancelOutcome, InputRevision, LedgerError, WorkFamilySpec, WorkTicket};

use super::{maybe_release, request_cancel, JobEntry, WorkService, WorkerShared};

/// Identity-bound cancellation custody for one accepted job.
///
/// Obtain this handle while submitting outside the navs mutex, then move it
/// into the request owner. It cannot be cloned and holds only a weak job
/// reference, so it cannot prolong the job or its charge. Dropping an armed
/// handle abandons that exact job with [`Cancellation::ScopeInvalidated`].
///
/// # Lock order
///
/// `cancel`, `abandon`, and `Drop` never acquire the WorkService mutex or the
/// navs mutex. They take only the existing per-entry cancellation/control
/// synchronization and WorkerShared's release/publication synchronization.
/// Entry control is released before waking the worker or releasing capacity.
/// No internal guard escapes a call, and no service/navs acquisition occurs
/// while an internal guard is held. The owner may therefore drop this handle
/// while holding navs, even while another thread holds the service mutex.
///
/// Terminal cancellation is a counted logical no-op: it cannot rewrite a
/// settlement. Abandoning an unconsumed terminal still relinquishes its
/// custody through the existing release path, and only after worker return.
#[must_use = "the request owner must retain its cancellation custody"]
pub struct WorkCancellation<F: WorkFamilySpec> {
    shared: Arc<WorkerShared>,
    entry: Weak<JobEntry>,
    identity: WorkIdentity,
    revision: InputRevision,
    // The seam checker refuses the function pointer in fn() -> F.
    // Each owning spec must keep this handle Send + Sync.
    family: PhantomData<F>,
    armed: bool,
}

impl<F: WorkFamilySpec> std::fmt::Debug for WorkCancellation<F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkCancellation")
            .field("identity", &self.identity)
            .field("revision", &self.revision)
            .field("armed", &self.armed)
            .finish_non_exhaustive()
    }
}

impl WorkService {
    /// Transfer the single cancellation capability for this exact ticket.
    ///
    /// Capture it outside navs immediately after admission. A job whose
    /// capability was already taken, or whose custody was released, has no
    /// claimable cancellation identity and returns [`LedgerError::StaleIdentity`].
    pub fn cancellation_handle<F: WorkFamilySpec>(
        &self,
        ticket: WorkTicket<F>,
    ) -> Result<WorkCancellation<F>, LedgerError> {
        let entry = self
            .jobs
            .get(&ticket.job())
            .filter(|entry| entry.matches(&ticket))
            .ok_or(LedgerError::StaleIdentity)?;
        {
            let mut control = entry.control.lock();
            if control.cancellation_claimed || entry.released.load(Ordering::Acquire) {
                return Err(LedgerError::StaleIdentity);
            }
            control.cancellation_claimed = true;
        }
        Ok(WorkCancellation {
            shared: Arc::clone(&self.shared),
            entry: Arc::downgrade(entry),
            identity: ticket.identity(),
            revision: ticket.revision(),
            family: PhantomData,
            armed: true,
        })
    }

    /// Count handle operations that named retired custody or an existing terminal.
    /// This includes owner Drop after normal terminal consumption; it does
    /// not count cancellation bugs or cooperation failures.
    pub fn cancellation_handle_noops(&self) -> u64 {
        self.shared.cancellation_noops.load(Ordering::Acquire)
    }
}

impl<F: WorkFamilySpec> WorkCancellation<F> {
    /// Request cancellation without taking the WorkService or navs mutex.
    /// The handle retains custody until explicit abandonment or owner drop.
    pub fn cancel(&mut self, reason: Cancellation) -> CancelOutcome {
        let Some(entry) = self.live_entry() else {
            return self.count_outcome(CancelOutcome::StaleIdentity);
        };
        self.count_outcome(request_cancel(&self.shared, &entry, reason))
    }

    /// Relinquish result custody without waiting for the worker to return.
    /// Repeated operations are counted no-ops and cannot touch another job.
    pub fn abandon(&mut self, reason: Cancellation) -> CancelOutcome {
        let entry = self.live_entry();
        self.armed = false;
        let Some(entry) = entry else {
            return self.count_outcome(CancelOutcome::StaleIdentity);
        };
        let outcome = request_cancel(&self.shared, &entry, reason);
        entry.control.lock().abandoned = true;
        maybe_release(&self.shared, &entry);
        self.count_outcome(outcome)
    }

    fn live_entry(&self) -> Option<Arc<JobEntry>> {
        if !self.armed {
            return None;
        }
        let ticket = WorkTicket::<F>::new(self.identity, self.revision);
        self.entry
            .upgrade()
            .filter(|entry| entry.matches(&ticket) && !entry.released.load(Ordering::Acquire))
    }

    fn count_outcome(&self, outcome: CancelOutcome) -> CancelOutcome {
        if outcome != CancelOutcome::Requested {
            self.shared
                .cancellation_noops
                .fetch_add(1, Ordering::AcqRel);
        }
        outcome
    }
}

impl<F: WorkFamilySpec> Drop for WorkCancellation<F> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.abandon(Cancellation::ScopeInvalidated);
        }
    }
}
