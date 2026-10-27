use api::identity::{Cancellation, PublishSequence, Scope, Settlement, WorkIdentity};
use api::interact::Driver;
use api::named_banks::{BankPreferences, NamedBankFacts};
use api::process_condition::ProcessConditions;
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, WorldTile};
use api::work::{
    BankPickPhase, Checkpoint, CooperativeJob, InputRevision, RetainedState, StepContext,
    StepOutcome, SubmissionRefusal, WorkAdmission, WorkCharge, WorkFamily, WorkFamilySpec,
    WorkTicket,
};
use nav_planner::bank_fetch::SAME_BANK;
#[cfg(any(test, all(target_os = "linux", feature = "test-support")))]
use nav_planner::router::{find_first_with, find_many_with_avoid_bounded, find_with};
use nav_planner::router::{
    CursorStep, FindOptions, SearchCursor, SearchOutcome, SearchPreflight, SearchRequest,
    SearchRequestShape, BANK_TARGET_BUDGET, NODE_BUDGET,
};
use nav_planner::world::NavWorld;
use nav_planner::WorldState;
use script::bank::ops;
use script::native::snapshot_source::BankSelectionInput;
use script::native::HostAuthority;
use script::native_bank::{
    AccessKind as NativeAccessKind, BankPickReceipt as NativeBankPickReceipt,
    BankPickRequest as NativeBankPickRequest, BankStandAccess, PickKind as NativePickKind,
    SelectedBank,
};
use script::work_admission::cancellation::WorkCancellation;
use script::work_admission::WorkService;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

// Host-worker ranking has a wall-time bound, not an observed-tick budget.
// Per-frame polls signal admitted work's fallback without PLAYER_INFO.
// Unadmitted non-walking picks publish the fallback at the same deadline.
const BANK_SELECTION_WINDOW: Duration = Duration::from_secs(5);

const BANK_PICK_JOB_BYTES: usize = 192 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickKind {
    NearShortcut = 1,
    Reachable = 2,
    AirFallback = 3,
    NoCandidate = 4,
}
pub(super) struct NativeBankPickInput {
    pub(super) request: NativeBankPickRequest,
    pub(super) authority: HostAuthority,
    pub(super) evidence: EvidenceStamp,
}

/// Selection completion is independent of movement. Each slot retains only
/// its latest bounded owner demand while shared WorkAdmission is full.
#[derive(Default)]
pub(crate) struct BankPickState {
    generation: u64,
    last_id: u64,
    current: Option<ActivePick>,
    pending_cancel: Option<(WorkCancellation<BankPickSpec>, Cancellation)>,
    // Owner-result pressure retains one payload only while publication waits.
    pending_inline: Option<Box<PendingInlinePick>>,
    owner_result: Option<Arc<nav_planner::producer::OwnerResult<PickResult>>>,
    retired_owner: Option<Arc<nav_planner::producer::OwnerResult<PickResult>>>,
    pub(crate) posted: BankSelectionInput,
    native_receipt: Option<(HostAuthority, NativeBankPickReceipt)>,
    #[cfg(test)]
    facts: Option<Arc<NamedBankFacts>>,
}

impl BankPickState {
    /// A `walk-nearest-bank` pick for the route of `route_generation` is
    /// still being resolved (its walk not armed yet).
    pub(crate) fn walking(&self, route_generation: u64) -> bool {
        self.current
            .as_ref()
            .is_some_and(|active| active.job.request.walk_generation == Some(route_generation))
    }

    pub(crate) fn take_native_receipt(&mut self) -> Option<(HostAuthority, NativeBankPickReceipt)> {
        self.native_receipt.take()
    }

    #[cfg(any(test, all(feature = "live-harness", feature = "test-support")))]
    pub(crate) fn work_observation_for_test(&self) -> Option<BankPickWorkObservation> {
        let active = self.current.as_ref()?;
        if let Some(ticket) = active.ticket {
            return Some(BankPickWorkObservation::Accepted {
                identity: ticket.identity,
                revision: ticket.revision,
            });
        }
        let pending = active.pending_job.as_ref()?;
        if active.submitting || active.last_submission_refusal != Some(SubmissionRefusal::Full) {
            return None;
        }
        Some(BankPickWorkObservation::Full {
            revision: pending.inner.input.revision,
            capacity_sequence: active.capacity_sequence?,
            charge: pending.inner.input.charge,
        })
    }

    #[cfg(any(test, all(feature = "live-harness", feature = "test-support")))]
    pub(crate) fn query_inputs_match_for_test(&self, other: &Self) -> bool {
        let (Some(left), Some(right)) = (&self.current, &other.current) else {
            return false;
        };
        let left = &left.job.request;
        let right = &right.job.request;
        left.lease.identity() == right.lease.identity()
            && left.facts.banks() == right.facts.banks()
            && left.from == right.from
            && left.opts == right.opts
            && left.state == right.state
            && left.target_count == right.target_count
            && left.walk_generation.is_some() == right.walk_generation.is_some()
            && left.native.is_some() == right.native.is_some()
            && left.allow_air_fallback() == right.allow_air_fallback()
            && left.order.len() == right.order.len()
            && left
                .order
                .iter()
                .zip(&right.order)
                .all(|(left, right)| left.index == right.index && left.access == right.access)
    }
}

#[cfg(any(test, all(feature = "live-harness", feature = "test-support")))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BankPickWorkObservation {
    Accepted {
        identity: WorkIdentity,
        revision: InputRevision,
    },
    Full {
        revision: InputRevision,
        capacity_sequence: PublishSequence,
        charge: WorkCharge,
    },
}

struct ActivePick {
    job: BankPickJob,
    scope: Scope,
    deadline: Option<Instant>,
    ticket: Option<BankPickTicketRef>,
    cancellation: Option<WorkCancellation<BankPickSpec>>,
    pending_job: Option<Box<AdmittedBankPickJob>>,
    submitting: bool,
    capacity_sequence: Option<PublishSequence>,
    owner_result_full: bool,
    #[cfg(any(test, all(feature = "live-harness", feature = "test-support")))]
    last_submission_refusal: Option<SubmissionRefusal>,
}

#[derive(Clone)]
struct BankPickJob {
    generation: u64,
    request: Arc<BankPickRequest>,
    deadline: Option<Instant>,
}

struct PendingInlinePick {
    generation: u64,
    request_id: u64,
    scope: Scope,
    job: Option<BankPickJob>,
    ticket: Option<BankPickTicketRef>,
    result: PickResult,
    posted: Option<BankSelectionInput>,
    native_receipt: Option<(HostAuthority, NativeBankPickReceipt)>,
}

struct PickCommit {
    generation: u64,
    request_id: u64,
    scope: Scope,
    job: Option<BankPickJob>,
    ticket: Option<BankPickTicketRef>,
    posted: Option<BankSelectionInput>,
    native_receipt: Option<(HostAuthority, NativeBankPickReceipt)>,
}

impl BankPickState {
    fn advance_generation(&mut self) -> Option<u64> {
        self.generation = self.generation.checked_add(1)?;
        Some(self.generation)
    }

    pub(crate) fn reset(&mut self) {
        let _ = self.advance_generation();
        self.cancel_active(Cancellation::ScopeInvalidated);
        self.clear_selection();
    }

    pub(crate) fn reset_for_route_replacement(&mut self) {
        let _ = self.advance_generation();
        self.cancel_active(Cancellation::Superseded);
        if let Some((mut cancellation, _)) = self.pending_cancel.take() {
            let _ = cancellation.abandon(Cancellation::Superseded);
        }
        self.clear_selection();
    }

    fn clear_selection(&mut self) {
        self.pending_inline = None;
        self.last_id = 0;
        if let Some(owner) = self.owner_result.take() {
            debug_assert!(self.retired_owner.is_none());
            self.retired_owner = Some(owner);
        }
        self.posted = BankSelectionInput::default();
        self.native_receipt = None;
    }

    fn cancel_active(&mut self, reason: Cancellation) {
        if let Some(mut active) = self.current.take() {
            if let Some(cancellation) = active.cancellation.take() {
                debug_assert!(self.pending_cancel.is_none());
                self.pending_cancel = Some((cancellation, reason));
            } else {
                debug_assert!(active.ticket.is_none());
            }
        }
    }
}

#[derive(Clone)]
struct NativePickContext {
    authority: HostAuthority,
    evidence: EvidenceStamp,
    allow_air_fallback: bool,
}

struct BankCandidate {
    index: usize,
    access: Option<(Vec<WorldTile>, BankStandAccess)>,
}
struct BankCandidates {
    order: Vec<BankCandidate>,
    target_count: usize,
}

struct BankPickRequest {
    request_id: u64,
    lease: nav_planner::contract::NavLease,
    facts: Arc<NamedBankFacts>,
    from: WorldTile,
    opts: FindOptions,
    state: Arc<WorldState>,
    timeout_fallback: AtomicBool,
    order: Vec<BankCandidate>,
    target_count: usize,
    walk_generation: Option<u64>,
    native: Option<NativePickContext>,
}

pub(super) struct CompactPlan {
    pub(super) lease: nav_planner::contract::RoutePlanLease,
}

#[cfg(any(test, all(target_os = "linux", feature = "test-support")))]
struct LegacyPickResult {
    index: Option<usize>,
    access_tile: Option<WorldTile>,
    kind: PickKind,
    route: Option<nav_planner::router::Route>,
}

pub(super) struct PickResult {
    index: Option<usize>,
    access_tile: Option<WorldTile>,
    kind: PickKind,
    route: Option<nav_planner::router::CompactRoute>,
    nav_lease: Option<nav_planner::contract::NavLease>,
    plan: Option<CompactPlan>,
}

impl PickResult {
    fn plain(index: Option<usize>, access_tile: Option<WorldTile>, kind: PickKind) -> Self {
        Self {
            index,
            access_tile,
            kind,
            route: None,
            nav_lease: None,
            plan: None,
        }
    }

    fn owner_result_bytes(&self) -> Option<usize> {
        let retained = self
            .route
            .as_ref()
            .map_or(Some(0), nav_planner::producer::compact_route_storage_bytes)?;
        nav_planner::producer::OwnerResultLedger::owner_result_bytes::<Self>(retained)
    }

    fn into_owner_result(
        mut self,
        identity: WorkIdentity,
        revision: InputRevision,
        reservation: nav_planner::producer::OwnerResultReservation,
    ) -> Arc<nav_planner::producer::OwnerResult<Self>> {
        if let Some(route) = self.route.take() {
            let lease = self
                .nav_lease
                .take()
                .expect("a routed bank pick retains its navigation lease");
            self.plan = Some(CompactPlan {
                lease: nav_planner::producer::publish_compact_route(
                    &lease,
                    identity,
                    revision,
                    route,
                    reservation.clone(),
                ),
            });
        }
        reservation.hold(self)
    }
}

#[derive(Clone, Copy)]
struct BankPickInput {
    revision: InputRevision,
    charge: WorkCharge,
    base_input_bytes: usize,
}

struct BankPickSpec;

const _: () = {
    fn assert_send_sync<T: Send + Sync>() {}
    let _ = assert_send_sync::<BankPickSpec>;
    let _ = assert_send_sync::<WorkCancellation<BankPickSpec>>;
};

impl WorkFamilySpec for BankPickSpec {
    const FAMILY: WorkFamily = WorkFamily::BankPick;
    type Input = BankPickInput;
    type Output = PickResult;
    type Error = std::convert::Infallible;
    type Phase = BankPickPhase;

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

// Keep only scalar ticket data in seam state. The WorkTicket marker contains
// a function pointer, so reconstruct its exact type for each service operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BankPickTicketRef {
    identity: WorkIdentity,
    revision: InputRevision,
}

impl BankPickTicketRef {
    fn from_ticket(ticket: WorkTicket<BankPickSpec>) -> Self {
        Self {
            identity: ticket.identity(),
            revision: ticket.revision(),
        }
    }

