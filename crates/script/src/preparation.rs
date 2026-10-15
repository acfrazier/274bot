//! Card, catalogue and profile preparation on the shared WorkAdmission worker.
//! A preparation capability exists only during a `Prepare` step; it never
//! crosses the job's yield or leaves the worker.

use std::convert::Infallible;
use std::marker::PhantomData;
use std::sync::Arc;

use api::identity::{Cancellation, IdentityError, Scope, Settlement, WorkIdentity};
use api::process_condition::ProcessConditions;
use api::selected::FamilyPreparation;
use api::work::{
    Checkpoint, CooperativeJob, InputRevision, JobError, LedgerError, PreparationPhase,
    RetainedState, StepContext, StepOutcome, SubmissionRefusal, WorkAdmission, WorkCharge,
    WorkFamily, WorkFamilySpec, WorkTicket,
};
use parking_lot::Mutex;

use crate::work_admission::{WorkService, WorkStartError};

/// The three complete preparation families admitted by I-work-prep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationKind {
    Card,
    Catalogue,
    Profile,
}

impl PreparationKind {
    const fn index(self) -> u8 {
        match self {
            Self::Card => 0,
            Self::Catalogue => 1,
            Self::Profile => 2,
        }
    }
}

/// A checked charge for source/retained input, peak scratch and the result.
/// The preparing owner must bound all three using the actual family input;
/// WorkAdmission reserves the sum, not just the source's logical length.
#[derive(Debug, Clone, Copy)]
pub struct PreparationInput {
    revision: InputRevision,
    charge: WorkCharge,
}

impl PreparationInput {
    /// A family revision and the three physical byte bounds. An overflow is
    /// an invalid input rather than a saturating/unbounded reservation.
    pub fn new(
        revision: u64,
        input_bytes: usize,
        scratch_bytes: usize,
        result_bytes: usize,
    ) -> Result<Self, SubmissionRefusal> {
        let charge = WorkCharge::new(input_bytes, scratch_bytes, result_bytes, 0);
        charge
            .total_bytes()
            .ok_or(SubmissionRefusal::InvalidArgument)?;
        Ok(Self {
            revision: InputRevision::new(revision),
            charge,
        })
    }

    /// The source revision fenced by the consuming owner.
    pub const fn revision(self) -> InputRevision {
        self.revision
    }

    /// The admission charge, including scratch and terminal custody.
    pub const fn charge(self) -> WorkCharge {
        self.charge
    }

    /// Count the actual retained settings tree before a native profile
    /// preparation. Shared selected data and bank facts remain charged in
    /// their own owner ledgers; this job charges its own bag and bounded
    /// scratch/result. Deep or oversized input is refused before custody.
    pub fn for_settings(
        revision: u64,
        bag: &crate::native::SettingsBag,
    ) -> Result<Self, SubmissionRefusal> {
        const MAX_SETTINGS_BYTES: usize = 8 * 1024 * 1024;
        const SCRATCH_BYTES: usize = 2 * 1024 * 1024;
        const RESULT_BYTES: usize = 2 * 1024 * 1024;
        let bytes = bag.iter().try_fold(0_usize, |total, (key, value)| {
            let item = key
                .capacity()
                .checked_add(retained_json_bytes(value, 0)?)?
                .checked_add(4 * std::mem::size_of::<(String, serde_json::Value)>())?;
            total.checked_add(item)
        });
        let bytes = bytes
            .filter(|bytes| *bytes <= MAX_SETTINGS_BYTES)
            .ok_or(SubmissionRefusal::InvalidArgument)?;
        Self::new(revision, bytes, SCRATCH_BYTES, RESULT_BYTES)
    }
}

fn retained_json_bytes(value: &serde_json::Value, depth: usize) -> Option<usize> {
    if depth >= 128 {
        return None;
    }
    let base = std::mem::size_of::<serde_json::Value>();
    match value {
        serde_json::Value::String(value) => base.checked_add(value.capacity()),
        serde_json::Value::Array(values) => {
            let allocation = values
                .capacity()
                .checked_mul(std::mem::size_of::<serde_json::Value>())?;
            values
                .iter()
                .try_fold(base.checked_add(allocation)?, |total, item| {
                    total.checked_add(retained_json_bytes(item, depth + 1)?)
                })
        }
        serde_json::Value::Object(values) => values.iter().try_fold(base, |total, (key, item)| {
            total
                .checked_add(key.capacity())?
                .checked_add(4 * std::mem::size_of::<(String, serde_json::Value)>())?
                .checked_add(retained_json_bytes(item, depth + 1)?)
        }),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Some(base)
        }
    }
}

struct PreparationSpec<R, const KIND: u8>(PhantomData<fn() -> R>);

