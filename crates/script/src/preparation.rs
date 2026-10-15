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

use crate::work_admission::{QualifiedUnit, WorkService, WorkStartError};

/// The three complete preparation families admitted by I-work-prep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationKind {
    Card,
    Catalogue,
    Profile,
}

/// A checked charge for source/retained input, peak scratch and the result.
/// The preparing owner must bound all three using the actual family input;
/// WorkAdmission reserves the sum, not just the source's logical length.
#[derive(Debug, Clone, Copy)]
pub struct PreparationInput {
    revision: InputRevision,
    charge: WorkCharge,
    qualified_unit: Option<QualifiedUnit>,
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
        // Worker job state stores each checked bound in 32 bits; reject a
        // bound that cannot be represented before constructing that state.
        for bytes in [input_bytes, scratch_bytes, result_bytes] {
            u32::try_from(bytes).map_err(|_| SubmissionRefusal::InvalidArgument)?;
        }
        Ok(Self {
            revision: InputRevision::new(revision),
            charge,
            qualified_unit: None,
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

    /// Qualify the first Prepare dispatch for the measured, immutable
    /// Gathering-family config decode. All later dispatches keep the normal
    /// WorkAdmission slice budget.
    pub fn with_gathering_family_decode(mut self) -> Self {
        self.qualified_unit = Some(QualifiedUnit::GatheringFamilyDecode);
        self
    }

    /// Qualify successive bounded Quester folder-registry actions (at most
    /// 1 MiB per document), after Load and before Seal. The adapter reports
    /// no known total until its last Prepare action; WorkAdmission uses that
    /// final completed total to restore the ordinary Seal allowance.
    pub fn with_quester_registry_actions(mut self) -> Self {
        self.qualified_unit = Some(QualifiedUnit::QuesterRegistryAction);
        self
    }

    /// Qualify one pinned bundled Quester Path compilation, including its
    /// selected quest facts, for the first Profile Prepare dispatch only.
    /// Every later step, including Seal, uses the ordinary allowance.
    pub fn with_quester_progress_compile(mut self) -> Self {
        self.qualified_unit = Some(QualifiedUnit::QuesterProgressCompile);
        self
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

    /// Reserve the bounded Quester folder's retained documents, current
    /// file, compiled rows and terminal in addition to the actual settings
    /// tree. Folder bytes reserve their fixed declared maxima (rather than
    /// the old 128 MiB reservation); settings and path bytes are measured.
    /// Scanning and parsing stay on the worker.
    pub fn for_quester_folder(
        revision: u64,
        settings: Self,
        source: &crate::quester::registry::FolderSource,
    ) -> Result<Self, SubmissionRefusal> {
        if !source.enabled {
            return Ok(settings.with_quester_registry_actions());
        }
        use crate::quester::registry::{
            PathRow, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_ROWS, MAX_RETAINED_FOLDER_DOCUMENT_BYTES,
        };
        let path_bytes = source.folder.as_os_str().len();
        if path_bytes > MAX_REGISTRY_FILE_BYTES {
            return Err(SubmissionRefusal::InvalidArgument);
        }
        let input = settings
            .charge
            .input_bytes()
            .checked_add(MAX_RETAINED_FOLDER_DOCUMENT_BYTES)
            .and_then(|bytes| bytes.checked_add(MAX_REGISTRY_FILE_BYTES))
            .and_then(|bytes| bytes.checked_add(path_bytes))
            .ok_or(SubmissionRefusal::InvalidArgument)?;
        let scratch = settings
            .charge
            .scratch_bytes()
            .checked_add(4 * MAX_REGISTRY_FILE_BYTES)
            .ok_or(SubmissionRefusal::InvalidArgument)?;
        let result = settings
            .charge
            .result_bytes()
            .checked_add(2 * MAX_RETAINED_FOLDER_DOCUMENT_BYTES)
            .and_then(|bytes| bytes.checked_add(MAX_REGISTRY_ROWS * std::mem::size_of::<PathRow>()))
            .ok_or(SubmissionRefusal::InvalidArgument)?;
        Self::new(revision, input, scratch, result).map(Self::with_quester_registry_actions)
    }
}

/// One owner-controlled unit of preparation on the shared worker.
///
/// A `Continue` retains the closure's bounded owned state and yields to the
/// WorkAdmission scheduler. `Complete` transfers the result and drops that
/// state before the separate seal step.
pub enum CooperativePreparationStep<R> {
    Continue {
        completed: u32,
        total: Option<u32>,
    },
    Complete {
        output: R,
        completed: u32,
        total: Option<u32>,
    },
}

impl<R> CooperativePreparationStep<R> {
    fn progress(&self) -> (u32, Option<u32>) {
        match self {
            Self::Continue { completed, total }
            | Self::Complete {
                completed, total, ..
            } => (*completed, *total),
        }
    }
}

// Progress is replaceable. Keep the first Prepare observation, then sample
// at 1,024-action boundaries; always report the final Prepare result. The
// finite sample budget leaves room for Load/Seal and does not bound work.
const PREPARE_PROGRESS_INTERVAL: u16 = 1024;
const MAX_PREPARE_PROGRESS_MILESTONES: u16 = 1024;

struct CooperativePreparationJob<R, W> {
    phase: PreparationPhase,
    work: Option<W>,
    output: Option<R>,
    retained_input: u32,
    retained_scratch: u32,
    retained_output: u32,
    actions_since_progress: u16,
    progress_milestones: u16,
}

impl<R, W> CooperativeJob for CooperativePreparationJob<R, W>
where
    R: Send + 'static,
    W: FnMut(&mut FamilyPreparation) -> CooperativePreparationStep<R> + Send + 'static,
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
                self.phase = PreparationPhase::Prepare;
                step.publish_progress(PreparationPhase::Load, 1, Some(1));
                let _ = step.consume_unit();
                StepOutcome::Yielded
            }
            PreparationPhase::Prepare => {
                let work = self.work.as_mut().expect("prepare phase has work");
                let result = FamilyPreparation::within(step, work);
                let (completed, total) = result.progress();
                self.actions_since_progress = self.actions_since_progress.saturating_add(1);
                let finished = matches!(&result, CooperativePreparationStep::Complete { .. });
                if finished
                    || (self.progress_milestones < MAX_PREPARE_PROGRESS_MILESTONES
                        && (self.progress_milestones == 0
                            || self.actions_since_progress == PREPARE_PROGRESS_INTERVAL))
                {
                    step.publish_progress(PreparationPhase::Prepare, completed, total);
                    self.progress_milestones += u16::from(!finished);
                    self.actions_since_progress = 0;
                }
                let _ = step.consume_unit();
                match result {
                    CooperativePreparationStep::Continue { .. } => StepOutcome::Yielded,
                    CooperativePreparationStep::Complete { output, .. } => {
                        self.work.take();
                        self.output = Some(output);
                        self.phase = PreparationPhase::Seal;
                        StepOutcome::Yielded
                    }
                }
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
            self.retained_input as usize
                + if self.work.is_some() {
                    self.retained_scratch as usize
                } else {
                    0
                }
                + if self.output.is_some() {
                    self.retained_output as usize
                } else {
                    0
                },
        )
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
    retained_input: u32,
    retained_output: u32,
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
            self.retained_input as usize
                + if self.output.is_some() {
                    self.retained_output as usize
                } else {
                    0
                },
        )
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PreparationError {
    #[error("could not start work admission: {0}")]
    Start(#[from] WorkStartError),
    #[error("could not mint preparation identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("preparation refused: {0}")]
    Refused(#[from] SubmissionRefusal),
    #[error("{0}")]
    Unavailable(String),
    #[error("preparation ledger: {0}")]
    Ledger(#[from] LedgerError),
    #[error("preparation job: {0}")]
    Job(#[from] JobError<Infallible>),
    #[error("preparation cancelled: {0:?}")]
    Cancelled(Cancellation),
}

// WorkTicket's frozen marker contains a function pointer, which is not a
// storable seam field. Keep only the accepted identity and input revision;
// each service operation reconstructs the exact typed ticket for this kind.
struct PreparationTicket {
    kind: PreparationKind,
    identity: WorkIdentity,
    revision: InputRevision,
}

/// An exact-job result lease. Dropping it revokes an unfinished job; a
/// finished, unconsumed result is consumed and discarded, releasing credit.
/// A stuck worker keeps its physical charge even after the ticket is dropped.
pub struct PreparedTask<R: Send + 'static> {
    service: Arc<Mutex<WorkService>>,
    ticket: Option<PreparationTicket>,
    _output: PhantomData<R>,
}

impl<R: Send + 'static> PreparedTask<R> {
    /// Correlation identity used to fence a later global publish.
    pub fn identity(&self) -> WorkIdentity {
        self.ticket
            .as_ref()
            .expect("live preparation ticket")
            .identity
    }

    /// Whether the worker published this exact terminal. No job is run by the caller.
    pub fn is_finished(&self) -> bool {
        let service = self.service.lock();
        let ticket = self.ticket.as_ref().expect("live preparation ticket");
        match ticket.kind {
            PreparationKind::Card => !matches!(
                service.borrow_terminal(&WorkTicket::<PreparationSpec<R, 0>>::new(
                    ticket.identity,
                    ticket.revision,
                )),
                Ok(None)
            ),
            PreparationKind::Catalogue => !matches!(
                service.borrow_terminal(&WorkTicket::<PreparationSpec<R, 1>>::new(
                    ticket.identity,
                    ticket.revision,
                )),
                Ok(None)
            ),
            PreparationKind::Profile => !matches!(
                service.borrow_terminal(&WorkTicket::<PreparationSpec<R, 2>>::new(
                    ticket.identity,
                    ticket.revision,
                )),
                Ok(None)
            ),
        }
    }

    /// Request exact-job cancellation without treating the request as worker
    /// return. Consumers still reject a stale generation after settlement.
    pub fn cancel(&mut self, reason: Cancellation) {
        let mut service = self.service.lock();
        let ticket = self.ticket.as_ref().expect("live preparation ticket");
        match ticket.kind {
            PreparationKind::Card => {
                service.cancel(
                    WorkTicket::<PreparationSpec<R, 0>>::new(ticket.identity, ticket.revision),
                    reason,
                );
            }
            PreparationKind::Catalogue => {
                service.cancel(
                    WorkTicket::<PreparationSpec<R, 1>>::new(ticket.identity, ticket.revision),
                    reason,
                );
            }
            PreparationKind::Profile => {
                service.cancel(
                    WorkTicket::<PreparationSpec<R, 2>>::new(ticket.identity, ticket.revision),
                    reason,
                );
            }
        }
    }

    /// Wait for and take a terminal. Only callers that are already off-pump
    /// or have checked `is_finished()` may block here. In a pump or tick, use
    /// `is_finished()` first; the wait releases the service mutex.
    pub fn join(mut self) -> Result<R, PreparationError> {
        // Leave the ticket in `self` until terminal custody transfers. If
        // waiting or consuming fails, Drop still abandons this exact job.
        let ticket = self.ticket.as_ref().expect("live preparation ticket");
        let terminal = match ticket.kind {
            PreparationKind::Card => {
                let exact =
                    WorkTicket::<PreparationSpec<R, 0>>::new(ticket.identity, ticket.revision);
                WorkService::wait_terminal(&self.service, &exact)?;
                self.service.lock().consume(&exact)?
            }
            PreparationKind::Catalogue => {
                let exact =
                    WorkTicket::<PreparationSpec<R, 1>>::new(ticket.identity, ticket.revision);
                WorkService::wait_terminal(&self.service, &exact)?;
                self.service.lock().consume(&exact)?
            }
            PreparationKind::Profile => {
                let exact =
                    WorkTicket::<PreparationSpec<R, 2>>::new(ticket.identity, ticket.revision);
                WorkService::wait_terminal(&self.service, &exact)?;
                self.service.lock().consume(&exact)?
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
        match ticket.kind {
            PreparationKind::Card => release_or_abandon(
                &mut service,
                WorkTicket::<PreparationSpec<R, 0>>::new(ticket.identity, ticket.revision),
            ),
            PreparationKind::Catalogue => release_or_abandon(
                &mut service,
                WorkTicket::<PreparationSpec<R, 1>>::new(ticket.identity, ticket.revision),
            ),
            PreparationKind::Profile => release_or_abandon(
                &mut service,
                WorkTicket::<PreparationSpec<R, 2>>::new(ticket.identity, ticket.revision),
            ),
        }
    }
}

fn release_or_abandon<F: WorkFamilySpec>(service: &mut WorkService, ticket: WorkTicket<F>) {
    if service
        .borrow_terminal(&ticket)
        .is_ok_and(|value| value.is_some())
    {
        let _ = service.consume(&ticket);
    } else {
        let _ = service.abandon(ticket, Cancellation::Superseded);
    }
}

/// Admit one opaque preparation callback. The capability is dropped before
/// the callback's result crosses the yield boundary.
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
    let job = PreparationJob {
        phase: PreparationPhase::Load,
        work: Some(work),
        output: None,
        retained_input: input.charge.input_bytes() as u32,
        retained_output: input.charge.result_bytes() as u32,
    };
    submit_job(kind, scope, input, job)
}

/// Admit a stateful preparation on the sole WorkAdmission worker. One
/// callback invocation is one bounded Prepare dispatch; `Continue` retains
/// the owner's bounded state across a scheduler yield.
pub fn submit_cooperative<R, W>(
    kind: PreparationKind,
    scope: Scope,
    input: PreparationInput,
    work: W,
) -> Result<PreparedTask<R>, PreparationError>
where
    R: Send + 'static,
    W: FnMut(&mut FamilyPreparation) -> CooperativePreparationStep<R> + Send + 'static,
{
    let job = CooperativePreparationJob {
        phase: PreparationPhase::Load,
        work: Some(work),
        output: None,
        retained_input: input.charge.input_bytes() as u32,
        retained_scratch: input.charge.scratch_bytes() as u32,
        retained_output: input.charge.result_bytes() as u32,
        actions_since_progress: 0,
        progress_milestones: 0,
    };
    submit_job(kind, scope, input, job)
}

fn submit_job<R, J>(
    kind: PreparationKind,
    scope: Scope,
    input: PreparationInput,
    job: J,
) -> Result<PreparedTask<R>, PreparationError>
where
    R: Send + 'static,
    J: CooperativeJob<Output = R, Error = Infallible, Phase = PreparationPhase>,
{
    let service = WorkService::shared()?;
    let ticket = {
        let mut admitted = service.lock();
        let identity: WorkIdentity = admitted.mint_identity(scope, None)?;
        let conditions = ProcessConditions::new(admitted.incarnation());
        match (kind, input.qualified_unit) {
            (PreparationKind::Card, None) => {
                let _ = admitted
                    .try_submit::<PreparationSpec<R, 0>, _>(identity, &input, job, &conditions)
                    .map_err(|refused| refused.refusal())?;
            }
            (PreparationKind::Card, Some(unit)) => {
                let _ = admitted
                    .try_submit_qualified::<PreparationSpec<R, 0>, _>(
                        identity,
                        &input,
                        job,
                        &conditions,
                        unit,
                    )
                    .map_err(|refused| refused.refusal())?;
            }
            (PreparationKind::Catalogue, None) => {
                let _ = admitted
                    .try_submit::<PreparationSpec<R, 1>, _>(identity, &input, job, &conditions)
                    .map_err(|refused| refused.refusal())?;
            }
            (PreparationKind::Catalogue, Some(unit)) => {
                let _ = admitted
                    .try_submit_qualified::<PreparationSpec<R, 1>, _>(
                        identity,
                        &input,
                        job,
                        &conditions,
                        unit,
                    )
                    .map_err(|refused| refused.refusal())?;
            }
            (PreparationKind::Profile, None) => {
                let _ = admitted
                    .try_submit::<PreparationSpec<R, 2>, _>(identity, &input, job, &conditions)
                    .map_err(|refused| refused.refusal())?;
            }
            (PreparationKind::Profile, Some(unit)) => {
                let _ = admitted
                    .try_submit_qualified::<PreparationSpec<R, 2>, _>(
                        identity,
                        &input,
                        job,
                        &conditions,
                        unit,
                    )
                    .map_err(|refused| refused.refusal())?;
            }
        }
        PreparationTicket {
            kind,
            identity,
            revision: input.revision(),
        }
    };
    Ok(PreparedTask {
        service,
        ticket: Some(ticket),
        _output: PhantomData,
    })
}

#[cfg(test)]
mod tests {
    use super::PreparationInput;
    use crate::quester::registry::{
        FolderSource, PathRow, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_ROWS,
        MAX_RETAINED_FOLDER_DOCUMENT_BYTES,
    };
    use std::path::PathBuf;

    #[test]
    fn folder_reserves_fixed_declared_maxima_plus_measured_settings_and_path() {
        let settings = PreparationInput::new(1, 4096, 2 << 20, 2 << 20).unwrap();
        let first = PreparationInput::for_quester_folder(
            1,
            settings,
            &FolderSource {
                enabled: true,
                folder: PathBuf::from("/a"),
            },
        )
        .unwrap();
        let second = PreparationInput::for_quester_folder(
            1,
            settings,
            &FolderSource {
                enabled: true,
                folder: PathBuf::from("/longer"),
            },
        )
        .unwrap();
        assert_eq!(
            first.charge().input_bytes(),
            4096 + MAX_RETAINED_FOLDER_DOCUMENT_BYTES + MAX_REGISTRY_FILE_BYTES + 2
        );
        assert_eq!(
            first.charge().scratch_bytes(),
            (2 << 20) + 4 * MAX_REGISTRY_FILE_BYTES
        );
        assert_eq!(
            first.charge().result_bytes(),
            (2 << 20)
                + 2 * MAX_RETAINED_FOLDER_DOCUMENT_BYTES
                + MAX_REGISTRY_ROWS * std::mem::size_of::<PathRow>()
        );
        assert_eq!(
            second.charge().input_bytes() - first.charge().input_bytes(),
            "/longer".len() - "/a".len()
        );
        assert_eq!(
            second.charge().scratch_bytes(),
            first.charge().scratch_bytes()
        );
        assert_eq!(
            second.charge().result_bytes(),
            first.charge().result_bytes()
        );
    }
    #[test]
    fn contained_preparation_panic_says_failed_internally() {
        let input = PreparationInput::new(1, 0, 0, 0).unwrap();
        let task = super::submit(
            super::PreparationKind::Profile,
            api::identity::Scope::Process,
            input,
            |_| -> () { panic!("injected preparation panic") },
        )
        .unwrap();
        let error = task.join().expect_err("panicking job fails");
        assert!(
            error
                .to_string()
                .contains("background work job failed internally"),
            "{error}"
        );
    }

    #[test]
    fn forty_thousand_preparation_dispatches_complete_without_progress_refusal() {
        let input = PreparationInput::new(1, 0, 0, 0).unwrap();
        let mut actions = 0_u32;
        let task = super::submit_cooperative(
            super::PreparationKind::Catalogue,
            api::identity::Scope::Process,
            input,
            move |_| {
                actions += 1;
                if actions == 40_000 {
                    super::CooperativePreparationStep::Complete {
                        output: actions,
                        completed: actions,
                        total: Some(actions),
                    }
                } else {
                    super::CooperativePreparationStep::Continue {
                        completed: actions,
                        total: Some(40_000),
                    }
                }
            },
        )
        .unwrap();
        assert_eq!(task.join().expect("large valid job completes"), 40_000);
    }
}