    fn ticket(self) -> WorkTicket<BankPickSpec> {
        WorkTicket::new(self.identity, self.revision)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BankPickStage {
    StartSearch,
    MaterializeTargets,
    Search,
    ResolveFirst,
    ResolveManyBest,
    ResolveMany,
    Finish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BankPickSearchKind {
    Single,
    Many,
    First,
}
type BankPickTargetResult =
    Result<nav_planner::router::TargetCost, nav_planner::router::TargetError>;

struct BankPickSearchPreflight {
    kind: BankPickSearchKind,
    token: SearchPreflight,
}

struct BankPickPreflights {
    primary: Option<BankPickSearchPreflight>,
    fallback: Option<SearchPreflight>,
}

struct CooperativeBankPickJob {
    job: BankPickJob,
    input: BankPickInput,
    lease: Option<nav_planner::contract::NavLease>,
    stage: BankPickStage,
    search_kind: Option<BankPickSearchKind>,
    search_preflights: BankPickPreflights,
    search_request: Option<SearchRequest>,
    target_buffer: Option<Vec<WorldTile>>,
    build_order: usize,
    build_access: usize,
    cursor: Option<SearchCursor>,
    scan_order: usize,
    scan_target: usize,
    best_scan: usize,
    best_cost: f64,
    first_target: Option<usize>,
    first_access_tile: Option<WorldTile>,
    first_order: usize,
    first_access: usize,
    pending_route: Option<nav_planner::router::CompactRoute>,
    pending_many_results: Option<Box<[BankPickTargetResult]>>,
    result: Option<PickResult>,
    fallback_started: bool,
}
impl CooperativeBankPickJob {
    fn new(job: BankPickJob, input: BankPickInput, preflights: BankPickPreflights) -> Self {
        let search_kind = preflights.primary.as_ref().map(|primary| primary.kind);
        Self {
            lease: Some(job.request.lease.clone()),
            job,
            input,
            stage: BankPickStage::StartSearch,
            search_kind,
            search_preflights: preflights,
            search_request: None,
            target_buffer: None,
            build_order: 0,
            build_access: 0,
            cursor: None,
            scan_order: 0,
            scan_target: 0,
            best_scan: 0,
            best_cost: f64::INFINITY,
            first_target: None,
            first_access_tile: None,
            first_order: 0,
            first_access: 0,
            pending_route: None,
            pending_many_results: None,
            result: None,
            fallback_started: false,
        }
    }

    fn record_result(
        &mut self,
        index: Option<usize>,
        access_tile: Option<WorldTile>,
        kind: PickKind,
        route: Option<nav_planner::router::CompactRoute>,
    ) -> PickResult {
        PickResult {
            index,
            access_tile,
            kind,
            route,
            nav_lease: None,
            plan: None,
        }
    }

    fn release_search_storage(&mut self, keep_fallback: bool) {
        self.cursor = None;
        self.search_request = None;
        self.target_buffer = None;
        self.search_preflights.primary = None;
        if !keep_fallback {
            self.search_preflights.fallback = None;
        }
    }

    fn start_fallback_search(&mut self) {
        self.fallback_started = true;
        self.release_search_storage(true);
        self.pending_route = None;
        self.pending_many_results = None;
        self.result = None;
        self.first_target = None;
        self.first_access_tile = None;
        self.search_kind = Some(BankPickSearchKind::Single);
        self.search_request = self.job.request.single_search_request();
        if self.search_request.is_some() {
            self.stage = BankPickStage::StartSearch;
        } else {
            self.search_preflights.fallback = None;
            let index = self.job.request.first();
            self.result = Some(self.record_result(
                index,
                None,
                if index.is_some() {
                    PickKind::AirFallback
                } else {
                    PickKind::NoCandidate
                },
                None,
            ));
            self.stage = BankPickStage::Finish;
        }
    }

    fn start_timeout_fallback(&mut self) {
        if self.job.request.walk_generation.is_some() {
            self.start_fallback_search();
            return;
        }
        self.fallback_started = true;
        self.release_search_storage(false);
        self.pending_route = None;
        self.pending_many_results = None;
        self.first_target = None;
        self.first_access_tile = None;
        self.search_kind = None;
        self.result =
            Some(self.record_result(self.job.request.first(), None, PickKind::AirFallback, None));
        self.stage = BankPickStage::Finish;
    }

    fn timeout_fallback_due(&self, now: Instant) -> bool {
        !self.fallback_started
            && (self.timeout_fallback_signaled()
                || self.job.deadline.is_some_and(|deadline| now >= deadline))
    }

    fn timeout_fallback_signaled(&self) -> bool {
        !self.fallback_started && self.job.request.timeout_fallback.load(Ordering::Acquire)
    }

    fn search_error(&mut self, kind: BankPickSearchKind) {
        self.release_search_storage(
            matches!(kind, BankPickSearchKind::Many) && self.job.request.walk_generation.is_some(),
        );
        self.search_kind = None;
        match kind {
            BankPickSearchKind::First => {
                self.result = Some(self.record_result(None, None, PickKind::NoCandidate, None));
                self.stage = BankPickStage::Finish;
            }
            BankPickSearchKind::Many if self.job.request.walk_generation.is_some() => {
                self.start_fallback_search();
            }
            BankPickSearchKind::Many | BankPickSearchKind::Single => {
                let index = self.job.request.first();
                self.result = Some(self.record_result(
                    index,
                    None,
                    if index.is_some() {
                        PickKind::AirFallback
                    } else {
                        PickKind::NoCandidate
                    },
                    None,
                ));
                self.stage = BankPickStage::Finish;
            }
        }
    }

    fn checkpoint(
        &mut self,
        checkpoint: Checkpoint,
    ) -> Option<StepOutcome<PickResult, std::convert::Infallible>> {
        match checkpoint {
            Checkpoint::Continue => None,
            Checkpoint::Yield => Some(StepOutcome::Yielded),
            Checkpoint::Cancel(_) => {
                self.release_search_storage(false);
                self.pending_route = None;
                self.pending_many_results = None;
                self.result = None;
                self.first_target = None;
                self.first_access_tile = None;
                self.search_kind = None;
                self.lease = None;
                Some(StepOutcome::Yielded)
            }
        }
    }

    fn consume_unit(
        &mut self,
        ctx: &mut StepContext<'_, BankPickPhase>,
    ) -> Option<StepOutcome<PickResult, std::convert::Infallible>> {
        self.checkpoint(ctx.consume_unit())
    }

    fn begin_search_materialization(
        &mut self,
        kind: BankPickSearchKind,
    ) -> Result<(), BankPickSearchKind> {
        match kind {
            BankPickSearchKind::Single => {
                self.search_request = self.job.request.single_search_request();
                if self.search_request.is_none() {
                    return Err(kind);
                }
                Ok(())
            }
            BankPickSearchKind::Many | BankPickSearchKind::First => {
                let mut targets = Vec::new();
                targets
                    .try_reserve_exact(self.job.request.target_count)
                    .map_err(|_| kind)?;
                self.target_buffer = Some(targets);
                self.build_order = 0;
                self.build_access = 0;
                self.stage = BankPickStage::MaterializeTargets;
                Ok(())
            }
        }
    }

    fn materialize_target_or_row(&mut self, kind: BankPickSearchKind) -> Result<(), ()> {
        let request = &self.job.request;
        let target = match kind {
            BankPickSearchKind::Many => {
                let Some(candidate) = request.order.get(self.build_order) else {
                    return Ok(());
                };
                self.build_order += 1;
                request.facts.banks()[candidate.index]
                    .routable
                    .then_some(request.facts.banks()[candidate.index].tile)
            }
            BankPickSearchKind::First => {
                let Some(candidate) = request.order.get(self.build_order) else {
                    return Ok(());
                };
                match candidate.access.as_ref() {
                    Some((tiles, _)) if self.build_access < tiles.len() => {
                        let target = tiles[self.build_access];
                        self.build_access += 1;
                        Some(target)
                    }
                    _ => {
                        self.build_order += 1;
                        self.build_access = 0;
                        None
                    }
                }
            }
            BankPickSearchKind::Single => unreachable!("single searches have no target array"),
        };
        if let Some(target) = target {
            let targets = self
                .target_buffer
                .as_mut()
                .expect("target array is being built");
            if targets.len() >= request.target_count {
                return Err(());
            }
            targets.push(target);
        }
        Ok(())
    }

    fn finish_target_materialization(&mut self, kind: BankPickSearchKind) -> Result<(), ()> {
        let targets = self
            .target_buffer
            .as_ref()
            .expect("target array is being built");
        if targets.len() != self.job.request.target_count {
            return Err(());
        }
        let targets = self
            .target_buffer
            .take()
            .expect("target array is being built")
            .into_boxed_slice();
        self.search_request = Some(match kind {
            BankPickSearchKind::Many => SearchRequest::Many {
                from: self.job.request.from,
                targets,
                options: self.job.request.opts,
                avoid: Box::default(),
                max_settles: BANK_TARGET_BUDGET,
                reconstruct_best: self.job.request.walk_generation.is_some(),
            },
            BankPickSearchKind::First => {
                let mut options = self.job.request.opts;
                options.allow_teleports = false;
                SearchRequest::First {
                    from: self.job.request.from,
                    targets,
                    fallback: Box::default(),
                    options,
                    avoid: Box::default(),
                    max_settles: self.job.request.target_budget(),
                }
            }
            BankPickSearchKind::Single => unreachable!("single searches have no target array"),
        });
        Ok(())
    }

    fn current_preflight(&mut self) -> Option<SearchPreflight> {
        if self.fallback_started {
            self.search_preflights.fallback.take()
        } else {
            self.search_preflights
                .primary
                .take()
                .map(|primary| primary.token)
        }
    }
}

impl CooperativeJob for CooperativeBankPickJob {
    type Output = PickResult;
    type Error = std::convert::Infallible;
    type Phase = BankPickPhase;

    fn retained(&self) -> RetainedState {
        let mut retained = self.input.base_input_bytes;
        let target_bytes = self.target_buffer.as_ref().map_or(0, |targets| {
            targets
                .capacity()
                .saturating_mul(std::mem::size_of::<WorldTile>())
        });
        let request_bytes = self.search_request.as_ref().map_or(0, |request| {
            request.retained_input_bytes().unwrap_or(usize::MAX)
        });
        let cursor_bytes = self.cursor.as_ref().map_or(0, |cursor| {
            cursor
                .input_bytes()
                .saturating_add(cursor.future_layout_bytes())
                .saturating_add(cursor.retained_bytes())
        });
        let route_bytes = self.pending_route.as_ref().map_or(0, |route| {
            nav_planner::producer::compact_route_storage_bytes(route).unwrap_or(usize::MAX)
        });
        let result_bytes = self.result.as_ref().map_or(0, |result| {
            result.route.as_ref().map_or(0, |route| {
                nav_planner::producer::compact_route_storage_bytes(route).unwrap_or(usize::MAX)
            })
        });
        let many_result_bytes = self.pending_many_results.as_ref().map_or(0, |results| {
            results
                .len()
                .saturating_mul(std::mem::size_of::<BankPickTargetResult>())
        });
        for bytes in [
            target_bytes,
            request_bytes,
            cursor_bytes,
            route_bytes,
            result_bytes,
            many_result_bytes,
        ] {
            retained = retained.saturating_add(bytes);
        }
        RetainedState::clean(retained)
    }

    fn step(
        &mut self,
        ctx: &mut StepContext<'_, Self::Phase>,
    ) -> StepOutcome<Self::Output, Self::Error> {
        if let Some(outcome) = self.checkpoint(ctx.checkpoint()) {
            return outcome;
        }
        if self.timeout_fallback_due(Instant::now()) {
            self.start_timeout_fallback();
        }
        loop {
            if self.timeout_fallback_signaled() {
                self.start_timeout_fallback();
                continue;
            }
            match self.stage {
                BankPickStage::StartSearch => {
                    if let Some(outcome) = self.checkpoint(ctx.checkpoint()) {
                        return outcome;
                    }
                    if self.search_request.is_none() {
                        let Some(kind) = self.search_kind else {
                            self.search_error(BankPickSearchKind::Single);
                            continue;
                        };
                        if let Err(kind) = self.begin_search_materialization(kind) {
                            self.search_error(kind);
                            continue;
                        }
                        if self.stage == BankPickStage::MaterializeTargets {
                            if let Some(outcome) = self.consume_unit(ctx) {
                                return outcome;
                            }
                            continue;
                        }
                    }
                    let Some(request) = self.search_request.take() else {
                        self.search_error(self.search_kind.unwrap_or(BankPickSearchKind::Single));
                        continue;
                    };
                    let kind = match &request {
                        SearchRequest::Single { .. } => BankPickSearchKind::Single,
                        SearchRequest::Many { .. } => BankPickSearchKind::Many,
                        SearchRequest::First { .. } => BankPickSearchKind::First,
                        SearchRequest::MissingItemReqs { .. }
                        | SearchRequest::MissingItemReqsForRoute { .. }
                        | SearchRequest::BlockingZones { .. }
                        | SearchRequest::EntranceRequirements { .. }
                        | SearchRequest::UnresolvedQuestGates { .. } => {
                            unreachable!("bank-pick request uses a route search variant")
                        }
                    };
                    let Some(preflight) = self.current_preflight() else {
                        drop(request);
                        self.search_error(kind);
                        continue;
                    };
                    match SearchCursor::new_preflighted(
                        request,
                        self.job.request.lease.clone(),
                        Arc::clone(&self.job.request.state),
                        preflight,
                    ) {
                        Ok(cursor) => {
                            self.cursor = Some(cursor);
                            self.stage = BankPickStage::Search;
                            if let Some(outcome) = self.consume_unit(ctx) {
                                return outcome;
                            }
                        }
                        Err(_) => {
                            self.search_error(kind);
                        }
                    }
                }
                BankPickStage::MaterializeTargets => {
                    if let Some(outcome) = self.checkpoint(ctx.checkpoint()) {
                        return outcome;
                    }
                    let kind = self
                        .search_kind
                        .expect("materializing search retains its shape kind");
                    if self.build_order >= self.job.request.order.len() {
                        if self.finish_target_materialization(kind).is_err() {
                            self.search_error(kind);
                            continue;
                        }
                        self.stage = BankPickStage::StartSearch;
                    } else if self.materialize_target_or_row(kind).is_err() {
                        self.search_error(kind);
                        continue;
                    }
                    if let Some(outcome) = self.consume_unit(ctx) {
                        return outcome;
                    }
                }
                BankPickStage::Search => {
                    let step = self
                        .cursor
                        .as_mut()
                        .expect("active bank-pick search retains its cursor")
                        .advance(ctx);
                    match step {
                        CursorStep::Yielded => {
                            return self
                                .checkpoint(ctx.checkpoint())
                                .unwrap_or(StepOutcome::Yielded);
                        }
                        CursorStep::Cancelled(reason) => {
                            return self
                                .checkpoint(Checkpoint::Cancel(reason))
                                .unwrap_or(StepOutcome::Yielded);
                        }
                        CursorStep::Failed(_) => {
                            let kind = self
                                .search_kind
                                .expect("active bank-pick search retains its shape kind");
                            self.search_error(kind);
                        }
                        CursorStep::Complete(completion) => {
                            let forward_settled = completion.forward_settled();
                            if self.timeout_fallback_due(Instant::now()) {
                                drop(completion);
                                self.start_timeout_fallback();
                                continue;
                            }
                            self.cursor = None;
                            let (outcome, state) = completion.into_parts();
                            drop(state);
                            match outcome {
                                SearchOutcome::Single { route } => {
                                    let index = self.job.request.first();
                                    let mut result = match route {
                                        Ok(route) => self.record_result(
                                            index,
                                            None,
                                            PickKind::AirFallback,
                                            Some(route),
                                        ),
                                        Err(_) => self.record_result(
                                            index,
                                            None,
                                            PickKind::AirFallback,
                                            None,
                                        ),
                                    };
                                    if result.route.is_some() {
                                        result.nav_lease = self.lease.take();
                                    }
                                    self.result = Some(result);
                                    self.stage = BankPickStage::Finish;
                                }
                                SearchOutcome::First { search } => match search.into_routes().0 {
                                    Ok(route) => {
                                        let access_tile = route.destination();
                                        drop(route);
                                        self.first_access_tile = Some(access_tile);
                                        self.first_order = 0;
                                        self.first_access = 0;
                                        self.stage = BankPickStage::ResolveFirst;
                                    }
                                    Err(error) => {
                                        api::host_log!(
                                            api::hostlog::Category::NavEvent,
                                            api::hostlog::Level::Warn,
                                            "bank selection search refused: reason=first-access-route error={error:?} from={:?} candidates={} targets={} settled={forward_settled} budget={}",
                                            self.job.request.from,
                                            self.job.request.order.len(),
                                            self.job.request.target_count,
                                            self.job.request.target_budget(),
                                        );
                                        self.result = Some(self.record_result(
                                            None,
                                            None,
                                            PickKind::NoCandidate,
                                            None,
                                        ));
                                        self.stage = BankPickStage::Finish;
                                    }
                                },
                                SearchOutcome::Many { results, best } => {
                                    if let Some((target, route)) = best {
                                        drop(results);
                                        self.first_target = Some(target);
                                        self.pending_route = Some(route);
                                        self.scan_order = 0;
                                        self.scan_target = 0;
                                        self.stage = BankPickStage::ResolveMany;
                                    } else if self.job.request.walk_generation.is_some() {
                                        drop(results);
                                        self.start_fallback_search();
                                    } else {
                                        self.pending_many_results = Some(results);
                                        self.best_scan = 0;
                                        self.best_cost = f64::INFINITY;
                                        self.first_target = None;
                                        self.stage = BankPickStage::ResolveManyBest;
                                    }
                                }
                                SearchOutcome::MissingItemReqs(_)
                                | SearchOutcome::MissingItemReqsForRoute { .. }
                                | SearchOutcome::BlockingZones(_)
                                | SearchOutcome::EntranceRequirements(_)
                                | SearchOutcome::UnresolvedQuestGates(_) => {
                                    unreachable!("bank-pick search returns a route outcome")
                                }
                            }
                            if forward_settled > 0 {
                                ctx.publish_progress(
                                    BankPickPhase::Search,
                                    u32::try_from(forward_settled)
                                        .expect("bank search settlement counts fit u32"),
                                    None,
                                );
                            }
                            if let Some(outcome) = self.checkpoint(ctx.checkpoint()) {
                                return outcome;
                            }
                        }
                    }
                }
                BankPickStage::ResolveFirst => {
                    let Some(target) = self.first_access_tile else {
                        self.result =
                            Some(self.record_result(None, None, PickKind::NoCandidate, None));
                        self.stage = BankPickStage::Finish;
                        continue;
                    };
                    if self.first_order >= self.job.request.order.len() {
                        self.result =
                            Some(self.record_result(None, Some(target), PickKind::Reachable, None));
                        self.stage = BankPickStage::Finish;
                        continue;
                    }
                    let candidate = &self.job.request.order[self.first_order];
                    match candidate.access.as_ref() {
                        Some((tiles, _)) if self.first_access < tiles.len() => {
                            let access_tile = tiles[self.first_access];
                            self.first_access += 1;
                            if access_tile == target {
                                self.result = Some(self.record_result(
                                    Some(candidate.index),
                                    Some(access_tile),
                                    PickKind::Reachable,
                                    None,
                                ));
                                self.stage = BankPickStage::Finish;
                            }
                        }
                        _ => {
                            self.first_order += 1;
                            self.first_access = 0;
                        }
                    }
                    if let Some(outcome) = self.consume_unit(ctx) {
                        return outcome;
                    }
                }
                BankPickStage::ResolveManyBest => {
                    let result_count = self
                        .pending_many_results
                        .as_ref()
                        .expect("non-reconstructing many search retains its bounded results")
                        .len();
                    if self.best_scan >= result_count {
                        self.pending_many_results = None;
                        if self.first_target.is_some() {
                            self.scan_order = 0;
                            self.scan_target = 0;
                            self.stage = BankPickStage::ResolveMany;
                        } else {
                            self.result = Some(self.record_result(
                                self.job.request.first(),
                                None,
                                PickKind::AirFallback,
                                None,
                            ));
                            self.stage = BankPickStage::Finish;
                        }
                        continue;
                    }
                    let ticks = self
                        .pending_many_results
                        .as_ref()
                        .expect("non-reconstructing many search retains its bounded results")
                        .get(self.best_scan)
                        .and_then(|result| result.as_ref().ok())
                        .map(|result| result.ticks);
                    if let Some(ticks) = ticks {
                        if ticks < self.best_cost {
                            self.best_cost = ticks;
                            self.first_target = Some(self.best_scan);
                        }
                    }
                    self.best_scan += 1;
                    if let Some(outcome) = self.consume_unit(ctx) {
                        return outcome;
                    }
                }
                BankPickStage::ResolveMany => {
                    let Some(target) = self.first_target else {
                        unreachable!("settled many-target outcome retains its target index");
                    };
                    if self.scan_order >= self.job.request.order.len() {
                        unreachable!("many-target index maps to a bank candidate");
                    }
                    let candidate = &self.job.request.order[self.scan_order];
                    self.scan_order += 1;
                    if self.job.request.facts.banks()[candidate.index].routable {
                        let flat = self.scan_target;
                        self.scan_target += 1;
                        if flat == target {
                            let route = if self.job.request.walk_generation.is_some() {
                                Some(
                                    self.pending_route
                                        .take()
                                        .expect("walk search reconstructs its selected route"),
                                )
                            } else {
                                self.pending_route = None;
                                None
                            };
                            let mut result = self.record_result(
                                Some(candidate.index),
                                None,
                                PickKind::Reachable,
                                route,
                            );
                            if result.route.is_some() {
                                result.nav_lease = self.lease.take();
                            }
                            self.result = Some(result);
                            self.stage = BankPickStage::Finish;
                        }
                    }
                    if let Some(outcome) = self.consume_unit(ctx) {
                        return outcome;
                    }
                }
                BankPickStage::Finish => {
                    if let Some(outcome) = self.checkpoint(ctx.checkpoint()) {
                        return outcome;
                    }
                    let _ = self
                        .result
                        .as_ref()
                        .expect("completed bank pick has a result");
                    ctx.publish_progress(BankPickPhase::Post, 1, Some(1));
                    if let Some(outcome) = self.checkpoint(ctx.checkpoint()) {
                        return outcome;
                    }
                    if self.timeout_fallback_due(Instant::now()) {
                        self.start_timeout_fallback();
                        continue;
                    }
                    self.search_preflights.primary = None;
                    self.search_preflights.fallback = None;
                    self.search_kind = None;
                    self.search_request = None;
                    self.target_buffer = None;
                    return StepOutcome::Completed(
                        self.result
                            .take()
                            .expect("completed bank pick retains its result"),
                    );
                }
            }
        }
    }
}

struct BankPickWorkerJob<J> {
    inner: J,
    thread_name: String,
    #[cfg(test)]
    gate: Option<BankPickStepGate>,
}

type AdmittedBankPickJob = BankPickWorkerJob<CooperativeBankPickJob>;

impl<J> BankPickWorkerJob<J> {
    fn new(name: &str, inner: J) -> Self {
        let mut thread_name = String::with_capacity("bank-pick-".len() + name.len());
        thread_name.push_str("bank-pick-");
        thread_name.push_str(name);
        Self {
            inner,
            thread_name,
            #[cfg(test)]
            gate: None,
        }
    }

    #[cfg(test)]
    fn gated(name: &str, inner: J, gate: BankPickStepGate) -> Self {
        let mut job = Self::new(name, inner);
        job.gate = Some(gate);
        job
    }
}

#[cfg(test)]
struct BankPickStepGate {
    entered: std::sync::mpsc::SyncSender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl<J> CooperativeJob for BankPickWorkerJob<J>
where
    J: CooperativeJob<Output = PickResult, Error = std::convert::Infallible, Phase = BankPickPhase>,
{
    type Output = PickResult;
    type Error = std::convert::Infallible;
    type Phase = BankPickPhase;

    fn request_thread(&self) -> Option<api::work::RequestThread<'_>> {
        Some(api::work::RequestThread::new(&self.thread_name))
    }

    fn retained(&self) -> RetainedState {
        self.inner.retained()
    }

    fn step(
        &mut self,
        context: &mut StepContext<'_, Self::Phase>,
    ) -> StepOutcome<Self::Output, Self::Error> {
        let outcome = self.inner.step(context);
        #[cfg(test)]
        if let Some(gate) = self.gate.take() {
            gate.entered
                .send(())
                .expect("bank-pick worker gate receiver remains alive");
            gate.release
                .recv_timeout(Duration::from_secs(4))
                .expect("bank-pick worker gate is released before its bound");
        }
        outcome
    }
}

impl BankPickWorkerJob<CooperativeBankPickJob> {
    fn for_job(
        name: &str,
        job: BankPickJob,
        input: BankPickInput,
        preflights: BankPickPreflights,
    ) -> Self {
        Self::new(name, CooperativeBankPickJob::new(job, input, preflights))
    }
}

impl BankPickRequest {
    fn single_search_request(&self) -> Option<SearchRequest> {
        let index = self.first()?;
        Some(SearchRequest::Single {
            from: self.from,
            to: self.facts.banks()[index].tile,
            options: self.opts,
            avoid: Box::default(),
            max_settles: NODE_BUDGET,
        })
    }

    fn single_search_shape(&self) -> Option<SearchRequestShape> {
        self.first()?;
        Some(SearchRequestShape::Single {
            options: self.opts,
            avoid_capacity: 0,
            max_settles: NODE_BUDGET,
        })
    }

    fn search_shape(&self, route_only: bool) -> Option<(BankPickSearchKind, SearchRequestShape)> {
        if route_only {
            return Some((BankPickSearchKind::Single, self.single_search_shape()?));
        }
        if self.native.is_some() {
            let mut options = self.opts;
            options.allow_teleports = false;
            return Some((
                BankPickSearchKind::First,
                SearchRequestShape::First {
                    target_capacity: self.target_count,
                    fallback_capacity: 0,
                    options,
                    avoid_capacity: 0,
                    max_settles: self.target_budget(),
                },
            ));
        }
        if self.target_count == 0 {
            return if self.walk_generation.is_some() {
                Some((BankPickSearchKind::Single, self.single_search_shape()?))
            } else {
                None
            };
        }
        Some((
            BankPickSearchKind::Many,
            SearchRequestShape::Many {
                target_capacity: self.target_count,
                options: self.opts,
                avoid_capacity: 0,
                max_settles: BANK_TARGET_BUDGET,
                reconstruct_best: self.walk_generation.is_some(),
            },
        ))
    }

    fn work_input(
        &self,
        revision: u64,
        route_only: bool,
        name: &str,
    ) -> Option<(BankPickInput, BankPickPreflights)> {
        let (kind, shape) = self.search_shape(route_only)?;
        let primary = SearchCursor::preflight(shape, &self.lease, &self.state)?;
        let fallback = if self.walk_generation.is_some() {
            let fallback_shape = self.single_search_shape()?;
            Some(SearchCursor::preflight(
                fallback_shape,
                &self.lease,
                &self.state,
            )?)
        } else {
            None
        };

        let mut base_input_bytes = std::mem::size_of::<Self>()
            .checked_add(std::mem::size_of::<ActivePick>())?
            .checked_add(std::mem::size_of::<AdmittedBankPickJob>())?
            .checked_add("bank-pick-".len().checked_add(name.len())?)?
            .checked_add(2 * std::mem::size_of::<usize>())?
            .checked_add(
                self.order
                    .capacity()
                    .checked_mul(std::mem::size_of::<BankCandidate>())?,
            )?
            .checked_add(self.state.try_shared_retained_bytes()?)?;
        for candidate in &self.order {
            if let Some((tiles, access)) = &candidate.access {
                base_input_bytes = base_input_bytes
                    .checked_add(
                        tiles
                            .capacity()
                            .checked_mul(std::mem::size_of::<WorldTile>())?,
                    )?
                    .checked_add(access.name.as_ref().map_or(0, |name| {
                        name.len().saturating_add(2 * std::mem::size_of::<usize>())
                    }))?
                    .checked_add(access.choose.as_ref().map_or(0, |choose| {
                        choose
                            .len()
                            .saturating_add(2 * std::mem::size_of::<usize>())
                    }))?;
            }
        }

        // The preflight peak includes the co-live search, backtrace, and
        // result heap. `result_bytes` is an output bound, not another charge.
        let phase_charge = |preflight: &SearchPreflight| -> Option<WorkCharge> {
            let reservation = preflight.reservation();
            let input_bytes = base_input_bytes
                .checked_add(preflight.input_bytes())?
                .checked_add(reservation.future_layout_bytes)?;
            Some(WorkCharge::new(input_bytes, reservation.peak_bytes, 0, 0))
        };
        let primary_charge = phase_charge(&primary)?;
        let fallback_charge = match fallback.as_ref() {
            Some(preflight) => Some(phase_charge(preflight)?),
            None => None,
        };
        let primary_total = primary_charge.total_bytes()?;
        let fallback_total = match fallback_charge {
            Some(charge) => charge.total_bytes()?,
            None => 0,
        };
        let charge = if fallback_total > primary_total {
            fallback_charge.expect("a larger fallback charge is present")
        } else {
            primary_charge
        };
        if charge.total_bytes()? > BANK_PICK_JOB_BYTES {
            return None;
        }
        Some((
            BankPickInput {
                revision: InputRevision::new(revision),
                charge,
                base_input_bytes,
            },
            BankPickPreflights {
                primary: Some(BankPickSearchPreflight {
                    kind,
                    token: primary,
                }),
                fallback,
            },
        ))
    }
}

impl BankPickJob {
    fn result(&self, index: Option<usize>, kind: PickKind) -> BankSelectionInput {
        BankSelectionInput {
            generation: self.generation,
            request_id: self.request.request_id,
            bank_index: index.map_or(-1, |i| i as i32),
            kind: kind as u8,
        }
    }
}
impl BankPickJob {
    fn pending_inline(self, scope: Scope, result: PickResult) -> PendingInlinePick {
        let posted = (self.request.walk_generation.is_none() && self.request.native.is_none())
            .then(|| self.result(result.index, result.kind));
        let native_receipt = self.request.native.as_ref().and_then(|native| {
            native.authority.live().then(|| {
                (
                    native.authority.clone(),
                    NativeBankPickReceipt {
                        request_id: native.authority.request_id().get(),
                        evidence: native.evidence,
                        selected: self.request.selected(
                            result.index,
                            result.kind,
                            result.access_tile,
                        ),
                    },
                )
            })
        });
        PendingInlinePick {
            generation: self.generation,
            request_id: self.request.request_id,
            scope,
            job: Some(self),
            ticket: None,
            result,
            posted,
            native_receipt,
        }
    }
}

impl BankPickRequest {
    fn allow_air_fallback(&self) -> bool {
        self.native
            .as_ref()
            .is_some_and(|native| native.allow_air_fallback)
    }

    fn target_budget(&self) -> usize {
        if self.allow_air_fallback() {
            BANK_TARGET_BUDGET
        } else {
            NODE_BUDGET
        }
    }

    fn first(&self) -> Option<usize> {
        self.order.first().map(|candidate| candidate.index)
    }

    fn route_key(
        &self,
        index: usize,
    ) -> (
        WorldTile,
        i32,
        bool,
        bool,
        bool,
        nav_planner::zones::ZoneExempt,
    ) {
        (
            self.facts.banks()[index].tile,
            0,
            self.opts.allow_teleports,
            self.opts.allow_wilderness,
            self.opts.allow_bank_fetch,
            self.opts.zones,
        )
    }

    fn route_target(&self, index: usize) -> WorldTile {
        self.order
            .iter()
            .find(|candidate| candidate.index == index)
            .and_then(|candidate| {
                candidate.access.as_ref().and_then(|(tiles, _)| {
                    // Timeout fallback remains candidate order, not a
                    // reachability claim. Normal native ranking routes all tiles.
                    tiles.iter().copied().min_by_key(|tile| {
                        (
                            *tile != self.facts.banks()[index].tile,
                            tile.level != self.from.level,
                            bank_air_distance(self.from, *tile),
                            tile.x,
                            tile.z,
                        )
                    })
                })
            })
            .unwrap_or(self.facts.banks()[index].tile)
    }

    fn selected(
        &self,
        index: Option<usize>,
        kind: PickKind,
        access_tile: Option<WorldTile>,
    ) -> SelectedBank {
        let Some(candidate) =
            index.and_then(|index| self.order.iter().find(|candidate| candidate.index == index))
        else {
            return native_no_candidate();
        };
        let index = candidate.index;
        let Ok(bank_index) = u16::try_from(index) else {
            return native_no_candidate();
        };
        if bank_index == u16::MAX {
            return native_no_candidate();
        }
        if kind == PickKind::AirFallback && self.allow_air_fallback() {
            return SelectedBank {
                bank_index,
                access_tile: self.facts.banks()[index].air_tile(),
                kind: NativePickKind::AirFallback,
                access: None,
            };
        }
        let Some((tiles, access)) = candidate.access.as_ref() else {
            return native_no_candidate();
        };
        let access_tile = match (kind, access_tile) {
            (PickKind::Reachable, Some(tile)) if tiles.contains(&tile) => tile,
            (PickKind::NearShortcut, None) if self.allow_air_fallback() => self.route_target(index),
            (PickKind::AirFallback, None) => self.route_target(index),
            _ => return native_no_candidate(),
        };
        SelectedBank {
            bank_index,
            access_tile,
            kind: match kind {
                PickKind::NearShortcut => NativePickKind::NearShortcut,
                PickKind::Reachable => NativePickKind::Reachable,
                PickKind::AirFallback => NativePickKind::AirFallback,
                PickKind::NoCandidate => NativePickKind::NoCandidate,
            },
            access: Some(Arc::new(access.clone())),
        }
    }
}

#[cfg(any(test, all(target_os = "linux", feature = "test-support")))]
impl BankPickRequest {
    fn legacy_fallback(&self, world: &NavWorld) -> LegacyPickResult {
        let index = self.first();
        let mut route = None;
        if self.walk_generation.is_some() {
            if let Some(index) = index {
                route = find_with(
                    &world.collision,
                    &world.graph,
                    self.from,
                    self.facts.banks()[index].tile,
                    self.opts,
                    &self.state,
                )
                .ok();
            }
        }
        LegacyPickResult {
            index,
            access_tile: None,
            kind: PickKind::AirFallback,
            route,
        }
    }

    fn legacy_calculate(&self, world: &NavWorld, route_only: bool) -> LegacyPickResult {
        if route_only {
            return self.legacy_fallback(world);
        }
        if self.native.is_some() {
            let targets: Vec<_> = self
                .order
                .iter()
                .filter_map(|candidate| candidate.access.as_ref())
                .flat_map(|(tiles, _)| tiles.iter().copied())
                .collect();
            let mut opts = self.opts;
            opts.allow_teleports = false;
            return match find_first_with(
                &world.collision,
                &world.graph,
                self.from,
                &targets,
                opts,
                &self.state,
            )
            .into_route()
            {
                Ok(route) => LegacyPickResult {
                    index: self.order.iter().find_map(|candidate| {
                        candidate.access.as_ref().and_then(|(tiles, _)| {
                            tiles.contains(&route.dest).then_some(candidate.index)
                        })
                    }),
                    access_tile: Some(route.dest),
                    kind: PickKind::Reachable,
                    route: None,
                },
                Err(_) => LegacyPickResult {
                    index: None,
                    access_tile: None,
                    kind: PickKind::NoCandidate,
                    route: None,
                },
            };
        }
        let routed: Vec<_> = self
            .order
            .iter()
            .filter(|candidate| self.facts.banks()[candidate.index].routable)
            .map(|candidate| candidate.index)
            .collect();
        if routed.is_empty() {
            return self.legacy_fallback(world);
        }
        let targets: Vec<_> = routed
            .iter()
            .map(|&index| self.route_target(index))
            .collect();
        let routes = find_many_with_avoid_bounded(
            &world.collision,
            &world.graph,
            self.from,
            &targets,
            self.opts,
            &self.state,
            &[],
            BANK_TARGET_BUDGET,
        );
        let mut best = None;
        let mut cost = f64::INFINITY;
        for (i, result) in routes.results().iter().enumerate() {
            if let Ok(result) = result {
                if result.ticks < cost {
                    cost = result.ticks;
                    best = Some(i);
                }
            }
        }
        if let Some(target) = best {
            let route = self.walk_generation.map(|_| {
                routes
                    .route(target)
                    .expect("a settled target has a reconstructible route")
            });
            return LegacyPickResult {
                index: Some(routed[target]),
                access_tile: None,
                kind: PickKind::Reachable,
                route,
            };
        }
        drop(routes);
        self.legacy_fallback(world)
    }
}

pub(super) fn native_no_candidate() -> SelectedBank {
    SelectedBank {
        bank_index: u16::MAX,
        access_tile: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        kind: NativePickKind::NoCandidate,
        access: None,
    }
}

fn bank_air_distance(from: WorldTile, to: WorldTile) -> i64 {
    let dx = i64::from(from.x) - i64::from(to.x);
    let dz = i64::from(from.z) - i64::from(to.z);
    dx * dx + dz * dz
}

fn near_bank(from: WorldTile, bank: WorldTile) -> bool {
    from.level == bank.level && from.x.abs_diff(bank.x).max(from.z.abs_diff(bank.z)) <= 4
}

fn native_bank_access(
    world: &NavWorld,
    bank: api::named_banks::NamedBank,
) -> Option<(Vec<WorldTile>, BankStandAccess)> {
    if let Some(definition) = bank.definition {
        if let Some(operation) = definition.open_first.or(definition.object) {
            return Some((
                vec![bank.tile],
                BankStandAccess {
                    bank,
                    // Catalog object tiles are walk stands; BankMachine resolves
                    // the live object row separately at that stand.
                    stand_tile: bank.tile,
                    kind: NativeAccessKind::Booth,
                    // Object operations are resolved against the live loc row
                    // by BankMachine immediately before dispatch.
                    stand_op: 0,
                    name: Some(Arc::from(operation.name)),
                    choose: definition.choose.map(Arc::from),
                },
            ));
        }
    }
    let definition_npc = bank.definition.and_then(|definition| definition.npc);
    let stand = world
        .banks()
        .iter()
        .filter(|stand| {
            stand.tile.level == bank.tile.level
                && (stand.tile.x - bank.tile.x)
                    .abs()
                    .max((stand.tile.z - bank.tile.z).abs())
                    <= SAME_BANK
                && definition_npc.is_none_or(|wanted| {
                    matches!(
                        &stand.access,
                        nav_planner::pack::BankAccess::Npc { name, .. }
                            if name.eq_ignore_ascii_case(wanted.name)
                    )
                })
        })
        .min_by_key(|stand| {
            let kind_order = match &stand.access {
                nav_planner::pack::BankAccess::Booth { .. } => 0,
                nav_planner::pack::BankAccess::Npc { .. } => 1,
            };
            (
                kind_order,
                bank_air_distance(bank.tile, stand.tile),
                stand.tile.x,
                stand.tile.z,
            )
        });
    let Some(stand) = stand else {
        let definition = bank.definition?;
        let npc = definition.npc?;
        // Declared teller access uses the shared live NPC opener. This is
        // an approach anchor, not fabricated packed stand geometry.
        return Some((
            vec![bank.tile],
            BankStandAccess {
                bank,
                stand_tile: definition.tile,
                kind: NativeAccessKind::Teller,
                stand_op: 0,
                name: Some(Arc::from(npc.name)),
                choose: definition.choose.map(Arc::from),
            },
        ));
    };
    let access_tiles: Vec<_> =
        nav_planner::bank_fetch::bank_access_tiles(&world.collision, stand).collect();
    if access_tiles.is_empty() {
        return None;
    }
    let (kind, stand_op, name, choose) = match &stand.access {
        nav_planner::pack::BankAccess::Booth { op } => (
            NativeAccessKind::Booth,
            *op,
            Some(Arc::from(stand.name.as_str())),
            None,
        ),
        nav_planner::pack::BankAccess::Npc { name, op, choose } => (
            NativeAccessKind::Teller,
            *op,
            Some(Arc::from(name.as_str())),
            choose.as_deref().map(Arc::from).or_else(|| {
                bank.definition
                    .and_then(|definition| definition.choose.map(Arc::from))
            }),
        ),
    };
    Some((
        access_tiles,
        BankStandAccess {
            bank,
            stand_tile: stand.tile,
            kind,
            stand_op,
            name,
            choose,
        },
    ))
}

fn bank_candidates(
    world: &NavWorld,
    facts: &NamedBankFacts,
    from: WorldTile,
    state: &WorldState,
    preferences: api::named_banks::BankPreferences,
    fishing_base: Option<i32>,
    native: Option<&NativeBankPickRequest>,
) -> BankCandidates {
    let mut target_count = 0usize;
    let mut order = Vec::new();
    if let Some(native) = native {
        for &candidate in native.eligible.iter() {
            let index = usize::from(candidate);
            if index >= facts.banks().len()
                || native
                    .explicit_bank
                    .is_some_and(|explicit| usize::from(explicit) != index)
                || (!facts.banks()[index].routable && !native.allow_air_fallback)
            {
                continue;
            }
            let access = facts.banks()[index]
                .routable
                .then(|| native_bank_access(world, facts.banks()[index]))
                .flatten();
            if let Some((tiles, _)) = &access {
                target_count = target_count.saturating_add(tiles.len());
            }
            if access.is_some() || native.allow_air_fallback {
                order.push(BankCandidate { index, access });
            }
        }
    } else {
        for index in 0..facts.banks().len() {
            if !facts.banks()[index].eligible(
                |id| if id == 10 { fishing_base } else { None },
                |quest| state.quests.contains(quest),
                preferences,
            ) {
                continue;
            }
            if facts.banks()[index].routable {
                target_count = target_count.saturating_add(1);
            }
            order.push(BankCandidate {
                index,
                access: None,
            });
        }
    }
    if !native.is_some_and(|native| native.allow_air_fallback) {
        order.sort_by_key(|candidate| {
            bank_air_distance(from, facts.banks()[candidate.index].air_tile())
        });
    }
    BankCandidates {
        order,
        target_count,
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "Each argument is one `BankPickRequest` field. The constructor adds the timeout flag and the state `Arc` for production, harness and tests."
)]
fn make_bank_pick_request(
    request_id: u64,
    lease: nav_planner::contract::NavLease,
    facts: Arc<NamedBankFacts>,
    from: WorldTile,
    opts: FindOptions,
    state: WorldState,
    order: Vec<BankCandidate>,
    target_count: usize,
    walk_generation: Option<u64>,
    native: Option<NativePickContext>,
) -> Arc<BankPickRequest> {
    Arc::new(BankPickRequest {
        request_id,
        lease,
        facts,
        from,
        opts,
        state: Arc::new(state),
        timeout_fallback: AtomicBool::new(false),
        order,
        target_count,
        walk_generation,
        native,
    })
}

pub(super) fn queue_native_bank_pick_scoped(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    world: &Option<Arc<NavWorld>>,
    state: Option<&WorldState>,
    input: NativeBankPickInput,
    session_permissions: Option<script::native::WalkPermissions>,
) {
    let from = input.request.from;
    let preferences = input.request.preferences;
    let request_id = input.authority.request_id().get();
    queue_bank(
        navs,
        name,
        scope,
        world,
        state,
        from,
        super::walk_permissions::native_admission(
            navs,
            name,
            script::native::WalkOptions {
                allow_teleports: script::native::WalkBit::Forbid,
                ..input.request.options
            },
            session_permissions,
        )
        .0,
        request_id,
        preferences,
        None,
        false,
        Some(input),
    );
}

/// The raw native walk-nearest-bank verb keeps native option defaults.
pub(super) fn queue_bank_walk_scoped(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    world: &Option<Arc<NavWorld>>,
    state: Option<&WorldState>,
    from: WorldTile,
    fishing_base: Option<i32>,
) -> bool {
    queue_bank(
        navs,
        name,
        scope,
        world,
        state,
        from,
        FindOptions::default(),
        0,
        BankPreferences::default(),
        fishing_base,
        true,
        None,
    )
}

/// `state` borrows the observe frame's gating facts. The request copies them
/// once, into its own shared backing, only after the admission gates pass, so
/// it keeps that frame's facts while it waits.
#[expect(
    clippy::too_many_arguments,
    reason = "one admission path serves native selection, compatibility selection and bank walking"
)]
fn queue_bank(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    world: &Option<Arc<NavWorld>>,
    state: Option<&WorldState>,
    from: WorldTile,
    mut opts: FindOptions,
    request_id: u64,
    preferences: BankPreferences,
    fishing_base: Option<i32>,
    walking: bool,
    native: Option<NativeBankPickInput>,
) -> bool {
    let queued_at = Instant::now();
    drain_bank_pick_cancellation(navs, name);
    drain_bank_pick_retired_owner(navs, name);
    let lease = match world.as_ref() {
        Some(world) => match nav_planner::producer::lease(world) {
            Ok(lease) => Some(lease),
            Err(error) => {
                api::host_log!(
                    api::hostlog::Category::NavEvent,
                    api::hostlog::Level::Error,
                    slot = name,
                    "bank pick navigation lease refused: {error}"
                );
                return false;
            }
        },
        None => None,
    };
    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
    let bot = all.entry(name.to_string()).or_default();
    if walking {
        if bot.bank_fetch.is_some() {
            return false;
        }
        if let Some(active) = &bot.bank_pick.current {
            match active.job.request.walk_generation {
                None => return false,
                Some(generation) if generation == bot.route_generation => return true,
                Some(_) => {}
            }
        }
    } else if native.is_none() && (request_id == 0 || request_id <= bot.bank_pick.last_id) {
        return false;
    }

    let pick = &mut bot.bank_pick;
    let Some(generation) = pick.advance_generation() else {
        api::host_log!(
            api::hostlog::Category::NavEvent,
            api::hostlog::Level::Error,
            slot = name,
            "bank pick generation exhausted"
        );
        return false;
    };
    pick.cancel_active(Cancellation::Superseded);
    pick.pending_inline = None;
    if let Some(owner) = pick.owner_result.take() {
        debug_assert!(pick.retired_owner.is_none());
        pick.retired_owner = Some(owner);
    }
    if !walking && native.is_none() {
        pick.last_id = request_id;
    }
    pick.native_receipt = None;
    let facts = native
        .as_ref()
        .map(|native| Arc::clone(&native.request.facts))
        .or_else(|| {
            world
                .as_ref()
                .and_then(|world| world.named_bank_facts().cloned())
        });
    #[cfg(test)]
    let facts = if native.is_none() {
        pick.facts.clone().or(facts)
    } else {
        facts
    };
    let (Some(world), Some(facts)) = (world.as_ref(), facts) else {
        let native_receipt = native.as_ref().and_then(|native| {
            native.authority.live().then(|| {
                (
                    native.authority.clone(),
                    NativeBankPickReceipt {
                        request_id,
                        evidence: native.evidence,
                        selected: native_no_candidate(),
                    },
                )
            })
        });
        let posted = (native.is_none() && !walking).then_some(BankSelectionInput {
            request_id,
            generation,
            kind: PickKind::NoCandidate as u8,
            ..Default::default()
        });
        pick.pending_inline = Some(Box::new(PendingInlinePick {
            generation,
            request_id,
            scope,
            job: None,
            ticket: None,
            result: PickResult::plain(None, None, PickKind::NoCandidate),
            posted,
            native_receipt,
        }));
        drop(all);
        poll_bank_pick(navs, name, queued_at);
        return false;
    };
    let Some(lease) = lease else {
        unreachable!("a present NavWorld has an owner lease");
    };
    let empty = WorldState::default();
    let state = state.unwrap_or(&empty);
    let from = native.as_ref().map_or(from, |native| native.request.from);
    let BankCandidates {
        order,
        target_count,
    } = bank_candidates(
        world.as_ref(),
        facts.as_ref(),
        from,
        state,
        preferences,
        fishing_base,
        native.as_ref().map(|native| &native.request),
    );
    opts.essence = bot.traveller.essence();
    let first = order.first().map(|candidate| candidate.index);
    let generation = pick.generation;
    let walk_generation = if walking && first.is_some() {
        bot.route_generation = bot.route_generation.wrapping_add(1);
        bot.walk_request_id = 0;
        bot.supersede_route_work();
        bot.pending_route = None;
        Some(bot.route_generation)
    } else {
        None
    };
    let native = native.map(|native| NativePickContext {
        authority: native.authority,
        evidence: native.evidence,
        allow_air_fallback: native.request.allow_air_fallback,
    });
    let request = make_bank_pick_request(
        request_id,
        lease,
        facts,
        from,
        opts,
        state.clone(),
        order,
        target_count,
        walk_generation,
        native,
    );
    let near_index = if request.allow_air_fallback() {
        request
            .order
            .iter()
            .find(|candidate| candidate.access.is_some())
            .map(|candidate| candidate.index)
    } else {
        first
    };
    let near = (request.native.is_none() || request.allow_air_fallback())
        && near_index.is_some_and(|index| near_bank(from, request.route_target(index)));
    let deadline = (!near).then_some(queued_at + BANK_SELECTION_WINDOW);
    let job = BankPickJob {
        generation,
        request,
        deadline,
    };
    let native_without_targets =
        job.request.native.is_some() && !job.request.allow_air_fallback() && target_count == 0;
    let inline_kind = if first.is_none() || native_without_targets {
        Some(PickKind::NoCandidate)
    } else if near && !walking {
        Some(PickKind::NearShortcut)
    } else if target_count == 0 && !walking {
        Some(PickKind::AirFallback)
    } else {
        None
    };
    if let Some(kind) = inline_kind {
        let index = match kind {
            PickKind::NoCandidate => None,
            PickKind::NearShortcut => near_index,
            _ => first,
        };
        bot.bank_pick.pending_inline = Some(Box::new(
            job.pending_inline(scope, PickResult::plain(index, None, kind)),
        ));
        drop(all);
        poll_bank_pick(navs, name, queued_at);
        return first.is_some();
    }
    if walking {
        bot.requested_route = first.map(|index| job.request.route_key(index));
    }
    let Some((input, preflights)) = job.request.work_input(generation, near, name) else {
        api::host_log!(
            api::hostlog::Category::NavEvent,
            api::hostlog::Level::Warn,
            slot = name,
            "bank pick request exceeds the bounded work charge"
        );
        bot.bank_pick.pending_inline = Some(Box::new(
            job.pending_inline(scope, PickResult::plain(None, None, PickKind::NoCandidate)),
        ));
        drop(all);
        poll_bank_pick(navs, name, queued_at);
        return false;
    };
    bot.bank_pick.current = Some(ActivePick {
        job: job.clone(),
        scope,
        deadline,
        ticket: None,
        cancellation: None,
        pending_job: Some(Box::new(BankPickWorkerJob::for_job(
            name, job, input, preflights,
        ))),
        submitting: false,
        capacity_sequence: None,
        owner_result_full: false,
        #[cfg(any(test, all(feature = "live-harness", feature = "test-support")))]
        last_submission_refusal: None,
    });
    drop(all);
    submit_pending_bank_pick(navs, name, scope, true)
}

pub(super) fn drain_bank_pick_cancellation(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
) {
    let pending = navs
        .lock()
        .expect("navigation state mutex is not poisoned")
        .get_mut(name)
        .and_then(|bot| bot.bank_pick.pending_cancel.take());
    if let Some((mut cancellation, reason)) = pending {
        let _ = cancellation.abandon(reason);
    }
}

pub(super) fn drain_bank_pick_retired_owner(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
) {
    let retired = navs
        .lock()
        .expect("navigation state mutex is not poisoned")
        .get_mut(name)
        .and_then(|bot| bot.bank_pick.retired_owner.take());
    drop(retired);
}

fn submit_pending_bank_pick(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    force: bool,
) -> bool {
    let Ok(shared) = WorkService::shared() else {
        return false;
    };
    submit_pending_bank_pick_with_service(navs, name, scope, force, &shared)
}

fn submit_pending_bank_pick_with_service(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    force: bool,
    shared: &Arc<parking_lot::Mutex<WorkService>>,
) -> bool {
    drain_bank_pick_cancellation(navs, name);
    drain_bank_pick_retired_owner(navs, name);
    let observed_capacity = shared.lock().publications().capacity().sequence();
    let Some((generation, request_id, job)) = ({
        let mut all = navs.lock().expect("navigation state mutex is not poisoned");
        let Some(bot) = all.get_mut(name) else {
            return false;
        };
        let Some(active) = bot.bank_pick.current.as_mut() else {
            return false;
        };
        if active.scope != scope
            || active.ticket.is_some()
            || active.submitting
            || active.pending_job.is_none()
            || (!force && active.capacity_sequence == Some(observed_capacity))
        {
            return false;
        }
        active.submitting = true;
        Some((
            active.job.generation,
            active.job.request.request_id,
            active.pending_job.take().expect("checked pending job"),
        ))
    }) else {
        return false;
    };

    enum Attempt {
        Accepted(WorkTicket<BankPickSpec>, WorkCancellation<BankPickSpec>),
        Refused(SubmissionRefusal, Box<AdmittedBankPickJob>, PublishSequence),
        IdentityFailed(Box<AdmittedBankPickJob>, String),
    }
    let attempt = {
        let mut service = shared.lock();
        let before = service.publications().capacity().sequence();
        match service.mint_identity(scope, None) {
            Ok(identity) => {
                let conditions = ProcessConditions::new(service.incarnation());
                let input = job.inner.input;
                match service.try_submit::<BankPickSpec, _>(identity, &input, *job, &conditions) {
                    Ok(ticket) => {
                        let cancellation = service
                            .cancellation_handle(ticket)
                            .expect("a freshly accepted bank-pick ticket has cancellation custody");
                        Attempt::Accepted(ticket, cancellation)
                    }
                    Err(refused) => {
                        let (reason, job) = refused.into_parts();
                        Attempt::Refused(reason, Box::new(job), before)
                    }
                }
            }
            Err(error) => Attempt::IdentityFailed(job, error.to_string()),
        }
    };

    let mut accepted_to_abandon: Option<WorkCancellation<BankPickSpec>> = None;
    let mut submission_ok = false;
    {
        let mut all = navs.lock().expect("navigation state mutex is not poisoned");
        if let Some(bot) = all.get_mut(name) {
            let pick = &mut bot.bank_pick;
            let current = pick.current.as_mut().filter(|active| {
                active.job.generation == generation
                    && active.job.request.request_id == request_id
                    && active.scope == scope
                    && active.submitting
            });
            match attempt {
                Attempt::Accepted(ticket, cancellation) => {
                    if let Some(active) = current {
                        active.ticket = Some(BankPickTicketRef::from_ticket(ticket));
                        active.cancellation = Some(cancellation);
                        active.submitting = false;
                        active.capacity_sequence = None;
                        #[cfg(any(test, all(feature = "live-harness", feature = "test-support")))]
                        {
                            active.last_submission_refusal = None;
                        }
                        submission_ok = true;
                    } else {
                        accepted_to_abandon = Some(cancellation);
                    }
                }
                Attempt::Refused(reason, job, before) => {
                    if let Some(active) = current {
                        if reason == SubmissionRefusal::Full {
                            active.pending_job = Some(job);
                            active.submitting = false;
                            active.capacity_sequence = Some(before);
                            #[cfg(any(
                                test,
                                all(feature = "live-harness", feature = "test-support")
                            ))]
                            {
                                active.last_submission_refusal = Some(reason);
                            }
                            submission_ok = true;
                        } else {
                            let job = active.job.clone();
                            pick.current = None;
                            pick.pending_inline = Some(Box::new(job.pending_inline(
                                scope,
                                PickResult::plain(None, None, PickKind::NoCandidate),
                            )));
                            api::host_log!(
                                api::hostlog::Category::NavEvent,
                                api::hostlog::Level::Warn,
                                slot = name,
                                "bank pick work admission refused: {reason:?}"
                            );
                        }
                    }
                }
                Attempt::IdentityFailed(_job, error) => {
                    if let Some(active) = current {
                        let job = active.job.clone();
                        pick.current = None;
                        pick.pending_inline = Some(Box::new(job.pending_inline(
                            scope,
                            PickResult::plain(None, None, PickKind::NoCandidate),
                        )));
                        api::host_log!(
                            api::hostlog::Category::NavEvent,
                            api::hostlog::Level::Warn,
                            slot = name,
                            "bank pick work identity refused: {error}"
                        );
                    }
                }
            }
        } else if let Attempt::Accepted(_, cancellation) = attempt {
            accepted_to_abandon = Some(cancellation);
        }
    }
    if let Some(mut cancellation) = accepted_to_abandon {
        let _ = cancellation.abandon(Cancellation::ScopeInvalidated);
    }
    drain_bank_pick_cancellation(navs, name);
    submission_ok
}