impl<R: Send + 'static, const KIND: u8> WorkFamilySpec for PreparationSpec<R, KIND> {
    const FAMILY: WorkFamily = match KIND {
        0 => WorkFamily::CardPreparation,
        1 => WorkFamily::CataloguePreparation,
        2 => WorkFamily::ProfilePreparation,
        _ => panic!("invalid private preparation family"),
    };
    type Input = PreparationInput;
    type Output = R;
    type Error = Infallible;
    type Phase = PreparationPhase;

    fn revision(input: &Self::Input) -> InputRevision {
        input.revision
    }

    fn charge(input: &Self::Input) -> WorkCharge {
        input.charge
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
}

struct PreparationJob<R, W> {
    phase: PreparationPhase,
    work: Option<W>,
    output: Option<R>,
    retained_input: usize,
    retained_output: usize,
}

impl<R, W> CooperativeJob for PreparationJob<R, W>
where
    R: Send + 'static,
    W: FnOnce(&mut FamilyPreparation) -> R + Send + 'static,
{
    type Output = R;
    type Error = Infallible;
    type Phase = PreparationPhase;

    fn step(&mut self, step: &mut StepContext<'_, PreparationPhase>) -> StepOutcome<R, Infallible> {
        match step.checkpoint() {
            Checkpoint::Cancel(_) | Checkpoint::Yield => return StepOutcome::Yielded,
            Checkpoint::Continue => {}
        }
        match self.phase {
            PreparationPhase::Load => {
                // Source and immutable owners were captured before submission.
                self.phase = PreparationPhase::Prepare;
                step.publish_progress(PreparationPhase::Load, 1, Some(1));
                let _ = step.consume_unit();
                StepOutcome::Yielded
            }
            PreparationPhase::Prepare => {
                let work = self.work.take().expect("prepare phase has work");
                self.output = Some(FamilyPreparation::within(step, work));
                self.phase = PreparationPhase::Seal;
                // The opaque family call may have consumed the slice or been
                // cancelled. Check again before reporting prepared progress.
                if !matches!(step.checkpoint(), Checkpoint::Continue) {
                    return StepOutcome::Yielded;
                }
                step.publish_progress(PreparationPhase::Prepare, 1, Some(1));
                let _ = step.consume_unit();
                StepOutcome::Yielded
            }
            PreparationPhase::Seal => {
                step.publish_progress(PreparationPhase::Seal, 1, Some(1));
                let _ = step.consume_unit();
                match step.checkpoint() {
                    Checkpoint::Continue => {
                        StepOutcome::Completed(self.output.take().expect("prepared result"))
                    }
                    Checkpoint::Cancel(_) | Checkpoint::Yield => StepOutcome::Yielded,
                }
            }
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(
            self.retained_input
                + if self.output.is_some() {
                    self.retained_output
                } else {
                    0
                },
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PreparationError {
    #[error("could not start work admission: {0}")]
    Start(#[from] WorkStartError),
    #[error("could not mint preparation identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("preparation refused: {0}")]
    Refused(#[from] SubmissionRefusal),
    #[error("preparation ledger: {0}")]
    Ledger(#[from] LedgerError),
    #[error("preparation job: {0}")]
    Job(#[from] JobError<Infallible>),
    #[error("preparation cancelled: {0:?}")]
    Cancelled(Cancellation),
}

enum PreparationTicket<R: Send + 'static> {
    Card(WorkTicket<PreparationSpec<R, 0>>),
    Catalogue(WorkTicket<PreparationSpec<R, 1>>),
    Profile(WorkTicket<PreparationSpec<R, 2>>),
}

/// An exact-job result lease. Dropping it revokes an unfinished job; a
/// finished, unconsumed result is consumed and discarded, releasing credit.
/// A stuck worker keeps its physical charge even after the ticket is dropped.
pub struct PreparedTask<R: Send + 'static> {
    service: Arc<Mutex<WorkService>>,
    ticket: Option<PreparationTicket<R>>,
}

impl<R: Send + 'static> PreparedTask<R> {
    /// Correlation identity used to fence a later global publish.
    pub fn identity(&self) -> WorkIdentity {
        match self.ticket.as_ref().expect("live preparation ticket") {
            PreparationTicket::Card(ticket) => ticket.identity(),
            PreparationTicket::Catalogue(ticket) => ticket.identity(),
            PreparationTicket::Profile(ticket) => ticket.identity(),
        }
    }

    /// Whether the worker published this exact terminal. No job is run by the caller.
    pub fn is_finished(&self) -> bool {
        let service = self.service.lock();
        match self.ticket.as_ref().expect("live preparation ticket") {
            PreparationTicket::Card(ticket) => !matches!(service.borrow_terminal(ticket), Ok(None)),
            PreparationTicket::Catalogue(ticket) => {
                !matches!(service.borrow_terminal(ticket), Ok(None))
            }
            PreparationTicket::Profile(ticket) => {
                !matches!(service.borrow_terminal(ticket), Ok(None))
            }
        }
    }

    /// Request exact-job cancellation without treating the request as worker
    /// return. Consumers still reject a stale generation after settlement.
    pub fn cancel(&mut self, reason: Cancellation) {
        let mut service = self.service.lock();
        match self.ticket.as_ref().expect("live preparation ticket") {
            PreparationTicket::Card(ticket) => {
                service.cancel(*ticket, reason);
            }
            PreparationTicket::Catalogue(ticket) => {
                service.cancel(*ticket, reason);
            }
            PreparationTicket::Profile(ticket) => {
                service.cancel(*ticket, reason);
            }
        }
    }

    /// Wait for and take a terminal. Only callers that are already off-pump
    /// or have checked `is_finished()` may block here. In a pump or tick, use
    /// `is_finished()` first; the wait releases the service mutex.
    pub fn join(mut self) -> Result<R, PreparationError> {
        // Leave the ticket in `self` until terminal custody transfers. If
        // waiting or consuming fails, Drop still abandons this exact job.
        let terminal = match self.ticket.as_ref().expect("live preparation ticket") {
            PreparationTicket::Card(ticket) => {
                WorkService::wait_terminal(&self.service, ticket)?;
                self.service.lock().consume(ticket)?
            }
            PreparationTicket::Catalogue(ticket) => {
                WorkService::wait_terminal(&self.service, ticket)?;
                self.service.lock().consume(ticket)?
            }
            PreparationTicket::Profile(ticket) => {
                WorkService::wait_terminal(&self.service, ticket)?;
                self.service.lock().consume(ticket)?
            }
        };
        let _ = self.ticket.take();
        match terminal {
            Settlement::Completed(output) => Ok(output),
            Settlement::Failed(error) => Err(error.into()),
            Settlement::Cancelled(reason) => Err(PreparationError::Cancelled(reason)),
        }
    }
}

impl<R: Send + 'static> Drop for PreparedTask<R> {
    fn drop(&mut self) {
        let Some(ticket) = self.ticket.take() else {
            return;
        };
        let mut service = self.service.lock();
        match ticket {
            PreparationTicket::Card(ticket) => {
                if service.borrow_terminal(&ticket).is_ok_and(|v| v.is_some()) {
                    let _ = service.consume(&ticket);
                } else {
                    let _ = service.abandon(ticket, Cancellation::Superseded);
                }
            }
            PreparationTicket::Catalogue(ticket) => {
                if service.borrow_terminal(&ticket).is_ok_and(|v| v.is_some()) {
                    let _ = service.consume(&ticket);
                } else {
                    let _ = service.abandon(ticket, Cancellation::Superseded);
                }
            }
            PreparationTicket::Profile(ticket) => {
                if service.borrow_terminal(&ticket).is_ok_and(|v| v.is_some()) {
                    let _ = service.consume(&ticket);
                } else {
                    let _ = service.abandon(ticket, Cancellation::Superseded);
                }
            }
        }
    }
}

/// Admit one real cooperative preparation job. The closure is the single
/// declared Prepare phase; the capability is dropped before Yielded. The
/// closure must have a bounded input and bounded output matching `charge`.
pub fn submit<R, W>(
    kind: PreparationKind,
    scope: Scope,
    input: PreparationInput,
    work: W,
) -> Result<PreparedTask<R>, PreparationError>
where
    R: Send + 'static,
    W: FnOnce(&mut FamilyPreparation) -> R + Send + 'static,
{
    let service = WorkService::shared()?;
    let job = PreparationJob {
        phase: PreparationPhase::Load,
        work: Some(work),
        output: None,
        retained_input: input.charge.input_bytes(),
        retained_output: input.charge.result_bytes(),
    };
    let ticket = {
        let mut admitted = service.lock();
        let identity: WorkIdentity = admitted.mint_identity(scope, None)?;
        let conditions = ProcessConditions::new(admitted.incarnation());
        match kind.index() {
            0 => PreparationTicket::Card(
                admitted
                    .try_submit::<PreparationSpec<R, 0>, _>(identity, &input, job, &conditions)
                    .map_err(|refused| refused.refusal())?,
            ),
            1 => PreparationTicket::Catalogue(
                admitted
                    .try_submit::<PreparationSpec<R, 1>, _>(identity, &input, job, &conditions)
                    .map_err(|refused| refused.refusal())?,
            ),
            _ => PreparationTicket::Profile(
                admitted
                    .try_submit::<PreparationSpec<R, 2>, _>(identity, &input, job, &conditions)
                    .map_err(|refused| refused.refusal())?,
            ),
        }
    };
    Ok(PreparedTask {
        service,
        ticket: Some(ticket),
    })
}
