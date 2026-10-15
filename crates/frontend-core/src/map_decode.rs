//! Revision-fenced WalkTo tile decode requests over the shared WorkAdmission worker.

use std::fmt;
use std::sync::Arc;

use api::identity::{Cancellation, IdentityError, PublishSequence, Scope, Settlement};
use api::process_condition::{ProcessConditions, UnavailableTerminal};
use api::work::{
    CooperativeViolation, InputRevision, JobError, JobSettlement, LedgerError, SubmissionRefusal,
    WorkAdmission, WorkCharge, WorkFamily, WorkFamilySpec, WorkTicket,
};
use host_play::map_bind::{
    WalkMapDecodeFailure, WalkMapDecodeJob, WALK_MAP_DECODE_INPUT_OVERHEAD,
    WALK_MAP_DECODE_PNG_LIMIT, WALK_MAP_DECODE_SCRATCH_LIMIT,
};
use nav::map::identity::ImageIdentity;
use nav::map::spatial::{TileKey, TILE_RGBA_BYTES};
use script::work_admission::{WorkService, WorkStartError};

struct DecodeDemand {
    generation: u64,
    revision: InputRevision,
    image: ImageIdentity,
    key: TileKey,
    png: Arc<[u8]>,
}

struct DecodeSpec;

impl WorkFamilySpec for DecodeSpec {
    const FAMILY: WorkFamily = WorkFamily::WalkMapDecode;
    type Input = DecodeDemand;
    type Output = Vec<u8>;
    type Error = WalkMapDecodeFailure;
    type Phase = api::work::WalkMapDecodePhase;

    fn revision(input: &Self::Input) -> InputRevision {
        input.revision
    }

    fn charge(input: &Self::Input) -> WorkCharge {
        let input_bytes = input
            .png
            .len()
            .saturating_add(WALK_MAP_DECODE_INPUT_OVERHEAD);
        WorkCharge::new(
            input_bytes,
            WALK_MAP_DECODE_SCRATCH_LIMIT,
            TILE_RGBA_BYTES,
            0,
        )
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
}

struct ActiveDecode {
    ticket: WorkTicket<DecodeSpec>,
    generation: u64,
    revision: InputRevision,
    image: ImageIdentity,
    key: TileKey,
    png_bytes: usize,
    stale: bool,
}

/// Typed decode refusal. Every variant carries only a Copy protocol value or
/// scalar code, so callers can match it without retaining an error source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WalkMapDecodeError {
    #[error("{0}")]
    ServiceStart(WorkStartError),
    #[error("{0}")]
    Identity(IdentityError),
    #[error("{0}")]
    Admission(SubmissionRefusal),
    #[error("{0}")]
    Ledger(LedgerError),
    #[error("{}", .0.operator_text())]
    Unavailable(UnavailableTerminal),
    #[error("walk map decode cancelled: {0:?}")]
    Cancelled(Cancellation),
    #[error("{0}")]
    Cooperation(CooperativeViolation),
    #[error("{0}")]
    Decode(WalkMapDecodeFailure),
    #[error("background work job failed internally")]
    FailedInternally,
    #[error("walk map decode revision exhausted")]
    RevisionExhausted,
}

/// One successfully consumed decode result. The pixel buffer is transferred
/// to its upload owner and is never cloned.
pub struct WalkMapDecodeResult {
    generation: u64,
    image: ImageIdentity,
    key: TileKey,
    rgba: Vec<u8>,
}

impl WalkMapDecodeResult {
    /// Renderer generation that requested these pixels.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Exact cache identity that supplied the PNG.
    pub const fn image_identity(&self) -> ImageIdentity {
        self.image
    }

    /// Tile key associated with the decoded pixels.
    pub const fn key(&self) -> TileKey {
        self.key
    }

    /// Move the sole RGBA allocation to the panel upload owner.
    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }
}