enum BankPickTerminal {
    Pending,
    Completed(usize),
    Failed,
    Cancelled,
    InvalidResultSize,
    LedgerError(api::work::LedgerError),
}

fn active_ticket_matches(
    active: &ActivePick,
    scope: Scope,
    generation: u64,
    request_id: u64,
    ticket: WorkTicket<BankPickSpec>,
) -> bool {
    active.scope == scope
        && active.job.generation == generation
        && active.job.request.request_id == request_id
        && active.ticket.map(BankPickTicketRef::ticket) == Some(ticket)
}

fn restore_pending_inline(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    pending: Box<PendingInlinePick>,
) {
    let mut pending = Some(pending);
    {
        let mut all = navs.lock().expect("navigation state mutex is not poisoned");
        if let Some(bot) = all.get_mut(name) {
            let pick = &mut bot.bank_pick;
            if pick.generation
                == pending
                    .as_ref()
                    .expect("the pending result is not moved yet")
                    .generation
                && pick.pending_inline.is_none()
            {
                pick.pending_inline = pending.take();
            }
        }
    }
    drop(pending);
}

fn pending_owner_result(
    pending: PendingInlinePick,
    reservation: nav_planner::producer::OwnerResultReservation,
) -> (
    PickCommit,
    Arc<nav_planner::producer::OwnerResult<PickResult>>,
) {
    let PendingInlinePick {
        generation,
        request_id,
        scope,
        job,
        ticket,
        result,
        posted,
        native_receipt,
    } = pending;
    let owner = match ticket {
        Some(ticket) => {
            let ticket = ticket.ticket();
            result.into_owner_result(ticket.identity(), ticket.revision(), reservation)
        }
        None => reservation.hold(result),
    };
    (
        PickCommit {
            generation,
            request_id,
            scope,
            job,
            ticket,
            posted,
            native_receipt,
        },
        owner,
    )
}

