//! Revision-fenced WalkTo tile decode requests over the shared WorkAdmission worker.

use std::fmt;
use std::sync::Arc;

use api::identity::{
    Cancellation, IdentityError, PublishSequence, Scope, Settlement, WorkIdentity,
};
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
        // This charge bounds each accepted decode job's PNG input, scratch, and RGBA result.
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
    // WorkTicket has a function-pointer marker, so store its identity and
    // revision. Rebuild the exact typed ticket for each service call.
    identity: WorkIdentity,
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
    #[cfg(feature = "test-support")]
    held_ticket_count: usize,
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
            #[cfg(feature = "test-support")]
            held_ticket_count: 0,
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
            if let Err(error) = self.try_submit_pending() {
                self.pending = None;
                self.latest = None;
                self.full_capacity = None;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Retry a Full demand only after a capacity publication changes.
    pub fn retry_pending(&mut self) -> Result<(), WalkMapDecodeError> {
        self.try_submit_pending()
    }

    /// Poll without waiting. A completed current result is consumed exactly
    /// once; stale terminals are consumed and dropped before pending work is
    /// submitted.
    pub fn poll(&mut self) -> Option<Result<WalkMapDecodeResult, WalkMapDecodeError>> {
        if self.active.is_none() {
            if let Err(error) = self.try_submit_pending() {
                self.pending = None;
                self.latest = None;
                self.full_capacity = None;
                return Some(Err(error));
            }
        }
        let active = self.active.as_ref()?;
        let ticket = WorkTicket::<DecodeSpec>::new(active.identity, active.revision);
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
        #[cfg(feature = "test-support")]
        self.release_ticket_for_test();
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
    /// Number of accepted tickets still in this controller's custody.
    #[cfg(feature = "test-support")]
    pub fn held_ticket_count_for_test(&self) -> usize {
        self.held_ticket_count
    }

    #[cfg(feature = "test-support")]
    fn release_ticket_for_test(&mut self) {
        assert!(
            self.held_ticket_count > 0,
            "controller released an untracked decode ticket"
        );
        self.held_ticket_count -= 1;
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
            .map(|active| WorkTicket::<DecodeSpec>::new(active.identity, active.revision));
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
                    let _ = shared.lock().abandon(
                        WorkTicket::<DecodeSpec>::new(active.identity, active.revision),
                        Cancellation::ScopeInvalidated,
                    );
                    #[cfg(feature = "test-support")]
                    self.release_ticket_for_test();
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

    fn try_submit_pending(&mut self) -> Result<(), WalkMapDecodeError> {
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
            if self.full_capacity == Some(sequence) {
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
            #[cfg(test)]
            let job = tests::decode_job(Arc::clone(&pending.png));
            #[cfg(not(test))]
            let job = WalkMapDecodeJob::new(Arc::clone(&pending.png));
            let submission =
                admitted.try_submit::<DecodeSpec, _>(identity, pending, job, &conditions);
            (sequence, submission)
        };
        match submission {
            Ok(ticket) => {
                let pending = self.pending.take().expect("accepted pending demand");
                self.active = Some(ActiveDecode {
                    identity: ticket.identity(),
                    generation: pending.generation,
                    revision: ticket.revision(),
                    image: pending.image,
                    key: pending.key,
                    png_bytes: pending.png.len(),
                    stale: false,
                });
                #[cfg(feature = "test-support")]
                {
                    self.held_ticket_count += 1;
                }
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
                let _ = shared.lock().abandon(
                    WorkTicket::<DecodeSpec>::new(active.identity, active.revision),
                    Cancellation::ScopeInvalidated,
                );
                #[cfg(feature = "test-support")]
                self.release_ticket_for_test();
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
        match self.try_submit_pending() {
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
#[cfg(test)]
mod tests {
    use super::*;
    use api::work::{CooperativeJob, RetainedState, StepContext, StepOutcome, WorkCapacity};
    use nav::map::identity::Digest;
    use nav::map::spatial::TILE_PIXELS;
    use std::sync::{Arc, Condvar, Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    static DECODE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn decode_test_lock() -> MutexGuard<'static, ()> {
        DECODE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn image() -> ImageIdentity {
        ImageIdentity {
            revision: 289,
            content: Digest([0x31; 32]),
            policy: Digest([0x42; 32]),
        }
    }

    fn key(x: i32) -> TileKey {
        TileKey {
            plane: 0,
            lod: 0,
            x,
            z: 50,
        }
    }

    fn png(marker: u8) -> Arc<[u8]> {
        let pixels = vec![marker; TILE_RGBA_BYTES];
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, TILE_PIXELS, TILE_PIXELS);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("PNG header");
            writer.write_image_data(&pixels).expect("PNG pixels");
        }
        Arc::from(encoded)
    }

    fn capacity() -> WorkCapacity {
        WorkService::shared()
            .expect("shared work service")
            .lock()
            .capacity()
    }

    fn wait_for_capacity_zero() {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let current = capacity();
            if current == WorkCapacity::EMPTY {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "decode capacity did not return to zero: {current:?}"
            );
            std::thread::yield_now();
        }
    }

    fn wait_for_jobs_below(jobs: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let current = capacity();
            if current.jobs() < jobs {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "decode capacity did not fall below {jobs}: {current:?}"
            );
            std::thread::yield_now();
        }
    }

    fn wait_for_terminal(ticket: &WorkTicket<DecodeSpec>) {
        let shared = WorkService::shared().expect("shared work service");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let published = {
                let service = shared.lock();
                service
                    .borrow_terminal(ticket)
                    .expect("decode ticket remains admitted")
                    .is_some()
            };
            if published {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "shared worker did not publish the decode terminal"
            );
            std::thread::yield_now();
        }
    }

    fn wait_for_result(controller: &mut WalkMapDecodeController) -> WalkMapDecodeResult {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match controller.poll() {
                Some(Ok(result)) => return result,
                Some(Err(error)) => panic!("decode failed: {error}"),
                None => {
                    assert!(
                        Instant::now() < deadline,
                        "shared worker did not return the decode result"
                    );
                    std::thread::yield_now();
                }
            }
        }
    }
    struct DecodeStepGateState {
        entered: bool,
        released: bool,
    }

    struct DecodeStepGate {
        state: Mutex<DecodeStepGateState>,
        changed: Condvar,
    }

    impl DecodeStepGate {
        fn new() -> Self {
            Self {
                state: Mutex::new(DecodeStepGateState {
                    entered: false,
                    released: false,
                }),
                changed: Condvar::new(),
            }
        }

        fn enter_and_wait(&self) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.entered = true;
            self.changed.notify_all();

            let deadline = Instant::now() + Duration::from_secs(2);
            while !state.released {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let (next, timeout) = self
                    .changed
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state = next;
                if timeout.timed_out() {
                    break;
                }
            }
        }

        fn wait_until_entered(&self) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let deadline = Instant::now() + Duration::from_secs(2);
            while !state.entered {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "decode worker did not enter the gate");
                let (next, timeout) = self
                    .changed
                    .wait_timeout(state, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state = next;
                assert!(
                    !timeout.timed_out() || state.entered,
                    "decode worker did not enter the gate"
                );
            }
        }

        fn release(&self) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.released = true;
            self.changed.notify_all();
        }
    }

    static NEXT_DECODE_STEP_GATE: Mutex<Option<Arc<DecodeStepGate>>> = Mutex::new(None);

    struct DecodeStepGateCleanup(Arc<DecodeStepGate>);

    impl Drop for DecodeStepGateCleanup {
        fn drop(&mut self) {
            self.0.release();
            let mut installed = NEXT_DECODE_STEP_GATE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if installed
                .as_ref()
                .is_some_and(|gate| Arc::ptr_eq(gate, &self.0))
            {
                *installed = None;
            }
        }
    }

    pub(super) struct TestDecodeJob {
        inner: WalkMapDecodeJob,
        gate: Option<Arc<DecodeStepGate>>,
    }

    impl CooperativeJob for TestDecodeJob {
        type Output = Vec<u8>;
        type Error = WalkMapDecodeFailure;
        type Phase = api::work::WalkMapDecodePhase;

        fn step(
            &mut self,
            ctx: &mut StepContext<'_, Self::Phase>,
        ) -> StepOutcome<Self::Output, Self::Error> {
            if let Some(gate) = self.gate.take() {
                gate.enter_and_wait();
            }
            self.inner.step(ctx)
        }

        fn retained(&self) -> RetainedState {
            self.inner.retained()
        }
    }

    pub(super) fn decode_job(png: Arc<[u8]>) -> TestDecodeJob {
        let gate = NEXT_DECODE_STEP_GATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        TestDecodeJob {
            inner: WalkMapDecodeJob::new(png),
            gate,
        }
    }

    fn install_decode_step_gate() -> (Arc<DecodeStepGate>, DecodeStepGateCleanup) {
        let gate = Arc::new(DecodeStepGate::new());
        let mut installed = NEXT_DECODE_STEP_GATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            installed.is_none(),
            "a decode step gate is already installed"
        );
        *installed = Some(Arc::clone(&gate));
        drop(installed);
        (Arc::clone(&gate), DecodeStepGateCleanup(gate))
    }

    #[test]
    fn poll_discards_a_finished_stale_result_before_submitting_latest_demand() {
        let _guard = decode_test_lock();
        wait_for_capacity_zero();
        let identity = image();
        let old_key = key(50);
        let new_key = key(51);
        let mut controller = WalkMapDecodeController::new();
        controller
            .request(7, identity, old_key, png(0x11))
            .expect("submit old tile");
        let active = controller.active.as_ref().expect("accepted old tile");
        let old_ticket = WorkTicket::<DecodeSpec>::new(active.identity, active.revision);

        controller
            .request(7, identity, new_key, png(0x22))
            .expect("retain new visible tile");
        wait_for_terminal(&old_ticket);
        assert!(controller.poll().is_none(), "stale pixels are dropped");
        assert_eq!(
            controller.active_request(),
            Some((7, identity, new_key)),
            "the latest demand is submitted after the stale terminal is consumed"
        );
        assert_eq!(controller.pending_request(), None);
        let result = wait_for_result(&mut controller);
        assert_eq!(result.key(), new_key);
        assert_eq!(result.into_rgba(), vec![0x22; TILE_RGBA_BYTES]);
        assert!(!controller.is_busy());
        drop(controller);
        wait_for_capacity_zero();
    }

    #[test]
    fn cancel_active_fences_and_consumes_the_tile_without_returning_pixels() {
        let _guard = decode_test_lock();
        wait_for_capacity_zero();
        let identity = image();
        let tile = key(52);
        let mut controller = WalkMapDecodeController::new();
        controller
            .request(8, identity, tile, png(0x33))
            .expect("submit tile");
        let active = controller.active.as_ref().expect("accepted tile");
        let ticket = WorkTicket::<DecodeSpec>::new(active.identity, active.revision);

        controller.cancel_active();
        assert!(controller.has_active(), "the stale ticket stays in custody");
        assert!(
            !controller.is_requested(8, identity, tile),
            "a cancelled tile is no longer a current demand"
        );
        wait_for_terminal(&ticket);
        assert!(
            controller.poll().is_none(),
            "cancelled pixels are discarded"
        );
        assert!(!controller.is_busy());
        drop(controller);
        wait_for_capacity_zero();
    }

    #[test]
    fn discard_pending_prevents_stale_completion_from_resubmitting_it() {
        let _guard = decode_test_lock();
        wait_for_capacity_zero();
        let identity = image();
        let old_key = key(53);
        let discarded_key = key(54);
        let mut controller = WalkMapDecodeController::new();
        controller
            .request(9, identity, old_key, png(0x44))
            .expect("submit old tile");
        let active = controller.active.as_ref().expect("accepted old tile");
        let old_ticket = WorkTicket::<DecodeSpec>::new(active.identity, active.revision);
        controller
            .request(9, identity, discarded_key, png(0x55))
            .expect("retain replacement tile");
        assert_eq!(
            controller.pending_request(),
            Some((9, identity, discarded_key))
        );

        controller.discard_pending();
        assert_eq!(controller.pending_request(), None);
        wait_for_terminal(&old_ticket);
        assert!(controller.poll().is_none());
        assert!(
            !controller.is_busy(),
            "discarded demand is never resubmitted"
        );
        wait_for_capacity_zero();
    }

    #[test]
    fn full_retry_waits_for_capacity_publication_then_retries_once() {
        let _guard = decode_test_lock();
        wait_for_capacity_zero();
        let identity = image();
        let shared_png = png(0x66);
        let mut fillers = Vec::with_capacity(64);
        for x in 0..64 {
            let mut controller = WalkMapDecodeController::new();
            controller
                .request(10, identity, key(x), Arc::clone(&shared_png))
                .expect("submit capacity filler");
            fillers.push(controller);
        }
        assert_eq!(capacity().jobs(), 64, "all admission slots are retained");

        let pending_key = key(100);
        let mut pending = WalkMapDecodeController::new();
        pending
            .request(10, identity, pending_key, Arc::clone(&shared_png))
            .expect("retain full demand");
        assert!(!pending.has_active());
        assert_eq!(pending.pending_request(), Some((10, identity, pending_key)));

        let service = WorkService::shared().expect("shared work service");
        let before = service
            .lock()
            .mint_identity(Scope::Process, None)
            .expect("mint identity before gated retries")
            .job();
        for _ in 0..8 {
            pending
                .retry_pending()
                .expect("same-sequence retry is gated");
        }
        let after = service
            .lock()
            .mint_identity(Scope::Process, None)
            .expect("mint identity after gated retries")
            .job();
        assert_eq!(
            after.get(),
            before.get() + 1,
            "same-sequence retries do not mint identities or submit"
        );
        assert!(!pending.has_active());

        let released = fillers
            .iter()
            .position(WalkMapDecodeController::has_active)
            .expect("at least one accepted ticket can be released");
        drop(fillers.swap_remove(released));
        wait_for_jobs_below(64);
        pending
            .retry_pending()
            .expect("changed capacity permits retry");
        assert!(
            pending.has_active(),
            "capacity publication admits the demand"
        );
        assert_eq!(pending.pending_request(), None);

        drop(pending);
        drop(fillers);
        wait_for_capacity_zero();
    }

    #[test]
    fn dropping_controller_releases_accepted_decode_charge() {
        let _guard = decode_test_lock();
        wait_for_capacity_zero();
        let mut controller = WalkMapDecodeController::new();
        controller
            .request(11, image(), key(55), png(0x77))
            .expect("submit tile");
        assert!(controller.has_active());
        assert!(capacity().jobs() > 0);

        drop(controller);
        wait_for_capacity_zero();
    }
    #[test]
    fn rebuilt_ticket_requires_exact_identity_and_revision_before_and_after_terminal() {
        use api::work::CancelOutcome;

        let _guard = decode_test_lock();
        wait_for_capacity_zero();
        let mut controller = WalkMapDecodeController::new();
        controller
            .request(12, image(), key(56), png(0x7a))
            .expect("submit tile");
        let active = controller.active.as_ref().expect("accepted tile");
        let exact = WorkTicket::<DecodeSpec>::new(active.identity, active.revision);
        let wrong_revision = WorkTicket::<DecodeSpec>::new(
            active.identity,
            InputRevision::new(active.revision.get() + 1),
        );
        let older_revision = WorkTicket::<DecodeSpec>::new(
            active.identity,
            InputRevision::new(active.revision.get() - 1),
        );
        let same_job_other_identity = WorkTicket::<DecodeSpec>::new(
            WorkIdentity::new(
                active.identity.job(),
                active.identity.incarnation(),
                active.identity.scope(),
                Some(active.identity.job()),
            )
            .expect("same job id, other origin"),
            active.revision,
        );
        let shared = WorkService::shared().expect("shared work service");
        let unrelated = WorkTicket::<DecodeSpec>::new(
            shared
                .lock()
                .mint_identity(Scope::Process, None)
                .expect("mint unrelated identity"),
            active.revision,
        );
        let stale = [
            wrong_revision,
            older_revision,
            same_job_other_identity,
            unrelated,
        ];
        let assert_stale_tickets_rejected = |tickets: &[WorkTicket<DecodeSpec>]| {
            for ticket in tickets {
                let mut service = shared.lock();
                assert_eq!(
                    service.cancel(*ticket, Cancellation::Operator),
                    CancelOutcome::StaleIdentity
                );
                assert_eq!(
                    service.abandon(*ticket, Cancellation::Operator),
                    CancelOutcome::StaleIdentity
                );
                assert_eq!(
                    service
                        .borrow_terminal(ticket)
                        .map(|terminal| terminal.is_some()),
                    Err(LedgerError::StaleIdentity)
                );
                assert_eq!(service.consume(ticket), Err(LedgerError::StaleIdentity));
            }
        };

        assert_stale_tickets_rejected(&stale);
        wait_for_terminal(&exact);
        assert_stale_tickets_rejected(&stale);

        let result = wait_for_result(&mut controller);
        assert_eq!(result.key(), key(56));
        assert_eq!(result.into_rgba(), vec![0x7a; TILE_RGBA_BYTES]);
        assert!(!controller.is_busy());
        drop(controller);
        wait_for_capacity_zero();
    }

    #[test]
    fn cancel_active_cancels_accepted_job_before_decode_step() {
        let _guard = decode_test_lock();
        wait_for_capacity_zero();
        let (gate, _cleanup) = install_decode_step_gate();
        let mut controller = WalkMapDecodeController::new();
        controller
            .request(13, image(), key(57), png(0x7b))
            .expect("submit tile");
        let active = controller.active.as_ref().expect("accepted tile");
        let exact = WorkTicket::<DecodeSpec>::new(active.identity, active.revision);

        gate.wait_until_entered();
        controller.cancel_active();
        gate.release();
        wait_for_terminal(&exact);
        let terminal = {
            let shared = WorkService::shared().expect("shared work service");
            let service = shared.lock();
            match service
                .borrow_terminal(&exact)
                .expect("exact ticket admitted")
            {
                Some(Settlement::Cancelled(reason)) => format!("Cancelled({reason:?})"),
                Some(Settlement::Completed(_)) => String::from("Completed"),
                Some(Settlement::Failed(error)) => format!("Failed({error:?})"),
                None => String::from("pending"),
            }
        };
        assert_eq!(terminal, "Cancelled(Superseded)");
        assert!(
            controller.poll().is_none(),
            "cancelled pixels are discarded"
        );
        drop(controller);
        wait_for_capacity_zero();
    }
}