impl fmt::Debug for WalkMapDecodeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WalkMapDecodeResult")
            .field("generation", &self.generation)
            .field("image", &self.image)
            .field("key", &self.key)
            .field("rgba_bytes", &self.rgba.len())
            .finish()
    }
}

/// One renderer's bounded WorkAdmission request state: at most one accepted
/// ticket and one replaceable caller-owned pending PNG.
pub struct WalkMapDecodeController {
    context: Option<(u64, ImageIdentity)>,
    active: Option<ActiveDecode>,
    pending: Option<DecodeDemand>,
    latest: Option<(u64, InputRevision, ImageIdentity, TileKey)>,
    next_revision: u64,
    full_capacity: Option<PublishSequence>,
}

impl WalkMapDecodeController {
    /// Empty controller. It starts no worker until an explicit tile demand.
    pub const fn new() -> Self {
        Self {
            context: None,
            active: None,
            pending: None,
            latest: None,
            next_revision: 1,
            full_capacity: None,
        }
    }

    /// Queue or submit one immutable PNG. A new tile replaces the sole pending
    /// demand and cancels a different accepted tile without waiting for it.
    pub fn request(
        &mut self,
        generation: u64,
        image: ImageIdentity,
        key: TileKey,
        png: Arc<[u8]>,
    ) -> Result<(), WalkMapDecodeError> {
        self.set_context(generation, image);
        if self.is_requested(generation, image, key) {
            return Ok(());
        }
        if png.len() > WALK_MAP_DECODE_PNG_LIMIT {
            self.cancel_active();
            self.pending = None;
            self.latest = None;
            self.full_capacity = None;
            return Err(WalkMapDecodeError::Decode(
                WalkMapDecodeFailure::PngBytesLimit,
            ));
        }
        let Some(next_revision) = self.next_revision.checked_add(1) else {
            self.cancel_active();
            self.pending = None;
            self.latest = None;
            self.full_capacity = None;
            return Err(WalkMapDecodeError::RevisionExhausted);
        };
        let revision = InputRevision::new(self.next_revision);
        self.next_revision = next_revision;
        if self.active.as_ref().is_some_and(|active| !active.stale) {
            self.cancel_active();
        }
        self.latest = Some((generation, revision, image, key));
        self.pending = Some(DecodeDemand {
            generation,
            revision,
            image,
            key,
            png,
        });
        self.full_capacity = None;
        if self.active.is_none() {
            if let Err(error) = self.try_submit_pending(true) {
                self.pending = None;
                self.latest = None;
                self.full_capacity = None;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Retry a Full demand only after a capacity publication changed or a
    /// later ordinary visible-demand turn explicitly permits another attempt.
    pub fn retry_pending(&mut self, ordinary_demand: bool) -> Result<(), WalkMapDecodeError> {
        self.try_submit_pending(ordinary_demand)
    }

    /// Poll without waiting. A completed current result is consumed exactly
    /// once; stale terminals are consumed and dropped before pending work is
    /// submitted.
    pub fn poll(&mut self) -> Option<Result<WalkMapDecodeResult, WalkMapDecodeError>> {
        if self.active.is_none() {
            if let Err(error) = self.try_submit_pending(false) {
                self.pending = None;
                self.latest = None;
                self.full_capacity = None;
                return Some(Err(error));
            }
        }
        let active = self.active.as_ref()?;
        let ticket = active.ticket;
        let shared = match WorkService::shared() {
            Ok(shared) => shared,
            Err(error) => return Some(Err(WalkMapDecodeError::ServiceStart(error))),
        };
        let settlement: Result<Option<JobSettlement<DecodeSpec>>, LedgerError> = {
            let mut admitted = shared.lock();
            match admitted
                .borrow_terminal(&ticket)
                .map(|terminal| terminal.is_some())
            {
                Ok(true) => admitted.consume(&ticket).map(Some),
                Ok(false) => Ok(None),
                Err(error) => Err(error),
            }
        };
        let settlement = match settlement {
            Ok(Some(settlement)) => settlement,
            Ok(None) => return None,
            Err(error) => return self.finish_ledger_error(error),
        };
        let active = self.active.take()?;
        self.full_capacity = None;
        let current = !active.stale
            && self.context == Some((active.generation, active.image))
            && self.latest == Some((active.generation, active.revision, active.image, active.key));
        if !current {
            drop(settlement);
            return self.retry_after_stale_terminal();
        }
        Some(match settlement {
            Settlement::Completed(rgba) => Ok(WalkMapDecodeResult {
                generation: active.generation,
                image: active.image,
                key: active.key,
                rgba,
            }),
            Settlement::Failed(JobError::Family(error)) => Err(WalkMapDecodeError::Decode(error)),
            Settlement::Failed(JobError::FailedInternally) => {
                Err(WalkMapDecodeError::FailedInternally)
            }
            Settlement::Failed(JobError::Cooperation(error)) => {
                Err(WalkMapDecodeError::Cooperation(error))
            }
            Settlement::Failed(JobError::Unavailable(error)) => {
                Err(WalkMapDecodeError::Unavailable(error))
            }
            Settlement::Cancelled(reason) => Err(WalkMapDecodeError::Cancelled(reason)),
        })
    }

    /// Active request envelope, including a stale ticket that is still
    /// settling in WorkAdmission.
    pub fn active_request(&self) -> Option<(u64, ImageIdentity, TileKey)> {
        self.active
            .as_ref()
            .map(|active| (active.generation, active.image, active.key))
    }

    /// The sole retained Full demand, if any.
    pub fn pending_request(&self) -> Option<(u64, ImageIdentity, TileKey)> {
        self.pending
            .as_ref()
            .map(|pending| (pending.generation, pending.image, pending.key))
    }

    /// Whether this exact tile is already active or retained as demand.
    pub fn is_requested(&self, generation: u64, image: ImageIdentity, key: TileKey) -> bool {
        self.active.as_ref().is_some_and(|active| {
            !active.stale
                && active.generation == generation
                && active.image == image
                && active.key == key
        }) || self.pending.as_ref().is_some_and(|pending| {
            pending.generation == generation && pending.image == image && pending.key == key
        })
    }

    /// Whether an accepted ticket is still awaiting its terminal.
    pub fn has_active(&self) -> bool {
        self.active.is_some()
    }

    /// Whether this controller owns accepted or pending decode work.
    pub fn is_busy(&self) -> bool {
        self.active.is_some() || self.pending.is_some()
    }

    /// Bounded staging custody: active input plus its reserved RGBA result,
    /// and the one caller-owned pending PNG.
    pub fn staging_bytes(&self) -> usize {
        self.active
            .as_ref()
            .map_or(0, |active| active.png_bytes + TILE_RGBA_BYTES)
            + self.pending.as_ref().map_or(0, |pending| pending.png.len())
    }

    /// Cancel an accepted tile that is no longer the visible demand. Its
    /// terminal remains owned until the WorkAdmission worker returns.
    pub fn cancel_active(&mut self) {
        let ticket = self
            .active
            .as_ref()
            .filter(|active| !active.stale)
            .map(|active| active.ticket);
        if let Some(ticket) = ticket {
            if let Ok(shared) = WorkService::shared() {
                let _ = shared.lock().cancel(ticket, Cancellation::Superseded);
            }
            if let Some(active) = self.active.as_mut() {
                active.stale = true;
            }
            if self.pending.is_none() {
                self.latest = None;
            }
        }
    }

    /// Drop the one replaceable pending demand.
    pub fn discard_pending(&mut self) {
        self.pending = None;
        self.full_capacity = None;
        if self.active.as_ref().is_none_or(|active| active.stale) {
            self.latest = None;
        }
    }

    /// Invalidate the current generation/image and abandon any accepted result
    /// custody without waiting for the worker.
    pub fn invalidate(&mut self) {
        if let Some(mut active) = self.active.take() {
            active.stale = true;
            match WorkService::shared() {
                Ok(shared) => {
                    let _ = shared
                        .lock()
                        .abandon(active.ticket, Cancellation::ScopeInvalidated);
                }
                Err(_) => self.active = Some(active),
            }
        }
        self.context = None;
        self.pending = None;
        self.latest = None;
        self.full_capacity = None;
    }

    fn set_context(&mut self, generation: u64, image: ImageIdentity) {
        if self
            .context
            .is_some_and(|context| context != (generation, image))
        {
            self.invalidate();
        }
        self.context = Some((generation, image));
    }

    fn try_submit_pending(&mut self, ordinary_demand: bool) -> Result<(), WalkMapDecodeError> {
        if self.active.is_some() || self.pending.is_none() {
            return Ok(());
        }
        let shared = match WorkService::shared() {
            Ok(shared) => shared,
            Err(error) => {
                self.pending = None;
                self.latest = None;
                self.full_capacity = None;
                return Err(WalkMapDecodeError::ServiceStart(error));
            }
        };
        let (sequence, submission) = {
            let mut admitted = shared.lock();
            let sequence = admitted.publications().capacity().sequence();
            if !ordinary_demand && self.full_capacity == Some(sequence) {
                return Ok(());
            }
            let identity = match admitted.mint_identity(Scope::Process, None) {
                Ok(identity) => identity,
                Err(error) => {
                    self.pending = None;
                    self.latest = None;
                    self.full_capacity = None;
                    return Err(WalkMapDecodeError::Identity(error));
                }
            };
            let conditions = ProcessConditions::new(admitted.incarnation());
            let Some(pending) = self.pending.as_ref() else {
                return Ok(());
            };
            let job = WalkMapDecodeJob::new(Arc::clone(&pending.png));
            let submission =
                admitted.try_submit::<DecodeSpec, _>(identity, pending, job, &conditions);
            (sequence, submission)
        };
        match submission {
            Ok(ticket) => {
                let pending = self.pending.take().expect("accepted pending demand");
                self.active = Some(ActiveDecode {
                    ticket,
                    generation: pending.generation,
                    revision: pending.revision,
                    image: pending.image,
                    key: pending.key,
                    png_bytes: pending.png.len(),
                    stale: false,
                });
                self.full_capacity = None;
                Ok(())
            }
            Err(refused) => {
                let (refusal, job) = refused.into_parts();
                drop(job);
                if refusal == SubmissionRefusal::Full {
                    self.full_capacity = Some(sequence);
                    Ok(())
                } else {
                    self.pending = None;
                    self.latest = None;
                    self.full_capacity = None;
                    Err(WalkMapDecodeError::Admission(refusal))
                }
            }
        }
    }

    fn finish_ledger_error(
        &mut self,
        error: LedgerError,
    ) -> Option<Result<WalkMapDecodeResult, WalkMapDecodeError>> {
        let Some(active) = self.active.take() else {
            return Some(Err(WalkMapDecodeError::Ledger(error)));
        };
        let stale = active.stale;
        match WorkService::shared() {
            Ok(shared) => {
                let _ = shared
                    .lock()
                    .abandon(active.ticket, Cancellation::ScopeInvalidated);
            }
            Err(_) => {
                self.active = Some(active);
                return Some(Err(WalkMapDecodeError::Ledger(error)));
            }
        }
        if stale {
            self.retry_after_stale_terminal()
        } else {
            self.pending = None;
            self.latest = None;
            self.full_capacity = None;
            Some(Err(WalkMapDecodeError::Ledger(error)))
        }
    }

    fn retry_after_stale_terminal(
        &mut self,
    ) -> Option<Result<WalkMapDecodeResult, WalkMapDecodeError>> {
        self.full_capacity = None;
        match self.try_submit_pending(true) {
            Ok(()) => None,
            Err(error) => {
                self.pending = None;
                self.latest = None;
                Some(Err(error))
            }
        }
    }
}

impl Default for WalkMapDecodeController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WalkMapDecodeController {
    fn drop(&mut self) {
        self.invalidate();
    }
}