fn set_active_owner_result_full(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    generation: u64,
    request_id: u64,
    ticket: WorkTicket<BankPickSpec>,
    full: bool,
) -> bool {
    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
    let Some(active) = all
        .get_mut(name)
        .and_then(|bot| bot.bank_pick.current.as_mut())
        .filter(|active| active_ticket_matches(active, scope, generation, request_id, ticket))
    else {
        return false;
    };
    let first_refusal = full && !active.owner_result_full;
    active.owner_result_full = full;
    first_refusal
}

fn bank_pick_result_within_bound(bytes: usize) -> bool {
    bytes <= nav_planner::producer::OWNER_RESULT_MAX_BYTES
}

fn bank_pick_result_bytes(result: &PickResult) -> Option<usize> {
    result
        .owner_result_bytes()
        .filter(|&bytes| bank_pick_result_within_bound(bytes))
}

fn bank_pick_terminal(
    ticket: WorkTicket<BankPickSpec>,
    shared: &Arc<parking_lot::Mutex<WorkService>>,
) -> Option<BankPickTerminal> {
    let service = shared.lock();
    Some(match service.borrow_terminal(&ticket) {
        Ok(None) => BankPickTerminal::Pending,
        Ok(Some(Settlement::Completed(result))) => bank_pick_result_bytes(result).map_or(
            BankPickTerminal::InvalidResultSize,
            BankPickTerminal::Completed,
        ),
        Ok(Some(Settlement::Failed(_))) => BankPickTerminal::Failed,
        Ok(Some(Settlement::Cancelled(_))) => BankPickTerminal::Cancelled,
        Err(error) => BankPickTerminal::LedgerError(error),
    })
}

fn publish_pending_inline(navs: &Arc<Mutex<HashMap<String, super::NavBot>>>, name: &str) {
    let pending = navs
        .lock()
        .expect("navigation state mutex is not poisoned")
        .get_mut(name)
        .and_then(|bot| bot.bank_pick.pending_inline.take());
    let Some(pending) = pending else {
        return;
    };
    let Some(bytes) = pending.result.owner_result_bytes() else {
        restore_pending_inline(navs, name, pending);
        return;
    };
    match nav_planner::producer::OwnerResultLedger::shared().try_reserve_bytes(bytes) {
        Ok(reservation) => {
            let (commit, owner) = pending_owner_result(*pending, reservation);
            apply_bank_pick_result(navs, name, commit, owner);
        }
        Err(nav_planner::producer::OwnerResultRefusal::Full { .. })
        | Err(nav_planner::producer::OwnerResultRefusal::TooLarge { .. })
        | Err(nav_planner::producer::OwnerResultRefusal::SizeOverflow) => {
            restore_pending_inline(navs, name, pending);
        }
    }
}

fn expire_bank_pick_deadline(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    now: Instant,
) {
    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
    let Some(bot) = all.get_mut(name) else {
        return;
    };
    let pick = &mut bot.bank_pick;
    let Some(active) = pick.current.as_mut() else {
        return;
    };
    if active.deadline.is_some_and(|deadline| now >= deadline) {
        active.deadline = None;
        active
            .job
            .request
            .timeout_fallback
            .store(true, Ordering::Release);
        if active.ticket.is_none() && active.job.request.walk_generation.is_none() {
            let job = active.job.clone();
            let scope = active.scope;
            let index = job.request.first();
            let kind = if index.is_none()
                || (job.request.native.is_some()
                    && !job.request.allow_air_fallback()
                    && job.request.target_count == 0)
            {
                PickKind::NoCandidate
            } else {
                PickKind::AirFallback
            };
            pick.cancel_active(Cancellation::Superseded);
            pick.pending_inline = Some(Box::new(
                job.pending_inline(scope, PickResult::plain(index, None, kind)),
            ));
        }
    }
}

fn apply_bank_pick_result(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    commit: PickCommit,
    owner: Arc<nav_planner::producer::OwnerResult<PickResult>>,
) {
    drain_bank_pick_retired_owner(navs, name);
    let mut completed_active = None;
    let mut retired_owner = None;
    let mut route_rejected = false;
    {
        let mut all = navs.lock().expect("navigation state mutex is not poisoned");
        let Some(bot) = all.get_mut(name) else {
            return;
        };
        let current = {
            let pick = &bot.bank_pick;
            if pick.generation != commit.generation {
                return;
            }
            match commit.ticket {
                Some(ticket) => pick.current.as_ref().is_some_and(|active| {
                    active_ticket_matches(
                        active,
                        commit.scope,
                        commit.generation,
                        commit.request_id,
                        ticket.ticket(),
                    )
                }),
                None => pick.current.is_none(),
            }
        };
        if !current {
            return;
        }
        if let Some(job) = commit
            .job
            .as_ref()
            .filter(|job| job.request.walk_generation.is_some())
        {
            let route_generation = job
                .request
                .walk_generation
                .expect("the job was filtered to walking requests above");
            let route_is_current = route_generation == bot.route_generation
                && bot.walk_request_id == 0
                && bot.requested_route
                    == job
                        .request
                        .first()
                        .map(|index| job.request.route_key(index));
            if !route_is_current {
                route_rejected = true;
            } else {
                let applied = if let Some(plan) = owner.value().plan.as_ref() {
                    let Some(ticket_ref) = commit.ticket else {
                        return;
                    };
                    let ticket = ticket_ref.ticket();
                    let custody: Arc<dyn std::any::Any + Send + Sync> = owner.clone();
                    let route = crate::walk_plan::RoutePath::compact(
                        plan.lease.clone(),
                        job.request.lease.clone(),
                        Arc::clone(&custody),
                    );
                    bot.publish_compact_route(
                        route_generation,
                        0,
                        ticket.identity(),
                        ticket.revision(),
                        job.request.opts.allow_teleports,
                        route,
                        None,
                        custody,
                        job.request.state.quest_evidence.clone(),
                    )
                } else {
                    bot.publish_route(
                        route_generation,
                        0,
                        job.request.opts.allow_teleports,
                        super::RouteOutcome::NoPath,
                    );
                    true
                };
                route_rejected = !applied;
            }
        }
        let pick = &mut bot.bank_pick;
        if let Some(ticket_ref) = commit.ticket {
            let ticket = ticket_ref.ticket();
            let matches = pick.current.as_ref().is_some_and(|active| {
                active_ticket_matches(
                    active,
                    commit.scope,
                    commit.generation,
                    commit.request_id,
                    ticket,
                )
            });
            if matches {
                completed_active = pick.current.take();
            } else {
                return;
            }
        }
        if !route_rejected {
            retired_owner = pick.owner_result.replace(Arc::clone(&owner));
            if let Some(posted) = commit.posted {
                pick.posted = posted;
            }
            if let Some(native_receipt) = commit.native_receipt {
                pick.native_receipt = Some(native_receipt);
            }
        }
    }
    drop(completed_active);
    drop(retired_owner);
    if route_rejected {
        drop(owner);
        return;
    }
    drain_bank_pick_cancellation(navs, name);
}

fn active_bank_pick_is_current(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    generation: u64,
    request_id: u64,
    ticket: WorkTicket<BankPickSpec>,
) -> bool {
    navs.lock()
        .expect("navigation state mutex is not poisoned")
        .get(name)
        .and_then(|bot| bot.bank_pick.current.as_ref())
        .is_some_and(|active| active_ticket_matches(active, scope, generation, request_id, ticket))
}

fn reserve_bank_pick_result(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    job: &BankPickJob,
    ticket: WorkTicket<BankPickSpec>,
    bytes: usize,
    ledger: &nav_planner::producer::OwnerResultLedger,
) -> Option<nav_planner::producer::OwnerResultReservation> {
    let generation = job.generation;
    let request_id = job.request.request_id;
    if !active_bank_pick_is_current(navs, name, scope, generation, request_id, ticket) {
        return None;
    }
    match ledger.try_reserve_bytes(bytes) {
        Ok(reservation) => {
            set_active_owner_result_full(navs, name, scope, generation, request_id, ticket, false);
            Some(reservation)
        }
        Err(nav_planner::producer::OwnerResultRefusal::Full { .. }) => {
            set_active_owner_result_full(navs, name, scope, generation, request_id, ticket, true);
            None
        }
        Err(nav_planner::producer::OwnerResultRefusal::TooLarge { .. })
        | Err(nav_planner::producer::OwnerResultRefusal::SizeOverflow) => {
            if retire_active_bank_pick(navs, name, scope, generation, request_id, ticket) {
                api::host_log!(
                    api::hostlog::Category::NavEvent,
                    api::hostlog::Level::Error,
                    slot = name,
                    "bank pick result exceeds its per-result bound or overflows size accounting"
                );
            }
            None
        }
    }
}

fn retire_active_bank_pick(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    generation: u64,
    request_id: u64,
    ticket: WorkTicket<BankPickSpec>,
) -> bool {
    let retired = {
        let mut all = navs.lock().expect("navigation state mutex is not poisoned");
        let Some(bot) = all.get_mut(name) else {
            return false;
        };
        if bot.bank_pick.current.as_ref().is_some_and(|active| {
            active_ticket_matches(active, scope, generation, request_id, ticket)
        }) {
            bot.bank_pick.current.take()
        } else {
            None
        }
    };
    let removed = retired.is_some();
    drop(retired);
    removed
}

fn publish_work_pick_result(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    job: BankPickJob,
    ticket: WorkTicket<BankPickSpec>,
    result: PickResult,
    reservation: nav_planner::producer::OwnerResultReservation,
) {
    let mut pending = job.pending_inline(scope, result);
    pending.ticket = Some(BankPickTicketRef::from_ticket(ticket));
    let (commit, owner) = pending_owner_result(pending, reservation);
    apply_bank_pick_result(navs, name, commit, owner);
}

fn publish_failed_bank_pick(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    job: BankPickJob,
    ticket: WorkTicket<BankPickSpec>,
    ledger: &nav_planner::producer::OwnerResultLedger,
) {
    let result = PickResult::plain(None, None, PickKind::NoCandidate);
    let Some(bytes) = result.owner_result_bytes() else {
        return;
    };
    let Some(reservation) =
        reserve_bank_pick_result(navs, name, scope, &job, ticket, bytes, ledger)
    else {
        return;
    };
    publish_work_pick_result(navs, name, scope, job, ticket, result, reservation);
}

fn consume_failed_bank_pick(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    job: BankPickJob,
    ticket: WorkTicket<BankPickSpec>,
    shared: &Arc<parking_lot::Mutex<WorkService>>,
    ledger: &nav_planner::producer::OwnerResultLedger,
) {
    let result = PickResult::plain(None, None, PickKind::NoCandidate);
    let Some(bytes) = result.owner_result_bytes() else {
        return;
    };
    let Some(reservation) =
        reserve_bank_pick_result(navs, name, scope, &job, ticket, bytes, ledger)
    else {
        return;
    };
    let settlement = {
        let mut service = shared.lock();
        service.consume(&ticket)
    };
    match settlement {
        Ok(Settlement::Failed(_)) | Ok(Settlement::Cancelled(_)) => {
            publish_work_pick_result(navs, name, scope, job, ticket, result, reservation);
        }
        Ok(Settlement::Completed(_)) => {
            drop(reservation);
            api::host_log!(
                api::hostlog::Category::NavEvent,
                api::hostlog::Level::Error,
                slot = name,
                "bank pick terminal changed after failure observation"
            );
            let _ = retire_active_bank_pick(
                navs,
                name,
                scope,
                job.generation,
                job.request.request_id,
                ticket,
            );
        }
        Err(error) => {
            drop(reservation);
            api::host_log!(
                api::hostlog::Category::NavEvent,
                api::hostlog::Level::Warn,
                slot = name,
                "bank pick terminal could not be consumed: {error:?}"
            );
            let _ = retire_active_bank_pick(
                navs,
                name,
                scope,
                job.generation,
                job.request.request_id,
                ticket,
            );
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Consumes one completed pick for one slot: the job, its ticket and result size, the work service and the ledger that tests replace."
)]
fn consume_completed_bank_pick(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    scope: Scope,
    job: BankPickJob,
    ticket: WorkTicket<BankPickSpec>,
    bytes: usize,
    shared: &Arc<parking_lot::Mutex<WorkService>>,
    ledger: &nav_planner::producer::OwnerResultLedger,
) {
    let Some(reservation) =
        reserve_bank_pick_result(navs, name, scope, &job, ticket, bytes, ledger)
    else {
        return;
    };
    let settlement = {
        let mut service = shared.lock();
        service.consume(&ticket)
    };
    match settlement {
        Ok(Settlement::Completed(result)) if result.owner_result_bytes() == Some(bytes) => {
            publish_work_pick_result(navs, name, scope, job, ticket, result, reservation);
        }
        Ok(Settlement::Completed(_)) => {
            drop(reservation);
            api::host_log!(
                api::hostlog::Category::NavEvent,
                api::hostlog::Level::Error,
                slot = name,
                "bank pick result size changed between borrow and consume"
            );
            let _ = retire_active_bank_pick(
                navs,
                name,
                scope,
                job.generation,
                job.request.request_id,
                ticket,
            );
        }
        Ok(Settlement::Failed(_)) | Ok(Settlement::Cancelled(_)) => {
            drop(reservation);
            publish_failed_bank_pick(navs, name, scope, job, ticket, ledger);
        }
        Err(error) => {
            drop(reservation);
            api::host_log!(
                api::hostlog::Category::NavEvent,
                api::hostlog::Level::Warn,
                slot = name,
                "bank pick terminal could not be consumed: {error:?}"
            );
            let _ = retire_active_bank_pick(
                navs,
                name,
                scope,
                job.generation,
                job.request.request_id,
                ticket,
            );
        }
    }
}

pub(super) fn poll_bank_pick(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    now: Instant,
) -> BankSelectionInput {
    let shared = WorkService::shared().ok();
    poll_bank_pick_inner(navs, name, now, shared.as_ref())
}

#[cfg(test)]
fn poll_bank_pick_with_service(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    now: Instant,
    shared: &Arc<parking_lot::Mutex<WorkService>>,
) -> BankSelectionInput {
    poll_bank_pick_inner(navs, name, now, Some(shared))
}

fn poll_bank_pick_inner(
    navs: &Arc<Mutex<HashMap<String, super::NavBot>>>,
    name: &str,
    now: Instant,
    shared: Option<&Arc<parking_lot::Mutex<WorkService>>>,
) -> BankSelectionInput {
    drain_bank_pick_cancellation(navs, name);
    drain_bank_pick_retired_owner(navs, name);
    expire_bank_pick_deadline(navs, name, now);
    drain_bank_pick_cancellation(navs, name);
    let scope = navs
        .lock()
        .expect("navigation state mutex is not poisoned")
        .get(name)
        .and_then(|bot| bot.bank_pick.current.as_ref())
        .map(|active| active.scope);
    if let (Some(scope), Some(shared)) = (scope, shared) {
        let _ = submit_pending_bank_pick_with_service(navs, name, scope, false, shared);
    }
    drain_bank_pick_cancellation(navs, name);
    publish_pending_inline(navs, name);
    let active = navs
        .lock()
        .expect("navigation state mutex is not poisoned")
        .get(name)
        .and_then(|bot| bot.bank_pick.current.as_ref())
        .map(|active| {
            (
                active.scope,
                active.job.clone(),
                active.ticket.map(BankPickTicketRef::ticket),
            )
        });
    if let (Some((scope, job, Some(ticket))), Some(shared)) = (active, shared) {
        match bank_pick_terminal(ticket, shared) {
            Some(BankPickTerminal::Pending) | None => {}
            Some(BankPickTerminal::Completed(bytes)) => {
                consume_completed_bank_pick(
                    navs,
                    name,
                    scope,
                    job,
                    ticket,
                    bytes,
                    shared,
                    nav_planner::producer::OwnerResultLedger::shared(),
                );
            }
            Some(BankPickTerminal::Failed | BankPickTerminal::Cancelled) => {
                consume_failed_bank_pick(
                    navs,
                    name,
                    scope,
                    job,
                    ticket,
                    shared,
                    nav_planner::producer::OwnerResultLedger::shared(),
                );
            }
            Some(BankPickTerminal::InvalidResultSize) => {
                if retire_active_bank_pick(
                    navs,
                    name,
                    scope,
                    job.generation,
                    job.request.request_id,
                    ticket,
                ) {
                    api::host_log!(
                        api::hostlog::Category::NavEvent,
                        api::hostlog::Level::Error,
                        slot = name,
                        "bank pick result exceeds its per-result bound or overflows size accounting"
                    );
                }
            }
            Some(BankPickTerminal::LedgerError(error)) => {
                api::host_log!(
                    api::hostlog::Category::NavEvent,
                    api::hostlog::Level::Warn,
                    slot = name,
                    "bank pick terminal was stale: {error:?}"
                );
                let _ = retire_active_bank_pick(
                    navs,
                    name,
                    scope,
                    job.generation,
                    job.request.request_id,
                    ticket,
                );
            }
        }
    }
    navs.lock()
        .expect("navigation state mutex is not poisoned")
        .get(name)
        .map_or_else(BankSelectionInput::default, |bot| bot.bank_pick.posted)
}

pub(super) fn dispatch_observed_bank_op(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    inventory: Option<&[(i32, i32)]>,
    req: &script::shim::InteractReq,
) -> Option<script::slot::PendingBankOp> {
    use api::interact::{ActionSpec, Interactions, OpTarget, SendResult};
    use script::shim::InteractReq;
    use script::slot::{PendingBankOp, PendingBankOpKind};

    if snapshot.bank_component_id() < 0 || !snapshot.bank_loaded() {
        return None;
    }
    let generation = snapshot.bank_session_generation();
    let inventory_count = |id| {
        inventory.map_or(0, |items| {
            items
                .iter()
                .filter(|(item_id, _)| *item_id == id)
                .map(|(_, count)| *count)
                .sum()
        })
    };
    match req {
        InteractReq::Deposit { name, id } => {
            // A selected id names exactly one bank-side item type; a noted
            // stack and the unnoted food of the same name differ by id.
            let item = snapshot.bank_side().iter().find(|item| {
                id.is_none_or(|id| item.def.id == id)
                    && obj_names
                        .and_then(|names| names.name(item.def.id))
                        .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = ops::all_op_in(&item.actions)?;
            let before_count = snapshot
                .bank_side()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    PendingBankOpKind::Deposit,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        InteractReq::Withdraw { name, action } => {
            let item = snapshot.bank().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = ops::action_op_in(&item.actions, action)?;
            let kind = if open_only_withdraw_x(&item.actions, op) {
                PendingBankOpKind::WithdrawXAction
            } else {
                PendingBankOpKind::Withdraw
            };
            let before_count = snapshot
                .bank()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    kind,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        _ => None,
    }
}

/// Counted withdrawals share the host's fenced amount-dialog continuation,
/// regardless of whether a native or compatibility caller selected the op.
pub(super) fn dispatch_observed_withdraw_x(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    inventory: Option<&[(i32, i32)]>,
    req: &script::shim::InteractReq,
) -> Option<script::slot::PendingWithdrawX> {
    use api::interact::{ActionSpec, Interactions, OpTarget, SendResult};
    let script::shim::InteractReq::WithdrawX {
        name,
        count,
        bank_item_id,
        lands_as_id,
        action,
        bank_generation,
    } = req
    else {
        return None;
    };
    if *count <= 0
        || snapshot.bank_component_id() < 0
        || !snapshot.bank_loaded()
        || snapshot.count_dialog_open()
        || snapshot.bank_session_generation() != *bank_generation
    {
        return None;
    }
    let item = snapshot.bank().iter().find(|item| {
        item.def.id == *bank_item_id
            && obj_names
                .and_then(|names| names.name(item.def.id))
                .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
    })?;
    let op = ops::action_op_in(&item.actions, action)?;
    if !matches!(
        Interactions::new(snapshot, driver)
            .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
        SendResult::Sent { .. }
    ) {
        return None;
    }
    let before = inventory.map_or_else(
        || {
            snapshot
                .inventory()
                .iter()
                .filter(|held| held.def.id == *lands_as_id)
                .map(|held| held.count)
                .sum::<i32>()
        },
        |rows| {
            rows.iter()
                .filter(|(id, _)| *id == *lands_as_id)
                .map(|(_, count)| *count)
                .sum::<i32>()
        },
    );
    let pending = script::slot::PendingWithdrawX::waiting_dialog(
        *lands_as_id,
        *count,
        before,
        before.saturating_add((*count).min(item.count)),
        *bank_generation,
    );
    Some(
        if matches!(*count, 1 | 5 | 10)
            && ops::withdraw_count_op_in(&item.actions, *count) == Some(op)
        {
            pending.waiting_settlement()
        } else {
            pending
        },
    )
}

/// Whether the resolved bank op is the open-only `Withdraw X` label: it
/// opens the amount dialog and cannot move inventory on its own, so the
/// accepted send *is* the result (frozen `Bank.withdraw` returns
/// `Input.invButton` → `actions.menuAction`). Every other withdraw label
/// settles on the observed delta. Label decisions themselves are the
/// kernel's ([`ops::action_op_in`], [`ops::all_op_in`],
/// [`ops::fill_click_in`]).
fn open_only_withdraw_x(actions: &[Option<String>], op: i32) -> bool {
    let index = match usize::try_from(op) {
        Ok(index) if index >= 1 => index - 1,
        _ => return false,
    };
    actions
        .get(index)
        .and_then(|label| label.as_deref())
        .is_some_and(|label| ops::normalized_action_eq(label, "Withdraw X"))
}

/// Open the bank the player stands at, by the packed stands of the one
/// bank building ([`SAME_BANK`]) nearest `here`: a teller in the scene
/// first (its packed op), then that building's booths nearest `here`
/// first, so the booth the player stands at is the one used.
pub(crate) fn open_bank_at_here<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    here: Option<(i32, i32, i32)>,
    world: Option<&NavWorld>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    use nav_planner::pack::BankAccess;
    if let Some(world) = world {
        if !world.banks().is_empty() {
            let mut stands: Vec<&nav_planner::pack::BankStand> = world.banks().iter().collect();
            if let Some((hx, hz, hl)) = here {
                stands.sort_by_key(|s| {
                    (
                        i32::from(s.tile.level != hl),
                        (s.tile.x - hx).abs().max((s.tile.z - hz).abs()),
                    )
                });
            }
            let nearest = stands[0];
            let cluster: Vec<&nav_planner::pack::BankStand> = stands
                .iter()
                .copied()
                .filter(|s| {
                    s.tile.level == nearest.tile.level
                        && (s.tile.x - nearest.tile.x)
                            .abs()
                            .max((s.tile.z - nearest.tile.z).abs())
                            <= SAME_BANK
                })
                .collect();
            for stand in cluster
                .iter()
                .filter(|s| matches!(s.access, BankAccess::Npc { .. }))
            {
                if let BankAccess::Npc { name, op, choose } = &stand.access {
                    if open_npc_bank_at_here(
                        driver,
                        snapshot,
                        here,
                        stand.tile,
                        name,
                        *op,
                        choose.as_deref(),
                    ) {
                        return true;
                    }
                }
            }
            // `cluster` keeps the nearest-`here` order of `stands`.
            for stand in cluster
                .iter()
                .filter(|s| matches!(s.access, BankAccess::Booth { .. }))
            {
                if open_booth_stand(driver, snapshot, stand) {
                    return true;
                }
            }
            return false;
        }
    }
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    for loc in snapshot.locs() {
        if let Some(op) = ops::op_slot(loc, "Use-quickly", ops::LabelMatch::Normalized) {
            return matches!(
                ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            );
        }
    }
    false
}

fn open_booth_stand<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    stand: &nav_planner::pack::BankStand,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    ops::booth_at(snapshot.locs(), stand.tile, None)
        .and_then(|loc| {
            ops::op_slot(loc, "Use-quickly", ops::LabelMatch::Normalized).map(|slot| {
                matches!(
                    ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(slot)),
                    SendResult::Sent { .. }
                )
            })
        })
        .unwrap_or(false)
}

fn open_npc_bank_at_here<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    here: Option<(i32, i32, i32)>,
    stand: WorldTile,
    name: &str,
    op: i32,
    choose: Option<&str>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    if let Some(want) = choose.map(str::trim).filter(|s| !s.is_empty()) {
        let want = want.to_lowercase();
        if let Some(option) = snapshot
            .chat_options()
            .iter()
            .position(|row| !row.text.is_empty() && row.text.to_lowercase().contains(&want))
        {
            return matches!(
                ix.answer_choice(i32::try_from(option + 1).unwrap_or(i32::MAX)),
                SendResult::Sent { .. }
            );
        }
        if snapshot.chat_continue_component_id() >= 0 {
            return matches!(ix.continue_dialog(None), SendResult::Sent { .. });
        }
    }
    let Some(npc) = teller_pick(snapshot.npcs(), here, stand, name) else {
        return false;
    };
    let slot =
        ops::packed_op(npc, op).or_else(|| ops::op_slot(npc, "Bank", ops::LabelMatch::Normalized));
    let Some(slot) = slot else {
        return false;
    };
    matches!(
        ix.interact(OpTarget::Npc(npc), ActionSpec::Operation(slot)),
        SendResult::Sent { .. }
    )
}

/// The host teller pick for an already nav-chosen `stand`: named `name`
/// (trimmed, ASCII-folded), within [`SAME_BANK`] of the stand or 10 of
/// `here` (Chebyshev, +10_000 per plane mismatch); the smallest
/// `(d_stand, d_here, distance, index)` key wins. This is BankBudget's
/// stand policy, not `bank::npc::banker_scan`'s scene-order policy.
fn teller_pick<'a>(
    npcs: &'a [api::snapshot::NpcView],
    here: Option<(i32, i32, i32)>,
    stand: WorldTile,
    name: &str,
) -> Option<&'a api::snapshot::NpcView> {
    let wanted = name.trim();
    let chebyshev = |npc: &api::snapshot::NpcView, x: i32, z: i32, level: i32| {
        (npc.tile.x - x).abs().max((npc.tile.z - z).abs())
            + i32::from(npc.tile.level != level) * 10_000
    };
    let key = |npc: &api::snapshot::NpcView| {
        let d_stand = chebyshev(npc, stand.x, stand.z, stand.level);
        let d_here = here
            .map(|(hx, hz, hl)| chebyshev(npc, hx, hz, hl))
            .unwrap_or(i32::MAX);
        (d_stand, d_here, npc.distance, npc.index)
    };
    npcs.iter()
        .filter(|npc| {
            npc.name
                .as_deref()
                .is_some_and(|got| got.trim().eq_ignore_ascii_case(wanted))
        })
        .filter(|npc| {
            let (d_stand, d_here, ..) = key(npc);
            d_stand <= SAME_BANK || d_here <= 10
        })
        .min_by_key(|npc| key(npc))
}

#[cfg(any(test, all(target_os = "linux", feature = "test-support")))]
#[path = "script_bank_harness.rs"]
pub(crate) mod harness;

#[cfg(test)]
#[path = "script_bank_tests.rs"]
mod tests;
