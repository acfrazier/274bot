use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
#[cfg(test)]
use std::time::Instant;

use super::route_inspect;
use api::bank_memory::Origin;
use api::identity::{Cancellation, PublishSequence, RequestId, Scope, Settlement, WorkIdentity};
use api::process_condition::ProcessConditions;
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, SnapshotView, WorldTile};
use api::work::{
    Checkpoint, CooperativeJob, InputRevision, RetainedState, StepContext, StepOutcome,
    SubmissionRefusal, WorkAdmission, WorkTicket,
};
use nav_executor::Traveller;
use nav_planner::arrival::ArrivalKind;
#[cfg(test)]
use nav_planner::bank_fetch::{plan_bank_fetch, planning_rows, BankFetchError};
use nav_planner::bank_fetch::{
    BankFetchCursor, BankFetchPlan, BankRows, BankStep, FetchableEdgeCursor, FetchableStateCursor,
    PlanningRowsCursor,
};
use nav_planner::contract::{NavLease, RoutePlanLease};
use nav_planner::producer::{lease as nav_lease, OwnerResultLedger, OwnerResultReservation};
#[cfg(test)]
use nav_planner::router::find_missing_item_reqs_with_avoid;
use nav_planner::router::{
    AvoidRect, BlockerDiagnosis, CompactFallbackRoute, CompactRoute, CursorStep, FindOptions,
    MissingReqView, MissingRequirements, Route, RouteError, SearchCursor, SearchOutcome,
    SearchPreflight, SearchRequest, SearchRequestShape, FIRST_TARGET_BUDGET, NODE_BUDGET,
};
use nav_planner::world::NavWorld;
use nav_planner::zones::{ZoneExempt, ZoneFilterOwned, ZoneKey};
use nav_planner::WorldState;
use script::work_admission::{cancellation::WorkCancellation, WorkService};

use crate::walk_plan::{PendingBankFetch, RouteOutcome, RoutePath, RouteSearchResultKind};

// The real v17 Dusty-key diagnosis needs 679,226 settlements before it
// finds the actionable item. Two first-goal allowances retain that result
// while bounding all post-failure searches together (about 180 MB of tables).
const DIAGNOSTIC_BUDGET: usize = 2 * FIRST_TARGET_BUDGET;

#[derive(Default)]
pub(crate) struct RouteCompletion {
    #[cfg(test)]
    sender: Option<std::sync::mpsc::Sender<()>>,
}

impl RouteCompletion {
    #[cfg(test)]
    fn channel() -> (Self, std::sync::mpsc::Receiver<()>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (
            Self {
                sender: Some(sender),
            },
            receiver,
        )
    }

    fn signal(self) {
        drop(self);
    }
}

impl Drop for RouteCompletion {
    fn drop(&mut self) {
        #[cfg(test)]
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(());
        }
    }
}
/// The one route-family result owner retained by a slot.
pub(crate) struct RouteTicketOwner {
    identity: WorkIdentity,
    revision: InputRevision,
    generation: u64,
    request_id: u64,
    kind: RouteDemandKind,
    cancellation: WorkCancellation<RouteWorkSpec>,
    active: bool,
}
impl RouteTicketOwner {
    fn ticket(&self) -> WorkTicket<RouteWorkSpec> {
        WorkTicket::new(self.identity, self.revision)
    }

    fn abandon(&mut self, reason: Cancellation) {
        if self.active {
            self.active = false;
            self.cancellation.abandon(reason);
        }
    }

    fn settled(&mut self) {
        self.active = false;
    }
}
impl Drop for RouteTicketOwner {
    fn drop(&mut self) {
        self.abandon(Cancellation::ScopeInvalidated);
    }
}

pub(crate) enum RouteDemand {
    Pending {
        request: Box<ScriptRouteRequest>,
        kind: RouteDemandKind,
    },
    Preparing {
        generation: u64,
        request_id: u64,
        /// The taken demand's kind: cleanup that finds only this placeholder
        /// must still tell walk work from a live advisory.
        kind: RouteDemandKind,
    },
    Prepared(Box<RouteJob>),
}

#[derive(Clone)]
pub(crate) enum RouteDemandKind {
    Walk {
        authority: Option<script::native::HostAuthority>,
    },
    Advisory {
        authority: script::native::HostAuthority,
        evidence: EvidenceStamp,
    },
}

impl RouteDemandKind {
    fn origin(&self) -> Option<RequestId> {
        match self {
            Self::Walk {
                authority: Some(authority),
            }
            | Self::Advisory { authority, .. } => RequestId::from_raw(authority.request_id().get()),
            Self::Walk { authority: None } => None,
        }
    }

    /// Whether the cleanup of a revoked or ended walk may discard work of this
    /// kind. Walk work always belongs to the walk. Advisory work has its own
    /// owner (its `HostAuthority`): only an advisory whose owner is already
    /// revoked is discarded, silently, because nobody awaits it.
    fn walk_cleanup_discards(&self) -> bool {
        match self {
            Self::Walk { .. } => true,
            Self::Advisory { authority, .. } => !authority.live(),
        }
    }
}

impl RouteDemand {
    fn generation(&self) -> u64 {
        match self {
            Self::Pending { request, .. } => request.generation,
            Self::Preparing { generation, .. } => *generation,
            Self::Prepared(job) => job.generation(),
        }
    }

    fn request_id(&self) -> u64 {
        match self {
            Self::Pending { request, .. } => request.request_id,
            Self::Preparing { request_id, .. } => *request_id,
            Self::Prepared(job) => job.request_id(),
        }
    }

    fn matches(&self, generation: u64, request_id: u64) -> bool {
        self.generation() == generation && self.request_id() == request_id
    }

    fn signal_completion(self) {
        match self {
            Self::Pending { request, .. } => request.completion.signal(),
            Self::Prepared(job) => job.request.completion.signal(),
            Self::Preparing { .. } => {}
        }
    }
    fn kind(&self) -> Option<RouteDemandKind> {
        match self {
            Self::Pending { kind, .. } => Some(kind.clone()),
            Self::Preparing { kind, .. } => Some(kind.clone()),
            Self::Prepared(job) => job.kind.as_ref().cloned(),
        }
    }
}

struct RouteWorkSpec;

#[expect(
    clippy::large_enum_variant,
    reason = "A completed result retains its charged inline state without another allocation."
)]
enum RouteStageResult {
    Continue(Box<RouteJob>),
    Complete(RouteJobOutput),
}

struct RouteWorkJob {
    job: Option<Box<RouteJob>>,
}

impl RouteWorkJob {
    fn new(job: Box<RouteJob>) -> Self {
        Self { job: Some(job) }
    }

    fn into_job(mut self) -> Box<RouteJob> {
        self.job.take().expect("unsubmitted route job is present")
    }
}

impl CooperativeJob for RouteWorkJob {
    type Output = RouteStageResult;
    type Error = RouteJobError;
    type Phase = nav_planner::contract::RoutePhase;

    fn request_thread(&self) -> Option<api::work::RequestThread<'_>> {
        self.job
            .as_ref()
            .map(|job| api::work::RequestThread::new(&job.thread_name))
    }

    fn step(
        &mut self,
        context: &mut StepContext<'_, Self::Phase>,
    ) -> StepOutcome<Self::Output, Self::Error> {
        let job = self.job.as_mut().expect("active route job is present");
        job.gap_custody = None;
        match job.step(context) {
            StepOutcome::Yielded if job.boundary_ready => {
                job.boundary_ready = false;
                StepOutcome::Completed(RouteStageResult::Continue(
                    self.job.take().expect("continuation moves once"),
                ))
            }
            StepOutcome::Yielded => StepOutcome::Yielded,
            StepOutcome::Completed(output) => {
                StepOutcome::Completed(RouteStageResult::Complete(output))
            }
            StepOutcome::Failed(error) => StepOutcome::Failed(error),
        }
    }

    fn retained(&self) -> RetainedState {
        self.job
            .as_ref()
            .map_or_else(|| RetainedState::clean(0), |job| job.retained())
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    let _ = assert_send_sync::<WorkCancellation<RouteWorkSpec>>;
};

#[cfg(test)]
#[derive(Clone, Copy)]
struct RouteBankCharges {
    planning: usize,
    rows: usize,
    fetch: usize,
    plan: usize,
    overlay: usize,
    undo: usize,
    post_bank_state: usize,
}

#[derive(Clone, Copy)]
struct RouteSubmission {
    revision: InputRevision,
    input_bytes: usize,
    scratch_bytes: usize,
    result_bytes: usize,
    assessment_future_bytes: usize,
    plan_item_rows: usize,
    #[cfg(test)]
    bank_charges: RouteBankCharges,
}

struct RoutePreflights {
    single: SearchPreflight,
    post_bank_single: SearchPreflight,
    first: SearchPreflight,
    bank_access_first: SearchPreflight,
    missing: SearchPreflight,
    missing_route: SearchPreflight,
    blocking: SearchPreflight,
    unresolved: SearchPreflight,
    actionable: Option<SearchPreflight>,
    entrance: SearchPreflight,
}

fn route_search_options(request: &ScriptRouteRequest) -> FindOptions {
    if request.admission.compat_v1
        || request.admission.policy == crate::admission::RiskPolicy::Proceed
    {
        FindOptions {
            zones: ZoneExempt::all(),
            ..request.opts
        }
    } else {
        request.opts
    }
}

fn strict_bank_access_options(mut options: FindOptions) -> FindOptions {
    options.allow_bank_fetch = false;
    options.prefer_target_order = true;
    options
}

fn route_blocker_diagnosis(request: &ScriptRouteRequest) -> BlockerDiagnosis {
    if request.radius <= 0 && request.candidate_goals.is_none() {
        BlockerDiagnosis::SingleTarget
    } else {
        BlockerDiagnosis::FirstGoal
    }
}

fn copy_boxed_slice<T: Copy>(values: &[T]) -> Result<Box<[T]>, RouteJobError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(values.len())
        .map_err(|_| RouteJobError::Allocation)?;
    copy.extend_from_slice(values);
    Ok(copy.into_boxed_slice())
}

impl RouteSubmission {
    fn prepare(
        request: &ScriptRouteRequest,
        state: &Arc<WorldState>,
        lease: &NavLease,
        bank_rows: usize,
        required_plan_rows: usize,
        thread_name_bytes: usize,
    ) -> Result<(Self, RoutePreflights), RouteJobError> {
        let candidate_capacity = RouteTargetCursor::capacity(request)?;
        let avoid_capacity = request
            .exclusions
            .as_ref()
            .map_or(0, |exclusions| exclusions.avoid.len());
        let options = route_search_options(request);
        let stand_rows = request.world.banks().len();
        let bank_access_target_capacity = stand_rows.checked_mul(4).ok_or(RouteJobError::Budget)?;
        let bank_access_options = strict_bank_access_options(options);
        let diagnosis = route_blocker_diagnosis(request);
        let mut post_bank_options = options;
        post_bank_options.allow_bank_fetch = false;
        let max_settles = NODE_BUDGET;
        let maximum_goals = candidate_capacity.max(4);
        let zone_count = request
            .world
            .graph
            .zones
            .as_ref()
            .map_or(0, |zones| zones.zones().len());
        let zone_filter_bytes =
            ZoneFilterOwned::worst_case_bytes(zone_count).ok_or(RouteJobError::Budget)?;
        let route_bytes =
            CompactRoute::storage_upper_bound(max_settles).ok_or(RouteJobError::Budget)?;
        let edge_capacity = request
            .world
            .graph
            .edges
            .len()
            .checked_add(request.world.graph.teleports.len())
            .ok_or(RouteJobError::Budget)?;
        let route_pack = lease.identity().pack();
        let preflight =
            |shape| SearchCursor::preflight(shape, lease, state).ok_or(RouteJobError::Budget);
        let single = preflight(SearchRequestShape::Single {
            options,
            avoid_capacity,
            max_settles,
        })?;
        let post_bank_single = preflight(SearchRequestShape::Single {
            options: post_bank_options,
            avoid_capacity,
            max_settles,
        })?;
        let bank_access_first = preflight(SearchRequestShape::First {
            target_capacity: bank_access_target_capacity,
            fallback_capacity: 0,
            options: bank_access_options,
            avoid_capacity,
            max_settles,
        })?;
        let first = preflight(SearchRequestShape::First {
            target_capacity: maximum_goals,
            fallback_capacity: candidate_capacity,
            options,
            avoid_capacity,
            max_settles,
        })?;
        let missing = preflight(SearchRequestShape::MissingItemReqs {
            target_capacity: maximum_goals,
            fallback_capacity: candidate_capacity,
            options,
            avoid_capacity,
            max_settles,
        })?;
        let missing_route = preflight(SearchRequestShape::MissingItemReqsForRoute {
            route_bytes,
            edge_capacity,
            route_pack,
        })?;
        let blocking = preflight(SearchRequestShape::BlockingZones {
            target_capacity: candidate_capacity,
            diagnosis,
            options,
            avoid_capacity,
            zone_filter_bytes,
            max_settles,
        })?;
        let unresolved = preflight(SearchRequestShape::UnresolvedQuestGates {
            target_capacity: candidate_capacity,
            options,
            avoid_capacity,
            max_settles,
        })?;
        let actionable = state
            .establishable
            .as_ref()
            .is_some_and(|windows| !windows.is_empty())
            .then(|| {
                preflight(SearchRequestShape::ActionableQuestGates {
                    target_capacity: candidate_capacity,
                    options,
                    avoid_capacity,
                    max_settles,
                    relaxation: nav_planner::router::GateRelaxation::EstablishableFalse,
                })
            })
            .transpose()?;
        let entrance = preflight(SearchRequestShape::EntranceRequirements {
            target_capacity: candidate_capacity,
            options,
            avoid_capacity,
            max_settles,
        })?;
        let preflights = RoutePreflights {
            single,
            post_bank_single,
            first,
            bank_access_first,
            missing,
            missing_route,
            blocking,
            unresolved,
            actionable,
            entrance,
        };

        // New diagnoses end the current search job before either row cursor
        // can advance. Retain the largest admitted diagnosis because Hint
        // overlay rows can survive a later, smaller diagnosis.
        let plan_item_rows = required_plan_rows.min(
            preflights
                .missing
                .reservation()
                .max_plan_item_rows
                .max(preflights.missing_route.reservation().max_plan_item_rows),
        );
        let carry_capacity = preflights
            .missing
            .reservation()
            .max_missing_carry_ids
            .max(preflights.missing_route.reservation().max_missing_carry_ids);
        let missing_result_bytes = preflights
            .missing
            .reservation()
            .result_bytes
            .max(preflights.missing_route.reservation().result_bytes);
        // These owners exist only after the bank-fetch transitions in
        // `handle_search` or `begin_fetchable_first`. Diagnostic searches
        // remain reachable with bank fetch disabled and stay charged below.
        let (
            bank_plan_bytes,
            planner_bytes,
            planned_rows_bytes,
            bank_direct_bytes,
            bank_planned_bytes,
            fetchable_state_bytes,
            fetchable_edge_bytes,
            post_bank_state_bytes,
            bank_undo_bytes,
        ) = if options.allow_bank_fetch {
            let bank_plan_bytes =
                BankFetchPlan::heap_bound(plan_item_rows).ok_or(RouteJobError::Budget)?;
            let planner_bytes = PlanningRowsCursor::heap_bound(bank_rows, plan_item_rows)
                .ok_or(RouteJobError::Budget)?;
            let planned_rows_bytes = PlanningRowsCursor::rows_heap_bound(bank_rows, plan_item_rows)
                .ok_or(RouteJobError::Budget)?;
            let planned_rows = bank_rows
                .checked_add(plan_item_rows)
                .ok_or(RouteJobError::Budget)?;
            let bank_direct_bytes =
                BankFetchCursor::heap_bound(bank_rows, plan_item_rows, stand_rows)
                    .ok_or(RouteJobError::Budget)?;
            let bank_planned_bytes =
                BankFetchCursor::heap_bound(planned_rows, plan_item_rows, stand_rows)
                    .ok_or(RouteJobError::Budget)?;
            let fetchable_state_bytes = FetchableStateCursor::heap_bound(
                state,
                planned_rows,
                !request.world.banks().is_empty(),
            )
            .ok_or(RouteJobError::Budget)?;
            let fetchable_edge_bytes =
                FetchableEdgeCursor::heap_bound(&request.world.graph, options.allow_teleports)
                    .ok_or(RouteJobError::Budget)?;
            let state_growth = state
                .growth_bound(plan_item_rows, plan_item_rows)
                .ok_or(RouteJobError::Budget)?;
            let post_bank_state_bytes = state_growth
                .inventory_new_bytes
                .checked_add(state_growth.worn_new_bytes)
                .ok_or(RouteJobError::Budget)?;
            let bank_undo_bytes = plan_item_rows
                .checked_mul(2)
                .and_then(|rows| rows.checked_mul(std::mem::size_of::<RouteBankUndo>()))
                .ok_or(RouteJobError::Budget)?;
            (
                bank_plan_bytes,
                planner_bytes,
                planned_rows_bytes,
                bank_direct_bytes,
                bank_planned_bytes,
                fetchable_state_bytes,
                fetchable_edge_bytes,
                post_bank_state_bytes,
                bank_undo_bytes,
            )
        } else {
            (0, 0, 0, 0, 0, 0, 0, 0, 0)
        };
        let risk = script::combat::risk::assessment_heap_bounds().ok_or(RouteJobError::Budget)?;
        let risk_admission_bytes =
            script::combat::risk::admission_peak_heap_bytes().ok_or(RouteJobError::Budget)?;
        let assessment_future_bytes =
            route_assessment_future_layout_bytes(&request.world, lease, request.admission)
                .ok_or(RouteJobError::Budget)?;
        let risk_tables_bytes = risk_tables_build_peak_bytes().ok_or(RouteJobError::Budget)?;
        let candidate_bytes = candidate_capacity
            .checked_mul(std::mem::size_of::<WorldTile>())
            .ok_or(RouteJobError::Budget)?;
        let (arrival_heap_bytes, arrival_future_bytes) =
            arrival_filter_charge(request, candidate_capacity)?;
        let basis_avoid_bytes =
            arc_slice_heap_bytes::<AvoidRect>(avoid_capacity).ok_or(RouteJobError::Budget)?;

        let input_bytes =
            route_capture_bytes(request, state, thread_name_bytes).ok_or(RouteJobError::Budget)?;

        let mut result_bytes = std::mem::size_of::<RouteJobOutput>()
            .checked_add(route_bytes.checked_mul(2).ok_or(RouteJobError::Budget)?)
            .and_then(|bytes| bytes.checked_add(bank_plan_bytes))
            .and_then(|bytes| bytes.checked_add(risk.retained_bytes))
            .and_then(|bytes| {
                carry_capacity
                    .checked_mul(std::mem::size_of::<MissingCarry>())
                    .and_then(|missing| bytes.checked_add(missing))
            })
            .and_then(|bytes| {
                preflights
                    .blocking
                    .reservation()
                    .result_bytes
                    .checked_add(bytes)
            })
            .and_then(|bytes| {
                route_output_detail_bound(request, carry_capacity)
                    .and_then(|detail| bytes.checked_add(detail))
            })
            .and_then(|bytes| {
                basis_avoid_bytes
                    .checked_mul(2)
                    .and_then(|avoid| bytes.checked_add(avoid))
            })
            .ok_or(RouteJobError::Budget)?;
        result_bytes = result_bytes.max(preflights.unresolved.reservation().result_bytes);
        if let Some(actionable) = &preflights.actionable {
            result_bytes = result_bytes.max(actionable.reservation().result_bytes);
        }
        result_bytes = result_bytes.max(preflights.entrance.reservation().result_bytes);
        result_bytes = result_bytes
            .checked_add(state.quest_evidence.as_ref().map_or(
                0,
                nav_planner::quest_gates::QuestEvidence::owned_provider_bytes,
            ))
            .ok_or(RouteJobError::Budget)?;

        let mut peak = input_bytes
            .checked_add(result_bytes)
            .ok_or(RouteJobError::Budget)?;
        let mut include_peak = |extra: usize| -> Result<(), RouteJobError> {
            let total = input_bytes
                .checked_add(extra)
                .ok_or(RouteJobError::Budget)?;
            peak = peak.max(total);
            Ok(())
        };
        let search_working_bytes = bank_plan_bytes
            .checked_add(planned_rows_bytes)
            .and_then(|bytes| bytes.checked_add(fetchable_state_bytes))
            .and_then(|bytes| bytes.checked_add(fetchable_edge_bytes))
            .and_then(|bytes| bytes.checked_add(post_bank_state_bytes))
            .and_then(|bytes| bytes.checked_add(bank_undo_bytes))
            .ok_or(RouteJobError::Budget)?;
        include_peak(
            candidate_bytes
                .checked_add(arrival_heap_bytes)
                .and_then(|bytes| bytes.checked_add(arrival_future_bytes))
                .ok_or(RouteJobError::Budget)?,
        )?;
        for search in [
            Some(&preflights.single),
            options
                .allow_bank_fetch
                .then_some(&preflights.post_bank_single),
            Some(&preflights.first),
            Some(&preflights.missing),
            Some(&preflights.missing_route),
            Some(&preflights.blocking),
            Some(&preflights.unresolved),
            Some(&preflights.entrance),
        ]
        .into_iter()
        .flatten()
        .chain(preflights.actionable.as_ref())
        {
            include_peak(
                search
                    .input_bytes()
                    .checked_add(search.reservation().peak_bytes)
                    .and_then(|bytes| bytes.checked_add(search.reservation().future_layout_bytes))
                    .and_then(|bytes| bytes.checked_add(candidate_bytes))
                    .and_then(|bytes| bytes.checked_add(search_working_bytes))
                    .ok_or(RouteJobError::Budget)?,
            )?;
        }
        if options.allow_bank_fetch {
            include_peak(
                preflights
                    .bank_access_first
                    .input_bytes()
                    .checked_add(preflights.bank_access_first.reservation().peak_bytes)
                    .and_then(|bytes| {
                        bytes.checked_add(
                            preflights
                                .bank_access_first
                                .reservation()
                                .future_layout_bytes,
                        )
                    })
                    .and_then(|bytes| bytes.checked_add(candidate_bytes))
                    .and_then(|bytes| bytes.checked_add(search_working_bytes))
                    .and_then(|bytes| bytes.checked_add(bank_direct_bytes.max(bank_planned_bytes)))
                    .ok_or(RouteJobError::Budget)?,
            )?;
            include_peak(
                missing_result_bytes
                    .checked_add(planner_bytes)
                    .ok_or(RouteJobError::Budget)?,
            )?;
            include_peak(
                missing_result_bytes
                    .checked_add(planned_rows_bytes)
                    .and_then(|bytes| bytes.checked_add(bank_planned_bytes))
                    .ok_or(RouteJobError::Budget)?,
            )?;
            include_peak(
                missing_result_bytes
                    .checked_add(bank_direct_bytes)
                    .ok_or(RouteJobError::Budget)?,
            )?;
            include_peak(
                planned_rows_bytes
                    .checked_add(fetchable_state_bytes)
                    .and_then(|bytes| bytes.checked_add(fetchable_edge_bytes))
                    .and_then(|bytes| bytes.checked_add(candidate_bytes))
                    .ok_or(RouteJobError::Budget)?,
            )?;
            include_peak(
                planned_rows_bytes
                    .checked_add(fetchable_state_bytes)
                    .and_then(|bytes| {
                        preflights
                            .first
                            .input_bytes()
                            .checked_add(preflights.first.reservation().peak_bytes)
                            .and_then(|bytes| {
                                bytes
                                    .checked_add(preflights.first.reservation().future_layout_bytes)
                            })
                            .and_then(|search| bytes.checked_add(search))
                    })
                    .ok_or(RouteJobError::Budget)?,
            )?;
            include_peak(
                bank_plan_bytes
                    .checked_add(post_bank_state_bytes)
                    .and_then(|bytes| bytes.checked_add(bank_undo_bytes))
                    .and_then(|bytes| {
                        preflights
                            .post_bank_single
                            .input_bytes()
                            .checked_add(preflights.post_bank_single.reservation().peak_bytes)
                            .and_then(|bytes| {
                                bytes.checked_add(
                                    preflights
                                        .post_bank_single
                                        .reservation()
                                        .future_layout_bytes,
                                )
                            })
                            .and_then(|search| bytes.checked_add(search))
                    })
                    .ok_or(RouteJobError::Budget)?,
            )?;
        }
        include_peak(
            route_bytes
                .checked_add(bank_plan_bytes)
                .and_then(|bytes| {
                    risk_tables_bytes
                        .checked_add(risk.peak_bytes)
                        .and_then(|risk| risk.checked_add(assessment_future_bytes))
                        .and_then(|risk| bytes.checked_add(risk))
                })
                .ok_or(RouteJobError::Budget)?,
        )?;
        include_peak(
            route_bytes
                .checked_add(bank_plan_bytes)
                .and_then(|bytes| {
                    risk_tables_bytes
                        .checked_add(risk_admission_bytes)
                        .and_then(|risk| risk.checked_add(assessment_future_bytes))
                        .and_then(|risk| bytes.checked_add(risk))
                })
                .ok_or(RouteJobError::Budget)?,
        )?;
        let available = peak.checked_sub(input_bytes).ok_or(RouteJobError::Budget)?;
        if peak > ROUTE_WORK_LIMIT_BYTES || result_bytes > available {
            return Err(RouteJobError::Budget);
        }
        let scratch_bytes = peak
            .checked_sub(input_bytes)
            .and_then(|bytes| bytes.checked_sub(result_bytes))
            .ok_or(RouteJobError::Budget)?;
        Ok((
            Self {
                revision: InputRevision::FIRST,
                input_bytes,
                scratch_bytes,
                result_bytes,
                assessment_future_bytes,
                plan_item_rows,
                // Record the same terms used in the admission sums, not a
                // second calculation of what those terms ought to charge.
                #[cfg(test)]
                bank_charges: RouteBankCharges {
                    planning: planner_bytes,
                    rows: planned_rows_bytes,
                    fetch: bank_direct_bytes.max(bank_planned_bytes),
                    plan: bank_plan_bytes,
                    overlay: fetchable_state_bytes,
                    undo: bank_undo_bytes,
                    post_bank_state: post_bank_state_bytes,
                },
            },
            preflights,
        ))
    }
}

fn route_vec_bytes<T>(values: &Vec<T>) -> Option<usize> {
    values.capacity().checked_mul(std::mem::size_of::<T>())
}

fn route_capture_bytes(
    request: &ScriptRouteRequest,
    state: &WorldState,
    thread_name_bytes: usize,
) -> Option<usize> {
    let mut bytes = std::mem::size_of::<RouteJob>()
        .checked_add(std::mem::size_of::<RouteWorkJob>())?
        .checked_add(std::mem::size_of::<ScriptRouteRequest>())?
        .checked_add(thread_name_bytes)?
        .checked_add(state.try_shared_retained_bytes()?)?
        .checked_add(route_vec_bytes(&request.bank)?)?
        .checked_add(arc_slice_heap_bytes::<AvoidRect>(request.avoid().len())?)?;
    if let Some(goals) = &request.candidate_goals {
        bytes = bytes.checked_add(goals.retained_bytes()?)?;
    }
    if let Some(geometry) = &request.live_geometry {
        bytes = bytes.checked_add(route_vec_bytes(&geometry.flags)?)?;
    }
    if let Some(exclusions) = request.exclusions.as_deref() {
        bytes = bytes.checked_add(route_exclusions_heap_bytes(exclusions)?)?;
    }
    Some(bytes)
}

fn route_excluded_target_bytes(capacity: usize) -> Option<usize> {
    if capacity == 0 {
        return Some(0);
    }
    // The same RawTable layout used by WorldState::growth_bound: one
    // control byte per bucket and one trailing 16-byte SIMD group.
    let buckets = if capacity < 4 {
        4
    } else if capacity < 8 {
        8
    } else if capacity < 15 {
        16
    } else {
        capacity
            .checked_mul(8)?
            .checked_div(7)?
            .checked_next_power_of_two()?
    };
    buckets
        .checked_mul(std::mem::size_of::<WorldTile>())?
        .checked_add(buckets)?
        .checked_add(16)
}

const ROUTE_WORK_LIMIT_BYTES: usize = 192 * 1024 * 1024;

fn arc_slice_heap_bytes<T>(length: usize) -> Option<usize> {
    let payload = std::alloc::Layout::array::<T>(length).ok()?;
    let (layout, _) = std::alloc::Layout::new::<[std::sync::atomic::AtomicUsize; 2]>()
        .extend(payload)
        .ok()?;
    Some(layout.pad_to_align().size())
}

fn route_exclusions_heap_bytes(exclusions: &ScriptRouteExclusions) -> Option<usize> {
    let payload = std::alloc::Layout::new::<ScriptRouteExclusions>();
    let (layout, _) = std::alloc::Layout::new::<[std::sync::atomic::AtomicUsize; 2]>()
        .extend(payload)
        .ok()?;
    let mut bytes = layout.pad_to_align().size();
    for allocation in [
        exclusions
            .avoid
            .capacity()
            .checked_mul(std::mem::size_of::<AvoidRect>())?,
        exclusions
            .avoid_wire
            .capacity()
            .checked_mul(std::mem::size_of::<script::interact::InspectAvoidWire>())?,
        exclusions
            .cross
            .capacity()
            .checked_mul(std::mem::size_of::<Arc<str>>())?,
    ] {
        bytes = bytes.checked_add(allocation)?;
    }
    for value in &exclusions.cross {
        bytes = bytes.checked_add(arc_slice_heap_bytes::<u8>(value.len())?)?;
    }
    Some(bytes)
}

fn risk_tables_build_peak_bytes() -> Option<usize> {
    let Ok(combat) = script::combat::guard::shared_tables() else {
        return Some(0);
    };
    let rows = combat
        .selected()
        .npc_names()
        .map_or(0, |names| names.rows.len());
    let backing_bytes =
        rows.checked_mul(std::mem::size_of::<(i32, script::combat::risk::KindRisk)>())?;
    let mut checkpoint: crate::admission::RouteYieldCallback = crate::admission::route_yield_unit;
    let future = script::combat::risk::RiskTables::build_cooperative(&combat, &mut checkpoint);
    let future_bytes = std::mem::size_of_val(&future);
    std::mem::drop(future);
    std::mem::size_of::<script::combat::risk::RiskTables>()
        .checked_add(backing_bytes.checked_mul(2)?)?
        .checked_add(future_bytes)
}

struct TextByteCounter(usize);

impl std::fmt::Write for TextByteCounter {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.0 = self.0.checked_add(value.len()).ok_or(std::fmt::Error)?;
        Ok(())
    }
}

fn route_output_detail_bound(
    request: &ScriptRouteRequest,
    missing_capacity: usize,
) -> Option<usize> {
    use std::fmt::Write as _;

    let mut context = TextByteCounter(0);
    write!(
        &mut context,
        "native walk route failed: {:?} arrival from ({}, {}, {}) to ({}, {}, {}) within radius {}",
        request.arrival,
        request.from.x,
        request.from.z,
        request.from.level,
        request.to.x,
        request.to.z,
        request.to.level,
        request.radius
    )
    .ok()?;
    let blocked = if let Some(table) = request.world.graph.zones.as_ref() {
        table
            .blocking_detail_bytes_bound()
            .checked_add(SCRIPT_DANGER_HINT.len())?
            .checked_add(2)?
    } else {
        crate::walk_map::LEGACY_ZONES_DETAIL.len()
    };
    let blocked_with_context = blocked.checked_add(2)?.checked_add(context.0)?;
    let item_name_bytes = request
        .world
        .transport_item_name_bytes_bound()
        .max("item #".len() + 11);
    let supply_row_bytes = 58usize.checked_add(item_name_bytes)?;
    let supply_bytes = missing_capacity
        .checked_mul(supply_row_bytes)?
        .checked_add(missing_capacity.saturating_sub(1).checked_mul(2)?)?;
    let mut diagnostic_limit = TextByteCounter(0);
    write!(
        &mut diagnostic_limit,
        "no route found; the route check stopped after {} places before it found the cause",
        usize::MAX
    )
    .ok()?;
    let detail_len = blocked_with_context
        .checked_add(2)?
        .checked_add(diagnostic_limit.0)?
        .checked_add(if supply_bytes == 0 {
            0
        } else {
            supply_bytes.checked_add(2)?
        })?;
    let search_result_bytes = "search_result="
        .len()
        .checked_add("BudgetExhausted nodes_settled=".len())?
        .checked_add(usize::MAX.ilog10() as usize + 1)?
        .checked_add(" (incomplete search)".len())?;
    let detail_len = detail_len
        .max("candidate_goal_tier=preferred".len())
        .checked_add(search_result_bytes.checked_add(2)?)?;
    detail_len.checked_add(arc_slice_heap_bytes::<u8>(detail_len)?)
}

fn arrival_filter_needed(request: &ScriptRouteRequest) -> bool {
    request.radius > 0
        && request.loc_id.is_none()
        && request.arrival != ArrivalKind::Area
        && request.live_geometry.is_none()
}

fn arrival_collision_flags(
    collision: &nav_planner::collision::WorldCollision,
    tile: WorldTile,
) -> Option<i32> {
    crate::walk_permissions::packed_arrival_flags(collision, tile)
}

async fn route_arrival_filter_future(
    world: Arc<NavWorld>,
    mut candidates: Vec<WorldTile>,
    destination: WorldTile,
    radius: i32,
    bounds: api::query::ArrivalCollisionBounds,
    checkpoint: crate::admission::RouteYieldCallback,
) -> Vec<WorldTile> {
    let mut checkpoint = || checkpoint();
    api::query::retain_arrival_candidates_cooperative(
        &mut candidates,
        destination,
        radius,
        bounds,
        move |tile| arrival_collision_flags(&world.collision, tile),
        &mut checkpoint,
    )
    .await;
    candidates
}

fn arrival_filter_charge(
    request: &ScriptRouteRequest,
    candidate_capacity: usize,
) -> Result<(usize, usize), RouteJobError> {
    if !arrival_filter_needed(request) {
        return Ok((0, 0));
    }
    let collision = &request.world.collision;
    let bounds = api::query::ArrivalCollisionBounds::new(
        collision.origin.x,
        collision.origin.z,
        collision.width,
        collision.height,
    )
    .ok_or(RouteJobError::Budget)?;
    let heap_bytes =
        api::query::arrival_candidates_heap_bound(request.radius, candidate_capacity, bounds)
            .ok_or(RouteJobError::Budget)?;
    let future = route_arrival_filter_future(
        Arc::clone(&request.world),
        Vec::new(),
        request.to,
        request.radius,
        bounds,
        crate::admission::route_yield_unit,
    );
    Ok((heap_bytes, std::mem::size_of_val(&future)))
}

impl api::work::WorkFamilySpec for RouteWorkSpec {
    const FAMILY: api::work::WorkFamily = api::work::WorkFamily::Route;
    type Input = RouteSubmission;
    type Output = RouteStageResult;
    type Error = RouteJobError;
    type Phase = nav_planner::contract::RoutePhase;

    fn revision(input: &Self::Input) -> InputRevision {
        input.revision
    }

    fn charge(input: &Self::Input) -> api::work::WorkCharge {
        api::work::WorkCharge::new(
            input.input_bytes,
            input.scratch_bytes,
            input.result_bytes,
            0,
        )
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
}
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RouteWorkSpec>();
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum RouteJobError {
    #[error("route request cannot fit the checked WorkAdmission bounds")]
    Budget,
    #[error("route search cursor failed")]
    Search,
    #[error("route job input or output allocation failed")]
    Allocation,
    #[error("route result shape did not match its bounded request")]
    ResultShape,
}

type RouteArrivalFuture = Pin<Box<dyn Future<Output = Vec<WorldTile>> + Send + 'static>>;
type RouteAssessmentFuture = Pin<
    Box<
        dyn Future<
                Output = Result<
                    (
                        RouteJobOutcome,
                        Option<Arc<script::combat::risk::RouteAssessment>>,
                        bool,
                    ),
                    RouteJobError,
                >,
            > + Send
            + 'static,
    >,
>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteJobStage {
    Targets,
    Arrival,
    CopyTargets,
    Search,
    CopyAvoid,
    PlanRows,
    BankPlan,
    FetchableState,
    FetchableEdges,
    RestoreFetchable,
    ApplyBankPlan,
    RollbackBankState,
    CopyBankAvoid,
    BeginDiagnostics,
    Diagnostics,
    Assess,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteSearchPurpose {
    Strict,
    StrictFirst,
    CandidateFallback,
    MissingTarget,
    MissingTargets,
    MissingRoute,
    BankAccess,
    PostBank,
    FetchableFirst,
    Blocking,
    Unresolved,
    Actionable,
    MissingCarry,
    Entrance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RouteGoalCopy {
    None,
    First,
    CandidateFallback,
    Missing,
    Fetchable,
    Diagnostics,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RouteRestoreNext {
    MissingRoute,
    AssessRoute,
    Diagnostics,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RouteSearchMode {
    InOrder,
    First,
}

pub(crate) struct RouteJob {
    request: Box<ScriptRouteRequest>,
    thread_name: String,
    kind: Option<RouteDemandKind>,
    // Minted under the service lock that also covers submission.
    identity: Option<WorkIdentity>,
    revision: InputRevision,
    admission_capacity: Option<PublishSequence>,
    lease: NavLease,
    state: Arc<WorldState>,
    basis_avoid: Arc<[AvoidRect]>,
    submission: RouteSubmission,
    preflights: RoutePreflights,
    bank_rows_limit: usize,
    gap_custody: Option<OwnerResultReservation>,
    boundary_ready: bool,
    stage: RouteJobStage,
    target_cursor: Option<RouteTargetCursor>,
    target_sort_started: bool,
    target_priority: Option<RouteTargetPriority>,
    arrival_future: Option<RouteArrivalFuture>,
    targets: Vec<WorldTile>,
    first_preferred: Vec<WorldTile>,
    first_fallback_enabled: bool,
    goal_copy: Vec<WorldTile>,
    fallback_copy: Vec<WorldTile>,
    goal_copy_index: usize,
    fallback_copy_index: usize,
    goal_copy_kind: RouteGoalCopy,
    search_cursor: Option<SearchCursor>,
    search_purpose: RouteSearchPurpose,
    search_avoid: Vec<AvoidRect>,
    search_avoid_index: usize,
    search_avoid_started: bool,
    search_mode: RouteSearchMode,
    target_index: usize,
    excluded_targets: std::collections::HashSet<WorldTile>,
    strict_fallback: Option<CompactRoute>,
    selected_route: Option<CompactRoute>,
    selected_target: Option<WorldTile>,
    missing_requirements: Option<MissingRequirements>,
    planning_cursor: Option<PlanningRowsCursor>,
    planned_rows: Option<Vec<(i32, i32)>>,
    bank_cursor: Option<BankFetchCursor>,
    bank_plan: Option<BankFetchPlan>,
    bank_plan_capture_step: usize,
    bank_apply_reserved: bool,
    bank_undo_reserved: bool,
    bank_plan_apply_step: usize,
    bank_undo: Vec<RouteBankUndo>,
    bank_rollback_index: usize,
    bank_access_route: Option<CompactRoute>,
    fetchable_cursor: Option<FetchableStateCursor>,
    fetchable_edges: Option<FetchableEdgeCursor>,
    fetchable_rows: Option<Vec<(i32, i32)>>,
    fetchable_overlay_ready: bool,
    fetchable_rows_planned: bool,
    restore_next: Option<RouteRestoreNext>,
    bank_avoid_copy: Vec<AvoidRect>,
    bank_avoid_index: usize,
    diagnostics_started: bool,
    diagnostic_settled: usize,
    #[cfg(test)]
    diagnostic_budget: usize,
    budget_settled: Option<usize>,
    outcome: Option<RouteJobOutcome>,
    assessment_future: Option<RouteAssessmentFuture>,
    assessment: Option<Arc<script::combat::risk::RouteAssessment>>,
    refusal: Option<script::native::WalkRefusal>,
    blocked: Box<[ZoneKey]>,
    native_failure: Option<script::native::WalkEnd>,
    detail: Option<String>,
    missing: Vec<MissingCarry>,
    missing_diag_index: usize,
}

#[derive(Clone, Copy)]
struct RouteBankUndo {
    id: i32,
    inventory: Option<i32>,
    worn: bool,
}

impl RouteJob {
    fn prepare(
        name: &str,
        mut request: Box<ScriptRouteRequest>,
        kind: RouteDemandKind,
    ) -> Result<Self, RouteJobError> {
        let state = request
            .state
            .take()
            .unwrap_or_else(|| Arc::new(WorldState::empty()));
        let state = if request.opts.allow_bank_fetch || request.prereq_catalog.is_some() {
            let mut private = match Arc::try_unwrap(state) {
                Ok(state) => state,
                Err(shared) => {
                    super::try_clone_world_state(&shared).map_err(|_| RouteJobError::Allocation)?
                }
            };
            if let Some(catalog) = request.prereq_catalog.as_ref() {
                private.establishable = Some(catalog.establishable_windows());
            }
            Arc::new(private)
        } else {
            state
        };
        let basis_avoid: Arc<[AvoidRect]> = Arc::from(copy_boxed_slice(request.avoid())?);
        let lease = nav_lease(&request.world).map_err(|_| RouteJobError::Budget)?;
        let bank_rows_limit = request.bank.capacity();
        let mut thread_name = String::with_capacity("nav-find-".len() + name.len());
        thread_name.push_str("nav-find-");
        thread_name.push_str(name);
        let (submission, preflights) = RouteSubmission::prepare(
            &request,
            &state,
            &lease,
            bank_rows_limit,
            0,
            thread_name.capacity(),
        )?;
        let missing_capacity = preflights
            .missing
            .reservation()
            .max_missing_carry_ids
            .max(preflights.missing_route.reservation().max_missing_carry_ids);
        let mut missing = Vec::new();
        missing
            .try_reserve_exact(missing_capacity)
            .map_err(|_| RouteJobError::Allocation)?;
        let target_cursor = RouteTargetCursor::new(&request)?;
        let targets = Vec::new();
        let search_mode = if request.candidate_goals.is_some()
            || (request.radius > 0
                && (request.arrival == ArrivalKind::Area
                    || request.live_geometry.is_some()
                    || (request.loc_id.is_none()
                        && !request.world.collision.standable(request.to))))
        {
            RouteSearchMode::First
        } else {
            RouteSearchMode::InOrder
        };
        let excluded_capacity = RouteTargetCursor::capacity(&request)?;
        let mut excluded_targets = std::collections::HashSet::new();
        excluded_targets
            .try_reserve(excluded_capacity)
            .map_err(|_| RouteJobError::Allocation)?;
        let first_preferred = Vec::new();
        let first_fallback_enabled = request.radius > 0
            && request.loc_id.is_none()
            && request.arrival != ArrivalKind::Area
            && request.live_geometry.is_none()
            && !request.world.collision.standable(request.to);
        let mut job = Self {
            revision: submission.revision,
            request,
            thread_name,
            kind: Some(kind),
            identity: None,
            admission_capacity: None,
            lease,
            basis_avoid,
            state,
            submission,
            preflights,
            bank_rows_limit,
            gap_custody: None,
            boundary_ready: false,
            stage: RouteJobStage::Targets,
            target_cursor: Some(target_cursor),
            target_sort_started: false,
            target_priority: None,
            arrival_future: None,
            targets,
            first_preferred,
            first_fallback_enabled,
            goal_copy: Vec::new(),
            fallback_copy: Vec::new(),
            goal_copy_index: 0,
            fallback_copy_index: 0,
            goal_copy_kind: RouteGoalCopy::None,
            search_cursor: None,
            search_purpose: if search_mode == RouteSearchMode::First {
                RouteSearchPurpose::StrictFirst
            } else {
                RouteSearchPurpose::Strict
            },
            search_avoid: Vec::new(),
            search_avoid_index: 0,
            search_avoid_started: false,
            search_mode,
            target_index: 0,
            excluded_targets,
            strict_fallback: None,
            selected_route: None,
            selected_target: None,
            missing_requirements: None,
            planning_cursor: None,
            planned_rows: None,
            bank_cursor: None,
            bank_plan: None,
            bank_plan_capture_step: 0,
            bank_apply_reserved: false,
            bank_plan_apply_step: 0,
            bank_undo_reserved: false,
            bank_undo: Vec::new(),
            bank_rollback_index: 0,
            bank_access_route: None,
            fetchable_cursor: None,
            fetchable_edges: None,
            fetchable_rows: None,
            fetchable_rows_planned: false,
            restore_next: None,
            fetchable_overlay_ready: false,
            bank_avoid_copy: Vec::new(),
            bank_avoid_index: 0,
            diagnostics_started: false,
            diagnostic_settled: 0,
            #[cfg(test)]
            diagnostic_budget: DIAGNOSTIC_BUDGET,
            budget_settled: None,
            outcome: None,
            assessment_future: None,
            assessment: None,
            refusal: None,
            blocked: Box::default(),
            native_failure: None,
            detail: None,
            missing_diag_index: 0,
            missing,
        };
        job.add_owned_input_to_submission()?;
        Ok(job)
    }

    fn submission(&self) -> &RouteSubmission {
        &self.submission
    }

    fn extra_retained_bytes(&self) -> Option<usize> {
        let mut bytes = route_excluded_target_bytes(self.excluded_targets.capacity())?;
        for allocation in [
            route_vec_bytes(&self.targets)?,
            route_vec_bytes(&self.first_preferred)?,
            route_vec_bytes(&self.goal_copy)?,
            route_vec_bytes(&self.fallback_copy)?,
            route_vec_bytes(&self.search_avoid)?,
            route_vec_bytes(&self.bank_avoid_copy)?,
            route_vec_bytes(&self.bank_undo)?,
            route_vec_bytes(&self.missing)?,
            self.blocked
                .len()
                .checked_mul(std::mem::size_of::<ZoneKey>())?,
        ] {
            bytes = bytes.checked_add(allocation)?;
        }
        if let Some(cursor) = &self.target_cursor {
            bytes = bytes.checked_add(route_vec_bytes(&cursor.targets)?)?;
        }
        for route in [
            self.strict_fallback.as_ref(),
            self.selected_route.as_ref(),
            self.bank_access_route.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            bytes = bytes.checked_add(route.try_retained_bytes()?)?;
        }
        if let Some(missing) = &self.missing_requirements {
            bytes = bytes.checked_add(missing.try_retained_bytes()?)?;
        }
        for rows in [self.planned_rows.as_ref(), self.fetchable_rows.as_ref()]
            .into_iter()
            .flatten()
        {
            bytes = bytes.checked_add(route_vec_bytes(rows)?)?;
        }
        if let Some(cursor) = &self.planning_cursor {
            bytes = bytes.checked_add(cursor.retained_bytes())?;
        }
        if let Some(cursor) = &self.bank_cursor {
            bytes = bytes.checked_add(cursor.retained_bytes())?;
        }
        if let Some(plan) = &self.bank_plan {
            bytes = bytes.checked_add(plan.retained_bytes())?;
        }
        if let Some(cursor) = &self.fetchable_cursor {
            bytes = bytes.checked_add(cursor.retained_bytes()?)?;
        }
        if let Some(edges) = &self.fetchable_edges {
            bytes = bytes.checked_add(edges.retained_bytes())?;
        }
        if let Some(detail) = &self.detail {
            bytes = bytes.checked_add(detail.capacity())?;
        }
        if let Some(script::native::WalkEnd::NeedsEvidence(gates)) = &self.native_failure {
            bytes =
                bytes
                    .checked_add(std::mem::size_of_val(gates.as_ref()).checked_add(
                        std::mem::size_of::<[std::sync::atomic::AtomicUsize; 2]>(),
                    )?)?;
        }
        if let Some(script::native::WalkEnd::NeedsStep(gates)) = &self.native_failure {
            bytes =
                bytes
                    .checked_add(std::mem::size_of_val(gates.as_ref()).checked_add(
                        std::mem::size_of::<[std::sync::atomic::AtomicUsize; 2]>(),
                    )?)?;
        }
        match &self.outcome {
            Some(RouteJobOutcome::Route(route)) => {
                bytes = bytes.checked_add(route.try_retained_bytes()?)?;
            }
            Some(RouteJobOutcome::Bank {
                steps,
                route,
                final_route,
            }) => {
                if let Some(route) = route {
                    bytes = bytes.checked_add(route.try_retained_bytes()?)?;
                }
                bytes = bytes.checked_add(final_route.try_retained_bytes()?)?;
                bytes = bytes.checked_add(
                    steps
                        .capacity()
                        .checked_mul(std::mem::size_of::<BankStep>())?,
                )?;
            }
            Some(RouteJobOutcome::NoPath) | None => {}
        }
        Some(bytes)
    }

    fn owned_retained_bytes(&self) -> Option<usize> {
        route_capture_bytes(&self.request, &self.state, self.thread_name.capacity())?
            .checked_add(self.extra_retained_bytes()?)
    }

    fn add_owned_input_to_submission(&mut self) -> Result<(), RouteJobError> {
        let extra = self
            .extra_retained_bytes()
            .and_then(|bytes| bytes.checked_add(OwnerResultReservation::storage_bytes()))
            .ok_or(RouteJobError::Budget)?;
        self.submission.input_bytes = self
            .submission
            .input_bytes
            .checked_add(extra)
            .ok_or(RouteJobError::Budget)?;
        let charge = <RouteWorkSpec as api::work::WorkFamilySpec>::charge(&self.submission);
        if charge.total_bytes().ok_or(RouteJobError::Budget)? > ROUTE_WORK_LIMIT_BYTES {
            return Err(RouteJobError::Budget);
        }
        Ok(())
    }

    fn prepare_continuation(&mut self) -> Result<(), RouteJobError> {
        let revision = self.revision.next().ok_or(RouteJobError::Budget)?;
        let (mut submission, preflights) = RouteSubmission::prepare(
            &self.request,
            &self.state,
            &self.lease,
            self.bank_rows_limit,
            self.submission.plan_item_rows.max(
                self.missing_requirements
                    .as_ref()
                    .map_or(0, MissingRequirements::len),
            ),
            self.thread_name.capacity(),
        )?;
        submission.revision = revision;
        self.revision = revision;
        self.submission = submission;
        self.preflights = preflights;
        self.add_owned_input_to_submission()
    }

    fn generation(&self) -> u64 {
        self.request.generation
    }

    fn request_id(&self) -> u64 {
        self.request.request_id
    }

    fn advance_targets(&mut self) -> bool {
        let Some(mut cursor) = self.target_cursor.take() else {
            return true;
        };
        let finished = if let Some(priority) = self.target_priority.as_mut() {
            priority.advance(&mut cursor.targets)
        } else if cursor.advance(&self.request) {
            if !self.target_sort_started {
                cursor.start_sort();
                self.target_sort_started = true;
                false
            } else if let Some(priority) = RouteTargetPriority::new(&self.request) {
                self.target_priority = Some(priority);
                false
            } else {
                true
            }
        } else {
            false
        };
        if finished {
            self.target_priority = None;
            self.targets = std::mem::take(&mut cursor.targets);
        } else {
            self.target_cursor = Some(cursor);
        }
        finished
    }

    fn start_arrival_filter(&mut self) -> Result<(), RouteJobError> {
        if !arrival_filter_needed(&self.request) {
            return self.prepare_search_targets();
        }
        let collision = &self.request.world.collision;
        let bounds = api::query::ArrivalCollisionBounds::new(
            collision.origin.x,
            collision.origin.z,
            collision.width,
            collision.height,
        )
        .ok_or(RouteJobError::Budget)?;
        let future = route_arrival_filter_future(
            Arc::clone(&self.request.world),
            std::mem::take(&mut self.targets),
            self.request.to,
            self.request.radius,
            bounds,
            crate::admission::route_yield_unit,
        );
        self.arrival_future = Some(Box::pin(future));
        self.stage = RouteJobStage::Arrival;
        Ok(())
    }

    fn prepare_search_targets(&mut self) -> Result<(), RouteJobError> {
        if self.search_mode == RouteSearchMode::First {
            if self.first_fallback_enabled {
                self.first_preferred
                    .try_reserve_exact(4)
                    .map_err(|_| RouteJobError::Allocation)?;
                for stand in api::query::arrival_stands(
                    self.request.to,
                    |stand| self.request.world.collision.standable(stand),
                    |stand| {
                        Some(self.request.world.collision.walkable_word(
                            stand.x,
                            stand.z,
                            stand.level,
                        ) as i32)
                    },
                ) {
                    self.first_preferred.push(stand);
                }
            }
            self.prepare_goal_copy(RouteGoalCopy::First, RouteSearchPurpose::StrictFirst)
        } else {
            self.search_purpose = RouteSearchPurpose::Strict;
            self.stage = RouteJobStage::Search;
            Ok(())
        }
    }

    fn goal_copy_sources(&self, kind: RouteGoalCopy) -> (&[WorldTile], &[WorldTile]) {
        if let Some(goals) = &self.request.candidate_goals {
            return match kind {
                // A safe wider goal must not suppress the preferred tier's
                // own-zone completion. Run candidate tiers separately.
                RouteGoalCopy::First => (goals.preferred(), &[]),
                RouteGoalCopy::Missing | RouteGoalCopy::Fetchable => (
                    goals.preferred(),
                    if self.strict_fallback.is_none() {
                        goals.fallback()
                    } else {
                        &[]
                    },
                ),
                RouteGoalCopy::CandidateFallback => (goals.fallback(), &[]),
                RouteGoalCopy::Diagnostics => (&self.targets, &[]),
                RouteGoalCopy::None => (&[], &[]),
            };
        }
        match kind {
            RouteGoalCopy::None => (&[], &[]),
            RouteGoalCopy::CandidateFallback => unreachable!("candidate request required"),
            RouteGoalCopy::Diagnostics => (&self.targets, &[]),
            RouteGoalCopy::First | RouteGoalCopy::Missing | RouteGoalCopy::Fetchable => {
                let primary = if self.first_fallback_enabled {
                    self.first_preferred.as_slice()
                } else {
                    self.targets.as_slice()
                };
                let fallback = if self.first_fallback_enabled
                    && (kind == RouteGoalCopy::First || self.strict_fallback.is_none())
                {
                    self.targets.as_slice()
                } else {
                    &[]
                };
                (primary, fallback)
            }
        }
    }

    fn prepare_goal_copy(
        &mut self,
        kind: RouteGoalCopy,
        purpose: RouteSearchPurpose,
    ) -> Result<(), RouteJobError> {
        let (primary, fallback) = self.goal_copy_sources(kind);
        let (primary_capacity, fallback_capacity) = (primary.len(), fallback.len());
        self.goal_copy.clear();
        self.goal_copy
            .try_reserve_exact(primary_capacity)
            .map_err(|_| RouteJobError::Allocation)?;
        self.fallback_copy.clear();
        self.fallback_copy
            .try_reserve_exact(fallback_capacity)
            .map_err(|_| RouteJobError::Allocation)?;
        self.goal_copy_index = 0;
        self.fallback_copy_index = 0;
        self.goal_copy_kind = kind;
        self.search_purpose = purpose;
        self.stage = RouteJobStage::CopyTargets;
        Ok(())
    }

    fn advance_goal_copy(&mut self) -> bool {
        if self.goal_copy_kind == RouteGoalCopy::None {
            return true;
        }
        let (primary, fallback) = self.goal_copy_sources(self.goal_copy_kind);
        let primary_target = primary.get(self.goal_copy_index).copied();
        let fallback_target = fallback.get(self.fallback_copy_index).copied();
        let filter_excluded = self.goal_copy_kind == RouteGoalCopy::Fetchable;
        let accepted = |target| {
            (self.request.candidate_goals.is_none()
                || self.request.world.collision.standable(target))
                && (!filter_excluded || !self.excluded_targets.contains(&target))
        };
        if let Some(target) = primary_target {
            let accepted = accepted(target);
            self.goal_copy_index += 1;
            if accepted {
                self.goal_copy.push(target);
            }
            return false;
        }
        if let Some(target) = fallback_target {
            let accepted = accepted(target);
            self.fallback_copy_index += 1;
            if accepted {
                self.fallback_copy.push(target);
            }
            return false;
        }
        self.stage = RouteJobStage::Search;
        true
    }

    fn start_search(&mut self) -> Result<bool, RouteJobError> {
        if !self.search_avoid_started {
            self.search_avoid
                .try_reserve_exact(self.request.avoid().len())
                .map_err(|_| RouteJobError::Allocation)?;
            self.search_avoid_index = 0;
            self.search_avoid_started = true;
            if !self.request.avoid().is_empty() {
                self.stage = RouteJobStage::CopyAvoid;
                return Ok(false);
            }
        }
        let avoid = std::mem::take(&mut self.search_avoid).into_boxed_slice();
        self.search_avoid_started = false;
        let options = route_search_options(&self.request);
        let max_settles = if self.diagnostics_started {
            self.diagnostic_budget()
                .saturating_sub(self.diagnostic_settled)
                .min(NODE_BUDGET)
        } else {
            NODE_BUDGET
        };
        let single_preflight = if self.search_purpose == RouteSearchPurpose::PostBank {
            self.preflights.post_bank_single
        } else {
            self.preflights.single
        };
        let search = match self.search_purpose {
            RouteSearchPurpose::Strict => {
                let Some(target) = self.targets.get(self.target_index).copied() else {
                    self.begin_diagnostics();
                    return Ok(true);
                };
                SearchCursor::new_preflighted(
                    SearchRequest::Single {
                        from: self.request.from,
                        to: target,
                        options,
                        avoid,
                        max_settles,
                    },
                    self.lease.clone(),
                    Arc::clone(&self.state),
                    single_preflight,
                )
                .map_err(|_| RouteJobError::Search)?
            }
            RouteSearchPurpose::StrictFirst
            | RouteSearchPurpose::CandidateFallback
            | RouteSearchPurpose::FetchableFirst => SearchCursor::new_preflighted(
                SearchRequest::First {
                    from: self.request.from,
                    targets: std::mem::take(&mut self.goal_copy).into_boxed_slice(),
                    fallback: std::mem::take(&mut self.fallback_copy).into_boxed_slice(),
                    options,
                    avoid,
                    max_settles,
                },
                self.lease.clone(),
                Arc::clone(&self.state),
                self.preflights.first,
            )
            .map_err(|_| RouteJobError::Search)?,
            RouteSearchPurpose::MissingTarget | RouteSearchPurpose::MissingCarry => {
                let target = if self.search_purpose == RouteSearchPurpose::MissingCarry {
                    self.request.to
                } else {
                    self.targets
                        .get(self.target_index)
                        .copied()
                        .ok_or(RouteJobError::ResultShape)?
                };
                SearchCursor::new_preflighted(
                    SearchRequest::MissingItemReqs {
                        from: self.request.from,
                        targets: Box::from([target]),
                        fallback: Box::default(),
                        options,
                        avoid,
                        max_settles,
                    },
                    self.lease.clone(),
                    Arc::clone(&self.state),
                    self.preflights.missing,
                )
                .map_err(|_| RouteJobError::Search)?
            }
            RouteSearchPurpose::MissingTargets => SearchCursor::new_preflighted(
                SearchRequest::MissingItemReqs {
                    from: self.request.from,
                    targets: std::mem::take(&mut self.goal_copy).into_boxed_slice(),
                    fallback: std::mem::take(&mut self.fallback_copy).into_boxed_slice(),
                    options,
                    avoid,
                    max_settles,
                },
                self.lease.clone(),
                Arc::clone(&self.state),
                self.preflights.missing,
            )
            .map_err(|_| RouteJobError::Search)?,
            RouteSearchPurpose::MissingRoute => {
                let route = self
                    .selected_route
                    .take()
                    .ok_or(RouteJobError::ResultShape)?;
                SearchCursor::new_preflighted(
                    SearchRequest::MissingItemReqsForRoute { route },
                    self.lease.clone(),
                    Arc::clone(&self.state),
                    self.preflights.missing_route,
                )
                .map_err(|_| RouteJobError::Search)?
            }
            RouteSearchPurpose::BankAccess => {
                let strict_options = strict_bank_access_options(options);
                let targets = self
                    .bank_cursor
                    .as_mut()
                    .ok_or(RouteJobError::ResultShape)?
                    .take_access_targets();
                if targets.is_empty() {
                    return Err(RouteJobError::ResultShape);
                }
                SearchCursor::new_preflighted(
                    SearchRequest::First {
                        from: self.request.from,
                        targets,
                        fallback: Box::default(),
                        options: strict_options,
                        avoid,
                        max_settles,
                    },
                    self.lease.clone(),
                    Arc::clone(&self.state),
                    self.preflights.bank_access_first,
                )
                .map_err(|_| RouteJobError::Search)?
            }
            RouteSearchPurpose::PostBank => {
                let target = self.selected_target.ok_or(RouteJobError::ResultShape)?;
                let mut strict_options = options;
                strict_options.allow_bank_fetch = false;
                SearchCursor::new_preflighted(
                    SearchRequest::Single {
                        from: self.request.from,
                        to: target,
                        options: strict_options,
                        avoid,
                        max_settles,
                    },
                    self.lease.clone(),
                    Arc::clone(&self.state),
                    single_preflight,
                )
                .map_err(|_| RouteJobError::Search)?
            }
            RouteSearchPurpose::Blocking => {
                let diagnosis = route_blocker_diagnosis(&self.request);
                let target_count = self.targets.len();
                let zone_count = self
                    .request
                    .world
                    .graph
                    .zones
                    .as_ref()
                    .map_or(0, |zones| zones.zones().len());
                let mut endpoints = [self.request.from, self.request.to];
                if target_count == 1 {
                    endpoints[1] = self.targets[0];
                }
                let attribution_endpoints = match diagnosis {
                    BlockerDiagnosis::SingleTarget => endpoints.as_slice(),
                    BlockerDiagnosis::FirstGoal => &endpoints[..1],
                };
                let zone_filter = ZoneFilterOwned::new(
                    zone_count,
                    self.request.admission.input.combat.map(i32::from),
                    attribution_endpoints,
                    options.zones,
                )
                .map_err(|_| RouteJobError::Allocation)?;
                SearchCursor::new_preflighted(
                    SearchRequest::BlockingZones {
                        from: self.request.from,
                        targets: std::mem::take(&mut self.goal_copy).into_boxed_slice(),
                        diagnosis,
                        options,
                        avoid,
                        max_settles,
                        zone_filter,
                    },
                    self.lease.clone(),
                    Arc::clone(&self.state),
                    self.preflights.blocking,
                )
                .map_err(|_| RouteJobError::Search)?
            }
            RouteSearchPurpose::Unresolved => SearchCursor::new_preflighted(
                SearchRequest::UnresolvedQuestGates {
                    from: self.request.from,
                    targets: std::mem::take(&mut self.goal_copy).into_boxed_slice(),
                    options,
                    avoid,
                    max_settles,
                },
                self.lease.clone(),
                Arc::clone(&self.state),
                self.preflights.unresolved,
            )
            .map_err(|_| RouteJobError::Search)?,
            RouteSearchPurpose::Actionable => SearchCursor::new_preflighted(
                SearchRequest::ActionableQuestGates {
                    from: self.request.from,
                    targets: std::mem::take(&mut self.goal_copy).into_boxed_slice(),
                    options,
                    avoid,
                    max_settles,
                    relaxation: nav_planner::router::GateRelaxation::EstablishableFalse,
                },
                self.lease.clone(),
                Arc::clone(&self.state),
                self.preflights.actionable.ok_or(RouteJobError::Search)?,
            )
            .map_err(|_| RouteJobError::Search)?,
            RouteSearchPurpose::Entrance => SearchCursor::new_preflighted(
                SearchRequest::EntranceRequirements {
                    from: self.request.from,
                    targets: std::mem::take(&mut self.goal_copy).into_boxed_slice(),
                    options,
                    avoid,
                    max_settles,
                },
                self.lease.clone(),
                Arc::clone(&self.state),
                self.preflights.entrance,
            )
            .map_err(|_| RouteJobError::Search)?,
        };
        #[cfg(test)]
        route_work_tests::record_search_event(&self.thread_name, self.search_purpose, true);
        self.search_cursor = Some(search);
        Ok(true)
    }

    fn finish_strict_searches(&mut self) {
        self.target_index = self.target_index.saturating_add(1);
        if self.target_index >= self.targets.len() {
            self.begin_diagnostics();
        } else {
            self.schedule_search(RouteSearchPurpose::Strict);
        }
    }

    fn schedule_search(&mut self, purpose: RouteSearchPurpose) {
        self.search_cursor = None;
        self.search_avoid.clear();
        self.search_avoid_index = 0;
        self.search_avoid_started = false;
        self.search_purpose = purpose;
        self.stage = RouteJobStage::Search;
    }

    fn record_budget_stop(&mut self, settled: usize) {
        self.budget_settled = Some(
            self.budget_settled
                .unwrap_or_default()
                .saturating_add(settled),
        );
    }

    fn begin_diagnostics(&mut self) {
        self.outcome = Some(RouteJobOutcome::NoPath);
        self.diagnostics_started = true;
        self.stage = RouteJobStage::BeginDiagnostics;
        self.native_failure = Some(if self.budget_settled.is_some() {
            script::native::WalkEnd::Refused
        } else {
            script::native::WalkEnd::NoPath
        });
    }

    fn diagnostic_budget(&self) -> usize {
        // Diagnosis cannot change the failed strict route. Share one finite
        // allowance across all post-failure searches.
        #[cfg(test)]
        {
            self.diagnostic_budget
        }
        #[cfg(not(test))]
        {
            DIAGNOSTIC_BUDGET
        }
    }

    fn finish_diagnostics_at_budget(&mut self) {
        if let Some(cursor) = self.search_cursor.take() {
            self.diagnostic_settled = self
                .diagnostic_settled
                .saturating_add(cursor.forward_settled());
        }
        let reason = format!(
            "no route found; the route check stopped after {} places before it found the cause",
            self.diagnostic_settled
        );
        self.detail = Some(match self.detail.take() {
            Some(detail) => format!("{detail}; {reason}"),
            None => reason,
        });
        // Preserve any gate, evidence or carry diagnosis that already finished.
        self.stage = RouteJobStage::Diagnostics;
    }

    fn first_search_stopped_on_budget(
        preferred: &Result<CompactRoute, RouteError>,
        fallback: &Option<CompactFallbackRoute>,
    ) -> bool {
        matches!(preferred, Err(RouteError::BudgetExhausted))
            || matches!(
                fallback,
                Some(CompactFallbackRoute::Failed(RouteError::BudgetExhausted))
            )
    }

    fn handle_strict_first_search(
        &mut self,
        preferred: Result<CompactRoute, RouteError>,
        fallback: Option<CompactFallbackRoute>,
        settled: usize,
    ) -> Result<(), RouteJobError> {
        if Self::first_search_stopped_on_budget(&preferred, &fallback) {
            self.record_budget_stop(settled);
        }
        if let Ok(route) = preferred {
            self.outcome = Some(RouteJobOutcome::Route(route));
            self.stage = RouteJobStage::Assess;
            return Ok(());
        }
        if let Some(CompactFallbackRoute::Routed(route)) = fallback {
            self.strict_fallback = Some(route);
        }
        if self
            .request
            .candidate_goals
            .as_ref()
            .is_some_and(|goals| !goals.fallback().is_empty())
        {
            return self.prepare_goal_copy(
                RouteGoalCopy::CandidateFallback,
                RouteSearchPurpose::CandidateFallback,
            );
        }
        self.begin_fetchable_first()
    }

    fn schedule_carry_diagnostics(&mut self) -> Result<(), RouteJobError> {
        if self.budget_settled.is_some() {
            self.prepare_goal_copy(RouteGoalCopy::Diagnostics, RouteSearchPurpose::Entrance)
        } else {
            self.schedule_search(RouteSearchPurpose::MissingCarry);
            Ok(())
        }
    }

    fn handle_search(
        &mut self,
        search: SearchOutcome,
        settled: usize,
    ) -> Result<(), RouteJobError> {
        match (self.search_purpose, search) {
            (RouteSearchPurpose::Strict, SearchOutcome::Single { route }) => match route {
                Ok(route) => {
                    self.outcome = Some(RouteJobOutcome::Route(route));
                    self.stage = RouteJobStage::Assess;
                }
                Err(RouteError::BudgetExhausted) => {
                    self.record_budget_stop(settled);
                    if self.request.opts.allow_bank_fetch {
                        self.schedule_search(RouteSearchPurpose::MissingTarget);
                    } else {
                        self.finish_strict_searches();
                    }
                }
                Err(RouteError::NoPath) if self.request.opts.allow_bank_fetch => {
                    self.schedule_search(RouteSearchPurpose::MissingTarget);
                }
                Err(RouteError::NoPath) => self.finish_strict_searches(),
            },
            (RouteSearchPurpose::StrictFirst, SearchOutcome::First { search }) => {
                let (preferred, fallback) = search.into_routes();
                self.handle_strict_first_search(preferred, fallback, settled)?;
            }
            (RouteSearchPurpose::CandidateFallback, SearchOutcome::First { search }) => {
                let (preferred, fallback) = search.into_routes();
                if preferred.is_err() && Self::first_search_stopped_on_budget(&preferred, &fallback)
                {
                    self.record_budget_stop(settled);
                }
                self.strict_fallback = preferred.ok().or_else(|| match fallback {
                    Some(CompactFallbackRoute::Routed(route)) => Some(route),
                    _ => None,
                });
                self.begin_fetchable_first()?;
            }
            (RouteSearchPurpose::MissingTarget, SearchOutcome::MissingItemReqs(missing)) => {
                if let Some(missing) = missing.filter(|missing| !missing.is_empty()) {
                    self.missing_requirements = Some(missing);
                    self.selected_target = self.targets.get(self.target_index).copied();
                    self.begin_bank_planning()?;
                } else {
                    self.advance_target_after_bank_failure();
                }
            }
            (RouteSearchPurpose::MissingTargets, SearchOutcome::MissingItemReqs(missing)) => {
                if let Some(missing) = missing {
                    self.missing_requirements = Some(missing);
                    self.begin_planning_rows()?;
                } else {
                    self.fetchable_rows_planned = false;
                    self.start_fetchable_overlay()?;
                }
            }
            (
                RouteSearchPurpose::MissingRoute,
                SearchOutcome::MissingItemReqsForRoute { route, missing },
            ) => {
                self.selected_route = Some(route);
                self.missing_requirements = Some(missing);
                match self.missing_requirements.as_ref() {
                    Some(missing) if missing.is_empty() => {
                        let route = self
                            .selected_route
                            .take()
                            .ok_or(RouteJobError::ResultShape)?;
                        self.outcome = Some(RouteJobOutcome::Route(route));
                        self.stage = RouteJobStage::Assess;
                    }
                    Some(_) => {
                        self.selected_route = None;
                        self.begin_bank_planning()?;
                    }
                    None => self.advance_target_after_bank_failure(),
                }
            }
            (RouteSearchPurpose::BankAccess, SearchOutcome::First { search }) => {
                let (route, _) = search.into_routes();
                if let Ok(route) = route {
                    let cursor = self
                        .bank_cursor
                        .as_mut()
                        .ok_or(RouteJobError::ResultShape)?;
                    if !cursor.select_access(route.destination()) {
                        return Err(RouteJobError::ResultShape);
                    }
                    let plan = self
                        .bank_cursor
                        .take()
                        .and_then(|mut cursor| cursor.take_plan())
                        .ok_or(RouteJobError::ResultShape)?;
                    self.bank_plan = Some(plan);
                    self.bank_plan_capture_step = 0;
                    self.bank_access_route = Some(route);
                    self.bank_plan_apply_step = 0;
                    self.stage = RouteJobStage::ApplyBankPlan;
                } else {
                    self.advance_target_after_bank_failure();
                }
            }
            (RouteSearchPurpose::PostBank, SearchOutcome::Single { route }) => {
                if let Ok(final_route) = route {
                    let plan = self.bank_plan.take().ok_or(RouteJobError::ResultShape)?;
                    let route = self.bank_access_route.take();
                    let steps = plan.into_steps().into();
                    self.bank_undo = Vec::new();
                    self.bank_undo_reserved = false;
                    self.bank_apply_reserved = false;
                    self.bank_plan_capture_step = 0;
                    self.planned_rows = None;
                    self.fetchable_rows = None;
                    self.outcome = Some(RouteJobOutcome::Bank {
                        steps,
                        route,
                        final_route,
                    });
                    self.prepare_bank_avoid_copy()?;
                } else {
                    self.advance_target_after_bank_failure();
                }
            }
            (RouteSearchPurpose::FetchableFirst, SearchOutcome::First { search }) => {
                let (preferred, fallback) = search.into_routes();
                if let Ok(route) = preferred {
                    self.selected_target = Some(route.destination());
                    self.selected_route = Some(route);
                    self.start_restore_fetchable(RouteRestoreNext::MissingRoute)?;
                } else if let Some(CompactFallbackRoute::Routed(route)) = fallback {
                    self.selected_target = Some(route.destination());
                    self.selected_route = Some(route);
                    self.start_restore_fetchable(RouteRestoreNext::MissingRoute)?;
                } else if let Some(route) = self.strict_fallback.take() {
                    self.outcome = Some(RouteJobOutcome::Route(route));
                    self.start_restore_fetchable(RouteRestoreNext::AssessRoute)?;
                } else {
                    self.start_restore_fetchable(RouteRestoreNext::Diagnostics)?;
                }
            }
            (RouteSearchPurpose::Blocking, SearchOutcome::BlockingZones(blocked)) => {
                if let Some(keys) = blocked.filter(|keys| !keys.is_empty()) {
                    self.blocked = keys;
                }
                if self.kind.as_ref().is_some_and(|kind| match kind {
                    RouteDemandKind::Walk { authority } => {
                        authority.is_some() || self.request.request_id != 0
                    }
                    RouteDemandKind::Advisory { .. } => false,
                }) {
                    if self.blocked.is_empty() {
                        self.prepare_goal_copy(
                            RouteGoalCopy::Diagnostics,
                            if self.preflights.actionable.is_some() {
                                RouteSearchPurpose::Actionable
                            } else {
                                RouteSearchPurpose::Unresolved
                            },
                        )?;
                    } else {
                        self.native_failure = Some(script::native::WalkEnd::Refused);
                        self.stage = RouteJobStage::Diagnostics;
                    }
                } else {
                    self.stage = RouteJobStage::Diagnostics;
                }
            }
            (RouteSearchPurpose::Actionable, SearchOutcome::ActionableQuestGates(result)) => {
                match result {
                    Some(gates) if !gates.is_empty() => {
                        self.native_failure =
                            Some(script::native::WalkEnd::NeedsStep(Arc::from(gates)));
                        self.schedule_search(RouteSearchPurpose::MissingCarry);
                    }
                    _ => {
                        self.prepare_goal_copy(
                            RouteGoalCopy::Diagnostics,
                            RouteSearchPurpose::Unresolved,
                        )?;
                    }
                }
            }
            (RouteSearchPurpose::Unresolved, SearchOutcome::UnresolvedQuestGates(result)) => {
                self.native_failure = Some(match result {
                    Some(gates) if !gates.is_empty() => {
                        script::native::WalkEnd::NeedsEvidence(Arc::from(gates))
                    }
                    _ if self.budget_settled.is_some() => script::native::WalkEnd::Refused,
                    _ => script::native::WalkEnd::NoPath,
                });
                self.stage = RouteJobStage::Diagnostics;
            }
            (RouteSearchPurpose::MissingCarry, SearchOutcome::MissingItemReqs(missing)) => {
                self.missing_requirements = missing;
                self.missing_diag_index = 0;
                if self
                    .missing_requirements
                    .as_ref()
                    .is_some_and(|requirements| !requirements.is_empty())
                {
                    // A concrete item shortfall is actionable by both the user
                    // and compat recovery. Broader searches add no useful cause.
                    self.stage = RouteJobStage::Diagnostics;
                } else {
                    self.prepare_goal_copy(
                        RouteGoalCopy::Diagnostics,
                        RouteSearchPurpose::Entrance,
                    )?;
                }
            }
            (RouteSearchPurpose::Entrance, SearchOutcome::EntranceRequirements(requirement)) => {
                if let Some(requirement) = requirement {
                    self.native_failure = Some(script::native::WalkEnd::Blocked);
                    self.detail = Some(requirement.to_string());
                    self.stage = RouteJobStage::Diagnostics;
                } else {
                    self.prepare_goal_copy(
                        RouteGoalCopy::Diagnostics,
                        RouteSearchPurpose::Blocking,
                    )?;
                }
            }
            _ => return Err(RouteJobError::ResultShape),
        }
        Ok(())
    }

    fn begin_fetchable_first(&mut self) -> Result<(), RouteJobError> {
        if !self.request.opts.allow_bank_fetch {
            if let Some(route) = self.strict_fallback.take() {
                self.outcome = Some(RouteJobOutcome::Route(route));
                self.stage = RouteJobStage::Assess;
            } else {
                self.begin_diagnostics();
            }
            return Ok(());
        }
        if self.request.bank_origin == Origin::Hint {
            self.prepare_goal_copy(RouteGoalCopy::Missing, RouteSearchPurpose::MissingTargets)?;
        } else {
            self.start_fetchable_overlay()?;
        }
        Ok(())
    }

    fn plan_item_limit(&self) -> usize {
        self.preflights
            .missing
            .reservation()
            .max_plan_item_rows
            .max(
                self.preflights
                    .missing_route
                    .reservation()
                    .max_plan_item_rows,
            )
    }

    fn begin_planning_rows(&mut self) -> Result<(), RouteJobError> {
        self.planning_cursor = Some(
            PlanningRowsCursor::with_limits(self.request.bank.len(), self.plan_item_limit())
                .ok_or(RouteJobError::Budget)?,
        );
        self.stage = RouteJobStage::PlanRows;
        Ok(())
    }

    fn start_fetchable_overlay(&mut self) -> Result<(), RouteJobError> {
        let rows = self
            .fetchable_rows
            .take()
            .unwrap_or_else(|| std::mem::take(&mut self.request.bank));
        let edges = FetchableEdgeCursor::new(
            &self.request.world.graph,
            route_search_options(&self.request).allow_teleports,
        )
        .map_err(|_| RouteJobError::Allocation)?;
        let cursor =
            FetchableStateCursor::new(&self.state, rows, !self.request.world.banks().is_empty())
                .map_err(|_| RouteJobError::Allocation)?;
        self.fetchable_cursor = Some(cursor);
        self.fetchable_edges = Some(edges);
        self.fetchable_overlay_ready = false;
        self.stage = RouteJobStage::FetchableEdges;
        Ok(())
    }

    fn begin_bank_planning(&mut self) -> Result<(), RouteJobError> {
        if self.request.bank_origin == Origin::Hint {
            self.begin_planning_rows()
        } else {
            self.start_bank_cursor()
        }
    }

    fn start_bank_cursor(&mut self) -> Result<(), RouteJobError> {
        if self.missing_requirements.is_none() {
            self.advance_target_after_bank_failure();
            return Ok(());
        }
        let bank_rows = self
            .planned_rows
            .as_ref()
            .map_or(self.request.bank.len(), Vec::len);
        self.bank_cursor = Some(
            BankFetchCursor::with_limits(
                bank_rows,
                self.plan_item_limit(),
                self.request.world.banks().len(),
            )
            .ok_or(RouteJobError::Budget)?,
        );
        self.bank_plan_capture_step = 0;
        self.bank_undo_reserved = false;
        self.bank_plan_apply_step = 0;
        self.bank_apply_reserved = false;
        self.bank_undo.clear();
        self.bank_access_route = None;
        self.stage = RouteJobStage::BankPlan;
        Ok(())
    }

    fn start_restore_fetchable(&mut self, next: RouteRestoreNext) -> Result<(), RouteJobError> {
        self.restore_next = Some(next);
        if let Some(cursor) = self.fetchable_cursor.as_mut() {
            if !cursor.begin_restore() {
                return Err(RouteJobError::ResultShape);
            }
            self.stage = RouteJobStage::RestoreFetchable;
        } else {
            self.finish_fetchable_restore()?;
        }
        Ok(())
    }

    fn finish_fetchable_restore(&mut self) -> Result<(), RouteJobError> {
        if let Some(cursor) = self.fetchable_cursor.as_mut() {
            let rows = cursor.take_rows().ok_or(RouteJobError::ResultShape)?;
            if self.fetchable_rows_planned {
                self.fetchable_rows = Some(rows);
            } else {
                self.request.bank = rows;
            }
        }
        self.fetchable_cursor = None;
        self.fetchable_edges = None;
        self.fetchable_overlay_ready = false;
        match self.restore_next.take() {
            Some(RouteRestoreNext::MissingRoute) => {
                self.schedule_search(RouteSearchPurpose::MissingRoute);
            }
            Some(RouteRestoreNext::AssessRoute) => {
                self.stage = RouteJobStage::Assess;
            }
            Some(RouteRestoreNext::Diagnostics) | None => self.begin_diagnostics(),
        }
        Ok(())
    }

    fn prepare_bank_avoid_copy(&mut self) -> Result<(), RouteJobError> {
        self.bank_avoid_copy
            .try_reserve_exact(self.request.avoid().len())
            .map_err(|_| RouteJobError::Allocation)?;
        self.bank_avoid_index = 0;
        self.stage = if self.request.avoid().is_empty() {
            RouteJobStage::Assess
        } else {
            RouteJobStage::CopyBankAvoid
        };
        Ok(())
    }

    fn advance_target_after_bank_failure(&mut self) {
        if !self.bank_undo.is_empty() {
            self.bank_rollback_index = self.bank_undo.len();
            self.stage = RouteJobStage::RollbackBankState;
            return;
        }
        self.finish_bank_failure();
    }

    fn finish_bank_failure(&mut self) {
        self.missing_requirements = None;
        self.planning_cursor = None;
        self.planned_rows = None;
        self.bank_cursor = None;
        self.bank_plan = None;
        self.bank_plan_capture_step = 0;
        self.bank_plan_apply_step = 0;
        self.bank_apply_reserved = false;
        self.bank_undo_reserved = false;
        self.bank_rollback_index = 0;
        self.bank_undo.clear();
        self.bank_access_route = None;
        self.selected_route = None;
        if self.search_mode == RouteSearchMode::InOrder {
            self.selected_target = None;
            self.target_index = self.target_index.saturating_add(1);
            if self.target_index < self.targets.len() {
                self.schedule_search(RouteSearchPurpose::Strict);
            } else {
                self.begin_diagnostics();
            }
        } else {
            if let Some(target) = self.selected_target.take() {
                self.excluded_targets.insert(target);
            }
            if let Err(error) = self.start_fetchable_overlay() {
                self.detail = Some(error.to_string());
                self.begin_diagnostics();
            }
        }
    }

    fn start_assessment(&mut self) -> Result<(), RouteJobError> {
        let outcome = self.outcome.take().unwrap_or(RouteJobOutcome::NoPath);
        let future = assess_route_outcome_cooperative(
            Arc::clone(&self.request.world),
            self.lease.clone(),
            self.request.admission,
            outcome,
        );
        if std::mem::size_of_val(&future) != self.submission.assessment_future_bytes {
            return Err(RouteJobError::ResultShape);
        }
        self.assessment_future = Some(Box::pin(future));
        Ok(())
    }

    fn search_result_kind(&self) -> RouteSearchResultKind {
        if let Some(settled) = self.budget_settled {
            RouteSearchResultKind::BudgetExhausted {
                settled: Some(settled),
            }
        } else if !self.blocked.is_empty()
            || matches!(
                self.native_failure.as_ref(),
                Some(script::native::WalkEnd::Blocked)
            )
        {
            RouteSearchResultKind::Danger
        } else if !self.missing.is_empty() {
            RouteSearchResultKind::Supply
        } else if self.refusal.is_some()
            || matches!(
                self.native_failure.as_ref(),
                Some(script::native::WalkEnd::Refused | script::native::WalkEnd::NeedsEvidence(_))
            )
        {
            RouteSearchResultKind::Refusal
        } else {
            RouteSearchResultKind::NoPath
        }
    }

    fn prereq_content_for_route(&self, route: &CompactRoute) -> Option<Arc<RoutePrereqContent>> {
        let catalog = self.request.prereq_catalog.as_ref()?;
        let run = self.state.quest_evidence.as_ref()?.required_after().run;
        let route = nav_planner::producer::compact_route_view(&self.lease, route).ok()?;
        let gates = nav_planner::router::route_establishable_quest_gates(route, &self.state)?;
        route_prereq_content(catalog, run, gates)
    }

    fn finish(&mut self) -> RouteJobOutput {
        let outcome = self.outcome.take().unwrap_or(RouteJobOutcome::NoPath);
        if let Some(settled) = self.budget_settled {
            if matches!(&outcome, RouteJobOutcome::NoPath)
                && matches!(
                    self.refusal.as_ref(),
                    None | Some(script::native::WalkRefusal::NoRouteWithinBounds { .. })
                )
            {
                self.refusal = Some(script::native::WalkRefusal::SearchIncomplete {
                    nodes_settled: Some(settled),
                });
            }
        }
        // A relaxed missing-item search can run after a strict budget stop.
        // Its requirements do not prove that a supply-gated route exists.
        if self.budget_settled.is_some() {
            self.missing.clear();
        }
        let search_result =
            matches!(&outcome, RouteJobOutcome::NoPath).then(|| self.search_result_kind());
        let (prereq_content, final_prereq_content) = match &outcome {
            RouteJobOutcome::Route(route) => (self.prereq_content_for_route(route), None),
            RouteJobOutcome::Bank {
                route, final_route, ..
            } => {
                let final_content = self.prereq_content_for_route(final_route);
                let current_content = route
                    .as_ref()
                    .map(|route| self.prereq_content_for_route(route))
                    .unwrap_or_else(|| final_content.clone());
                (current_content, final_content)
            }
            RouteJobOutcome::NoPath => (None, None),
        };
        let mut detail = self.detail.take();
        if let Some(search_result) = search_result {
            let search_detail = format!("search_result={search_result}");
            detail = Some(match detail {
                Some(detail) => format!("{search_detail}; {detail}"),
                None => search_detail,
            });
        }
        let final_goal = match &outcome {
            RouteJobOutcome::Route(route)
            | RouteJobOutcome::Bank {
                final_route: route, ..
            } => Some(route.destination()),
            RouteJobOutcome::NoPath => None,
        };
        if let (Some(goals), Some(goal)) = (&self.request.candidate_goals, final_goal) {
            let tier = if goals.preferred().contains(&goal) {
                "preferred"
            } else {
                "fallback"
            };
            detail = Some(format!("candidate_goal_tier={tier}"));
        }
        if matches!(&outcome, RouteJobOutcome::NoPath) {
            let mut route_detail = if self.blocked.is_empty() {
                self.request
                    .world
                    .graph
                    .zones
                    .is_none()
                    .then(|| crate::walk_map::LEGACY_ZONES_DETAIL.to_owned())
            } else {
                self.request.world.graph.zones.as_ref().map(|table| {
                    format!(
                        "{}; {SCRIPT_DANGER_HINT}",
                        blocked_zone_detail(table, &self.blocked)
                    )
                })
            };
            if self.refusal.is_some() {
                route_detail = self
                    .assessment
                    .as_ref()
                    .map(|assessment| assessment.reason.to_string())
                    .or(route_detail);
            }
            if self
                .kind
                .as_ref()
                .is_some_and(|kind| matches!(kind, RouteDemandKind::Walk { authority: Some(_) }))
            {
                let context = format!(
                    "native walk route failed: {:?} arrival from ({}, {}, {}) to ({}, {}, {}) within radius {}",
                    self.request.arrival,
                    self.request.from.x,
                    self.request.from.z,
                    self.request.from.level,
                    self.request.to.x,
                    self.request.to.z,
                    self.request.to.level,
                    self.request.radius
                );
                route_detail = Some(match route_detail {
                    Some(detail) => format!("{detail}; {context}"),
                    None => context,
                });
            }
            if self.budget_settled.is_none() && !self.missing.is_empty() {
                if let Some(shortfall) = crate::walk_map::route_supply_shortfall_detail(
                    &self.request.world,
                    &self.state,
                    self.missing.iter().map(|row| (row.id, row.count)),
                ) {
                    let shortfall =
                        crate::walk_map::ActionError::InsufficientItems { detail: shortfall }
                            .to_string();
                    route_detail = Some(match route_detail {
                        Some(nav_detail) => format!("{shortfall}; {nav_detail}"),
                        None => shortfall,
                    });
                }
            }
            if let Some(route_detail) = route_detail {
                detail = Some(match detail {
                    Some(detail) => format!("{detail}; {route_detail}"),
                    None => route_detail,
                });
            }
        }
        if matches!(&outcome, RouteJobOutcome::NoPath) && detail.is_none() {
            detail = Some(format!(
                "no route found from ({}, {}, {}) to ({}, {}, {}) within radius {}",
                self.request.from.x,
                self.request.from.z,
                self.request.from.level,
                self.request.to.x,
                self.request.to.z,
                self.request.to.level,
                self.request.radius
            ));
        }
        RouteJobOutput {
            candidate_goals: self.request.candidate_goals.is_some(),
            kind: self.kind.take().expect("route demand kind is present"),
            completion: std::mem::take(&mut self.request.completion),
            identity: self
                .identity
                .expect("executed route has an admitted identity"),
            revision: self.revision,
            generation: self.request.generation,
            request_id: self.request.request_id,
            lease: Some(self.lease.clone()),
            basis_avoid: Arc::clone(&self.basis_avoid),
            opts: self.request.opts,
            compat_v1: self.request.admission.compat_v1,
            quest_evidence: self.state.quest_evidence.clone(),
            prereq_content,
            final_prereq_content,
            outcome,
            search_result,
            assessment: self.assessment.take(),
            refusal: self.refusal.take(),
            blocked: std::mem::take(&mut self.blocked),
            native_failure: self.native_failure.take(),
            detail,
            missing: std::mem::take(&mut self.missing),
            bank_avoid_copy: std::mem::take(&mut self.bank_avoid_copy),
        }
    }
}

impl CooperativeJob for RouteJob {
    type Output = RouteJobOutput;
    type Error = RouteJobError;
    type Phase = nav_planner::contract::RoutePhase;

    fn step(
        &mut self,
        ctx: &mut StepContext<'_, Self::Phase>,
    ) -> StepOutcome<Self::Output, Self::Error> {
        match ctx.checkpoint() {
            Checkpoint::Continue => {}
            Checkpoint::Yield | Checkpoint::Cancel(_) => return StepOutcome::Yielded,
        }
        loop {
            match ctx.checkpoint() {
                Checkpoint::Continue => {}
                Checkpoint::Yield | Checkpoint::Cancel(_) => return StepOutcome::Yielded,
            }
            match self.stage {
                RouteJobStage::Targets => {
                    let finished = self.advance_targets();
                    if finished {
                        if let Err(error) = self.start_arrival_filter() {
                            return StepOutcome::Failed(error);
                        }
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::Arrival => {
                    let Some(future) = self.arrival_future.as_mut() else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    let mut poll_context = Context::from_waker(Waker::noop());
                    match future.as_mut().poll(&mut poll_context) {
                        Poll::Ready(targets) => {
                            self.targets = targets;
                            self.arrival_future = None;
                            if let Err(error) = self.prepare_search_targets() {
                                return StepOutcome::Failed(error);
                            }
                        }
                        Poll::Pending => match ctx.consume_unit() {
                            Checkpoint::Continue => {}
                            Checkpoint::Yield | Checkpoint::Cancel(_) => {
                                return StepOutcome::Yielded;
                            }
                        },
                    }
                }
                RouteJobStage::CopyTargets => {
                    if self.advance_goal_copy() {
                        continue;
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::CopyAvoid => {
                    if let Some(&avoid) = self.request.avoid().get(self.search_avoid_index) {
                        self.search_avoid.push(avoid);
                        self.search_avoid_index += 1;
                    } else {
                        self.stage = RouteJobStage::Search;
                        continue;
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::PlanRows => {
                    let done = {
                        let Some(cursor) = self.planning_cursor.as_mut() else {
                            return StepOutcome::Failed(RouteJobError::ResultShape);
                        };
                        let Some(missing) = self.missing_requirements.as_ref() else {
                            return StepOutcome::Failed(RouteJobError::ResultShape);
                        };
                        cursor.advance(&self.request.bank, missing)
                    };
                    if done {
                        let Some(rows) = self
                            .planning_cursor
                            .take()
                            .and_then(|mut cursor| cursor.take_rows())
                        else {
                            return StepOutcome::Failed(RouteJobError::ResultShape);
                        };
                        if self.search_purpose == RouteSearchPurpose::MissingTargets {
                            self.fetchable_rows = Some(rows);
                            self.fetchable_rows_planned = true;
                            if let Err(error) = self.start_fetchable_overlay() {
                                return StepOutcome::Failed(error);
                            }
                        } else {
                            self.planned_rows = Some(rows);
                            if let Err(error) = self.start_bank_cursor() {
                                return StepOutcome::Failed(error);
                            }
                        }
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::BankPlan => {
                    let done = {
                        let Some(cursor) = self.bank_cursor.as_mut() else {
                            return StepOutcome::Failed(RouteJobError::ResultShape);
                        };
                        let bank = self
                            .planned_rows
                            .as_deref()
                            .unwrap_or(self.request.bank.as_slice());
                        let Some(missing) = self.missing_requirements.as_ref() else {
                            return StepOutcome::Failed(RouteJobError::ResultShape);
                        };
                        cursor.advance(
                            missing,
                            &self.state,
                            bank,
                            self.request.world.banks(),
                            self.request.from,
                            &self.request.world.collision,
                        )
                    };
                    if done {
                        let has_access = self
                            .bank_cursor
                            .as_ref()
                            .and_then(BankFetchCursor::planned_access)
                            .is_some();
                        if has_access {
                            self.schedule_search(RouteSearchPurpose::BankAccess);
                        } else if let Some(plan) = self
                            .bank_cursor
                            .take()
                            .and_then(|mut cursor| cursor.take_plan())
                        {
                            self.bank_plan = Some(plan);
                            self.bank_plan_capture_step = 0;
                            self.stage = RouteJobStage::ApplyBankPlan;
                        } else {
                            self.bank_cursor = None;
                            self.advance_target_after_bank_failure();
                        }
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::FetchableEdges => {
                    if !self.fetchable_overlay_ready {
                        let captured = {
                            let Some(edges) = self.fetchable_edges.as_mut() else {
                                return StepOutcome::Failed(RouteJobError::ResultShape);
                            };
                            edges.capture_strict(&self.request.world.graph, &self.state)
                        };
                        if captured {
                            self.stage = RouteJobStage::FetchableState;
                        }
                    } else {
                        let opened = {
                            let Some(edges) = self.fetchable_edges.as_mut() else {
                                return StepOutcome::Failed(RouteJobError::ResultShape);
                            };
                            edges.opens_next(&self.request.world.graph, &self.state)
                        };
                        match opened {
                            Some(true) => {
                                if let Err(error) = self.prepare_goal_copy(
                                    RouteGoalCopy::Fetchable,
                                    RouteSearchPurpose::FetchableFirst,
                                ) {
                                    return StepOutcome::Failed(error);
                                }
                            }
                            Some(false) => {
                                let next = if let Some(route) = self.strict_fallback.take() {
                                    self.outcome = Some(RouteJobOutcome::Route(route));
                                    RouteRestoreNext::AssessRoute
                                } else {
                                    RouteRestoreNext::Diagnostics
                                };
                                if let Err(error) = self.start_restore_fetchable(next) {
                                    return StepOutcome::Failed(error);
                                }
                            }
                            None => {}
                        }
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::FetchableState => {
                    let Some(cursor) = self.fetchable_cursor.as_mut() else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    let Some(state) = Arc::get_mut(&mut self.state) else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    match cursor.advance(state) {
                        Ok(true) => {
                            self.fetchable_overlay_ready = true;
                            self.stage = RouteJobStage::FetchableEdges;
                        }
                        Ok(false) => {}
                        Err(_) => return StepOutcome::Failed(RouteJobError::Allocation),
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::RestoreFetchable => {
                    let Some(cursor) = self.fetchable_cursor.as_mut() else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    let Some(state) = Arc::get_mut(&mut self.state) else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    let restored = cursor.restore_next(state);
                    if restored {
                        if let Err(error) = self.finish_fetchable_restore() {
                            return StepOutcome::Failed(error);
                        }
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::ApplyBankPlan => {
                    if !self.bank_undo_reserved {
                        let missing_capacity = self.submission.plan_item_rows;
                        let Some(capacity) = missing_capacity.checked_mul(2) else {
                            return StepOutcome::Failed(RouteJobError::Budget);
                        };
                        if self.bank_undo.try_reserve_exact(capacity).is_err() {
                            return StepOutcome::Failed(RouteJobError::Allocation);
                        }
                        self.bank_undo_reserved = true;
                    }
                    let Some(plan) = self.bank_plan.as_ref() else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    if let Some(step) = plan.steps().get(self.bank_plan_capture_step) {
                        let id = match step {
                            BankStep::Withdraw { id, .. }
                            | BankStep::WithdrawXAmount { id, .. }
                            | BankStep::Wear { id } => Some(*id),
                            _ => None,
                        };
                        if let Some(id) = id {
                            self.bank_undo.push(RouteBankUndo {
                                id,
                                inventory: self.state.inv.get(&id).copied(),
                                worn: self.state.worn.contains(&id),
                            });
                        }
                        self.bank_plan_capture_step += 1;
                        match ctx.consume_unit() {
                            Checkpoint::Continue => {}
                            Checkpoint::Yield | Checkpoint::Cancel(_) => {
                                return StepOutcome::Yielded;
                            }
                        }
                        continue;
                    }
                    if !self.bank_apply_reserved {
                        let missing_capacity = self.submission.plan_item_rows;
                        let Some(state) = Arc::get_mut(&mut self.state) else {
                            return StepOutcome::Failed(RouteJobError::ResultShape);
                        };
                        if state.inv.try_reserve(missing_capacity).is_err()
                            || state.worn.try_reserve(missing_capacity).is_err()
                        {
                            return StepOutcome::Failed(RouteJobError::Allocation);
                        }
                        self.bank_apply_reserved = true;
                    }
                    let Some(state) = Arc::get_mut(&mut self.state) else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    let Some(plan) = self.bank_plan.as_mut() else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    if plan.apply_next(state) {
                        self.schedule_search(RouteSearchPurpose::PostBank);
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::RollbackBankState => {
                    if self.bank_rollback_index == 0 {
                        self.bank_undo.clear();
                        self.bank_plan = None;
                        self.bank_apply_reserved = false;
                        self.bank_undo_reserved = false;
                        self.bank_plan_capture_step = 0;
                        self.bank_plan_apply_step = 0;
                        self.finish_bank_failure();
                        continue;
                    }
                    self.bank_rollback_index -= 1;
                    let undo = self.bank_undo[self.bank_rollback_index];
                    let Some(state) = Arc::get_mut(&mut self.state) else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    if let Some(count) = undo.inventory {
                        state.inv.insert(undo.id, count);
                    } else {
                        state.inv.remove(&undo.id);
                    }
                    if undo.worn {
                        state.worn.insert(undo.id);
                    } else {
                        state.worn.remove(&undo.id);
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::CopyBankAvoid => {
                    if let Some(&avoid) = self.request.avoid().get(self.bank_avoid_index) {
                        self.bank_avoid_copy.push(avoid);
                        self.bank_avoid_index += 1;
                    } else {
                        self.stage = RouteJobStage::Assess;
                        continue;
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::BeginDiagnostics => {
                    // A concrete item shortfall is the most actionable cause.
                    // Look for it before mainland-wide policy checks.
                    if let Err(error) = self.schedule_carry_diagnostics() {
                        return StepOutcome::Failed(error);
                    }
                    continue;
                }
                RouteJobStage::Diagnostics => {
                    let Some(missing) = self.missing_requirements.as_ref() else {
                        self.stage = RouteJobStage::Assess;
                        continue;
                    };
                    if let Some(MissingReqView::Carry { id, count }) =
                        missing.requirement_at(self.missing_diag_index)
                    {
                        if count > 0 {
                            self.missing.push(MissingCarry { id, count });
                        }
                        self.missing_diag_index += 1;
                    } else {
                        self.stage = RouteJobStage::Assess;
                        continue;
                    }
                    match ctx.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield | Checkpoint::Cancel(_) => {
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::Search => {
                    if self.search_cursor.is_none() {
                        if let Err(error) = self.start_search() {
                            return StepOutcome::Failed(error);
                        }
                        continue;
                    }
                    let step = self
                        .search_cursor
                        .as_mut()
                        .expect("search cursor is present")
                        .advance(ctx);
                    match step {
                        CursorStep::Yielded => {
                            if self.diagnostics_started
                                && self.diagnostic_settled.saturating_add(
                                    self.search_cursor
                                        .as_ref()
                                        .expect("search cursor is present")
                                        .forward_settled(),
                                ) >= self.diagnostic_budget()
                            {
                                self.finish_diagnostics_at_budget();
                                continue;
                            }
                            return StepOutcome::Yielded;
                        }
                        CursorStep::Cancelled(_) => return StepOutcome::Yielded,
                        CursorStep::Failed(_) => return StepOutcome::Failed(RouteJobError::Search),
                        CursorStep::Complete(completion) => {
                            #[cfg(test)]
                            route_work_tests::record_search_event(
                                &self.thread_name,
                                self.search_purpose,
                                false,
                            );
                            if self.diagnostics_started {
                                self.diagnostic_settled = self
                                    .diagnostic_settled
                                    .saturating_add(completion.forward_settled());
                            }
                            self.search_cursor = None;
                            let settled = completion.forward_settled();
                            let (search, _state) = completion.into_parts();
                            if let Err(error) = self.handle_search(search, settled) {
                                return StepOutcome::Failed(error);
                            }
                            if self.diagnostics_started
                                && self.diagnostic_settled >= self.diagnostic_budget()
                                && self.stage != RouteJobStage::Diagnostics
                            {
                                self.finish_diagnostics_at_budget();
                            }
                            self.boundary_ready = true;
                            return StepOutcome::Yielded;
                        }
                    }
                }
                RouteJobStage::Assess => {
                    if self.assessment_future.is_none() {
                        if let Err(error) = self.start_assessment() {
                            return StepOutcome::Failed(error);
                        }
                    }
                    let Some(future) = self.assessment_future.as_mut() else {
                        return StepOutcome::Failed(RouteJobError::ResultShape);
                    };
                    let mut poll_context = Context::from_waker(Waker::noop());
                    match future.as_mut().poll(&mut poll_context) {
                        Poll::Ready(Ok((outcome, assessment, permitted))) => {
                            self.assessment_future = None;
                            self.outcome = Some(if permitted {
                                outcome
                            } else {
                                RouteJobOutcome::NoPath
                            });
                            self.assessment = assessment;
                            if !permitted {
                                self.refusal = Some(
                                    if self
                                        .request
                                        .admission
                                        .escape
                                        .is_some_and(crate::admission::escape_live)
                                    {
                                        script::native::WalkRefusal::EscapeInProgress
                                    } else if let Some(assessment) = self.assessment.as_ref() {
                                        crate::admission::verdict_refusal(assessment.verdict)
                                    } else {
                                        script::native::WalkRefusal::NoRouteWithinBounds {
                                            tried: 0,
                                            last: None,
                                        }
                                    },
                                );
                            }
                            self.stage = RouteJobStage::Complete;
                        }
                        Poll::Ready(Err(error)) => return StepOutcome::Failed(error),
                        Poll::Pending => match ctx.consume_unit() {
                            Checkpoint::Continue => {}
                            Checkpoint::Yield | Checkpoint::Cancel(_) => {
                                return StepOutcome::Yielded;
                            }
                        },
                    }
                }
                RouteJobStage::Complete => match ctx.checkpoint() {
                    Checkpoint::Continue => {
                        return StepOutcome::Completed(self.finish());
                    }
                    Checkpoint::Yield | Checkpoint::Cancel(_) => {
                        return StepOutcome::Yielded;
                    }
                },
            }
        }
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(
            self.submission
                .input_bytes
                .saturating_add(self.submission.scratch_bytes),
        )
    }
}

pub(crate) struct RouteJobOutput {
    candidate_goals: bool,
    kind: RouteDemandKind,
    completion: RouteCompletion,
    identity: WorkIdentity,
    revision: InputRevision,
    generation: u64,
    request_id: u64,
    lease: Option<NavLease>,
    basis_avoid: Arc<[AvoidRect]>,
    opts: FindOptions,
    compat_v1: bool,
    quest_evidence: Option<nav_planner::quest_gates::QuestEvidence>,
    prereq_content: Option<Arc<RoutePrereqContent>>,
    final_prereq_content: Option<Arc<RoutePrereqContent>>,
    outcome: RouteJobOutcome,
    search_result: Option<RouteSearchResultKind>,
    assessment: Option<Arc<script::combat::risk::RouteAssessment>>,
    refusal: Option<script::native::WalkRefusal>,
    blocked: Box<[ZoneKey]>,
    native_failure: Option<script::native::WalkEnd>,
    detail: Option<String>,
    missing: Vec<MissingCarry>,
    bank_avoid_copy: Vec<AvoidRect>,
}

#[expect(
    clippy::large_enum_variant,
    reason = "The bank outcome retains both charged routes without a separate result allocation."
)]
enum RouteJobOutcome {
    Route(CompactRoute),
    Bank {
        steps: VecDeque<BankStep>,
        route: Option<CompactRoute>,
        final_route: CompactRoute,
    },
    NoPath,
}

impl RouteJobOutcome {
    fn current_route(&self) -> Option<&CompactRoute> {
        match self {
            Self::Route(route) => Some(route),
            Self::Bank {
                route, final_route, ..
            } => Some(route.as_ref().unwrap_or(final_route)),
            Self::NoPath => None,
        }
    }
}
#[derive(Debug)]
struct RouteGateGroup {
    leg: u16,
    crossing: api::prereq_steps::CrossingId,
    gates: Arc<[api::prereq_steps::ActionableGate]>,
}

#[derive(Debug, PartialEq, Eq)]
struct RouteRefusalCandidate {
    leg: u16,
    gate: api::selected::QuestGate,
    row: u16,
    crossing_entry: u8,
    page_entry: u8,
    quest: api::selected::FactKey,
    signal: api::selected::FactKey,
    possible: api::selected::InclusiveRange,
    after: api::selected::InclusiveRange,
    refused: api::selected::InclusiveRange,
}

pub(crate) struct RoutePrereqContent {
    pub(crate) run: api::selected::RunKey,
    pub(crate) gates: Arc<[api::prereq_steps::ActionableGate]>,
    gate_groups: Arc<[RouteGateGroup]>,
    pub(crate) crossing_passes: Arc<[nav_executor::CrossingPass]>,
    pub(crate) refusal_texts: Arc<[(api::prereq_steps::CrossingId, Arc<str>)]>,
    refusal_candidates: Arc<[RouteRefusalCandidate]>,
}

impl RoutePrereqContent {
    pub(crate) fn refusal_observation(
        &self,
        leg: usize,
        crossing: api::prereq_steps::CrossingId,
        edge_gates: &[api::selected::QuestGate],
        matched: nav_executor::RefusalMatch,
    ) -> Option<Arc<api::prereq_knowledge::RefusalObservation>> {
        let leg = u16::try_from(leg).ok()?;
        let mut ordinal = 0usize;
        let candidate = self
            .refusal_candidates
            .iter()
            .zip(self.refusal_texts.iter())
            .find_map(|(candidate, (text_crossing, _))| {
                if *text_crossing != crossing {
                    return None;
                }
                let current = ordinal;
                ordinal += 1;
                (current == usize::from(matched.text)).then_some(candidate)
            })?;
        if candidate.leg != leg || !edge_gates.contains(&candidate.gate) {
            return None;
        }
        let group = self
            .gate_groups
            .iter()
            .find(|group| group.leg == leg && group.crossing == crossing)?;
        if !group
            .gates
            .iter()
            .all(|gate| edge_gates.contains(&gate.gate))
        {
            return None;
        }
        Some(Arc::new(api::prereq_knowledge::RefusalObservation {
            crossing,
            leg,
            gates: Arc::clone(&group.gates),
            matched: api::prereq_knowledge::CatalogRef {
                row: candidate.row,
                entry: candidate.crossing_entry,
            },
            fact: api::prereq_knowledge::Fact {
                quest: candidate.quest.clone(),
                signal: candidate.signal.clone(),
                possible: candidate.possible,
                after: candidate.after,
                refused: Some(candidate.refused),
                source: api::prereq_knowledge::FactSource::Refusal {
                    row: candidate.row,
                    entry: candidate.page_entry,
                },
                observed: EvidenceStamp {
                    run: self.run,
                    tick: matched.tick,
                    sequence: matched.sequence,
                },
            },
        }))
    }

    pub(crate) fn retained_bytes(&self) -> Option<usize> {
        let (content_layout, _) = std::alloc::Layout::new::<[std::sync::atomic::AtomicUsize; 2]>()
            .extend(std::alloc::Layout::new::<Self>())
            .ok()?;
        let mut bytes = content_layout.pad_to_align().size();
        for allocation in [
            arc_slice_heap_bytes::<api::prereq_steps::ActionableGate>(self.gates.len())?,
            arc_slice_heap_bytes::<RouteGateGroup>(self.gate_groups.len())?,
            arc_slice_heap_bytes::<nav_executor::CrossingPass>(self.crossing_passes.len())?,
            arc_slice_heap_bytes::<(api::prereq_steps::CrossingId, Arc<str>)>(
                self.refusal_texts.len(),
            )?,
            arc_slice_heap_bytes::<RouteRefusalCandidate>(self.refusal_candidates.len())?,
        ] {
            bytes = bytes.checked_add(allocation)?;
        }
        for group in self.gate_groups.iter() {
            bytes = bytes.checked_add(
                arc_slice_heap_bytes::<api::prereq_steps::ActionableGate>(group.gates.len())?,
            )?;
        }
        Some(bytes)
    }
}

pub(crate) fn route_prereq_content(
    catalog: &api::prereq_steps::PrereqCatalog,
    run: api::selected::RunKey,
    gates: Arc<[api::prereq_steps::ActionableGate]>,
) -> Option<Arc<RoutePrereqContent>> {
    if gates.is_empty() {
        return None;
    }
    // Proved crossings need no repair row. They must not discard refusal
    // metadata for later registered crossings on the same accepted route.
    let mut registered: Option<Vec<api::prereq_steps::ActionableGate>> = None;
    for (index, gate) in gates.iter().enumerate() {
        if catalog.for_journal(&gate.gate).is_some()
            || catalog.for_gate(&gate.gate, &gate.crossing).is_some()
        {
            if let Some(registered) = registered.as_mut() {
                registered.push(gate.clone());
            }
        } else {
            if gate.truth != api::selected::Truth::True {
                return None;
            }
            if registered.is_none() {
                let mut retained = Vec::new();
                retained.try_reserve_exact(gates.len() - 1).ok()?;
                retained.extend_from_slice(&gates[..index]);
                registered = Some(retained);
            }
        }
    }
    let gates = registered.map_or(gates, Arc::from);
    if gates.is_empty() {
        return None;
    }

    let mut grouped = Vec::<(
        u16,
        api::prereq_steps::CrossingId,
        Vec<api::prereq_steps::ActionableGate>,
    )>::new();
    grouped.try_reserve_exact(gates.len()).ok()?;
    for gate in gates.iter() {
        if let Some((leg, crossing, group)) = grouped.last_mut() {
            if *leg == gate.leg && *crossing == gate.crossing {
                group.try_reserve(1).ok()?;
                group.push(gate.clone());
                continue;
            }
        }
        let mut group = Vec::new();
        group.try_reserve_exact(1).ok()?;
        group.push(gate.clone());
        grouped.push((gate.leg, gate.crossing, group));
    }
    let mut gate_groups = Vec::new();
    gate_groups.try_reserve_exact(grouped.len()).ok()?;
    for (leg, crossing, group) in grouped {
        gate_groups.push(RouteGateGroup {
            leg,
            crossing,
            gates: group.into(),
        });
    }

    let mut crossing_passes = Vec::<nav_executor::CrossingPass>::new();
    crossing_passes.try_reserve_exact(gates.len()).ok()?;
    let mut ambiguous_passes = Vec::<api::prereq_steps::CrossingId>::new();
    ambiguous_passes.try_reserve_exact(gates.len()).ok()?;
    let mut refusal_entries = Vec::<(
        api::prereq_steps::CrossingId,
        Arc<str>,
        RouteRefusalCandidate,
    )>::new();
    let mut refusal_conflicts = Vec::<api::prereq_steps::CrossingId>::new();
    refusal_conflicts.try_reserve_exact(gates.len()).ok()?;

    for group in &gate_groups {
        for gate in group.gates.iter() {
            // Journal evidence is acquired before route acceptance. It does
            // not invent a server crossing message or refusal classifier.
            if catalog.for_journal(&gate.gate).is_some() {
                continue;
            }
            let (row_index, row) = catalog.for_gate(&gate.gate, &gate.crossing)?;
            let (crossing_entry, entry) = row.crossing(&gate.crossing)?;
            let pass = nav_executor::CrossingPass {
                crossing: entry.crossing,
                message: Arc::clone(&entry.pass.message),
                landing: entry.pass.landing,
                delay_ticks: entry.delay.ticks,
            };
            if !ambiguous_passes.contains(&gate.crossing) {
                if let Some(existing) = crossing_passes
                    .iter()
                    .find(|existing| existing.crossing == gate.crossing)
                {
                    if existing != &pass {
                        ambiguous_passes.push(gate.crossing);
                    }
                } else {
                    crossing_passes.try_reserve(1).ok()?;
                    crossing_passes.push(pass);
                }
            }

            if refusal_conflicts.contains(&gate.crossing) {
                continue;
            }
            for (page_index, page) in row.probe.refuse.iter().enumerate() {
                let Some(page_entry) = u8::try_from(page_index).ok() else {
                    refusal_conflicts.push(gate.crossing);
                    break;
                };
                let Some(after) = row.after_for(page.proves) else {
                    continue;
                };
                let candidate = RouteRefusalCandidate {
                    leg: gate.leg,
                    gate: gate.gate.clone(),
                    row: row_index,
                    crossing_entry,
                    page_entry,
                    quest: row.establishes.quest.clone(),
                    signal: row.establishes.signal.clone(),
                    possible: page.proves,
                    after,
                    refused: row.establishes.values,
                };
                if let Some((_, _, existing)) =
                    refusal_entries.iter().find(|(crossing, text, _)| {
                        *crossing == gate.crossing && text.as_ref() == page.page.as_ref()
                    })
                {
                    if existing != &candidate {
                        refusal_conflicts.push(gate.crossing);
                        break;
                    }
                    continue;
                }
                let text_count = refusal_entries
                    .iter()
                    .filter(|(crossing, _, _)| *crossing == gate.crossing)
                    .count();
                if text_count > usize::from(u8::MAX) {
                    refusal_conflicts.push(gate.crossing);
                    break;
                }
                refusal_entries.try_reserve(1).ok()?;
                refusal_entries.push((gate.crossing, Arc::clone(&page.page), candidate));
            }
        }
    }

    crossing_passes.retain(|pass| !ambiguous_passes.contains(&pass.crossing));
    refusal_entries.retain(|(crossing, _, _)| !refusal_conflicts.contains(crossing));
    let mut refusal_texts = Vec::new();
    let mut refusal_candidates = Vec::new();
    refusal_texts
        .try_reserve_exact(refusal_entries.len())
        .ok()?;
    refusal_candidates
        .try_reserve_exact(refusal_entries.len())
        .ok()?;
    for (crossing, text, candidate) in refusal_entries {
        refusal_texts.push((crossing, text));
        refusal_candidates.push(candidate);
    }

    Some(Arc::new(RoutePrereqContent {
        run,
        gates,
        gate_groups: gate_groups.into(),
        crossing_passes: crossing_passes.into(),
        refusal_texts: refusal_texts.into(),
        refusal_candidates: refusal_candidates.into(),
    }))
}

fn route_assessment_future_layout_bytes(
    world: &Arc<NavWorld>,
    lease: &NavLease,
    admission: crate::admission::Admission,
) -> Option<usize> {
    let future = assess_route_outcome_cooperative(
        Arc::clone(world),
        lease.clone(),
        admission,
        RouteJobOutcome::NoPath,
    );
    Some(std::mem::size_of_val(&future))
}

async fn assess_route_outcome_cooperative(
    world: Arc<NavWorld>,
    lease: NavLease,
    admission: crate::admission::Admission,
    outcome: RouteJobOutcome,
) -> Result<
    (
        RouteJobOutcome,
        Option<Arc<script::combat::risk::RouteAssessment>>,
        bool,
    ),
    RouteJobError,
> {
    let Some(route) = outcome.current_route() else {
        return Ok((outcome, None, true));
    };
    if admission.escape.is_some_and(crate::admission::escape_live) {
        return Ok((RouteJobOutcome::NoPath, None, false));
    }
    let view = nav_planner::producer::compact_route_view(&lease, route)
        .map_err(|_| RouteJobError::ResultShape)?;
    let mut yield_unit = crate::admission::route_yield_unit;
    let assessment =
        crate::admission::assess_view_cooperative(view, &world, &admission, &mut yield_unit).await;
    let view = nav_planner::producer::compact_route_view(&lease, route)
        .map_err(|_| RouteJobError::ResultShape)?;
    let permitted = crate::admission::permits_view_cooperative(
        view,
        &world,
        &admission,
        &assessment,
        &mut yield_unit,
    )
    .await;
    Ok((outcome, Some(assessment), permitted))
}

enum RouteReservation {
    NotNeeded,
    Full,
    Failed(String),
    Refused(nav_planner::producer::OwnerResultRefusal),
    Reserved(Option<OwnerResultReservation>),
}

fn route_output_retained_bytes(
    output: &RouteJobOutput,
) -> Result<Option<usize>, nav_planner::producer::OwnerResultRefusal> {
    if !matches!(&output.kind, RouteDemandKind::Walk { .. }) {
        return Ok(None);
    }
    let custody_bytes = OwnerResultLedger::owner_result_bytes::<RoutePlanLease>(0)
        .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?;
    let basis_bytes = arc_slice_heap_bytes::<AvoidRect>(output.basis_avoid.len())
        .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?;
    let route_basis_layout = std::alloc::Layout::new::<[std::sync::atomic::AtomicUsize; 2]>()
        .extend(std::alloc::Layout::new::<crate::admission::RouteBasis>())
        .map_err(|_| nav_planner::producer::OwnerResultRefusal::SizeOverflow)?
        .0
        .pad_to_align()
        .size();
    let sidecar_bytes = output
        .prereq_content
        .as_ref()
        .map_or(Some(0), |content| content.retained_bytes())
        .and_then(|current| {
            let final_bytes = match (
                output.prereq_content.as_ref(),
                output.final_prereq_content.as_ref(),
            ) {
                (Some(current), Some(final_content)) if Arc::ptr_eq(current, final_content) => 0,
                (_, Some(final_content)) => final_content.retained_bytes()?,
                (_, None) => 0,
            };
            current.checked_add(final_bytes)
        })
        .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?;
    let retained = match &output.outcome {
        RouteJobOutcome::NoPath => return Ok(None),
        RouteJobOutcome::Route(route) => nav_planner::producer::compact_route_storage_bytes(route)
            .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?
            .checked_add(std::mem::size_of::<RoutePath>())
            .and_then(|bytes| bytes.checked_add(custody_bytes))
            .and_then(|bytes| bytes.checked_add(basis_bytes))
            .and_then(|bytes| bytes.checked_add(route_basis_layout))
            .and_then(|bytes| bytes.checked_add(sidecar_bytes))
            .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?,
        RouteJobOutcome::Bank {
            steps,
            route,
            final_route,
        } => {
            let current = match route {
                Some(route) => nav_planner::producer::compact_route_storage_bytes(route)
                    .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?,
                None => 0,
            };
            let final_route = nav_planner::producer::compact_route_storage_bytes(final_route)
                .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?;
            let shared_custody = if route.is_some() {
                custody_bytes
                    .checked_mul(2)
                    .and_then(|bytes| bytes.checked_sub(OwnerResultReservation::storage_bytes()))
                    .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?
            } else {
                custody_bytes
            };
            let step_bytes = steps
                .capacity()
                .checked_mul(std::mem::size_of::<BankStep>())
                .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?;
            let avoid_bytes = output
                .bank_avoid_copy
                .capacity()
                .checked_mul(std::mem::size_of::<AvoidRect>())
                .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?;
            current
                .checked_add(final_route)
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<RoutePath>()))
                .and_then(|bytes| bytes.checked_add(std::mem::size_of::<PendingBankFetch>()))
                .and_then(|bytes| bytes.checked_add(step_bytes))
                .and_then(|bytes| bytes.checked_add(avoid_bytes))
                .and_then(|bytes| bytes.checked_add(shared_custody))
                .and_then(|bytes| bytes.checked_add(basis_bytes))
                .and_then(|bytes| bytes.checked_add(route_basis_layout))
                .and_then(|bytes| bytes.checked_add(sidecar_bytes))
                .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?
        }
    };
    Ok(Some(retained))
}

fn route_ticket_is_current(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    identity: WorkIdentity,
    revision: InputRevision,
) -> bool {
    let all = navs.lock().expect("navigation state mutex is not poisoned");
    all.get(name).is_some_and(|bot| {
        bot.route_ticket
            .as_ref()
            .is_some_and(|owner| owner.identity == identity && owner.revision == revision)
    })
}

fn take_route_demand(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    capacity: Option<PublishSequence>,
) -> Option<(RouteDemand, Option<Scope>)> {
    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
    let bot = all.get_mut(name)?;
    if bot.route_ticket.is_some() {
        return None;
    }
    let pending = bot.pending_route.as_deref()?;
    if matches!(pending, RouteDemand::Preparing { .. })
        || matches!(pending, RouteDemand::Prepared(job) if capacity.is_some() && job.admission_capacity == capacity)
    {
        return None;
    }
    let scope = match pending {
        RouteDemand::Pending { kind, .. } => bot.route_work_scope(kind),
        RouteDemand::Prepared(job) => job
            .kind
            .as_ref()
            .and_then(|kind| bot.route_work_scope(kind)),
        RouteDemand::Preparing { .. } => None,
    };
    let placeholder = RouteDemand::Preparing {
        generation: pending.generation(),
        request_id: pending.request_id(),
        kind: pending
            .kind()
            .expect("queued route work retains its demand kind"),
    };
    let demand = std::mem::replace(bot.pending_route.as_deref_mut()?, placeholder);
    Some((demand, scope))
}

fn route_demand_placeholder_is_current(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    generation: u64,
    request_id: u64,
) -> bool {
    let all = navs.lock().expect("navigation state mutex is not poisoned");
    all.get(name).is_some_and(|bot| {
        bot.route_ticket.is_none()
            && bot
                .pending_route
                .as_ref()
                .is_some_and(|demand| demand.matches(generation, request_id))
    })
}

fn restore_prepared_route_demand(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    generation: u64,
    request_id: u64,
    job: Box<RouteJob>,
) -> bool {
    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
    let Some(bot) = all.get_mut(name) else {
        return false;
    };
    if bot.route_ticket.is_some()
        || !bot
            .pending_route
            .as_ref()
            .is_some_and(|demand| demand.matches(generation, request_id))
    {
        return false;
    }
    **bot
        .pending_route
        .as_mut()
        .expect("current demand retains its box") = RouteDemand::Prepared(job);
    true
}

fn publish_unsubmitted_route_failure(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    generation: u64,
    request_id: u64,
    kind: RouteDemandKind,
    detail: &str,
) {
    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
    let Some(bot) = all.get_mut(name) else {
        return;
    };
    if !bot
        .pending_route
        .as_ref()
        .is_some_and(|demand| demand.matches(generation, request_id))
        || bot.route_ticket.is_some()
    {
        return;
    }
    bot.pending_route = None;
    match kind {
        RouteDemandKind::Advisory {
            authority,
            evidence,
        } => {
            if bot.advisory_request_id == request_id && authority.live() {
                bot.assess_result = Some(Box::new((
                    authority.clone(),
                    advisory_refusal(&authority, evidence, detail),
                )));
            }
        }
        RouteDemandKind::Walk { authority } => {
            if bot.route_generation != generation
                || bot.walk_request_id != request_id
                || authority.as_ref().is_some_and(|owner| !owner.live())
            {
                return;
            }
            if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
                if bot.armed_outcome_may_publish(request_id) {
                    bot.note_failure(generation, request_id, to, radius, allow_teleports);
                }
            }
            if authority.is_some() {
                bot.native_walk_failure = Some((request_id, script::native::WalkEnd::Refused));
                if bot.walk_outcome_generation == generation
                    && bot.walk_outcome_request_id == request_id
                {
                    bot.walk_outcome_detail = Some(Arc::from(detail));
                }
            }
            bot.requested_route = None;
        }
    }
}

fn compact_route_ticks(lease: &NavLease, route: &CompactRoute) -> u32 {
    nav_planner::producer::compact_route_view(lease, route)
        .ok()
        .and_then(|view| script::combat::risk::RoutePath::new(view).ok())
        .and_then(|path| u32::try_from(path.ticks()).ok())
        .unwrap_or(0)
}

pub(crate) fn pump_route_work(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    let shared = match WorkService::shared() {
        Ok(shared) => shared,
        Err(error) => {
            if let Some((demand, _)) = take_route_demand(navs, name, None) {
                let generation = demand.generation();
                let request_id = demand.request_id();
                if let Some(kind) = demand.kind() {
                    publish_unsubmitted_route_failure(
                        navs,
                        name,
                        generation,
                        request_id,
                        kind,
                        &error.to_string(),
                    );
                }
                demand.signal_completion();
            }
            return;
        }
    };

    pump_route_work_on_service(&shared, navs, name, OwnerResultLedger::shared());
}

fn pump_route_work_on_service(
    shared: &Arc<parking_lot::Mutex<WorkService>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    ledger: &OwnerResultLedger,
) {
    // A completed search can submit its continuation in this same frame.
    // Do not loop: each pass harvests at most one admitted route job.
    harvest_route_terminal(shared, navs, name, ledger);
    let capacity = {
        let service = shared.lock();
        service
            .worker_available()
            .then(|| service.publications().capacity().sequence())
    };
    if let Some((demand, scope)) = take_route_demand(navs, name, capacity) {
        let generation = demand.generation();
        let request_id = demand.request_id();
        let Some(scope) = scope else {
            if let Some(kind) = demand.kind() {
                publish_unsubmitted_route_failure(
                    navs,
                    name,
                    generation,
                    request_id,
                    kind,
                    "compiled slot scope is unavailable",
                );
            }
            demand.signal_completion();
            return;
        };
        let (mut job, kind) = match demand {
            RouteDemand::Pending { request, kind } => {
                let failure_kind = kind.clone();
                match RouteJob::prepare(name, request, kind) {
                    Ok(job) => {
                        let kind = job
                            .kind
                            .as_ref()
                            .expect("prepared route has a demand kind")
                            .clone();
                        (Box::new(job), kind)
                    }
                    Err(error) => {
                        publish_unsubmitted_route_failure(
                            navs,
                            name,
                            generation,
                            request_id,
                            failure_kind,
                            &error.to_string(),
                        );
                        return;
                    }
                }
            }
            RouteDemand::Prepared(job) => {
                let kind = job
                    .kind
                    .as_ref()
                    .expect("prepared route has a demand kind")
                    .clone();
                (job, kind)
            }
            RouteDemand::Preparing { .. } => return,
        };

        if !route_demand_placeholder_is_current(navs, name, generation, request_id) {
            drop(job);
            return;
        }
        let revision = job.revision;
        let input = *job.submission();
        enum Submission {
            Accepted(WorkTicket<RouteWorkSpec>, WorkCancellation<RouteWorkSpec>),
            Full(Box<RouteJob>),
            Refused(String),
        }
        let submission = {
            let mut service = shared.lock();
            let before = service.publications().capacity().sequence();
            let conditions = ProcessConditions::new(service.incarnation());
            match service.mint_identity(scope, kind.origin()) {
                Err(error) => Submission::Refused(error.to_string()),
                Ok(identity) => {
                    job.identity = Some(identity);
                    let job = RouteWorkJob::new(job);
                    match service.try_submit::<RouteWorkSpec, _>(identity, &input, job, &conditions)
                    {
                        Ok(ticket) => match service.cancellation_handle(ticket) {
                            Ok(cancellation) => Submission::Accepted(ticket, cancellation),
                            Err(error) => {
                                let _ = service.abandon(ticket, Cancellation::ScopeInvalidated);
                                Submission::Refused(error.to_string())
                            }
                        },
                        Err(refused) => {
                            let (reason, job) = refused.into_parts();
                            if reason == SubmissionRefusal::Full {
                                let mut job = job.into_job();
                                // A release racing admission must permit another attempt.
                                job.admission_capacity = Some(before);
                                Submission::Full(job)
                            } else {
                                Submission::Refused(reason.to_string())
                            }
                        }
                    }
                }
            }
        };
        match submission {
            Submission::Accepted(ticket, cancellation) => {
                let identity = ticket.identity();
                let mut owner = Some(Box::new(RouteTicketOwner {
                    identity,
                    revision,
                    generation,
                    request_id,
                    kind: kind.clone(),
                    cancellation,
                    active: true,
                }));
                let installed = {
                    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
                    all.get_mut(name).is_some_and(|bot| {
                        if bot.route_ticket.is_some()
                            || !bot
                                .pending_route
                                .as_ref()
                                .is_some_and(|demand| demand.matches(generation, request_id))
                        {
                            return false;
                        }
                        bot.pending_route = None;
                        bot.route_ticket = owner.take();
                        true
                    })
                };
                if !installed {
                    if let Some(mut owner) = owner {
                        owner.abandon(Cancellation::Superseded);
                    }
                }
            }
            Submission::Full(job) => {
                restore_prepared_route_demand(navs, name, generation, request_id, job);
            }
            Submission::Refused(detail) => {
                publish_unsubmitted_route_failure(
                    navs, name, generation, request_id, kind, &detail,
                );
            }
        }
    }
}

fn take_route_ticket_owner(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    identity: WorkIdentity,
    revision: InputRevision,
) -> Option<Box<RouteTicketOwner>> {
    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
    let bot = all.get_mut(name)?;
    if !bot
        .route_ticket
        .as_ref()
        .is_some_and(|owner| owner.identity == identity && owner.revision == revision)
    {
        return None;
    }
    bot.route_ticket.take()
}

fn publish_accepted_route_failure(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    generation: u64,
    request_id: u64,
    kind: RouteDemandKind,
    detail: &str,
) {
    publish_accepted_route_refusal(navs, name, generation, request_id, kind, detail, None);
}

fn publish_accepted_route_refusal(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    generation: u64,
    request_id: u64,
    kind: RouteDemandKind,
    detail: &str,
    refusal: Option<script::native::WalkRefusal>,
) {
    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
    let Some(bot) = all.get_mut(name) else {
        return;
    };
    match kind {
        RouteDemandKind::Advisory {
            authority,
            evidence,
        } => {
            if bot.advisory_request_id == request_id && authority.live() {
                let mut receipt = advisory_refusal(&authority, evidence, detail);
                if let Some(refusal) = refusal {
                    receipt.refusal = Some(refusal);
                }
                bot.assess_result = Some(Box::new((authority.clone(), receipt)));
            }
        }
        RouteDemandKind::Walk { authority } => {
            if bot.route_generation != generation
                || bot.walk_request_id != request_id
                || authority.as_ref().is_some_and(|owner| !owner.live())
            {
                return;
            }
            if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
                if bot.armed_outcome_may_publish(request_id) {
                    bot.note_failure(generation, request_id, to, radius, allow_teleports);
                }
            }
            if let Some(refusal) = refusal {
                bot.risk_refusal = Some(Box::new((request_id, refusal)));
            }
            if authority.is_some() {
                bot.native_walk_failure = Some((request_id, script::native::WalkEnd::Refused));
            }
            if bot.walk_outcome_generation == generation
                && bot.walk_outcome_request_id == request_id
            {
                bot.walk_outcome_detail = Some(Arc::from(detail));
            }
            bot.requested_route = None;
        }
    }
}

pub(crate) fn publish_route_owner_refusal(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    generation: u64,
    request_id: u64,
    kind: RouteDemandKind,
    error: nav_planner::producer::OwnerResultRefusal,
) {
    let refusal = match &error {
        nav_planner::producer::OwnerResultRefusal::TooLarge { requested, maximum } => {
            Some(script::native::WalkRefusal::RouteTooLarge {
                required_bytes: *requested,
                maximum_bytes: *maximum,
            })
        }
        nav_planner::producer::OwnerResultRefusal::Full { .. }
        | nav_planner::producer::OwnerResultRefusal::SizeOverflow => None,
    };
    let detail = refusal
        .as_ref()
        .map_or_else(|| error.to_string(), |refusal| refusal.to_string());
    publish_accepted_route_refusal(navs, name, generation, request_id, kind, &detail, refusal);
}

fn make_compact_route_path(
    route: CompactRoute,
    lease: &NavLease,
    identity: WorkIdentity,
    revision: InputRevision,
    reservation: &OwnerResultReservation,
    prereq_content: Option<Arc<RoutePrereqContent>>,
) -> (RoutePath, Arc<dyn std::any::Any + Send + Sync>) {
    let plan = nav_planner::producer::publish_compact_route(
        lease,
        identity,
        revision,
        route,
        reservation.clone(),
    );
    let custody = Arc::new(reservation.clone().hold(plan.clone()));
    let custody_any: Arc<dyn std::any::Any + Send + Sync> = custody;
    (
        RoutePath::compact(
            plan,
            lease.clone(),
            Arc::clone(&custody_any),
            prereq_content,
        ),
        custody_any,
    )
}

fn apply_route_output(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    mut output: RouteJobOutput,
    reservation: Option<OwnerResultReservation>,
) {
    let _completion = output.completion;
    if let Some(search_result) = output.search_result {
        log_walk_arm_bot(|| {
            format!(
                "route_search search_result={search_result} generation={} request_id={}",
                output.generation, output.request_id
            )
        });
    }
    match output.kind {
        RouteDemandKind::Advisory {
            authority,
            evidence,
        } => {
            if !authority.live() {
                return;
            }
            let lease = output.lease.as_ref();
            let route_ticks = lease
                .zip(output.outcome.current_route())
                .map_or(0, |(lease, route)| compact_route_ticks(lease, route));
            let no_path = matches!(output.outcome, RouteJobOutcome::NoPath);
            let detail = output
                .assessment
                .as_ref()
                .map(|assessment| Arc::clone(&assessment.reason))
                .or_else(|| {
                    no_path.then(|| Arc::from("No route within the request's hard constraints"))
                });
            let detail = match (output.search_result, detail) {
                (Some(search_result), Some(detail)) => Some(Arc::from(format!(
                    "search_result={search_result}; {detail}"
                ))),
                (Some(search_result), None) => {
                    Some(Arc::from(format!("search_result={search_result}")))
                }
                (None, detail) => detail,
            };
            let refusal = output.refusal.take().or_else(|| {
                no_path.then_some(script::native::WalkRefusal::NoRouteWithinBounds {
                    tried: 0,
                    last: None,
                })
            });
            let receipt = script::native::AssessReceipt {
                request_id: authority.request_id().get(),
                evidence,
                assessment: output.assessment.take(),
                refusal,
                detail,
                route_ticks,
            };
            let mut all = navs.lock().expect("navigation state mutex is not poisoned");
            if let Some(bot) = all.get_mut(name) {
                let stored =
                    bot.advisory_request_id == authority.request_id().get() && authority.live();
                log_walk_arm(name, || {
                    format!(
                        "assess-result request_id={} stored={stored} route_ticks={} refusal={:?}",
                        receipt.request_id, receipt.route_ticks, receipt.refusal
                    )
                });
                if stored {
                    bot.assess_result = Some(Box::new((authority, receipt)));
                }
            }
        }
        RouteDemandKind::Walk { authority } => {
            if authority.as_ref().is_some_and(|owner| !owner.live()) {
                return;
            }
            let Some(lease) = output.lease.take() else {
                publish_accepted_route_failure(
                    navs,
                    name,
                    output.generation,
                    output.request_id,
                    RouteDemandKind::Walk { authority },
                    "route job omitted its navigation lease",
                );
                return;
            };
            let mut reservation = reservation;
            let no_path = matches!(&output.outcome, RouteJobOutcome::NoPath);
            let (route_path, pending, custody) = match output.outcome {
                RouteJobOutcome::NoPath => (None, None, None),
                RouteJobOutcome::Route(route) => {
                    let Some(reservation) = reservation.as_ref() else {
                        publish_accepted_route_failure(
                            navs,
                            name,
                            output.generation,
                            output.request_id,
                            RouteDemandKind::Walk { authority },
                            "route result could not obtain owner custody",
                        );
                        return;
                    };
                    let (path, custody) = make_compact_route_path(
                        route,
                        &lease,
                        output.identity,
                        output.revision,
                        reservation,
                        output.prereq_content.take(),
                    );
                    (Some(path), None, Some(custody))
                }
                RouteJobOutcome::Bank {
                    steps,
                    route,
                    final_route,
                } => {
                    let Some(reservation) = reservation.as_ref() else {
                        publish_accepted_route_failure(
                            navs,
                            name,
                            output.generation,
                            output.request_id,
                            RouteDemandKind::Walk { authority },
                            "bank route result could not obtain owner custody",
                        );
                        return;
                    };
                    let (final_path, final_custody) = make_compact_route_path(
                        final_route,
                        &lease,
                        output.identity,
                        output.revision,
                        reservation,
                        output.final_prereq_content.take(),
                    );
                    let (path, custody) = match route {
                        Some(route) => make_compact_route_path(
                            route,
                            &lease,
                            output.identity,
                            output.revision,
                            reservation,
                            output.prereq_content.take(),
                        ),
                        None => (final_path.clone(), final_custody),
                    };
                    let pending = PendingBankFetch {
                        steps,
                        dest: final_path.destination(),
                        opts: output.opts,
                        final_route: final_path,
                        avoid: std::mem::take(&mut output.bank_avoid_copy),
                        progress: Default::default(),
                    };
                    (Some(path), Some(pending), Some(custody))
                }
            };

            let mut all = navs.lock().expect("navigation state mutex is not poisoned");
            let Some(bot) = all.get_mut(name) else {
                return;
            };
            if bot.route_generation != output.generation || bot.walk_request_id != output.request_id
            {
                return;
            }
            let chosen_goal = if output.candidate_goals {
                pending.as_ref().map_or_else(
                    || route_path.as_ref().map(RoutePath::destination),
                    |pending| Some(pending.dest),
                )
            } else {
                None
            };
            if let Some(assessment) = output.assessment.as_ref() {
                crate::admission::log(Some(name), assessment, output.compat_v1);
            }
            bot.risk_refusal = output
                .refusal
                .take()
                .map(|refusal| Box::new((output.request_id, refusal)));
            bot.assessment = output.assessment.take();
            if bot.risk_refusal.is_some() {
                if let Some(assessment) = bot.assessment.as_ref() {
                    bot.last_assessment = Some(Arc::clone(assessment));
                }
            }
            let native_failure = output.native_failure.take().or_else(|| {
                bot.risk_refusal
                    .as_ref()
                    .map(|_| script::native::WalkEnd::Refused)
            });
            let native_failure = if no_path
                && native_failure
                    .as_ref()
                    .is_none_or(|failure| *failure == script::native::WalkEnd::Failed)
            {
                Some(script::native::WalkEnd::NoPath)
            } else {
                native_failure
            };
            let native_refused = native_failure == Some(script::native::WalkEnd::Refused);
            let detail = output.detail.take().map(Arc::from);
            let blocked = std::mem::take(&mut output.blocked);
            if let Some(path) = route_path {
                let Some(custody) = custody else {
                    return;
                };
                if !bot.publish_compact_route(
                    output.generation,
                    output.request_id,
                    output.identity,
                    output.revision,
                    output.opts.allow_teleports,
                    path,
                    pending,
                    custody,
                    output.quest_evidence.clone(),
                ) {
                    return;
                }
                if let Some(goal) = chosen_goal {
                    // Recovery follows the chosen exact goal, not the caller's
                    // ranking tile that may be in another component.
                    if let Some(key) = bot.requested_route.as_mut() {
                        key.0 = goal;
                    }
                }
                if bot.route_basis.is_none() {
                    if let Some(route) = bot.route_path_for_follow() {
                        bot.route_basis = Some(Arc::new(crate::admission::RouteBasis {
                            _route: route,
                            options: output.opts,
                            avoid: Arc::clone(&output.basis_avoid),
                            quest_evidence: output.quest_evidence.clone(),
                        }));
                    }
                }
                bot.admission_pending = true;
                bot.walk_outcome_detail = detail;
                if bot.walk_outcome_generation == output.generation
                    && bot.walk_outcome_request_id == output.request_id
                {
                    bot.native_walk_failure = native_failure.map(|end| (output.request_id, end));
                    bot.native_walk_blocked = if native_refused {
                        (!blocked.is_empty())
                            .then(|| (output.request_id, Arc::<[ZoneKey]>::from(blocked)))
                    } else {
                        None
                    };
                }
            } else {
                bot.publish_compact_route_failure(
                    output.generation,
                    output.request_id,
                    detail,
                    std::mem::take(&mut output.missing),
                    native_failure,
                    blocked,
                );
            }
            drop(reservation.take());
        }
    }
}

fn route_stage_retained_bytes(
    result: &RouteStageResult,
) -> Result<Option<usize>, nav_planner::producer::OwnerResultRefusal> {
    match result {
        RouteStageResult::Complete(output) => route_output_retained_bytes(output),
        RouteStageResult::Continue(job) => job
            .owned_retained_bytes()
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<RouteDemand>()))
            .and_then(|bytes| bytes.checked_add(OwnerResultReservation::storage_bytes()))
            .map(Some)
            .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow),
    }
}

fn retain_route_continuation(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    mut job: Box<RouteJob>,
    previous_identity: WorkIdentity,
    previous_revision: InputRevision,
    reservation: OwnerResultReservation,
) {
    let generation = job.generation();
    let request_id = job.request_id();
    let kind = job
        .kind
        .as_ref()
        .expect("continuation retains its demand kind")
        .clone();
    if job.identity != Some(previous_identity) || job.revision != previous_revision {
        return;
    }
    let preparation = job.prepare_continuation();
    if let Err(error) = preparation {
        drop(job);
        publish_accepted_route_failure(
            navs,
            name,
            generation,
            request_id,
            kind,
            &error.to_string(),
        );
        return;
    }
    job.gap_custody = Some(reservation);
    let mut all = navs.lock().expect("navigation state mutex is not poisoned");
    let Some(bot) = all.get_mut(name) else {
        return;
    };
    if bot.route_ticket.is_some() || bot.pending_route.is_some() {
        return;
    }
    let current = match &kind {
        RouteDemandKind::Walk { authority } => {
            bot.route_generation == generation
                && bot.walk_request_id == request_id
                && authority.as_ref().is_none_or(|authority| authority.live())
        }
        RouteDemandKind::Advisory { authority, .. } => {
            bot.advisory_request_id == request_id && authority.live()
        }
    };
    if current {
        bot.pending_route = Some(Box::new(RouteDemand::Prepared(job)));
    }
}

fn harvest_route_terminal(
    shared: &Arc<parking_lot::Mutex<WorkService>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    ledger: &OwnerResultLedger,
) {
    let (ticket, identity, revision, generation, request_id, kind) = {
        let all = navs.lock().expect("navigation state mutex is not poisoned");
        let Some(owner) = all.get(name).and_then(|bot| bot.route_ticket.as_ref()) else {
            return;
        };
        (
            owner.ticket(),
            owner.identity,
            owner.revision,
            owner.generation,
            owner.request_id,
            owner.kind.clone(),
        )
    };
    let reservation = {
        let service = shared.lock();
        match service.borrow_terminal(&ticket) {
            Ok(None) => return,
            Err(error) => RouteReservation::Failed(error.to_string()),
            Ok(Some(Settlement::Completed(output))) => match route_stage_retained_bytes(output) {
                Err(error) => RouteReservation::Refused(error),
                Ok(None) => RouteReservation::NotNeeded,
                Ok(Some(bytes)) => match ledger.try_reserve_bytes(bytes) {
                    Ok(reservation) => RouteReservation::Reserved(Some(reservation)),
                    Err(nav_planner::producer::OwnerResultRefusal::Full { .. }) => {
                        RouteReservation::Full
                    }
                    Err(error) => RouteReservation::Refused(error),
                },
            },
            Ok(Some(Settlement::Failed(_))) | Ok(Some(Settlement::Cancelled(_))) => {
                RouteReservation::NotNeeded
            }
        }
    };
    if matches!(&reservation, RouteReservation::Full) {
        return;
    }
    if !route_ticket_is_current(navs, name, identity, revision) {
        return;
    }
    let settlement = match shared.lock().consume(&ticket) {
        Ok(settlement) => settlement,
        Err(error) => {
            let owner = take_route_ticket_owner(navs, name, identity, revision);
            drop(owner);
            publish_accepted_route_failure(
                navs,
                name,
                generation,
                request_id,
                kind,
                &error.to_string(),
            );
            return;
        }
    };
    let Some(mut owner) = take_route_ticket_owner(navs, name, identity, revision) else {
        return;
    };
    owner.settled();
    drop(owner);
    match (settlement, reservation) {
        (
            Settlement::Completed(RouteStageResult::Complete(output)),
            RouteReservation::Reserved(reservation),
        ) => apply_route_output(navs, name, output, reservation),
        (
            Settlement::Completed(RouteStageResult::Complete(output)),
            RouteReservation::NotNeeded,
        ) => apply_route_output(navs, name, output, None),
        (
            Settlement::Completed(RouteStageResult::Continue(job)),
            RouteReservation::Reserved(Some(reservation)),
        ) => {
            retain_route_continuation(navs, name, job, identity, revision, reservation);
        }
        (Settlement::Completed(output), RouteReservation::Failed(detail)) => {
            drop(output);
            publish_accepted_route_failure(navs, name, generation, request_id, kind, &detail);
        }
        (settlement, RouteReservation::Refused(error)) => {
            drop(settlement);
            publish_route_owner_refusal(navs, name, generation, request_id, kind, error);
        }
        (
            Settlement::Completed(RouteStageResult::Continue(_)),
            RouteReservation::NotNeeded | RouteReservation::Reserved(None),
        ) => {
            publish_accepted_route_failure(
                navs,
                name,
                generation,
                request_id,
                kind,
                "route continuation has no retained-input custody",
            );
        }
        (Settlement::Failed(error), RouteReservation::NotNeeded) => {
            publish_accepted_route_failure(
                navs,
                name,
                generation,
                request_id,
                kind,
                &error.to_string(),
            );
        }
        (Settlement::Failed(error), RouteReservation::Failed(detail)) => {
            publish_accepted_route_failure(
                navs,
                name,
                generation,
                request_id,
                kind,
                &format!("{error}; {detail}"),
            );
        }
        (Settlement::Cancelled(_), RouteReservation::NotNeeded) => {}
        (Settlement::Cancelled(_), RouteReservation::Failed(detail)) => {
            publish_accepted_route_failure(navs, name, generation, request_id, kind, &detail);
        }
        (Settlement::Failed(error), RouteReservation::Reserved(_)) => {
            publish_accepted_route_failure(
                navs,
                name,
                generation,
                request_id,
                kind,
                &error.to_string(),
            );
        }
        (Settlement::Cancelled(_), RouteReservation::Reserved(_)) => {
            publish_accepted_route_failure(
                navs,
                name,
                generation,
                request_id,
                kind,
                "route work was cancelled",
            );
        }
        (_, RouteReservation::Full) => unreachable!("full owner results keep the terminal"),
    }
}
#[cfg(feature = "js")]
/// Host-published walk outcome copied onto the isolate snapshot.
#[derive(Clone, Copy, Default)]
pub(crate) struct PostedWalkOutcome {
    pub(crate) seq: u64,
    pub(crate) generation: u64,
    pub(crate) request_id: u64,
    pub(crate) failed: bool,
    pub(crate) x: i32,
    pub(crate) z: i32,
    pub(crate) level: i32,
    pub(crate) radius: i32,
    pub(crate) allow_teleports: bool,
    /// The settled route end is frozen `'blocked'`.
    pub(crate) blocked: bool,
    pub(crate) cancel_reason: script::native::WalkCancelReason,
    pub(crate) user_move_intent_seq: u64,
}

/// One navigator-named gate short of a failed walk: the `MissingReq::Carry`
/// row [`find_missing_item_reqs`] reported for that `NoPath`. It carries only
/// what the diagnosis itself named — the display name is resolved at pack time
/// from the host obj table, and a missing one never drops the row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct MissingCarry {
    pub(crate) id: i32,
    pub(crate) count: i32,
}

/// Exclusions retained with a request so reconnects re-resolve the original
/// symbolic catalog entries at their new arm-time endpoints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScriptRouteExclusions {
    pub(crate) avoid: Vec<AvoidRect>,
    pub(crate) avoid_wire: Vec<script::interact::InspectAvoidWire>,
    pub(crate) cross: Vec<Arc<str>>,
}
impl ScriptRouteExclusions {
    fn is_empty(&self) -> bool {
        self.avoid.is_empty() && self.avoid_wire.is_empty() && self.cross.is_empty()
    }
}

fn native_route_exclusions(
    avoid: &[Arc<str>],
    cross: &[Arc<str>],
    compat: Option<&ScriptRouteExclusions>,
) -> ScriptRouteExclusions {
    // Compatibility requests keep their complete original wire entries.
    // Re-resolve them at the new endpoints, not the rectangles derived by
    // the previous find, which would otherwise accumulate on every reissue.
    let mut exclusions = compat.map_or_else(ScriptRouteExclusions::default, |original| {
        ScriptRouteExclusions {
            avoid_wire: original.avoid_wire.clone(),
            cross: original.cross.clone(),
            ..Default::default()
        }
    });
    for name in avoid {
        if !exclusions.avoid_wire.iter().any(|entry| {
            matches!(entry, script::interact::InspectAvoidWire::Catalog(existing) if existing == name.as_ref())
        }) {
            exclusions
                .avoid_wire
                .push(script::interact::InspectAvoidWire::Catalog(name.to_string()));
        }
    }
    for name in cross {
        if !exclusions.cross.contains(name) {
            exclusions.cross.push(Arc::clone(name));
        }
    }
    exclusions
}

#[derive(Clone)]
pub(crate) struct RouteQuestContext {
    selected: Arc<api::game_data::SelectedGameData>,
    prereq_catalog: Option<Arc<api::prereq_steps::PrereqCatalog>>,
    stamp: EvidenceStamp,
    local_state: Option<Arc<script::local_state::SlotState>>,
}
/// Retain only immutable catalog ownership and the observation's actual
/// causal stamp. Status rows are copied only when a route is requested.
pub(crate) fn route_quest_context(
    slot: &script::SlotScript,
    tick: u64,
) -> Option<RouteQuestContext> {
    route_quest_context_with_data(slot, tick, slot.compiled_game_data()?)
}

fn route_quest_context_with_data(
    slot: &script::SlotScript,
    tick: u64,
    selected: Arc<api::game_data::SelectedGameData>,
) -> Option<RouteQuestContext> {
    let api::identity::Scope::Slot(key) = slot.work_scope()? else {
        return None;
    };
    let run = slot
        .native_run()
        .or_else(|| slot.seat_run())
        .unwrap_or(api::selected::RunKey {
            slot: key.incarnation().get(),
            run: slot.runtime_generation(),
            session: slot.work_epoch(),
        });
    Some(RouteQuestContext {
        prereq_catalog: selected.try_prereq_steps(),
        selected,
        stamp: EvidenceStamp {
            run,
            tick,
            sequence: tick,
        },
        local_state: slot.local_state().cloned(),
    })
}

pub(crate) fn manual_route_state(
    slot: &mut script::SlotScript,
    snapshot: &GameSnapshot,
    base: &WorldState,
    profile_data: Option<&Arc<api::game_data::SelectedGameData>>,
) -> Option<(
    api::selected::RunKey,
    Option<Arc<api::prereq_steps::PrereqCatalog>>,
    WorldState,
)> {
    let selected = slot
        .compiled_game_data()
        .or_else(|| profile_data.cloned())?;
    let mut context = route_quest_context_with_data(slot, u64::from(snapshot.tick()), selected)?;
    context.stamp.run = slot.capture_manual_route_run();
    let mut state = base.clone();
    state.establishable = context
        .prereq_catalog
        .as_ref()
        .map(|catalog| catalog.establishable_windows());
    state.quest_evidence = captured_route_quests(Some(&context), snapshot, None, context.stamp);
    Some((context.stamp.run, context.prereq_catalog, state))
}

struct RouteQuestProvider {
    memory: api::prereq_knowledge::SessionMemory,
    explicit: Option<Arc<dyn api::quest_progress::EvidenceProvider>>,
    ordinary: api::quest_progress::QuestListEvidenceProvider,
}

impl api::quest_progress::EvidenceProvider for RouteQuestProvider {
    fn observe_gate(
        &self,
        gate: &api::selected::QuestGate,
        required_after: EvidenceStamp,
    ) -> Option<api::prereq_knowledge::Observation> {
        Some(api::prereq_knowledge::compose(
            gate,
            required_after,
            &self.memory,
            self.explicit.as_deref(),
            &self.ordinary,
        ))
    }
}

fn captured_route_quests(
    context: Option<&RouteQuestContext>,
    snapshot: &GameSnapshot,
    explicit: Option<Arc<dyn api::quest_progress::EvidenceProvider>>,
    required_after: EvidenceStamp,
) -> Option<nav_planner::quest_gates::QuestEvidence> {
    let prepared = context.and_then(|context| {
        let quests = context.selected.try_quests()?;
        Some(
            api::quest_progress::QuestListEvidenceProvider::from_snapshot(
                &quests,
                SnapshotView::new(Some(snapshot), context.stamp),
            ),
        )
    });
    if let Some(ordinary) = prepared {
        let layout = std::alloc::Layout::new::<[std::sync::atomic::AtomicUsize; 2]>()
            .extend(std::alloc::Layout::new::<RouteQuestProvider>())
            .ok()?
            .0
            .pad_to_align();
        let memory = context
            .and_then(|context| context.local_state.as_ref())
            .map(|state| state.prereq_snapshot())
            .unwrap_or_default();
        let bytes = layout
            .size()
            .checked_add(ordinary.retained_status_bytes())?
            .checked_add(
                memory
                    .retained_bytes()
                    .checked_sub(std::mem::size_of::<api::prereq_knowledge::SessionMemory>())?,
            )?;
        let provider = Arc::new(RouteQuestProvider {
            memory,
            explicit,
            ordinary,
        });
        Some(
            nav_planner::quest_gates::QuestEvidence::new(provider, required_after)
                .with_owned_provider_bytes(bytes),
        )
    } else {
        explicit
            .map(|provider| nav_planner::quest_gates::QuestEvidence::new(provider, required_after))
    }
}
/// Service selection uses the same request-time evidence view as a walk.
pub(super) fn captured_service_quests(
    context: Option<&RouteQuestContext>,
    snapshot: &GameSnapshot,
    required_after: Option<EvidenceStamp>,
) -> Option<nav_planner::quest_gates::QuestEvidence> {
    let context = context?;
    captured_route_quests(
        Some(context),
        snapshot,
        None,
        required_after.unwrap_or(context.stamp),
    )
}

/// The first Load request remains the logical owner while native repair walks
/// use their own request IDs. Only an active compatibility walk allocates this.
pub(crate) struct CompatWalk {
    run: api::selected::RunKey,
    request_id: u64,
    generation: u64,
    key: (WorldTile, i32, bool, bool, bool, ZoneExempt),
    exclusions: Option<Arc<ScriptRouteExclusions>>,
    /// Taken once when the direct traveller hands ownership to the API seat.
    outer: Option<script::native::OuterWalk>,
}

/// Ten distinct observed ticks identify a sustained wait before the default
/// 60-tick hop budget. This controls logging only and never delays or retries.
pub(crate) const WALK_NO_PROGRESS_INFO_TICKS: u32 = 10;

pub(crate) struct WalkNoProgress {
    request_id: u64,
    tile: WorldTile,
    last_tick: u32,
    ticks: u32,
    reported: bool,
}

impl Default for WalkNoProgress {
    fn default() -> Self {
        Self {
            request_id: 0,
            tile: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            last_tick: 0,
            ticks: 0,
            reported: false,
        }
    }
}

impl WalkNoProgress {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Return the tick count once, on the first threshold crossing in an
    /// episode. Repeated snapshots do not count. Movement starts a new episode.
    pub(crate) fn observe(&mut self, request_id: u64, tile: WorldTile, tick: u32) -> Option<u32> {
        if request_id == 0 {
            self.reset();
            return None;
        }
        if self.request_id != request_id {
            self.request_id = request_id;
            self.tile = tile;
            self.last_tick = tick;
            self.ticks = 0;
            self.reported = false;
            return None;
        }

        if self.tile != tile {
            self.tile = tile;
            self.last_tick = tick;
            self.ticks = 0;
            self.reported = false;
            return None;
        }

        let elapsed = nav_executor::TickCount::credit_since(self.last_tick, tick);
        self.last_tick = tick;
        self.ticks = self.ticks.saturating_add(elapsed);
        if !self.reported && self.ticks >= WALK_NO_PROGRESS_INFO_TICKS {
            self.reported = true;
            Some(self.ticks)
        } else {
            None
        }
    }
}

/// Per-uid nav state: the whole-world traveller plus the route it is
/// following. `ctx.walk` stores the route, produced by the shared route-work
/// service over [`NavWorld`]. The slot pump polls [`Traveller::follow`] with a
/// clone of it one step per player-info tick. [`NavBot::has_route`] is the
/// "armed" gate the walk hook and the busy flag read. A pending BankBudget
/// session freezes follow until its steps finish.
#[derive(Default)]
pub(crate) struct NavBot {
    /// The slot-thread condition owner, also read by admission at arm time.
    pub(crate) condition: super::slot_condition::SlotCondition,
    /// The profile's Cure poison policy for this slot, copied from the
    /// arm's mirror on every observed frame beside the condition fact.
    /// Host consumers with no script context (the walk guard) read it
    /// next to [`NavBot::condition`]; native machines read the same
    /// value through `ActionContext::cure_poison`.
    pub(crate) cure_poison: script::survival::CurePoison,
    /// Shared session globals, attached before a production slot starts.
    pub(crate) walk_globals: Option<Arc<Mutex<super::WalkGlobals>>>,
    pub(crate) quest_context: Option<RouteQuestContext>,
    /// Actual compiled-slot identity. None is a fail-closed admission state.
    pub(crate) work_scope: Option<Scope>,
    /// The shared preferences each admission reads the globals from.
    pub(crate) walk_globals_store: Option<super::local_state::Preferences>,
    /// The compiled instance's effective Start/config revision, never a draft.
    /// `None` identifies isolate walks, which keep their existing option wiring.
    pub(crate) native_permissions: Option<script::native::WalkPermissions>,
    pub(crate) compat_v1: bool,
    /// The held runtime-net setting was reported during this session.
    pub(crate) runtime_gate_logged: bool,
    pub(crate) admission: Option<Box<crate::admission::Admission>>,
    pub(crate) assessment: Option<Arc<script::combat::risk::RouteAssessment>>,
    pub(crate) poison_hold: Option<crate::poison_hold::PoisonHold>,
    pub(crate) last_assessment: Option<Arc<script::combat::risk::RouteAssessment>>,
    pub(crate) route_basis: Option<Arc<crate::admission::RouteBasis>>,
    pub(crate) admission_pending: bool,
    pub(crate) risk_refusal: Option<Box<(u64, script::native::WalkRefusal)>>,
    pub(crate) assess_result:
        Option<Box<(script::native::HostAuthority, script::native::AssessReceipt)>>,
    pub(crate) advisory_request_id: u64,
    pub(crate) manual_arm: Option<std::sync::Weak<Mutex<crate::WalkArm>>>,
    pub(crate) route_generation: u64,
    pub(crate) map_route_generation: u64,
    pub(crate) pending_route: Option<Box<RouteDemand>>,
    pub(crate) route_ticket: Option<Box<RouteTicketOwner>>,
    /// Route family metadata retained with the caller-owned result.
    pub(crate) route_result_custody: Option<Arc<dyn std::any::Any + Send + Sync>>,
    /// Native ownership survives worker handoff, but not action revocation.
    pub(crate) native_walk: Option<script::native::HostAuthority>,
    pub(crate) native_receipt_seq: u64,
    pub(crate) compat_walk: Option<Box<CompatWalk>>,
    pub(crate) native_walk_failure: Option<(u64, script::native::WalkEnd)>,
    pub(crate) native_walk_blocked: Option<(u64, Arc<[ZoneKey]>)>,
    /// A native walk's terminal that no published walk outcome carries: a
    /// host refusal, or a Pause / displacement / abort that ended its follow.
    /// Delivered once to its still-live owner by the next observation.
    pub(crate) native_end: Option<(script::native::HostAuthority, script::native::WalkEnd)>,
    pub(crate) native_end_risk: Option<Box<NativeEndRisk>>,
    /// Evidence retained with the published route, not a newer pending retarget.
    pub(crate) route_quest_evidence: Option<nav_planner::quest_gates::QuestEvidence>,
    /// Dest, radius, FindOptions bits, and the per-walk zone exemptions.
    pub(crate) requested_route: Option<(WorldTile, i32, bool, bool, bool, ZoneExempt)>,
    /// The current route came from a compat tile-destination walk.
    pub(crate) route_compat_tile_intent: bool,
    /// Explicit loc identity; tile walks never infer it from scene contents.
    pub(crate) route_loc_id: Option<i32>,
    /// Settlement mode for the current armed route and its live refreshes.
    pub(crate) route_arrival: ArrivalKind,
    /// Arm-time estimate inputs: target in scene, footprint modeled.
    pub(crate) route_loc_geometry: (bool, bool),
    /// Request exclusions retained for reconnect carry and retransmission gates.
    pub(crate) requested_exclusions: Option<Arc<ScriptRouteExclusions>>,
    pub(crate) traveller: Traveller,
    /// Counts the current no-tile-progress episode for this slot's walk.
    pub(crate) walk_no_progress: WalkNoProgress,
    pub(crate) route: Option<Arc<Route>>,
    pub(crate) compact_route: Option<RoutePath>,
    /// The walk request id `publish_route` installed `route` for. A retarget
    /// moves `walk_request_id` / `requested_route` to the new walk while the
    /// old route is still followed; only a route end whose owner is the
    /// armed walk may settle that walk's wait.
    pub(crate) route_request_id: u64,
    pub(crate) bank_fetch: Option<PendingBankFetch>,
    /// The clue duel partner this slot's accept gate validated on the offer
    /// screen: its confirm accepts only that partner, in this session.
    pub(crate) duel_offer_partner: Option<String>,
    /// Isolate-allocated walk request id for the armed / in-flight find.
    /// Distinct from `route_generation`, which remains the route-job /
    /// retained-route token.
    pub(crate) walk_request_id: u64,
    /// Unpublished current-wait refusal id, distinct from `walk_request_id`.
    /// Older armed-route NoPath / mid-follow terminals must not overwrite this
    /// published outcome until a snapshot copies it.
    pub(crate) walk_live_refusal_id: u64,
    /// Last packed-walk `allow_teleports` opt-in (`Traversal.teleportsEnabled`).
    pub(crate) allow_teleports: bool,
    /// Bounded published walk outcome. Seq `0` means never published.
    pub(crate) walk_outcome_seq: u64,
    pub(crate) walk_outcome_generation: u64,
    pub(crate) walk_outcome_request_id: u64,
    pub(crate) walk_outcome_failed: bool,
    pub(crate) walk_outcome_x: i32,
    pub(crate) walk_outcome_z: i32,
    pub(crate) walk_outcome_level: i32,
    pub(crate) walk_outcome_radius: i32,
    pub(crate) walk_outcome_allow_teleports: bool,
    /// The published route end is frozen `'blocked'`
    /// ([`nav_executor::HopFailure::EndBlocked`]).
    pub(crate) walk_outcome_blocked: bool,
    pub(crate) walk_outcome_cancel_reason: script::native::WalkCancelReason,
    /// UI-only cancellation detail, cleared by a subsequent arm or outcome.
    pub(crate) manual_walk_cancelled_detail: bool,
    /// Every qualifying human gesture, including when no route exists.
    pub(crate) user_move_intent_seq: u64,
    /// Requests based on an older outcome cannot reclaim movement ownership.
    pub(crate) manual_takeover_watermark: u64,
    /// The navigator-named gate shorts of that same published outcome: set
    /// only where a `NoPath` was diagnosed, and cleared wherever the outcome
    /// is, so a list can never outlive the failure it belongs to.
    pub(crate) walk_missing_carry: Vec<MissingCarry>,
    pub(crate) walk_outcome_detail: Option<Arc<str>>,
    pub(crate) inspect: route_inspect::InspectNav,
    pub(crate) bank_pick: super::bank::BankPickState,
    /// The game requests this slot's script sent, as dispatched here. Host
    /// data the catalog hunt watch reads; not an isolate wire.
    pub(crate) acts: crate::catalog_core::ScriptActLedger,
    /// A held script walk, or receipt-only identity while the watchdog replaces
    /// its route. Only a held walk is eligible for automatic carry dispatch.
    /// Boxed only while held or recovering; idle slots retain no wire payload.
    pub(crate) carried_walk: Option<Box<CarriedWalk>>,
    /// Hold-mode protect driver for this followed route.
    pub(crate) walk_guard: Option<script::combat::WalkGuard>,
    /// Retired guard's bounded, observation-settled protect cleanup.
    pub(crate) walk_guard_off: Option<super::script_walk::WalkGuardOff>,
    /// Accepted Combat raises retired by operator Stop.
    pub(crate) combat_prayer_off: Option<super::script_walk::CombatPrayerOff>,
    /// Non-terminal protection warnings, drained to the correlated owner.
    pub(crate) walk_guard_events: Vec<(script::native::HostAuthority, script::native::WalkEvent)>,
    /// Latest terminal script generation whose navigation state was reset.
    pub(crate) terminal_nav_reset_generation: Option<u64>,
}

pub(crate) struct NativeEndRisk {
    pub assessment: Option<Arc<script::combat::risk::RouteAssessment>>,
    pub refusal: Option<script::native::WalkRefusal>,
    pub escape: Option<script::combat::guard::Escape>,
    pub blocked: Option<Arc<[ZoneKey]>>,
}

/// A held script walk or its identity across a watchdog-owned replacement.
pub(crate) struct CarriedWalk {
    /// `Some(run)` may be re-sent across Pause/reconnect. `None` is receipt-only
    /// recovery identity: it must never dispatch as carry after recovery.
    runtime_generation: Option<u64>,
    /// Route generation before `hold_script_nav` ends the session's follow.
    route_generation: u64,
    request: script::interact::InteractReq,
}

/// The route-work service handles searches; the slot pump owns follow
/// and dispatch.
#[derive(Clone)]
pub(crate) struct ScriptWalkArm {
    pub(crate) here: Option<(i32, i32, i32)>,
    pub(crate) world: Option<Arc<NavWorld>>,
    pub(crate) navs: Arc<Mutex<HashMap<String, NavBot>>>,
    pub(crate) name: String,
    /// The slot's gating facts at arm time (from its live snapshot);
    /// `None` when no player is decoded — the route job then searches with
    /// the fail-closed empty [`WorldState`].
    pub(crate) state: Option<Arc<WorldState>>,
    /// The account's bank memory rows at arm time
    /// ([`super::slot_bank_memory::planner_rows`]): what a BankBudget
    /// session is planned over ([`nav_planner::bank_fetch::planning_rows`]).
    pub(crate) bank: BankRows,
}

fn walk_arm_outcome_tag(outcome: &RouteOutcome) -> &'static str {
    match outcome {
        RouteOutcome::Routed(_) => "Routed",
        RouteOutcome::BankSession { .. } => "BankSession",
        RouteOutcome::IncompleteSearch { .. } => "IncompleteSearch",
        RouteOutcome::NoPath => "NoPath",
    }
}

/// Gating facts for a native walk.
///
/// Reuse the arm's observe-time [`WorldState`] when it exists (Starting and
/// Running frames already built it from this snapshot, including profile
/// membership). Rebuild from the snapshot only when the arm is `None` — a
/// Starting-frame walk if observe did not pass facts, or a test that arms
/// with `state: None`. That rebuild is what lets posted skills open a
/// skill-gated door (Fishing Guild 68) instead of routing on the fail-closed
/// empty state.
pub(crate) fn route_world_state(
    snapshot: &GameSnapshot,
    armed: Option<Arc<WorldState>>,
) -> Arc<WorldState> {
    armed.unwrap_or_else(|| Arc::new(WorldState::from_snapshot(snapshot)))
}

/// Attach evidence to a request-owned view without changing the cached base.
/// An unchanged empty value can share the original backing.
/// A private-copy allocation failure returns `RouteJobError::Allocation`.
pub(super) fn patch_route_quest_evidence(
    state: Arc<WorldState>,
    evidence: Option<nav_planner::quest_gates::QuestEvidence>,
) -> Result<Arc<WorldState>, RouteJobError> {
    if evidence.is_none() && state.quest_evidence.is_none() {
        return Ok(state);
    }
    let mut private =
        super::try_clone_world_state(&state).map_err(|_| RouteJobError::Allocation)?;
    private.quest_evidence = evidence;
    Ok(Arc::new(private))
}

/// Bank diagnosis may request a journal observation, not a world interaction.
/// Keep the cached snapshot immutable and use the normal fallible private copy.
pub(super) fn patch_bank_quest_evidence(
    state: Arc<WorldState>,
    evidence: Option<nav_planner::quest_gates::QuestEvidence>,
    context: Option<&RouteQuestContext>,
) -> Result<Arc<WorldState>, RouteJobError> {
    // No matched request-time provider means no journal diagnosis authority.
    let context = context.filter(|_| evidence.is_some());
    let establishable =
        if let Some(catalog) = context.and_then(|context| context.prereq_catalog.as_ref()) {
            let all = catalog.establishable_windows();
            let journal = |window: &api::selected::StageWindow| {
                catalog
                    .for_journal(&api::selected::QuestGate::Window(window.clone()))
                    .is_some()
            };
            if all.is_empty() {
                None
            } else if all.iter().all(journal) {
                Some(all)
            } else {
                let mut windows = Vec::new();
                windows
                    .try_reserve_exact(all.len())
                    .map_err(|_| RouteJobError::Allocation)?;
                windows.extend(all.iter().filter(|window| journal(window)).cloned());
                (!windows.is_empty()).then(|| Arc::from(windows))
            }
        } else {
            None
        };
    if evidence.is_none()
        && state.quest_evidence.is_none()
        && establishable.is_none()
        && state.establishable.is_none()
    {
        return Ok(state);
    }
    let mut private =
        super::try_clone_world_state(&state).map_err(|_| RouteJobError::Allocation)?;
    private.quest_evidence = evidence;
    private.establishable = establishable;
    Ok(Arc::new(private))
}

fn log_walk_arm(name: &str, build: impl FnOnce() -> String) {
    api::host_log!(
        api::hostlog::Category::NavTrace,
        api::hostlog::Level::Debug,
        slot = name,
        "walk-arm {}",
        build()
    );
}

pub(super) fn log_walk_arm_bot(build: impl FnOnce() -> String) {
    api::host_log!(
        api::hostlog::Category::NavTrace,
        api::hostlog::Level::Debug,
        "walk-arm {}",
        build()
    );
}
fn format_zone_witness(table: &nav_planner::zones::ZoneTable, keys: &[ZoneKey]) -> String {
    keys.iter()
        .map(|&key| format!("{} [{}]", table.label(key), table.name(key)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn blocked_zone_detail(
    table: &nav_planner::zones::ZoneTable,
    keys: &[ZoneKey],
) -> String {
    format!(
        "blocked by danger zones: {}",
        format_zone_witness(table, keys)
    )
}

/// How a user lets a script walk cross the zones its refusal names.
/// WalkTo carries its own one-walk hint (`walk_map/actions.rs`).
const SCRIPT_DANGER_HINT: &str =
    "to allow it, set Danger routing to Always in Nav config, or allow danger zones in the script's own settings if it has that option";

pub(crate) fn resolve_route_exclusions(
    mut opts: FindOptions,
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    state: &WorldState,
    mut exclusions: ScriptRouteExclusions,
) -> Result<(FindOptions, ScriptRouteExclusions), String> {
    if exclusions.avoid_wire.len() > route_inspect::MAX_AVOID {
        return Err("avoidZones: more than 16 entries".to_string());
    }
    let zones = world.graph.zones.as_ref();
    for entry in &exclusions.avoid_wire {
        match entry {
            script::interact::InspectAvoidWire::Rect {
                min_x,
                max_x,
                min_z,
                max_z,
                level,
            } => exclusions.avoid.push(AvoidRect {
                min_x: *min_x,
                max_x: *max_x,
                min_z: *min_z,
                max_z: *max_z,
                level: *level,
            }),
            script::interact::InspectAvoidWire::Catalog(id) => {
                let Some(table) = zones else {
                    return Err(format!(
                        "avoidZones: zone catalog unavailable for {id:?} (legacy grid pack)"
                    ));
                };
                if !nav_planner::zones::AVOID_CATALOG_IDS.contains(&id.as_str()) {
                    return Err(format!("avoidZones: unknown zone {id:?}"));
                }
                let Some(group) = table.resolve(id).and_then(|key| match key {
                    ZoneKey::Group(index) => table.groups().get(usize::from(index)),
                    ZoneKey::Zone(_) => None,
                }) else {
                    return Err(format!(
                        "avoidZones: catalog zone {id:?} has no baked group geometry"
                    ));
                };
                match id.as_str() {
                    "white-wolf-mountain" => exclusions.avoid.push(group.rect),
                    "draynor-jail-guards" => {
                        let member_rect = |member: &u16| {
                            let zone = &table.zones()[usize::from(*member)];
                            AvoidRect {
                                min_x: zone.min_x,
                                max_x: zone.max_x,
                                min_z: zone.min_z,
                                max_z: zone.max_z,
                                level: Some(i32::from(zone.level)),
                            }
                        };
                        let endpoint_inside = group.members.iter().any(|member| {
                            let rect = member_rect(member);
                            rect.contains(from) || rect.contains(to)
                        });
                        let combat_high = state.combat_level.is_some_and(|level| level > 50);
                        if !endpoint_inside && !combat_high {
                            exclusions
                                .avoid
                                .extend(group.members.iter().map(member_rect));
                        }
                    }
                    _ => unreachable!(),
                }
            }
            script::interact::InspectAvoidWire::Unsupported => {
                return Err(
                    "avoidZones: expected a rectangle or a known catalog zone id".to_string(),
                );
            }
        }
    }
    if exclusions.avoid.len() > route_inspect::MAX_AVOID {
        return Err("avoidZones: more than 16 entries".to_string());
    }
    if exclusions.avoid.iter().any(|rect| {
        rect.min_x > rect.max_x
            || rect.min_z > rect.max_z
            || rect.level.is_some_and(|level| !(0..=3).contains(&level))
    }) {
        return Err("avoidZones: a rectangle with inverted bounds or a level off 0-3".to_string());
    }
    if exclusions.cross.len() > 8 {
        return Err("crossZones: more than 8 zone names".to_string());
    }
    let mut keys = Vec::with_capacity(exclusions.cross.len());
    for name in &exclusions.cross {
        let Some(table) = zones else {
            return Err(format!(
                "crossZones: zone catalog unavailable (legacy grid pack); cannot resolve {name:?}"
            ));
        };
        let Some(key) = table.resolve(name) else {
            return Err(format!("crossZones: unknown zone {name:?}"));
        };
        keys.push(key);
    }
    let named = ZoneExempt::named(&keys)
        .map_err(|_| "crossZones: more than 8 distinct zones".to_string())?;
    opts.zones = opts
        .zones
        .union(named)
        .map_err(|_| "crossZones: more than 8 distinct zones".to_string())?;
    Ok((opts, exclusions))
}

impl ScriptWalkArm {
    /// Advisory routing shares normalization, hard authority and assessment,
    /// but owns neither a follow route nor the native walk/event budget.
    pub(crate) fn queue_native_assess(
        mut self,
        snapshot: &GameSnapshot,
        request: script::native::WalkRequest,
        authority: script::native::HostAuthority,
        evidence: api::quest_progress::EvidenceStamp,
        session_permissions: Option<script::native::WalkPermissions>,
        manual_bank_fetch: Option<bool>,
    ) {
        if !authority.live() {
            return;
        }
        let refusal = |detail: &str| {
            log_walk_arm(&self.name, || {
                format!(
                    "assess-request refused tick={} request_id={} detail={detail}",
                    snapshot.tick(),
                    authority.request_id().get()
                )
            });
            let receipt = script::native::AssessReceipt {
                request_id: authority.request_id().get(),
                evidence,
                assessment: None,
                route_ticks: 0,
                refusal: Some(script::native::WalkRefusal::NoRouteWithinBounds {
                    tried: 0,
                    last: None,
                }),
                detail: Some(Arc::from(detail)),
            };
            let displaced = {
                let mut navs = self
                    .navs
                    .lock()
                    .expect("navigation state mutex is not poisoned");
                let bot = navs.entry(self.name.clone()).or_default();
                bot.advisory_request_id = authority.request_id().get();
                let has_advisory = bot
                    .pending_route
                    .as_ref()
                    .and_then(|demand| demand.kind())
                    .is_some_and(|kind| matches!(kind, RouteDemandKind::Advisory { .. }));
                let displaced = has_advisory.then(|| bot.pending_route.take()).flatten();
                bot.assess_result = Some(Box::new((authority.clone(), receipt)));
                displaced
            };
            if let Some(displaced) = displaced {
                (*displaced).signal_completion();
            }
        };
        let (Some(world), Some((x, z, level))) = (self.world.as_ref(), self.here) else {
            refusal("Advisory route unavailable: no navigation world or observed player");
            return;
        };
        if request.required_after.run != authority.run()
            || request.arrival == ArrivalKind::Area && request.loc_id.is_some()
        {
            refusal("Advisory route request has invalid evidence or arrival authority");
            return;
        }
        if request.protect {
            let view = SnapshotView::new(Some(snapshot), evidence);
            if let Err(reason) = script::combat::WalkGuard::begin(&request, &view) {
                refusal(&format!("Walk protection: {reason:?}"));
                return;
            }
        }
        let state = route_world_state(snapshot, self.state.take());
        let context = self
            .navs
            .lock()
            .unwrap()
            .get(&self.name)
            .and_then(|bot| bot.quest_context.clone());
        let quest_evidence = captured_route_quests(
            context.as_ref(),
            snapshot,
            request.evidence,
            request.required_after,
        );
        self.state = match patch_route_quest_evidence(state, quest_evidence) {
            Ok(state) => Some(state),
            Err(_) => {
                refusal("Advisory route allocation failed");
                return;
            }
        };
        let from = WorldTile { x, z, level };
        let radius = i32::from(request.radius);
        let (mut options, policy, enforce) = super::walk_permissions::native_admission(
            &self.navs,
            &self.name,
            request.options,
            session_permissions,
        );
        if let Some(allow) = manual_bank_fetch {
            options.allow_bank_fetch = allow;
        }
        let empty = WorldState::empty();
        let state = self.state.as_deref().unwrap_or(&empty);
        let (options, exclusions) = match resolve_route_exclusions(
            options,
            world,
            from,
            request.target,
            state,
            native_route_exclusions(&request.avoid, &request.cross, None),
        ) {
            Ok(resolved) => resolved,
            Err(detail) => {
                refusal(&detail);
                return;
            }
        };
        let admission = {
            let navs = self.navs.lock().unwrap();
            let bot = navs.get(&self.name);
            let input = crate::admission::capture(
                snapshot,
                state.map_members,
                bot.and_then(|bot| bot.condition.current()),
                bot.is_some_and(|bot| bot.walk_guard_off.is_some()),
            );
            // A v1 card keeps frozen danger routing on the machine-owned
            // native walks too, as on its compiled walks (`compiled_options`).
            let compat_v1 = bot.is_some_and(|bot| bot.compat_v1);
            crate::admission::Admission {
                policy,
                enforce: enforce || !request.cross.is_empty(),
                grants: options.zones,
                allow: request.allow,
                input,
                generation: authority.request_id().get(),
                compat_v1,
                escape: None,
            }
        };
        let live_geometry = if radius > 0 {
            request
                .loc_id
                .and_then(|id| capture_loc_geometry(snapshot, request.target, radius, id))
        } else {
            None
        };
        let request = ScriptRouteRequest {
            candidate_goals: request.candidate_goals,
            generation: admission.generation,
            request_id: authority.request_id().get(),
            world: Arc::clone(world),
            from,
            to: request.target,
            radius,
            loc_id: request.loc_id,
            arrival: request.arrival,
            opts: options,
            admission,
            state: self.state,
            prereq_catalog: None,
            compat_tile_intent: false,
            bank_origin: self.bank.origin,
            bank: self.bank.rows,
            live_geometry,
            exclusions: (!exclusions.is_empty()).then(|| Arc::new(exclusions)),
            completion: RouteCompletion::default(),
        };
        let request_id = request.request_id;
        let to = request.to;
        let demand = RouteDemand::Pending {
            request: Box::new(request),
            kind: RouteDemandKind::Advisory {
                authority: authority.clone(),
                evidence,
            },
        };
        let (queued, displaced) = {
            let mut navs = self
                .navs
                .lock()
                .expect("navigation state mutex is not poisoned");
            let bot = navs.entry(self.name.clone()).or_default();
            let replaceable = bot
                .pending_route
                .as_ref()
                .and_then(|pending| pending.kind())
                .is_some_and(|kind| matches!(kind, RouteDemandKind::Advisory { .. }));
            let displaced = replaceable.then(|| bot.pending_route.take()).flatten();
            let queued = bot.pending_route.is_none();
            if queued {
                bot.advisory_request_id = request_id;
                bot.assess_result = None;
                bot.pending_route = Some(Box::new(demand));
            }
            (queued, displaced)
        };
        let displaced_advisory = displaced.is_some();
        if let Some(displaced) = displaced {
            (*displaced).signal_completion();
        }
        if queued {
            log_walk_arm(&self.name, || {
                format!(
                    "assess-request queued tick={} request_id={request_id} from={from:?} to={to:?} radius={radius} displaced_advisory={displaced_advisory}",
                    snapshot.tick()
                )
            });
        }
        if !queued {
            refusal("Advisory route work is already pending for this slot");
        }
    }

    pub(crate) fn queue_native_route(
        mut self,
        snapshot: &GameSnapshot,
        request: script::native::WalkRequest,
        authority: script::native::HostAuthority,
        session_permissions: Option<script::native::WalkPermissions>,
        manual_bank_fetch: Option<bool>,
    ) -> bool {
        if !authority.live() {
            return false;
        }
        if request.required_after.run != authority.run() {
            self.refuse_native(authority);
            return false;
        }
        if request.arrival == ArrivalKind::Area && request.loc_id.is_some() {
            self.refuse_native_with_detail(
                authority,
                Some(Arc::from("Area arrival cannot be combined with loc_id")),
            );
            return false;
        }
        self.state = Some(route_world_state(snapshot, self.state.take()));
        {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
            if let Some(escape) = bot.slot_escape() {
                bot.refuse_native_with_detail(
                    authority,
                    Some(Arc::from(crate::admission::escape_reason(escape))),
                );
                bot.native_end_risk = Some(Box::new(NativeEndRisk {
                    assessment: None,
                    refusal: Some(script::native::WalkRefusal::EscapeInProgress),
                    escape: Some(escape),
                    blocked: None,
                }));
                return false;
            }
        }
        let protect = request.protect;
        let guard = if protect || request.food_guard || request.cure_guard {
            let view = SnapshotView::new(
                Some(snapshot),
                EvidenceStamp {
                    run: authority.run(),
                    tick: u64::from(snapshot.tick()),
                    sequence: 0,
                },
            );
            let admission = if protect {
                script::combat::WalkGuard::begin(&request, &view)
            } else {
                script::combat::WalkGuard::begin_food_only(&request, &view)
            };
            match admission {
                Ok(guard) => Some(guard),
                Err(reason) => {
                    self.refuse_native_with_detail(
                        authority,
                        Some(Arc::from(format!("Walk protection: {reason:?}"))),
                    );
                    return false;
                }
            }
        } else {
            None
        };
        let (mut options, policy, enforce) = super::walk_permissions::native_admission(
            &self.navs,
            &self.name,
            request.options,
            session_permissions,
        );
        if let Some(allow) = manual_bank_fetch {
            options.allow_bank_fetch = allow;
        }
        // A v1 card keeps frozen danger routing on the machine-owned native
        // walks too (Banking.open's stand walk), as on its compiled walks.
        let (off_debt, compat_v1, condition, compat_policy) = self
            .navs
            .lock()
            .unwrap()
            .get(&self.name)
            .map_or((false, false, None, None), |bot| {
                (
                    bot.walk_guard_off.is_some(),
                    bot.compat_v1,
                    bot.condition.current().copied(),
                    bot.compat_walk_native_policy(authority.run()),
                )
            });
        if let Some((original, _)) = compat_policy.as_ref() {
            options.allow_teleports = original.allow_teleports
                && request.options.allow_teleports != script::native::WalkBit::Forbid;
            options.allow_wilderness = original.allow_wilderness
                && request.options.allow_wilderness != script::native::WalkBit::Forbid;
            options.allow_bank_fetch = original.allow_bank_fetch;
            options.zones = original.zones;
        }
        let admission = crate::admission::Admission {
            policy,
            enforce: enforce || !request.cross.is_empty(),
            grants: options.zones,
            allow: request.allow,
            input: crate::admission::capture(
                snapshot,
                self.state.as_ref().is_some_and(|state| state.map_members),
                condition.as_ref(),
                off_debt,
            ),
            generation: 0,
            compat_v1,
            escape: None,
        };
        let context = self
            .navs
            .lock()
            .unwrap()
            .get(&self.name)
            .and_then(|bot| bot.quest_context.clone());
        let evidence = self.world.as_ref().and_then(|_| {
            captured_route_quests(
                context.as_ref(),
                snapshot,
                request.evidence,
                request.required_after,
            )
        });
        let state = self.state.take().expect("request state was captured");
        self.state = match patch_route_quest_evidence(state, evidence) {
            Ok(state) => Some(state),
            Err(_) => {
                self.refuse_native(authority);
                return false;
            }
        };
        let exclusions = native_route_exclusions(
            &request.avoid,
            &request.cross,
            compat_policy
                .as_ref()
                .and_then(|(_, exclusions)| exclusions.as_deref()),
        );
        let queued = self.queue_route_impl(
            request.target.x,
            request.target.z,
            request.target.level,
            options,
            i32::from(request.radius),
            true,
            authority.request_id().get(),
            Some(snapshot),
            RouteCompletion::default(),
            exclusions,
            Some(authority),
            request.loc_id,
            request.arrival,
            Some(admission),
            false,
            request.candidate_goals,
        );
        if queued {
            if let Some(bot) = self.navs.lock().unwrap().get_mut(&self.name) {
                bot.walk_guard = guard;
            }
        }
        queued
    }

    /// Re-find an estimated endpoint against newly observed live geometry
    /// without replacing the owning script action. The caller removes the old
    /// follow first and only calls this when no worker/bank session is active.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refresh_route_in_snapshot(
        mut self,
        snapshot: &GameSnapshot,
        to: WorldTile,
        radius: i32,
        opts: FindOptions,
        request_id: u64,
        exclusions: ScriptRouteExclusions,
        authority: Option<script::native::HostAuthority>,
        loc_id: Option<i32>,
        arrival: ArrivalKind,
        compat_tile_intent: bool,
    ) -> bool {
        self.queue_route_impl(
            to.x,
            to.z,
            to.level,
            opts,
            radius,
            true,
            request_id,
            Some(snapshot),
            RouteCompletion::default(),
            exclusions,
            authority,
            loc_id,
            arrival,
            None,
            compat_tile_intent,
            None,
        )
    }

    /// Explicit WalkNear, including radius 0: an armed or in-flight route is
    /// replaced through the existing generation / pending-route coalescing path.
    /// A latched bank-fetch session still refuses.
    #[cfg(test)]
    pub(crate) fn route_with_radius(
        &self,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
    ) -> bool {
        self.queue_route(x, z, level, opts, radius, true, 0)
    }
    pub(crate) fn publish_refusal(
        &self,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
        request_id: u64,
    ) {
        let mut navs = self.navs.lock().unwrap();
        let bot = navs.entry(self.name.clone()).or_default();
        // Do not bump route_generation or clear a retained route / bank-fetch.
        bot.note_failure(
            bot.route_generation,
            request_id,
            to,
            radius,
            allow_teleports,
        );
    }

    /// A native walk the host will not arm gets its typed `Refused` terminal.
    /// It never publishes through the Load walk outcome: that refusal guard
    /// is released only by an isolate snapshot a compiled slot never posts.
    fn refuse_native(&self, authority: script::native::HostAuthority) {
        self.refuse_native_with_detail(authority, None);
    }

    fn refuse_native_with_detail(
        &self,
        authority: script::native::HostAuthority,
        detail: Option<Arc<str>>,
    ) {
        if !authority.live() {
            return;
        }
        let mut navs = self.navs.lock().unwrap();
        navs.entry(self.name.clone())
            .or_default()
            .refuse_native_with_detail(authority, detail);
    }

    fn refuse(
        &self,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
        request_id: u64,
        native: Option<&script::native::HostAuthority>,
        detail: Option<Arc<str>>,
    ) {
        match native {
            Some(authority) => self.refuse_native_with_detail(authority.clone(), detail),
            None => self.publish_refusal(to, radius, allow_teleports, request_id),
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)] // route queue packs dest/options/request id fields
    pub(crate) fn queue_route(
        &self,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
    ) -> bool {
        self.clone().queue_route_impl(
            x,
            z,
            level,
            opts,
            radius,
            retarget,
            request_id,
            None,
            RouteCompletion::default(),
            ScriptRouteExclusions::default(),
            None,
            None,
            ArrivalKind::Reach,
            None,
            false,
            None,
        )
    }

    /// Queue one walk toward `(x, z, level)` with `opts`; search runs through
    /// the shared route-work service over [`NavWorld`]. When `allow_bank_fetch`
    /// is on and a strict find fails only for missing item or worn requirements,
    /// the route job latches a [`PendingBankFetch`] session and the post-session
    /// route. Requests are refused synchronously only when there is no player
    /// tile, no nav world, or a route/session already queued; the service stores
    /// the result on the uid's nav bot. Returns whether the request was queued,
    /// not whether a path exists.
    #[allow(clippy::too_many_arguments)] // plus the borrowed arm-time scene
    pub(crate) fn queue_route_in_snapshot(
        mut self,
        snapshot: &GameSnapshot,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
    ) -> bool {
        let opts = super::walk_permissions::compiled_options(&self.navs, &self.name, opts);
        self.queue_route_impl(
            x,
            z,
            level,
            opts,
            radius,
            retarget,
            request_id,
            Some(snapshot),
            RouteCompletion::default(),
            ScriptRouteExclusions::default(),
            None,
            None,
            ArrivalKind::Reach,
            None,
            false,
            None,
        )
    }

    /// Queue a walk with the request's frozen avoid rectangles and zone names.
    #[allow(clippy::too_many_arguments)] // route queue plus the walk's exclusions
    pub(crate) fn queue_fixed_route_in_snapshot_avoiding(
        mut self,
        snapshot: &GameSnapshot,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        request_id: u64,
        exclusions: ScriptRouteExclusions,
    ) -> bool {
        self.queue_route_impl(
            x,
            z,
            level,
            opts,
            0,
            false,
            request_id,
            Some(snapshot),
            RouteCompletion::default(),
            exclusions,
            None,
            None,
            ArrivalKind::Reach,
            None,
            false,
            None,
        )
    }

    /// [`Self::queue_route_in_snapshot`] for a walk with request exclusions.
    #[allow(clippy::too_many_arguments)] // plus the borrowed arm-time scene and exclusions
    pub(crate) fn queue_route_in_snapshot_avoiding(
        mut self,
        snapshot: &GameSnapshot,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        request_id: u64,
        compat_tile_intent: bool,
        exclusions: ScriptRouteExclusions,
    ) -> bool {
        self.queue_route_impl(
            x,
            z,
            level,
            opts,
            radius,
            true,
            request_id,
            Some(snapshot),
            RouteCompletion::default(),
            exclusions,
            None,
            None,
            ArrivalKind::Reach,
            None,
            compat_tile_intent,
            None,
        )
    }

    #[cfg(test)]
    #[expect(
        clippy::too_many_arguments,
        reason = "the deterministic route-work test seam supplies each service input explicitly"
    )]
    pub(crate) fn queue_route_in_snapshot_synced(
        &self,
        snapshot: &GameSnapshot,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
        loc_id: Option<i32>,
    ) -> Option<std::sync::mpsc::Receiver<()>> {
        let (completion, receiver) = RouteCompletion::channel();
        self.clone()
            .queue_route_impl(
                x,
                z,
                level,
                opts,
                radius,
                retarget,
                request_id,
                Some(snapshot),
                completion,
                ScriptRouteExclusions::default(),
                None,
                loc_id,
                ArrivalKind::Reach,
                None,
                false,
                None,
            )
            .then_some(receiver)
    }

    #[allow(clippy::too_many_arguments)] // admission mirrors the wire request identity
    fn gate_route(
        &self,
        bot: &mut NavBot,
        to: WorldTile,
        radius: i32,
        key: (WorldTile, i32, bool, bool, bool, ZoneExempt),
        exclusions: &Option<Arc<ScriptRouteExclusions>>,
        retarget: bool,
        request_id: u64,
        native: Option<&script::native::HostAuthority>,
        loc_id: Option<i32>,
        arrival: ArrivalKind,
        compat_tile_intent: bool,
    ) -> Option<bool> {
        if native.is_some() && bot.native_walk.as_ref().is_some_and(|owner| !owner.live()) {
            // The owner cancelled or replaced its previous walk: that follow
            // (and any bank session it latched) is abandoned, not in flight.
            super::script_walk::abort_walk_on_bot(bot);
        }
        let refuse = |bot: &mut NavBot| match native {
            Some(owner) => bot.refuse_native_with_detail(
                owner.clone(),
                Some(Arc::from(format!(
                    "native walk refused: route admission is blocked by active work; \
                     {:?} target {to:?}, radius {radius}",
                    arrival
                ))),
            ),
            None => bot.note_failure(bot.route_generation, request_id, to, radius, key.2),
        };
        if bot.bank_fetch.is_some() || (!retarget && (bot.has_route() || bot.route_work_pending()))
        {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-in-flight dest={to:?} r={radius} request_id={request_id} \
                     bank_fetch={} retarget={retarget} route={} route_work_pending={}",
                    bot.bank_fetch.is_some(),
                    bot.has_route(),
                    bot.route_work_pending()
                )
            });
            refuse(bot);
            return Some(false);
        }
        if bot.requested_route == Some(key)
            && bot.requested_exclusions.as_deref() == exclusions.as_deref()
            && bot.route_loc_id == loc_id
            && bot.route_arrival == arrival
            && bot.route_compat_tile_intent == compat_tile_intent
            && (bot.route_work_pending() || bot.has_route())
        {
            // Same-id retransmission and legacy request_id 0 keep the
            // in-flight find. A later distinct nonzero wait is refused
            // without restarting search or reassigning the armed id.
            if request_id != 0 && request_id != bot.walk_request_id {
                log_walk_arm(&self.name, || {
                    format!(
                        "queue_route coalesced-refused-distinct-id dest={to:?} r={radius} \
                         request_id={request_id} armed_id={}",
                        bot.walk_request_id
                    )
                });
                refuse(bot);
                return Some(false);
            }
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route coalesced dest={to:?} r={radius} request_id={request_id} \
                     generation={}",
                    bot.route_generation
                )
            });
            return Some(true);
        }
        None
    }

    /// Queue one route request. The request takes the arm's own captured
    /// [`WorldState`] (moved, not copied again), so it keeps the facts of the
    /// frame that armed it.
    #[allow(clippy::too_many_arguments)] // queue state plus borrowed arm-time scene
    fn queue_route_impl(
        &mut self,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        retarget: bool,
        request_id: u64,
        snapshot: Option<&GameSnapshot>,
        completion: RouteCompletion,
        exclusions: ScriptRouteExclusions,
        authority: Option<script::native::HostAuthority>,
        loc_id: Option<i32>,
        arrival: ArrivalKind,
        request_admission: Option<crate::admission::Admission>,
        compat_tile_intent: bool,
        candidate_goals: Option<script::native::WalkGoals>,
    ) -> bool {
        if authority.as_ref().is_some_and(|owner| !owner.live()) {
            return false;
        }
        let to = WorldTile { x, z, level };
        let Some((hx, hz, hl)) = self.here else {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-no-here dest={to:?} r={radius} request_id={request_id}"
                )
            });
            self.refuse(
                to,
                radius,
                opts.allow_teleports,
                request_id,
                authority.as_ref(),
                authority.as_ref().map(|_| {
                    Arc::from(format!(
                        "native walk refused: no player tile for {:?} arrival to {to:?}, radius {radius}",
                        arrival
                    ))
                }),
            );
            return false;
        };
        let Some(world) = self.world.as_ref() else {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-no-world dest={to:?} r={radius} request_id={request_id}"
                )
            });
            self.refuse(
                to,
                radius,
                opts.allow_teleports,
                request_id,
                authority.as_ref(),
                authority.as_ref().map(|_| {
                    Arc::from(format!(
                        "native walk refused: no navigation world for {:?} arrival to {to:?}, radius {radius}",
                        arrival
                    ))
                }),
            );
            return false;
        };
        let from = WorldTile {
            x: hx,
            z: hz,
            level: hl,
        };
        let original_key = (
            to,
            radius,
            opts.allow_teleports,
            opts.allow_wilderness,
            opts.allow_bank_fetch,
            opts.zones,
        );
        let frozen = {
            let navs = self.navs.lock().unwrap();
            navs.get(&self.name).and_then(|bot| {
                let same_owner = bot
                    .native_walk
                    .as_ref()
                    .zip(authority.as_ref())
                    .is_some_and(|(old, new)| {
                        old.run() == new.run() && old.request_id() == new.request_id()
                    })
                    || (authority.is_none()
                        && bot.native_walk.is_none()
                        && bot.walk_request_id == request_id
                        && bot.requested_route == Some(original_key));
                same_owner
                    .then(|| {
                        bot.route_basis
                            .as_ref()
                            .map(|basis| (Arc::clone(basis), bot.requested_exclusions.clone()))
                    })
                    .flatten()
            })
        };
        let mut state = self.state.take();
        if state
            .as_ref()
            .is_none_or(|state| state.quest_evidence.is_none())
        {
            if let Some(snapshot) = snapshot {
                let context = self
                    .navs
                    .lock()
                    .unwrap()
                    .get(&self.name)
                    .and_then(|bot| bot.quest_context.clone());
                if let Some(context) = context.as_ref() {
                    let base_state = state
                        .take()
                        .unwrap_or_else(|| Arc::new(WorldState::from_snapshot(snapshot)));
                    let evidence =
                        captured_route_quests(Some(context), snapshot, None, context.stamp);
                    state = match patch_route_quest_evidence(base_state, evidence) {
                        Ok(state) => Some(state),
                        Err(_) => {
                            self.refuse(
                                to,
                                radius,
                                opts.allow_teleports,
                                request_id,
                                authority.as_ref(),
                                None,
                            );
                            return false;
                        }
                    };
                }
            }
        }
        let (mut opts, exclusions) = if let Some((basis, exclusions)) = frozen {
            let base_state = state
                .take()
                .unwrap_or_else(|| Arc::new(WorldState::empty()));
            state = match patch_route_quest_evidence(base_state, basis.quest_evidence.clone()) {
                Ok(state) => Some(state),
                Err(_) => {
                    self.refuse(
                        to,
                        radius,
                        opts.allow_teleports,
                        request_id,
                        authority.as_ref(),
                        None,
                    );
                    return false;
                }
            };
            (basis.options, exclusions)
        } else {
            let empty = WorldState::empty();
            let route_state = state.as_deref().unwrap_or(&empty);
            match resolve_route_exclusions(opts, world, from, to, route_state, exclusions) {
                Ok((opts, exclusions)) => {
                    let exclusions = (!exclusions.is_empty()).then(|| Arc::new(exclusions));
                    (opts, exclusions)
                }
                Err(reason) => {
                    log_walk_arm(&self.name, || {
                        format!("queue_route refused {reason} dest={to:?} request_id={request_id}")
                    });
                    if let Some(authority) = authority.as_ref() {
                        self.refuse_native_with_detail(
                            authority.clone(),
                            Some(Arc::from(reason.as_str())),
                        );
                    } else {
                        self.publish_refusal(to, radius, opts.allow_teleports, request_id);
                    }
                    return false;
                }
            }
        };
        let key = (
            to,
            radius,
            opts.allow_teleports,
            opts.allow_wilderness,
            opts.allow_bank_fetch,
            opts.zones,
        );
        {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
            if let Some(result) = self.gate_route(
                bot,
                to,
                radius,
                key,
                &exclusions,
                retarget,
                request_id,
                authority.as_ref(),
                loc_id,
                arrival,
                compat_tile_intent,
            ) {
                return result;
            }
        }
        let captured_input = request_admission
            .map(|admission| admission.input)
            .or_else(|| {
                snapshot.map(|snapshot| {
                    crate::admission::capture(
                        snapshot,
                        state.as_ref().is_some_and(|state| state.map_members),
                        self.navs
                            .lock()
                            .unwrap()
                            .get(&self.name)
                            .and_then(|bot| bot.condition.current().copied())
                            .as_ref(),
                        false,
                    )
                })
            });
        // Resolve live-scene geometry only after this request passes the
        // refusal/coalescing gates, but outside the process-wide nav mutex.
        // The second gate below closes the race with another arming thread.
        let live_geometry = if radius > 0 {
            snapshot.and_then(|snapshot| {
                loc_id.and_then(|id| capture_loc_geometry(snapshot, to, radius, id))
            })
        } else {
            None
        };
        let loc_geometry = (
            loc_id.is_some()
                && snapshot.is_some_and(|snapshot| {
                    api::query::SceneQuery::new(snapshot.scene(), None).contains(to)
                }),
            snapshot.is_some_and(|snapshot| {
                loc_id.is_some_and(|id| {
                    api::query::loc_approach::arrived_at(snapshot, from, to, radius, id).is_some()
                })
            }),
        );
        let allow_teleports = opts.allow_teleports;
        let allow_wilderness = opts.allow_wilderness;
        let allow_bank_fetch = opts.allow_bank_fetch;
        let (generation, displaced, displaced_kind) = {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
            if let Some(result) = self.gate_route(
                bot,
                to,
                radius,
                key,
                &exclusions,
                retarget,
                request_id,
                authority.as_ref(),
                loc_id,
                arrival,
                compat_tile_intent,
            ) {
                return result;
            }
            if let Some(ess) = bot.traveller.essence() {
                opts.essence = Some(ess);
            }
            let same_owner = bot
                .native_walk
                .as_ref()
                .zip(authority.as_ref())
                .is_some_and(|(old, new)| {
                    old.run() == new.run() && old.request_id() == new.request_id()
                })
                || (authority.is_none()
                    && bot.native_walk.is_none()
                    && bot.walk_request_id == request_id
                    && bot.requested_route == Some(key));
            let mut admission = request_admission
                .or_else(|| {
                    same_owner
                        .then(|| bot.admission.as_deref().copied())
                        .flatten()
                })
                .unwrap_or_else(|| crate::admission::Admission {
                    policy: if bot.compat_v1 || opts.zones.is_all() {
                        script::native::RiskPolicy::Proceed
                    } else {
                        script::native::RiskPolicy::Avoid
                    },
                    enforce: super::walk_permissions::NET_AVAILABLE
                        || opts.zones != ZoneExempt::NONE,
                    grants: opts.zones,
                    allow: script::native::WalkAllow {
                        prayer: !bot.compat_v1,
                        food: !bot.compat_v1,
                        escape: !bot.compat_v1,
                    },
                    input: Default::default(),
                    generation: 0,
                    compat_v1: bot.compat_v1,
                    escape: None,
                });
            if let Some(mut input) = captured_input {
                input.poison = admission.input.poison;
                admission.input = input;
            }
            if !same_owner {
                bot.end_native_walk(script::native::WalkEnd::Cancelled);
                bot.route_basis = None;
                bot.assessment = None;
                bot.risk_refusal = None;
                bot.admission_pending = false;
            }
            if bot.bank_pick.walking(bot.route_generation) {
                bot.bank_pick.reset();
            }
            if bot
                .route_ticket
                .as_ref()
                .is_some_and(|owner| matches!(&owner.kind, RouteDemandKind::Walk { .. }))
            {
                bot.supersede_walk_route_work();
            }
            let displaced = bot.pending_route.take();
            let displaced_kind = displaced.as_ref().and_then(|demand| demand.kind());
            admission.input.set_off_debt(bot.walk_guard_off.is_some());
            admission.grants = opts.zones;
            admission.generation = bot.route_generation.wrapping_add(1);
            match bot.admission.as_deref_mut() {
                Some(pending) => *pending = admission,
                None => bot.admission = Some(Box::new(admission)),
            }
            if bot.bank_pick.walking(bot.route_generation) {
                bot.bank_pick.reset_for_route_replacement();
            }
            bot.route_generation = admission.generation;
            if authority.is_none() {
                if let Some(walk) = bot
                    .compat_walk
                    .as_mut()
                    .filter(|walk| walk.request_id == request_id && walk.outer.is_some())
                {
                    walk.generation = bot.route_generation;
                }
            }
            bot.manual_walk_cancelled_detail = false;
            bot.walk_request_id = request_id;
            bot.native_walk_blocked = None;
            bot.walk_outcome_detail = None;
            bot.requested_route = Some(key);
            bot.route_loc_id = loc_id;
            bot.route_arrival = arrival;
            bot.route_loc_geometry = loc_geometry;
            bot.requested_exclusions = exclusions.clone();
            bot.native_walk = authority.clone();
            bot.native_walk_failure = None;
            bot.route_compat_tile_intent = compat_tile_intent;
            let request = ScriptRouteRequest {
                candidate_goals,
                generation: bot.route_generation,
                request_id,
                world: Arc::clone(world),
                from,
                to,
                radius,
                loc_id,
                arrival,
                opts,
                admission,
                state,
                prereq_catalog: bot
                    .quest_context
                    .as_ref()
                    .and_then(|context| context.prereq_catalog.clone()),
                compat_tile_intent,
                bank_origin: self.bank.origin,
                bank: self.bank.rows.clone(),
                live_geometry,
                exclusions,
                completion,
            };
            bot.pending_route = Some(Box::new(RouteDemand::Pending {
                request: Box::new(request),
                kind: RouteDemandKind::Walk { authority },
            }));
            (bot.route_generation, displaced, displaced_kind)
        };
        if let Some(displaced) = displaced {
            (*displaced).signal_completion();
        }
        if let Some(RouteDemandKind::Advisory {
            authority,
            evidence,
        }) = displaced_kind
        {
            if authority.live() {
                let mut navs = self
                    .navs
                    .lock()
                    .expect("navigation state mutex is not poisoned");
                if let Some(bot) = navs.get_mut(&self.name) {
                    if bot.advisory_request_id == authority.request_id().get() {
                        bot.assess_result = Some(Box::new((
                            authority.clone(),
                            advisory_refusal(
                                &authority,
                                evidence,
                                "Advisory route was superseded by a walk",
                            ),
                        )));
                    }
                }
            }
        }
        log_walk_arm(&self.name, || {
            format!(
                "queue_route admitted dest={to:?} r={radius} request_id={request_id} \
                 generation={generation} from={from:?} allow_teleports={} \
                 allow_wilderness={} allow_bank_fetch={}{}",
                allow_teleports,
                allow_wilderness,
                allow_bank_fetch,
                if world.graph.zones.is_none() {
                    " zones: unavailable (legacy grid pack)"
                } else {
                    ""
                }
            )
        });
        true
    }
}

/// Captured loc footprint input. The same scalar geometry/window are scanned
/// by selection without copying this scene-clipped collision-flag array.
#[derive(Debug)]
pub(crate) struct LocGeometryCapture {
    geometry: crate::walk_permissions::LocAccessGeometry,
    window: crate::walk_permissions::LocAccessWindow,
    flags: Vec<i32>,
}

/// Capture only the footprint's operable window, clipped to the live scene.
/// `None` preserves the legacy distinction between an unmodeled/off-scene loc
/// and a modeled loc with no legal stands.
fn capture_loc_geometry(
    snapshot: &GameSnapshot,
    to: WorldTile,
    radius: i32,
    loc_id: i32,
) -> Option<LocGeometryCapture> {
    let (geometry, window, scene) =
        crate::walk_permissions::route_loc_access_geometry(snapshot, to, loc_id, radius)?;
    let mut flags = Vec::new();
    flags.try_reserve_exact(window.len()).ok()?;
    for index in 0..window.len() {
        let flags_at = window
            .collision_index(index, scene.height)
            .and_then(|index| scene.collision_flags.get(index).copied())
            .unwrap_or(client::dash3d::CollisionFlag::SQ_BLOCKED);
        flags.push(flags_at);
    }
    Some(LocGeometryCapture {
        geometry,
        window,
        flags,
    })
}

impl LocGeometryCapture {
    fn candidate_at(&self, index: usize) -> Option<WorldTile> {
        let tile = self.window.tile_at(index)?;
        let flags = *self.flags.get(index)?;
        self.geometry.candidate_at(tile, flags)
    }

    #[cfg(test)]
    fn candidates(&self) -> Vec<WorldTile> {
        (0..self.flags.len())
            .filter_map(|index| self.candidate_at(index))
            .collect()
    }
}
/// Bounded target construction for one admitted route job. It retains only
/// the candidate array; target ordering is an in-place cooperative heapsort.
struct RouteTargetCursor {
    mode: RouteTargetMode,
    index: usize,
    radius: Option<crate::walk_permissions::RouteRadiusGeometry>,
    targets: Vec<WorldTile>,
    sort: Option<RouteTargetSort>,
}

#[derive(Clone, Copy)]
enum RouteTargetMode {
    Exact,
    Geometry,
    Radius,
    Candidates,
}

impl RouteTargetCursor {
    fn shape(
        request: &ScriptRouteRequest,
    ) -> Result<
        (
            RouteTargetMode,
            usize,
            Option<crate::walk_permissions::RouteRadiusGeometry>,
        ),
        RouteJobError,
    > {
        if let Some(goals) = &request.candidate_goals {
            return Ok((RouteTargetMode::Candidates, goals.as_slice().len(), None));
        }
        if request.radius <= 0 {
            return Ok((RouteTargetMode::Exact, 1, None));
        }
        if let Some(geometry) = request.live_geometry.as_ref() {
            return Ok((RouteTargetMode::Geometry, geometry.flags.len(), None));
        }
        let radius = crate::walk_permissions::RouteRadiusGeometry::new(request.to, request.radius)
            .ok_or(RouteJobError::Budget)?;
        let capacity = radius
            .len()
            .checked_add(usize::from(
                request.compat_tile_intent
                    && request.loc_id.is_none()
                    && request.arrival != ArrivalKind::Area,
            ))
            .ok_or(RouteJobError::Budget)?;
        Ok((RouteTargetMode::Radius, capacity, Some(radius)))
    }

    fn capacity(request: &ScriptRouteRequest) -> Result<usize, RouteJobError> {
        Self::shape(request).map(|(_, capacity, _)| capacity)
    }

    fn new(request: &ScriptRouteRequest) -> Result<Self, RouteJobError> {
        let (mode, capacity, radius) = Self::shape(request)?;
        let mut targets = Vec::new();
        targets
            .try_reserve_exact(capacity)
            .map_err(|_| RouteJobError::Allocation)?;
        Ok(Self {
            mode,
            index: 0,
            radius,
            targets,
            sort: None,
        })
    }

    /// Advance one candidate or ordering unit. `true` means all candidates
    /// are ready for the optional arrival filter.
    fn advance(&mut self, request: &ScriptRouteRequest) -> bool {
        if let Some(sort) = self.sort.as_mut() {
            if sort.advance(&mut self.targets, request.from, request.to) {
                self.sort = None;
                return true;
            }
            return false;
        }
        match self.mode {
            RouteTargetMode::Candidates => {
                let goals = request
                    .candidate_goals
                    .as_ref()
                    .expect("candidate mode")
                    .as_slice();
                if let Some(&tile) = goals.get(self.index) {
                    if request.world.collision.standable(tile) {
                        self.targets.push(tile);
                    }
                }
                self.index += 1;
                self.index >= goals.len()
            }
            RouteTargetMode::Exact => {
                if self.index == 0 {
                    self.targets.push(request.to);
                    self.index = 1;
                }
                true
            }
            RouteTargetMode::Geometry => {
                let Some(geometry) = request.live_geometry.as_ref() else {
                    return true;
                };
                if let Some(tile) = geometry.candidate_at(self.index) {
                    self.targets.push(tile);
                }
                self.index += 1;
                self.index >= geometry.flags.len()
            }
            RouteTargetMode::Radius => {
                let Some(radius) = self.radius else {
                    return true;
                };
                if self.index >= radius.len() {
                    return true;
                }
                if let Some(tile) = radius.candidate_at(self.index) {
                    if request.world.collision.standable(tile) {
                        self.targets.push(tile);
                    }
                }
                self.index += 1;
                false
            }
        }
    }

    fn start_sort(&mut self) {
        if matches!(self.mode, RouteTargetMode::Radius) {
            self.sort = Some(RouteTargetSort::new(self.targets.len()));
        }
    }
}

/// Cooperative in-place max-heapsort. The route target key includes x/z, so
/// distinct grid candidates have a total order even though heapsort is not
/// stable.
struct RouteTargetSort {
    build_parent: usize,
    end: usize,
    sift: Option<(usize, usize)>,
    extracting: bool,
}

impl RouteTargetSort {
    fn new(len: usize) -> Self {
        Self {
            build_parent: len / 2,
            end: len,
            sift: None,
            extracting: false,
        }
    }

    fn advance(&mut self, targets: &mut [WorldTile], from: WorldTile, to: WorldTile) -> bool {
        let key = |tile: WorldTile| {
            (
                tile.x.abs_diff(from.x).max(tile.z.abs_diff(from.z)),
                tile.x.abs_diff(to.x).max(tile.z.abs_diff(to.z)),
                tile.x,
                tile.z,
            )
        };
        if targets.len() < 2 {
            return true;
        }
        if let Some((root, limit)) = self.sift {
            let left = root.saturating_mul(2).saturating_add(1);
            if left >= limit {
                self.sift = None;
                return false;
            }
            let right = left + 1;
            let largest = if right < limit && key(targets[right]) > key(targets[left]) {
                right
            } else {
                left
            };
            if key(targets[root]) < key(targets[largest]) {
                targets.swap(root, largest);
                self.sift = Some((largest, limit));
            } else {
                self.sift = None;
            }
            return false;
        }
        if !self.extracting {
            if self.build_parent != 0 {
                self.build_parent -= 1;
                self.sift = Some((self.build_parent, self.end));
                return false;
            }
            self.extracting = true;
        }
        if self.end <= 1 {
            return true;
        }
        self.end -= 1;
        targets.swap(0, self.end);
        self.sift = Some((0, self.end));
        false
    }
}

/// The compat target is a stable first goal, even when its normal radius key
/// would sort later. Both the search and the shift are cooperative.
struct RouteTargetPriority {
    target: WorldTile,
    scan: usize,
    found: Option<usize>,
    shift: Option<usize>,
    phase: RouteTargetPriorityPhase,
}

#[derive(Clone, Copy)]
enum RouteTargetPriorityPhase {
    Scan,
    ShiftExisting,
    ShiftInserted,
    Done,
}

impl RouteTargetPriority {
    fn new(request: &ScriptRouteRequest) -> Option<Self> {
        (request.compat_tile_intent
            && request.loc_id.is_none()
            && request.arrival != ArrivalKind::Area
            && request.radius > 0
            && request.world.collision.walkable(request.to)
            && request.world.collision.standable(request.to))
        .then_some(Self {
            target: request.to,
            scan: 0,
            found: None,
            shift: None,
            phase: RouteTargetPriorityPhase::Scan,
        })
    }

    fn advance(&mut self, targets: &mut Vec<WorldTile>) -> bool {
        match self.phase {
            RouteTargetPriorityPhase::Scan => {
                if let Some(&tile) = targets.get(self.scan) {
                    if tile == self.target {
                        self.found = Some(self.scan);
                        self.shift = Some(self.scan);
                        self.phase = RouteTargetPriorityPhase::ShiftExisting;
                    } else {
                        self.scan += 1;
                    }
                } else {
                    targets.push(self.target);
                    self.shift = Some(targets.len() - 1);
                    self.phase = RouteTargetPriorityPhase::ShiftInserted;
                }
                false
            }
            RouteTargetPriorityPhase::ShiftExisting => {
                let shift = self.shift.unwrap_or_default();
                if shift == 0 {
                    targets[0] = self.target;
                    self.phase = RouteTargetPriorityPhase::Done;
                    true
                } else {
                    targets[shift] = targets[shift - 1];
                    self.shift = Some(shift - 1);
                    false
                }
            }
            RouteTargetPriorityPhase::ShiftInserted => {
                let shift = self.shift.unwrap_or_default();
                if shift == 0 {
                    targets[0] = self.target;
                    self.phase = RouteTargetPriorityPhase::Done;
                    true
                } else {
                    targets[shift] = targets[shift - 1];
                    self.shift = Some(shift - 1);
                    false
                }
            }
            RouteTargetPriorityPhase::Done => true,
        }
    }
}

#[cfg(test)]
/// Anchor-radius destinations. Unknown/off-scene explicit locs need only a
/// standable approach estimate; plain tile walks retain target-side filtering.
/// Neither estimate admission nor scene proximity proves loc arrival.
pub(crate) fn approach_tiles(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    radius: i32,
    estimate: bool,
) -> Vec<WorldTile> {
    let r = radius.clamp(0, 104);
    let mut tiles = Vec::new();
    for dx in -r..=r {
        for dz in -r..=r {
            let (Some(x), Some(z)) = (to.x.checked_add(dx), to.z.checked_add(dz)) else {
                continue;
            };
            let t = WorldTile {
                x,
                z,
                level: to.level,
            };
            if world.collision.standable(t) {
                tiles.push(t);
            }
        }
    }
    if !estimate {
        let collision = &world.collision;
        api::query::retain_arrival_candidates(&mut tiles, to, r, |tile: WorldTile| {
            let x = usize::try_from(tile.x.checked_sub(collision.origin.x)?).ok()?;
            let z = usize::try_from(tile.z.checked_sub(collision.origin.z)?).ok()?;
            ((0..=3).contains(&tile.level) && x < collision.width && z < collision.height)
                .then(|| collision.walkable_word(tile.x, tile.z, tile.level) as i32)
        });
    }
    tiles.sort_by_key(|t| {
        (
            t.x.abs_diff(from.x).max(t.z.abs_diff(from.z)),
            t.x.abs_diff(to.x).max(t.z.abs_diff(to.z)),
            t.x,
            t.z,
        )
    });
    tiles
}

fn advisory_refusal(
    authority: &script::native::HostAuthority,
    evidence: api::quest_progress::EvidenceStamp,
    detail: &str,
) -> script::native::AssessReceipt {
    script::native::AssessReceipt {
        request_id: authority.request_id().get(),
        evidence,
        assessment: None,
        refusal: Some(script::native::WalkRefusal::NoRouteWithinBounds {
            tried: 0,
            last: None,
        }),
        detail: Some(Arc::from(detail)),
        route_ticks: 0,
    }
}

pub(crate) struct ScriptRouteRequest {
    pub(crate) generation: u64,
    pub(crate) request_id: u64,
    pub(crate) world: Arc<NavWorld>,
    pub(crate) from: WorldTile,
    pub(crate) to: WorldTile,
    pub(crate) candidate_goals: Option<script::native::WalkGoals>,
    pub(crate) radius: i32,
    pub(crate) loc_id: Option<i32>,
    pub(crate) arrival: ArrivalKind,
    pub(crate) opts: FindOptions,
    pub(crate) state: Option<Arc<WorldState>>,
    pub(crate) admission: crate::admission::Admission,
    pub(crate) prereq_catalog: Option<Arc<api::prereq_steps::PrereqCatalog>>,
    /// The arm's bank memory ([`ScriptWalkArm::bank`]), its origin beside
    /// its rows so the byte packs into this request's own padding.
    pub(crate) bank_origin: Origin,
    pub(crate) bank: Vec<(i32, i32)>,
    /// Explicit live-loc footprint input; stands are built after admission.
    pub(crate) live_geometry: Option<LocGeometryCapture>,
    /// Frozen request exclusions: every search keeps out of rects and carries
    /// the original wire entries across a resume for arm-time re-resolution.
    pub(crate) exclusions: Option<Arc<ScriptRouteExclusions>>,
    /// A compat tile walk prefers its requested walkable tile, then falls back
    /// to the ordinary in-radius endpoints if that route is unavailable.
    pub(crate) compat_tile_intent: bool,
    pub(crate) completion: RouteCompletion,
}

#[cfg(test)]
// Refusal hints must not fan out into a separate full-world diagnosis for
// every Area goal. Routing still searches the complete goal set; only these
// optional bank-fetch attribution probes have a fixed candidate budget.
const BANK_ZONE_DIAGNOSTIC_TARGETS: usize = 8;
impl ScriptRouteRequest {
    #[cfg(test)]
    fn targets(&self) -> Vec<WorldTile> {
        if let Some(goals) = &self.candidate_goals {
            goals
                .as_slice()
                .iter()
                .copied()
                .filter(|&tile| self.world.collision.standable(tile))
                .collect()
        } else if self.radius <= 0 {
            vec![self.to]
        } else if let Some(geometry) = self.live_geometry.as_ref() {
            geometry.candidates()
        } else {
            let mut targets = approach_tiles(
                &self.world,
                self.from,
                self.to,
                self.radius,
                self.loc_id.is_some() || self.arrival == ArrivalKind::Area,
            );
            if self.compat_tile_intent
                && self.loc_id.is_none()
                && self.arrival != ArrivalKind::Area
                && self.world.collision.walkable(self.to)
                && self.world.collision.standable(self.to)
            {
                // Prefer the requested tile, but retain today's arrival-valid
                // endpoints so routing constraints can fall back without
                // reporting success outside the requested radius.
                targets.retain(|target| *target != self.to);
                targets.insert(0, self.to);
            }
            targets
        }
    }

    fn avoid(&self) -> &[AvoidRect] {
        self.exclusions
            .as_deref()
            .map_or(&[][..], |exclusions| exclusions.avoid.as_slice())
    }
    #[cfg(test)]
    fn blocking_zones(
        &self,
        outcome: &RouteOutcome,
        targets: &[WorldTile],
    ) -> Option<Vec<ZoneKey>> {
        if !matches!(outcome, RouteOutcome::NoPath) {
            return None;
        }
        let empty = WorldState::empty();
        let state = self.state.as_deref().unwrap_or(&empty);
        let bank_targets = if self.radius <= 0 || self.arrival == ArrivalKind::Area {
            targets
        } else {
            self.live_geometry.as_ref().map_or(&[][..], |_| targets)
        };
        let direct = match route_blocker_diagnosis(self) {
            BlockerDiagnosis::SingleTarget => nav_planner::router::find_blocking_zones(
                &self.world.collision,
                &self.world.graph,
                self.from,
                self.to,
                self.opts,
                state,
                self.avoid(),
            ),
            BlockerDiagnosis::FirstGoal => nav_planner::router::find_first_blocking_zones(
                &self.world.collision,
                &self.world.graph,
                self.from,
                targets,
                self.opts,
                state,
                self.avoid(),
            ),
        }
        .filter(|keys| !keys.is_empty());
        if direct.is_some() {
            return direct;
        }
        if !self.opts.allow_bank_fetch {
            return None;
        }
        let strict_opts = FindOptions {
            allow_bank_fetch: false,
            ..self.opts
        };
        for &target in bank_targets.iter().take(BANK_ZONE_DIAGNOSTIC_TARGETS) {
            #[cfg(test)]
            diagnostic_tests::BANK_TARGET_PROBES.with(|count| count.set(count.get() + 1));
            let Some(missing) = find_missing_item_reqs_with_avoid(
                &self.world.collision,
                &self.world.graph,
                self.from,
                target,
                self.opts,
                state,
                self.avoid(),
            ) else {
                continue;
            };
            let plan = match plan_bank_fetch(
                &missing,
                state,
                &planning_rows(self.bank_origin, &self.bank, &missing),
                self.world.banks(),
                self.from,
                &self.world.collision,
                &self.world.graph,
                self.opts,
                self.avoid(),
            ) {
                Ok(plan) => plan,
                Err(BankFetchError::Access { blocked_zones, .. }) if !blocked_zones.is_empty() => {
                    return Some(blocked_zones);
                }
                Err(_) => continue,
            };
            if let Some(keys) = nav_planner::router::find_blocking_zones(
                &self.world.collision,
                &self.world.graph,
                self.from,
                target,
                strict_opts,
                &plan.state,
                self.avoid(),
            )
            .filter(|keys| !keys.is_empty())
            {
                return Some(keys);
            }
        }
        None
    }
}
#[cfg(test)]
impl ScriptRouteRequest {
    /// Convert the completed production job to the test route view.
    fn project_route_job_outcome(
        outcome: RouteJobOutcome,
        opts: FindOptions,
        bank_avoid_copy: Vec<AvoidRect>,
        world: &NavWorld,
    ) -> RouteOutcome {
        match outcome {
            RouteJobOutcome::Route(route) => RouteOutcome::Routed(route.to_route(&world.graph)),
            RouteJobOutcome::Bank {
                steps,
                route,
                final_route,
            } => {
                let final_route = final_route.to_route(&world.graph);
                let route = route
                    .map(|route| route.to_route(&world.graph))
                    .unwrap_or_else(|| final_route.clone());
                RouteOutcome::BankSession {
                    pending: PendingBankFetch {
                        steps,
                        dest: final_route.dest,
                        opts,
                        final_route: RoutePath::legacy(Arc::new(final_route.clone())),
                        avoid: bank_avoid_copy,
                        progress: Default::default(),
                    },
                    route,
                }
            }
            RouteJobOutcome::NoPath => RouteOutcome::NoPath,
        }
    }

    fn route_test_identity(index: u64) -> WorkIdentity {
        WorkIdentity::new(
            RequestId::from_raw(RequestId::HOST_FIRST + index)
                .expect("test route identity is valid"),
            api::identity::ProcessIncarnation::from_start_nonce(
                std::num::NonZeroU64::new(1).expect("test incarnation is nonzero"),
            ),
            Scope::Slot(
                api::identity::SlotKey::from_incarnation(1)
                    .expect("test slot incarnation is valid"),
            ),
            None,
        )
        .expect("test route identity is valid")
    }

    fn drive_route_stage_for_test(worker: &mut RouteWorkJob) -> RouteStageResult {
        use api::identity::Deadline;
        use api::work::{MonotonicClock, SharedCancellation, SliceBudget, SystemClock};

        let clock = SystemClock;
        let cancellation = SharedCancellation::new();
        // Keep production's 64-unit yields, including in the real-pack
        // parity driver. Its finite 4M-settlement queries need more slices.
        for _ in 0..25_000_000 {
            let budget = SliceBudget::new(
                std::num::NonZeroU32::new(script::work_admission::WORK_UNITS_PER_SLICE)
                    .expect("slice budget is nonzero"),
                Deadline::after(clock.now(), script::work_admission::WORK_SLICE),
            );
            let mut context = StepContext::new(&clock, budget, &cancellation);
            match worker.step(&mut context) {
                StepOutcome::Yielded => {}
                StepOutcome::Completed(result) => return result,
                StepOutcome::Failed(error) => panic!("route stage failed: {error}"),
            }
        }
        panic!("route stage exceeded the bounded slice count");
    }

    fn drive_prepared_route_job_for_test(
        job: RouteJob,
        world: Arc<NavWorld>,
    ) -> (RouteJobOutput, Vec<WorldTile>, Arc<NavWorld>) {
        let mut next = Some(Box::new(job));
        for stage_index in 0..64u64 {
            let job = next.take().expect("the prior route stage continued");
            let mut worker = RouteWorkJob::new(job);
            match Self::drive_route_stage_for_test(&mut worker) {
                RouteStageResult::Continue(mut job) => {
                    assert!(job.search_cursor.is_none());
                    job.prepare_continuation()
                        .expect("test route continuation prepares");
                    job.identity = Some(Self::route_test_identity(stage_index + 1));
                    next = Some(job);
                }
                RouteStageResult::Complete(output) => {
                    let targets = worker
                        .job
                        .as_ref()
                        .expect("completed route worker retains its job")
                        .targets
                        .clone();
                    return (output, targets, world);
                }
            }
        }
        panic!("route job exceeded the bounded stage count");
    }

    fn drive_route_job_output_for_test(self) -> (RouteJobOutput, Vec<WorldTile>, Arc<NavWorld>) {
        self.drive_route_job_output_with_diagnostic_budget_for_test(DIAGNOSTIC_BUDGET)
    }

    fn drive_route_job_output_with_diagnostic_budget_for_test(
        self,
        diagnostic_budget: usize,
    ) -> (RouteJobOutput, Vec<WorldTile>, Arc<NavWorld>) {
        let world = Arc::clone(&self.world);
        let mut job = RouteJob::prepare(
            "route-rule-test",
            Box::new(self),
            RouteDemandKind::Walk { authority: None },
        )
        .expect("test route request prepares");
        job.diagnostic_budget = diagnostic_budget;
        job.identity = Some(Self::route_test_identity(0));
        Self::drive_prepared_route_job_for_test(job, world)
    }

    /// Drive admission through the production route job and preserve its output.
    pub(crate) fn drive_route_admission_for_test(
        self,
    ) -> (
        crate::admission::RouteAdmission,
        Vec<WorldTile>,
        FindOptions,
    ) {
        let (output, targets, world) = Self::drive_route_job_output_for_test(self);
        let RouteJobOutput {
            outcome,
            opts,
            bank_avoid_copy,
            assessment,
            refusal,
            blocked,
            ..
        } = output;
        let outcome = Self::project_route_job_outcome(outcome, opts, bank_avoid_copy, &world);
        let tried = match &refusal {
            Some(script::native::WalkRefusal::NoRouteWithinBounds { tried, .. }) => *tried,
            _ => 0,
        };
        (
            crate::admission::RouteAdmission {
                outcome,
                assessment,
                refusal,
                blocked,
                tried,
            },
            targets,
            opts,
        )
    }

    /// Drive this request through the cooperative route jobs used by the production worker.
    /// Keep Admission unchanged because its policy can change production search options.
    pub(crate) fn drive_route_job_for_test(self) -> (RouteOutcome, Vec<WorldTile>) {
        let (output, targets, world) = Self::drive_route_job_output_for_test(self);
        let RouteJobOutput {
            outcome,
            opts,
            bank_avoid_copy,
            ..
        } = output;
        (
            Self::project_route_job_outcome(outcome, opts, bank_avoid_copy, &world),
            targets,
        )
    }
}
impl NavBot {
    /// Retain policy only after the host accepted this exact Load request.
    pub(crate) fn retain_compat_walk(
        &mut self,
        request_id: u64,
        mut options: script::native::WalkOptions,
        prereq_allow: script::native::PrereqAllow,
    ) -> bool {
        let Some(context) = self.quest_context.as_ref() else {
            return false;
        };
        let Some(key) = self.requested_route else {
            return false;
        };
        if request_id == 0 || request_id != self.walk_request_id || self.native_walk.is_some() {
            return false;
        }
        if let Some(live) = self.compat_walk.as_ref() {
            return live.request_id == request_id && live.run == context.stamp.run;
        }
        let Ok(radius) = u16::try_from(key.1) else {
            return false;
        };
        let Some(admission) = self.admission.as_deref() else {
            return false;
        };
        options.allow_teleports = script::native::WalkBit::from(Some(key.2));
        options.allow_wilderness = script::native::WalkBit::from(Some(key.3));
        options.allow_danger_zones = match admission.policy {
            script::native::RiskPolicy::Inherit => script::native::WalkBit::Inherit,
            script::native::RiskPolicy::Proceed => script::native::WalkBit::Allow,
            script::native::RiskPolicy::Avoid => script::native::WalkBit::Forbid,
        };
        let exclusions = self.requested_exclusions.clone();
        let (avoid, cross) = exclusions.as_ref().map_or_else(
            || (Box::default(), Box::default()),
            |exclusions| {
                (
                    exclusions
                        .avoid_wire
                        .iter()
                        .filter_map(|entry| match entry {
                            script::interact::InspectAvoidWire::Catalog(name) => {
                                Some(Arc::<str>::from(name.as_str()))
                            }
                            _ => None,
                        })
                        .collect(),
                    exclusions.cross.clone().into_boxed_slice(),
                )
            },
        );
        self.compat_walk = Some(Box::new(CompatWalk {
            run: context.stamp.run,
            request_id,
            generation: self.route_generation,
            key,
            exclusions,
            outer: Some(script::native::OuterWalk {
                identity: script::native::OuterIdentity::Compat { request_id },
                target: key.0,
                radius,
                arrival: self.route_arrival,
                avoid,
                cross,
                options,
                allow_bank_fetch: key.4,
                prereq_allow,
            }),
        }));
        true
    }

    pub(crate) fn compat_walk_request_id(&self) -> Option<u64> {
        self.compat_walk.as_ref().map(|walk| walk.request_id)
    }

    pub(crate) fn compat_walk_run(&self) -> Option<api::selected::RunKey> {
        self.compat_walk.as_ref().map(|walk| walk.run)
    }

    pub(crate) fn compat_walk_context_current(&self) -> bool {
        self.compat_walk.as_ref().is_some_and(|walk| {
            self.quest_context
                .as_ref()
                .is_some_and(|context| context.stamp.run == walk.run)
        })
    }

    pub(crate) fn compat_walk_direct_terminal(&self) -> bool {
        self.compat_walk.as_ref().is_some_and(|walk| {
            walk.outer.is_some()
                && self.walk_outcome_seq != 0
                && walk.generation == self.walk_outcome_generation
                && walk.request_id == self.walk_outcome_request_id
        })
    }

    fn compat_walk_native_policy(
        &self,
        run: api::selected::RunKey,
    ) -> Option<(FindOptions, Option<Arc<ScriptRouteExclusions>>)> {
        let walk = self.compat_walk.as_ref()?;
        if walk.run != run || walk.outer.is_some() || !self.compat_walk_context_current() {
            return None;
        }
        Some((
            FindOptions {
                allow_teleports: walk.key.2,
                allow_wilderness: walk.key.3,
                allow_bank_fetch: walk.key.4,
                zones: walk.key.5,
                ..Default::default()
            },
            walk.exclusions.clone(),
        ))
    }

    pub(crate) fn compat_walk_is_following(
        &self,
        request_id: u64,
        key: (WorldTile, i32, bool, bool, bool),
        exclusions: &ScriptRouteExclusions,
    ) -> bool {
        let same_exclusions = |original: Option<&ScriptRouteExclusions>| {
            original.map_or_else(
                || exclusions.is_empty(),
                |original| {
                    original.avoid_wire == exclusions.avoid_wire
                        && original.cross == exclusions.cross
                },
            )
        };
        if let Some(walk) = self.compat_walk.as_ref() {
            return self.compat_walk_context_current()
                && walk.request_id == request_id
                && (walk.key.0, walk.key.1, walk.key.2, walk.key.3, walk.key.4) == key
                && same_exclusions(walk.exclusions.as_deref());
        }
        self.native_walk.is_none()
            && self.walk_request_id == request_id
            && self.requested_route.is_some_and(|original| {
                (original.0, original.1, original.2, original.3, original.4) == key
            })
            && same_exclusions(self.requested_exclusions.as_deref())
            && (self.has_route() || self.route_work_pending())
    }

    pub(crate) fn retire_compat_walk(&mut self, request_id: u64) -> bool {
        if self.compat_walk_request_id() != Some(request_id) {
            return false;
        }
        self.compat_walk = None;
        true
    }

    pub(crate) fn compat_walk_refusal(
        &self,
    ) -> Option<&Arc<api::prereq_knowledge::RefusalObservation>> {
        let walk = self.compat_walk.as_ref()?;
        if walk.outer.is_none()
            || walk.generation != self.walk_outcome_generation
            || walk.request_id != self.walk_outcome_request_id
        {
            return None;
        }
        match self.native_walk_failure.as_ref()? {
            (request, script::native::WalkEnd::GateRefused(observation))
                if *request == walk.request_id =>
            {
                Some(observation)
            }
            _ => None,
        }
    }

    /// The observation must be recorded before this ownership transfer.
    pub(crate) fn take_compat_walk_handover(
        &mut self,
        here: WorldTile,
    ) -> Option<script::native::WalkHandover> {
        if !self.compat_walk_context_current() {
            return None;
        }
        let walk = self.compat_walk.as_mut()?;
        if walk.generation != self.walk_outcome_generation
            || walk.request_id != self.walk_outcome_request_id
        {
            return None;
        }
        let (request, failure) = self.native_walk_failure.as_ref()?;
        if *request != walk.request_id {
            return None;
        }
        let entry = match failure {
            script::native::WalkEnd::NeedsStep(gates) => {
                script::native::HandoverEntry::NeedsStep(Arc::clone(gates))
            }
            script::native::WalkEnd::GateRefused(observation)
                if observation.fact.observed.run == walk.run =>
            {
                script::native::HandoverEntry::GateRefused(Arc::clone(observation))
            }
            _ => return None,
        };
        let outer = walk.outer.take()?;
        super::script_walk::owe_walk_guard_off(self);
        self.route_generation = self.route_generation.wrapping_add(1);
        self.route_ticket = None;
        self.pending_route = None;
        self.clear_route();
        self.traveller.clear();
        self.bank_fetch = None;
        self.requested_route = None;
        self.route_quest_evidence = None;
        self.walk_request_id = 0;
        self.native_walk_failure = None;
        self.clear_walk_outcome();
        Some(script::native::WalkHandover { outer, entry, here })
    }

    pub(crate) fn compat_walk_handed_over(&self) -> bool {
        self.compat_walk
            .as_ref()
            .is_some_and(|walk| walk.outer.is_none())
    }

    /// Publish the wrapper's one logical terminal, never an inner native ID.
    pub(crate) fn complete_compat_walk(
        &mut self,
        request_id: u64,
        end: script::native::WalkEnd,
        detail: Option<Arc<str>>,
    ) -> bool {
        if self.compat_walk_request_id() != Some(request_id) {
            return false;
        }
        let walk = self.compat_walk.take().expect("matched compatibility walk");
        super::script_walk::owe_walk_guard_off(self);
        self.route_generation = self.route_generation.wrapping_add(1);
        self.route_ticket = None;
        self.pending_route = None;
        self.clear_route();
        self.traveller.clear();
        self.bank_fetch = None;
        self.requested_route = None;
        self.native_walk = None;
        self.native_walk_failure = None;
        self.route_quest_evidence = None;
        self.walk_request_id = 0;
        let (target, radius, teleports, ..) = walk.key;
        let blocked = matches!(
            &end,
            script::native::WalkEnd::Stalled(stall)
                if stall.why == script::native::WalkStallReason::EndBlocked
        );
        if blocked
            || matches!(
                &end,
                script::native::WalkEnd::Arrived | script::native::WalkEnd::RouteEnded
            )
        {
            self.note_route_end(
                self.route_generation,
                request_id,
                target,
                radius,
                teleports,
                blocked,
            );
        } else {
            self.note_failure(self.route_generation, request_id, target, radius, teleports);
        }
        if end == script::native::WalkEnd::UserInput {
            self.walk_outcome_cancel_reason = script::native::WalkCancelReason::UserInput;
        }
        self.walk_outcome_detail = detail;
        true
    }

    pub(crate) fn has_route(&self) -> bool {
        self.route.is_some() || self.compact_route.is_some()
    }

    /// Whether the slot has route or advisory work that needs a pump turn.
    pub(crate) fn route_work_pending(&self) -> bool {
        self.route_ticket.is_some() || self.pending_route.is_some()
    }

    /// Host walking work still in progress: a route being followed (legacy
    /// or compact), route work waiting for a pump turn or a worker, a bank
    /// fetch, or a carried walk. A registered native walk owner is not part
    /// of it, because the owner stays live after its route arrives.
    pub(crate) fn walk_work_live(&self) -> bool {
        self.has_route()
            || self.route_work_pending()
            || self.bank_fetch.is_some()
            || self.carried_walk.is_some()
    }

    #[cfg(test)]
    pub(crate) fn set_route_work_preparing_for_test(&mut self) {
        self.pending_route = Some(Box::new(RouteDemand::Preparing {
            generation: self.route_generation,
            request_id: self.walk_request_id,
            kind: RouteDemandKind::Walk { authority: None },
        }));
    }

    /// The gating facts of the route request still waiting for a pump turn.
    #[cfg(test)]
    pub(crate) fn pending_route_state_for_test(&self) -> Option<&WorldState> {
        match self.pending_route.as_deref()? {
            RouteDemand::Pending { request, .. } => request.state.as_deref(),
            RouteDemand::Preparing { .. } | RouteDemand::Prepared(_) => None,
        }
    }

    /// Abandon the walk's own route work (its accepted ticket and its queued
    /// demand) when bank work takes over the nav entry or the walk is revoked
    /// or ended. Advisory work has its own owner and is never the walk's: a
    /// live advisory and its result stay, and only an advisory whose owner is
    /// already revoked is dropped, without a receipt, since nobody awaits it.
    pub(crate) fn supersede_walk_route_work(&mut self) {
        if self
            .route_ticket
            .as_ref()
            .is_some_and(|owner| owner.kind.walk_cleanup_discards())
        {
            if let Some(mut owner) = self.route_ticket.take() {
                owner.abandon(Cancellation::Superseded);
            }
        }
        if self
            .pending_route
            .as_ref()
            .and_then(|demand| demand.kind())
            .is_none_or(|kind| kind.walk_cleanup_discards())
        {
            self.pending_route = None;
        }
        log_walk_arm_bot(|| {
            format!(
                "walk-cleanup kept_advisory_ticket={} kept_advisory_pending={} assess_result={:?} advisory_request_id={}",
                self.route_ticket.is_some(),
                self.pending_route.is_some(),
                self.assess_result
                    .as_ref()
                    .map(|result| result.1.request_id),
                self.advisory_request_id
            )
        });
    }

    /// The host ends all advisory route work of the slot (a pause, a relog or
    /// the end of the run). Work whose owner is still live gets exactly one
    /// correlated refusal receipt; work whose owner is revoked is dropped
    /// without one. The caller must not clear `assess_result` afterwards.
    pub(crate) fn cancel_advisory_route_work(&mut self, detail: &str) {
        let from_ticket = if self
            .route_ticket
            .as_ref()
            .is_some_and(|owner| matches!(owner.kind, RouteDemandKind::Advisory { .. }))
        {
            self.route_ticket.take().map(|mut owner| {
                owner.abandon(Cancellation::ScopeInvalidated);
                owner.kind.clone()
            })
        } else {
            None
        };
        let from_pending = if self
            .pending_route
            .as_ref()
            .and_then(|demand| demand.kind())
            .is_some_and(|kind| matches!(kind, RouteDemandKind::Advisory { .. }))
        {
            self.pending_route.take().and_then(|demand| {
                let kind = demand.kind();
                (*demand).signal_completion();
                kind
            })
        } else {
            None
        };
        // At most one advisory is current: the one the slot last queued.
        for kind in [from_ticket, from_pending].into_iter().flatten() {
            if let RouteDemandKind::Advisory {
                authority,
                evidence,
            } = kind
            {
                if authority.live() && self.advisory_request_id == authority.request_id().get() {
                    self.assess_result = Some(Box::new((
                        authority.clone(),
                        advisory_refusal(&authority, evidence, detail),
                    )));
                }
            }
        }
    }

    /// Native request IDs belong to their run, within the bound compiled slot.
    fn route_work_scope(&self, kind: &RouteDemandKind) -> Option<Scope> {
        let slot_scope = self
            .work_scope
            .filter(|scope| matches!(scope, Scope::Slot(_)))?;
        let scope = match kind {
            RouteDemandKind::Walk {
                authority: Some(authority),
            }
            | RouteDemandKind::Advisory { authority, .. } => Scope::Run(authority.run()),
            RouteDemandKind::Walk { authority: None } => slot_scope,
        };
        slot_scope.contains(&scope).then_some(scope)
    }
    pub(crate) fn route_destination(&self) -> Option<WorldTile> {
        self.compact_route
            .as_ref()
            .map(RoutePath::destination)
            .or_else(|| self.route.as_ref().map(|route| route.dest))
    }

    pub(crate) fn route_path_for_follow(&self) -> Option<RoutePath> {
        self.compact_route.clone().or_else(|| {
            self.route
                .as_ref()
                .map(|route| RoutePath::legacy(Arc::clone(route)))
        })
    }

    pub(crate) fn install_route_path(&mut self, route: RoutePath) {
        self.poison_hold = None;
        match route {
            RoutePath::Legacy(route) => {
                self.route = Some(route);
                self.compact_route = None;
            }
            compact @ RoutePath::Compact { .. } => {
                self.route = None;
                self.compact_route = Some(compact);
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Publication checks the frozen request identity and transfers the charged route with its custody."
    )]
    pub(crate) fn publish_compact_route(
        &mut self,
        generation: u64,
        request_id: u64,
        identity: WorkIdentity,
        revision: InputRevision,
        allow_teleports: bool,
        route: RoutePath,
        pending: Option<PendingBankFetch>,
        custody: Arc<dyn std::any::Any + Send + Sync>,
        quest_evidence: Option<nav_planner::quest_gates::QuestEvidence>,
    ) -> bool {
        let Some(plan) = route.plan() else {
            return false;
        };
        if self.route_generation != generation
            || self.walk_request_id != request_id
            || plan.identity() != identity
            || plan.revision() != revision
        {
            return false;
        }
        self.traveller.clear();
        self.install_route_path(route);
        self.route_result_custody = Some(custody);
        self.map_route_generation = crate::walk_map::next_map_route_generation();
        self.route_request_id = request_id;
        self.bank_fetch = pending;
        self.allow_teleports = allow_teleports;
        self.route_quest_evidence = quest_evidence;
        true
    }

    pub(crate) fn publish_compact_route_failure(
        &mut self,
        generation: u64,
        request_id: u64,
        detail: Option<Arc<str>>,
        missing: Vec<MissingCarry>,
        native_failure: Option<script::native::WalkEnd>,
        blocked: Box<[ZoneKey]>,
    ) {
        if self.route_generation != generation || self.walk_request_id != request_id {
            return;
        }
        if let Some((to, radius, allow_teleports, ..)) = self.requested_route {
            if self.armed_outcome_may_publish(request_id) {
                self.note_failure(generation, request_id, to, radius, allow_teleports);
            }
        }
        self.note_missing_carry(generation, request_id, missing);
        if self.walk_outcome_generation == generation && self.walk_outcome_request_id == request_id
        {
            let native_refused = matches!(
                native_failure.as_ref(),
                Some(script::native::WalkEnd::Refused)
            );
            self.native_walk_failure = native_failure.map(|end| (request_id, end));
            self.walk_outcome_detail = detail;
            self.native_walk_blocked = if native_refused {
                (!blocked.is_empty()).then(|| (request_id, Arc::<[ZoneKey]>::from(blocked)))
            } else {
                None
            };
        }
        self.requested_route = None;
    }
    pub(crate) fn route_projection(&self) -> Option<crate::walk_map::RouteProjectionPath<'_>> {
        self.compact_route
            .as_ref()
            .map(RoutePath::projection)
            .or_else(|| {
                self.route
                    .as_deref()
                    .map(crate::walk_map::RouteProjectionPath::Legacy)
            })
    }

    pub(crate) fn clear_route(&mut self) {
        self.route = None;
        self.compact_route = None;
        self.poison_hold = None;
    }

    /// A plain increment from 1. It never wraps in practice: see the 63-bit
    /// lifetime note at `script::native::ledger::ObservedWalkOutcome`.
    fn bump_walk_outcome_seq(&mut self) {
        self.manual_walk_cancelled_detail = false;
        self.walk_outcome_seq += 1;
    }

    pub(crate) fn ordinary_movement_owned(&self, tick: u16) -> bool {
        self.compat_walk_handed_over()
            || self.native_walk.as_ref().is_some_and(|owner| owner.live())
            || self.has_route()
            || self.bank_fetch.is_some()
            || self.slot_escape().is_some()
            || self
                .walk_guard_off
                .as_ref()
                .is_some_and(|off| off.blocks_nav(tick))
            || self.combat_prayer_off.is_some()
    }

    pub(crate) fn slot_escape(&self) -> Option<script::combat::guard::Escape> {
        self.admission
            .as_deref()
            .and_then(|admission| admission.escape)
            .filter(|escape| crate::admission::escape_live(*escape))
            .or_else(|| {
                self.manual_arm
                    .as_ref()?
                    .upgrade()?
                    .lock()
                    .ok()?
                    .admission
                    .as_deref()?
                    .escape
                    .filter(|escape| crate::admission::escape_live(*escape))
            })
    }

    /// End the native walk this bot follows: its still-live owner gets `end`
    /// on the next observation; a revoked owner is owed nothing.
    pub(crate) fn end_native_walk(&mut self, end: script::native::WalkEnd) {
        super::script_walk::owe_walk_guard_off(self);
        if let Some(owner) = self.native_walk.take() {
            self.native_walk_failure = None;
            self.walk_outcome_detail = None;
            if owner.live() {
                self.native_end_risk = Some(Box::new(NativeEndRisk {
                    assessment: self.assessment.clone(),
                    refusal: self
                        .risk_refusal
                        .as_deref()
                        .filter(|(request, _)| *request == owner.request_id().get())
                        .map(|(_, refusal)| refusal.clone()),
                    escape: self
                        .admission
                        .as_deref()
                        .and_then(|admission| admission.escape),
                    blocked: self
                        .native_walk_blocked
                        .as_ref()
                        .filter(|(request, _)| *request == owner.request_id().get())
                        .map(|(_, keys)| Arc::clone(keys)),
                }));
                self.native_end = Some((owner, end));
            }
            self.native_walk_blocked = None;
        }
    }

    /// A native walk the host refused to arm: nothing was routed for it.
    pub(crate) fn refuse_native_with_detail(
        &mut self,
        owner: script::native::HostAuthority,
        detail: Option<Arc<str>>,
    ) {
        self.native_walk_blocked = None;
        self.walk_outcome_detail = None;
        if owner.live() {
            let request_id = owner.request_id().get();
            self.native_end_risk = None;
            self.native_end = Some((owner, script::native::WalkEnd::Refused));
            if self
                .native_end
                .as_ref()
                .is_some_and(|(owner, _)| owner.request_id().get() == request_id)
            {
                self.walk_outcome_request_id = request_id;
                self.walk_outcome_detail = detail;
            }
        }
    }

    /// The terminal a follow failure owes a native walk beyond `Failed`: a
    /// host send refusal is `Refused`, an unworkable leg is `Blocked`, and a
    /// crossing whose quest evidence was unproven at send time names the
    /// gates to acquire, or blocks honestly when evidence disproves them.
    pub(crate) fn note_native_follow_failure(&mut self, outcome: &nav_executor::TravelOutcome) {
        if self.walk_request_id == 0
            || self.route_request_id != self.walk_request_id
            || self.walk_outcome_request_id != self.walk_request_id
            || self.walk_outcome_generation != self.route_generation
        {
            return;
        }
        use api::selected::Truth;
        let end = match outcome {
            nav_executor::TravelOutcome::Stalled {
                at,
                aiming,
                why,
                tries,
                ..
            } => Some(script::native::WalkEnd::Stalled(Arc::new(
                script::native::WalkStall {
                    at: *at,
                    aiming: *aiming,
                    why: match why {
                        nav_executor::HopFailure::Expired => {
                            script::native::WalkStallReason::Expired
                        }
                        nav_executor::HopFailure::Dropped => {
                            script::native::WalkStallReason::Dropped
                        }
                        nav_executor::HopFailure::EndBlocked => {
                            script::native::WalkStallReason::EndBlocked
                        }
                    },
                    tries: *tries,
                },
            ))),
            nav_executor::TravelOutcome::Refused { .. } => Some(script::native::WalkEnd::Refused),
            nav_executor::TravelOutcome::Blocked { .. }
            | nav_executor::TravelOutcome::EntranceRefused { .. } => {
                Some(script::native::WalkEnd::Blocked)
            }
            nav_executor::TravelOutcome::GateRefused {
                leg,
                crossing,
                gates,
                matched,
                ..
            } => {
                let observation = self.route_path_for_follow().and_then(|path| {
                    path.prereq_content().and_then(|content| {
                        content.refusal_observation(*leg, *crossing, gates, *matched)
                    })
                });
                Some(observation.map_or(
                    script::native::WalkEnd::Blocked,
                    script::native::WalkEnd::GateRefused,
                ))
            }
            nav_executor::TravelOutcome::EvidenceUnproven {
                verdict: Truth::Unknown,
                unresolved,
                ..
            } if !unresolved.is_empty() => Some(script::native::WalkEnd::NeedsEvidence(
                Arc::clone(unresolved),
            )),
            nav_executor::TravelOutcome::EvidenceUnproven { .. } => {
                Some(script::native::WalkEnd::Blocked)
            }
            _ => None,
        };
        self.native_walk_failure = end.map(|end| (self.walk_request_id, end));
        if self
            .native_walk
            .as_ref()
            .is_some_and(|owner| owner.live() && owner.request_id().get() == self.walk_request_id)
        {
            self.walk_outcome_detail = match outcome {
                // The typed stall keeps its fields in the receipt. An agility
                // crossing that was refused names that refusal in the detail,
                // which the receipt prefers to the stall's own text.
                nav_executor::TravelOutcome::Stalled {
                    at,
                    aiming,
                    why,
                    tries,
                    last_refusal: Some(refusal),
                } => Some(Arc::from(format!(
                    "walk stalled ({why:?}) at {at:?}, aiming at {aiming:?}, tries: {tries}, \
                     last refusal: {refusal}"
                ))),
                nav_executor::TravelOutcome::Stalled { .. } => None,
                _ => Some(Arc::from(format!("native follow failed: {outcome:?}"))),
            };
        }
    }

    /// Older armed-route results may publish only after the current wait
    /// refusal has been copied into a snapshot, or when they *are* that wait.
    pub(super) fn armed_outcome_may_publish(&self, request_id: u64) -> bool {
        self.walk_live_refusal_id == 0 || request_id == self.walk_live_refusal_id
    }

    #[cfg(any(test, feature = "js"))]
    /// A snapshot carrying outcome `posted_seq` reached the isolate. The
    /// live refusal guard is released only when that was the latest
    /// published outcome; a newer one stays guarded until it is posted.
    pub(crate) fn mark_walk_outcome_posted(&mut self, posted_seq: u64) {
        if self.walk_outcome_seq == posted_seq {
            self.walk_live_refusal_id = 0;
            if self.native_walk.is_none() {
                self.native_walk_blocked = None;
                self.walk_outcome_detail = None;
            }
        }
    }

    pub(crate) fn note_failure(
        &mut self,
        generation: u64,
        request_id: u64,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
    ) {
        // Legacy request id 0 never settles a wait. Do not replace a live
        // nonzero refusal with it before the isolate can observe the refusal.
        if request_id == 0 && self.walk_live_refusal_id != 0 {
            log_walk_arm_bot(|| {
                format!(
                    "note_failure skipped legacy-id dest={to:?} r={radius} request_id={request_id} \
                     live_refusal_id={}",
                    self.walk_live_refusal_id
                )
            });
            return;
        }
        log_walk_arm_bot(|| {
            format!(
                "note_failure generation={generation} walk_request_id={} request_id={request_id} \
                 dest={to:?} r={radius} allow_teleports={allow_teleports} seq={}",
                self.walk_request_id,
                self.walk_outcome_seq.wrapping_add(1)
            )
        });
        self.bump_walk_outcome_seq();
        self.walk_outcome_generation = generation;
        self.walk_outcome_request_id = request_id;
        self.walk_outcome_failed = true;
        self.walk_outcome_blocked = false;
        self.walk_outcome_cancel_reason = script::native::WalkCancelReason::None;
        self.walk_outcome_x = to.x;
        self.walk_outcome_z = to.z;
        self.walk_outcome_level = to.level;
        self.walk_outcome_radius = radius;
        self.walk_outcome_allow_teleports = allow_teleports;
        // A failure raised here names no short: only the strict find's own
        // diagnosis attaches a shopping list, and it lands after this.
        self.native_walk_blocked = None;
        self.walk_outcome_detail = None;
        self.walk_missing_carry.clear();
        if request_id != 0 && request_id != self.walk_request_id {
            self.walk_live_refusal_id = request_id;
        } else {
            self.walk_live_refusal_id = 0;
        }
    }

    /// The armed walk's route reached its end: publish a settled, not
    /// failed, outcome for `request_id` (frozen `'closest'`,
    /// `WalkExecutor.ts:316-325`, or `'blocked'` when the follow ended next
    /// to a refused last tile, `WalkExecutor.ts:1092-1094`). The matching
    /// isolate wait settles true.
    #[allow(clippy::too_many_arguments)] // the published outcome's fields
    pub(super) fn note_route_end(
        &mut self,
        generation: u64,
        request_id: u64,
        to: WorldTile,
        radius: i32,
        allow_teleports: bool,
        blocked: bool,
    ) {
        log_walk_arm_bot(|| {
            format!(
                "note_route_end generation={generation} request_id={request_id} dest={to:?} \
                 r={radius} seq={}",
                self.walk_outcome_seq.wrapping_add(1)
            )
        });
        self.bump_walk_outcome_seq();
        self.walk_outcome_generation = generation;
        self.walk_outcome_request_id = request_id;
        self.walk_outcome_failed = false;
        self.walk_outcome_blocked = blocked;
        self.walk_outcome_cancel_reason = script::native::WalkCancelReason::None;
        self.walk_outcome_x = to.x;
        self.walk_outcome_z = to.z;
        self.walk_outcome_level = to.level;
        self.walk_outcome_radius = radius;
        self.walk_outcome_allow_teleports = allow_teleports;
        self.native_walk_blocked = None;
        // The request arm clears stale detail. Keep this route's host detail
        // for its correlated terminal receipt.
        self.walk_missing_carry.clear();
        self.walk_live_refusal_id = 0;
    }

    pub(crate) fn clear_walk_outcome(&mut self) {
        // AbortWalk is a release of the current operation, not a script-run
        // boundary. Preserve its published UserInput terminal until an
        // isolate observes it.
        if self.walk_outcome_cancel_reason == script::native::WalkCancelReason::UserInput {
            return;
        }
        self.reset_walk_outcome();
    }

    fn reset_walk_outcome(&mut self) {
        self.bump_walk_outcome_seq();
        self.walk_outcome_failed = false;
        self.walk_outcome_blocked = false;
        self.walk_outcome_generation = self.route_generation;
        self.walk_outcome_request_id = 0;
        self.walk_outcome_x = 0;
        self.walk_outcome_z = 0;
        self.walk_outcome_level = 0;
        self.walk_outcome_radius = 0;
        self.route_compat_tile_intent = false;
        self.walk_outcome_allow_teleports = false;
        self.native_walk_blocked = None;
        self.walk_outcome_detail = None;
        self.walk_live_refusal_id = 0;
        self.walk_outcome_cancel_reason = script::native::WalkCancelReason::None;
        // The family posts a present empty vector: a routed outcome names no
        // short, and the clear is never omitted.
        self.walk_missing_carry.clear();
    }

    pub(crate) fn walking_decision_is_current(&self, observed_seq: u64) -> bool {
        observed_seq >= self.manual_takeover_watermark
    }

    /// The cancel reason a compiled card observes for the latest walk
    /// outcome. A manual takeover always moves the sequence, but it publishes
    /// `UserInput` only for a correlated request (`cancel_for_manual_input`);
    /// the takeover of a walk that already arrived leaves the posted reason
    /// `None`. Its own outcome is still the watermark, so the compiled side
    /// reads `UserInput` for it. The posted Load reason is unchanged.
    pub(crate) fn compiled_walk_cancel_reason(&self) -> script::native::WalkCancelReason {
        let takeover_outcome = self.manual_takeover_watermark != 0
            && self.walk_outcome_seq == self.manual_takeover_watermark;
        if takeover_outcome
            || self.walk_outcome_cancel_reason == script::native::WalkCancelReason::UserInput
        {
            script::native::WalkCancelReason::UserInput
        } else {
            script::native::WalkCancelReason::None
        }
    }

    /// Cancel active follow/work without sending anything over the human move.
    /// Route generation fences every late route and bank completion.
    pub(crate) fn cancel_for_manual_input(&mut self) {
        let current = self
            .requested_route
            // A live outer family may outlast an already-settled route leg.
            .filter(|_| {
                self.walk_request_id != 0 && self.walk_outcome_request_id != self.walk_request_id
            })
            .map(|(to, radius, teleports, _, _, _)| {
                (
                    self.walk_request_id,
                    to,
                    radius,
                    teleports,
                    self.route_generation,
                )
            });
        let carried = self.carried_walk.as_deref().and_then(|walk| {
            let (request_id, to, radius, allow_teleports) = match &walk.request {
                script::interact::InteractReq::Walk {
                    x,
                    z,
                    level,
                    allow_teleports,
                    request_id,
                    ..
                } => (
                    *request_id,
                    WorldTile {
                        x: *x,
                        z: *z,
                        level: *level,
                    },
                    0,
                    *allow_teleports,
                ),
                script::interact::InteractReq::WalkNear {
                    x,
                    z,
                    level,
                    radius,
                    allow_teleports,
                    request_id,
                    ..
                } => (
                    *request_id,
                    WorldTile {
                        x: *x,
                        z: *z,
                        level: *level,
                    },
                    *radius,
                    *allow_teleports,
                ),
                _ => return None,
            };
            Some((
                request_id,
                to,
                radius,
                allow_teleports,
                walk.route_generation,
            ))
        });
        let identity = current.or(carried);
        let native_owner = self.native_walk.is_some();
        super::script_walk::abort_walk_on_bot_with_end(self, script::native::WalkEnd::UserInput);
        // Route-less walking families still advance the dispatch fence, but
        // cannot manufacture a correlated receipt for legacy request id zero.
        self.bump_walk_outcome_seq();
        self.manual_takeover_watermark = self.walk_outcome_seq;
        if let Some((request_id, to, radius, teleports, generation)) =
            identity.filter(|(request_id, ..)| *request_id != 0)
        {
            self.walk_outcome_generation = generation;
            self.walk_outcome_request_id = request_id;
            self.walk_outcome_failed = true;
            self.walk_outcome_blocked = false;
            self.walk_outcome_cancel_reason = script::native::WalkCancelReason::UserInput;
            self.walk_outcome_x = to.x;
            self.walk_outcome_z = to.z;
            self.walk_outcome_level = to.level;
            self.walk_outcome_radius = radius;
            self.walk_outcome_allow_teleports = teleports;
            // Native terminals travel directly to their typed owner; this
            // isolate-only guard has no compiled-slot release path.
            self.walk_live_refusal_id = if native_owner { 0 } else { request_id };
        }
        self.manual_walk_cancelled_detail = true;
    }

    /// Whether a script walk is armed, in flight or following.
    pub(crate) fn script_walk_armed(&self) -> bool {
        self.compat_walk.is_some()
            || self.has_route()
            || self.bank_fetch.is_some()
            || self.route_work_pending()
            || self.requested_route.is_some()
            || self.walk_request_id != 0
    }

    /// Record the shopping list of the failure this attempt published. Only
    /// the outcome that is live when the diagnosis lands carries one: a
    /// superseded worker (stale generation) and a refusal `note_failure`
    /// declined to publish (legacy request id) both leave the list they never
    /// named cleared rather than attaching it to another failure's page.
    pub(crate) fn note_missing_carry(
        &mut self,
        generation: u64,
        request_id: u64,
        missing: Vec<MissingCarry>,
    ) {
        if self.route_generation != generation
            || !self.walk_outcome_failed
            || self.walk_outcome_generation != generation
            || self.walk_outcome_request_id != request_id
        {
            return;
        }
        self.walk_missing_carry = missing;
    }

    pub(crate) fn publish_route(
        &mut self,
        generation: u64,
        request_id: u64,
        allow_teleports: bool,
        outcome: RouteOutcome,
    ) {
        // Superseded workers and aborted/restarted runs keep a newer
        // route_generation. A late NoPath for the same dest must not publish.
        if self.route_generation != generation {
            log_walk_arm_bot(|| {
                format!(
                    "publish_route discard-stale-generation published={generation} \
                     live={} request_id={request_id} outcome={}",
                    self.route_generation,
                    walk_arm_outcome_tag(&outcome)
                )
            });
            return;
        }
        let (route, pending) = match outcome {
            RouteOutcome::Routed(route) => {
                log_walk_arm_bot(|| {
                    format!(
                        "publish_route Routed request_id={request_id} walk_request_id={} \
                         route_dest={:?}",
                        self.walk_request_id, route.dest
                    )
                });
                (Arc::new(route), None)
            }
            RouteOutcome::BankSession { pending, route } => {
                log_walk_arm_bot(|| {
                    format!(
                        "publish_route BankSession request_id={request_id} walk_request_id={} \
                         route_dest={:?} session_steps={} session_dest={:?}",
                        self.walk_request_id,
                        route.dest,
                        pending.steps.len(),
                        pending.dest
                    )
                });
                (Arc::new(route), Some(pending))
            }
            outcome @ (RouteOutcome::NoPath | RouteOutcome::IncompleteSearch { .. }) => {
                log_walk_arm_bot(|| {
                    let search_result = match &outcome {
                        RouteOutcome::NoPath => RouteSearchResultKind::NoPath,
                        RouteOutcome::IncompleteSearch { search_result } => *search_result,
                        RouteOutcome::Routed(_) | RouteOutcome::BankSession { .. } => {
                            unreachable!()
                        }
                    };
                    format!(
                        "publish_route search_result={search_result} request_id={request_id} \
                         walk_request_id={} may_publish={}",
                        self.walk_request_id,
                        self.armed_outcome_may_publish(request_id)
                    )
                });
                if let Some((to, radius, allow_teleports, ..)) = self.requested_route {
                    if self.armed_outcome_may_publish(request_id) {
                        self.note_failure(generation, request_id, to, radius, allow_teleports);
                    }
                }
                // The retained route belongs to the previous request. A later
                // request for this failed destination must be allowed to retry.
                self.requested_route = None;
                return;
            }
        };
        self.traveller.clear();
        self.poison_hold = None;
        self.route = Some(route);
        self.compact_route = None;
        self.map_route_generation = crate::walk_map::next_map_route_generation();
        self.route_request_id = request_id;
        self.bank_fetch = pending;
        self.allow_teleports = allow_teleports;
    }
}

/// End the session's navigation: every route, find, bank-fetch session,
/// inspect and bank pick of the slot, a walk a reconnect was carrying, and
/// the duel offer its accept gate validated. When `terminal_generation` is
/// present, the same lifecycle receipt resets this state only once. Returns
/// whether it reset the navigation, so the caller that owns the status rows
/// can retire the published script walk of the same run.
pub(crate) fn reset_script_nav(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    terminal_generation: Option<u64>,
) -> bool {
    let mut navs = navs.lock().unwrap();
    let Some(nav) = navs.get_mut(name) else {
        return false;
    };
    if terminal_generation.is_some_and(|generation| {
        nav.terminal_nav_reset_generation
            .is_some_and(|last| last >= generation)
    }) {
        return false;
    }
    end_route_follow(nav);
    nav.runtime_gate_logged = false;
    nav.reset_walk_outcome();
    route_inspect::reset_inspect(nav);
    nav.bank_pick.reset();
    nav.carried_walk = None;
    nav.duel_offer_partner = None;
    if let Some(generation) = terminal_generation {
        nav.terminal_nav_reset_generation = Some(generation);
    }
    true
}

/// Keep the pending script receipt identity when watchdog nav replaces its route.
/// This reuses the rare carry allocation but never grants recovery a resend.
///
/// The reconstructed request payload is only a receipt placeholder: it does
/// not retain the original walk mode or exclusions and must never be replayed.
pub(super) fn preserve_recovery_walk_identity(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
) {
    let mut bots = navs.lock().unwrap();
    let Some(bot) = bots.get_mut(name) else {
        return;
    };
    if bot.native_walk.is_some() {
        return;
    }
    if bot.carried_walk.is_some()
        || bot.walk_request_id == 0
        || bot.walk_outcome_request_id == bot.walk_request_id
    {
        return;
    }
    let Some((to, radius, allow_teleports, allow_wilderness, allow_bank_fetch, _)) =
        bot.requested_route
    else {
        return;
    };
    bot.carried_walk = Some(Box::new(CarriedWalk {
        runtime_generation: None,
        route_generation: bot.route_generation,
        request: script::interact::InteractReq::WalkNear {
            x: to.x,
            z: to.z,
            level: to.level,
            radius,
            allow_teleports,
            allow_wilderness,
            compat_tile_intent: bot.route_compat_tile_intent,
            allow_bank_fetch,
            allow_prereq_dialogue: None,
            allow_quest_start: None,
            request_id: bot.walk_request_id,
            avoid: Vec::new(),
            cross: Vec::new(),
        },
    }));
}

/// A reconnect the slot relogs through with its Load script's work held
/// (`SlotScript::reconnect_session_work`). The connection's route follow
/// ends as in [`reset_script_nav`]; what the held script still waits on
/// stays: the published walk outcome, and the route-inspect and bank-pick
/// workers with their results (pure computations, valid on any session).
///
/// With `carry` (the script run's `runtime_generation`), the walk the
/// script had armed is kept and re-armed on the relogged session under its
/// own request id ([`take_carried_walk`]), so the held walk wait settles on
/// it. Frozen resumes the paused `WalkExecutor.walkTo` itself
/// (`AutoRelogin.ts:159-163`), which repaths from wherever the player
/// stands when its follow sees a deviation or a stall
/// (`WalkExecutor.ts:916-918`, `1097-1099`). A typed native walk is never
/// carried as a legacy request: its follow ends `Cancelled` to its owner,
/// which revalidates and re-walks after Resume or the relog.
pub(crate) fn hold_script_nav(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    carry: Option<u64>,
) {
    if let Some(nav) = navs.lock().unwrap().get_mut(name) {
        let route_generation = nav.route_generation;
        // A duel's screens belong to the dropped connection.
        nav.duel_offer_partner = None;
        let armed = nav.has_route() || nav.route_work_pending() || nav.bank_fetch.is_some();
        let picking = nav.bank_pick.walking(nav.route_generation);
        if let (Some(runtime_generation), true, true) = (carry, picking, !armed) {
            // A nearest-bank walk still choosing its bank: re-ask for it.
            nav.carried_walk = Some(Box::new(CarriedWalk {
                runtime_generation: Some(runtime_generation),
                route_generation,
                request: script::interact::InteractReq::WalkNearestBank,
            }));
        } else if let (Some(runtime_generation), true, Some(requested), None) =
            (carry, armed, nav.requested_route, &nav.native_walk)
        {
            let (to, radius, allow_teleports, allow_wilderness, allow_bank_fetch, _) = requested;
            let request_id = nav.walk_request_id;
            let (allow_prereq_dialogue, allow_quest_start) = nav
                .compat_walk
                .as_ref()
                .and_then(|walk| walk.outer.as_ref())
                .map_or((None, None), |outer| {
                    (
                        Some(outer.prereq_allow.dialogue),
                        Some(outer.prereq_allow.quest_start),
                    )
                });
            let (avoid, cross) = nav.requested_exclusions.as_deref().map_or_else(
                || (Vec::new(), Vec::new()),
                |exclusions| {
                    (
                        exclusions.avoid_wire.clone(),
                        exclusions
                            .cross
                            .iter()
                            .map(|name| name.to_string())
                            .collect(),
                    )
                },
            );
            let request = if radius > 0 {
                script::interact::InteractReq::WalkNear {
                    x: to.x,
                    z: to.z,
                    level: to.level,
                    radius,
                    compat_tile_intent: nav.route_compat_tile_intent,
                    allow_teleports,
                    allow_wilderness,
                    allow_bank_fetch,
                    allow_prereq_dialogue,
                    allow_quest_start,
                    request_id,
                    avoid,
                    cross,
                }
            } else {
                script::interact::InteractReq::Walk {
                    x: to.x,
                    z: to.z,
                    level: to.level,
                    allow_teleports,
                    allow_wilderness,
                    allow_bank_fetch,
                    allow_prereq_dialogue,
                    allow_quest_start,
                    request_id,
                    avoid,
                    cross,
                }
            };
            nav.carried_walk = Some(Box::new(CarriedWalk {
                runtime_generation: Some(runtime_generation),
                route_generation,
                request,
            }));
        }
        end_route_follow(nav);
    }
}

/// The walk a reconnect carried for the script run `runtime_generation`,
/// once: the caller dispatches it ahead of the run's own requests.
pub(crate) fn take_carried_walk(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    runtime_generation: u64,
) -> Option<script::interact::InteractReq> {
    let carried = navs.lock().unwrap().get_mut(name)?.carried_walk.take()?;
    (carried.runtime_generation == Some(runtime_generation)).then_some(carried.request)
}

fn end_route_follow(nav: &mut NavBot) {
    if nav.bank_pick.walking(nav.route_generation) {
        nav.bank_pick.reset();
    }
    nav.supersede_walk_route_work();
    nav.route_generation = nav.route_generation.wrapping_add(1);
    // The host ends the slot's follow and its advisory work together. A live
    // advisory is answered with one refusal; a revoked one is dropped.
    nav.cancel_advisory_route_work(
        "Advisory route was cancelled when the host ended route work; the script may ask again",
    );
    nav.requested_route = None;
    nav.traveller.clear();
    nav.clear_route();
    nav.bank_fetch = None;
    nav.walk_request_id = 0;
    nav.route_quest_evidence = None;
    super::script_walk::owe_walk_guard_off(nav);
    nav.end_native_walk(script::native::WalkEnd::Cancelled);
    nav.admission_pending = false;
    // A finished advisory answer of a live owner still awaits delivery.
    if nav
        .assess_result
        .as_ref()
        .is_some_and(|result| !result.0.live())
    {
        nav.assess_result = None;
    }
    nav.risk_refusal = None;
    if nav
        .admission
        .as_deref()
        .and_then(|admission| admission.escape)
        .is_none_or(|escape| !crate::admission::escape_live(escape))
    {
        nav.admission = None;
        nav.assessment = None;
        nav.route_basis = None;
    }
}

#[cfg(test)]
#[path = "script_nav_facade_tests.rs"]
mod facade_tests;

#[cfg(test)]
#[path = "route_work_tests.rs"]
mod route_work_tests;

#[cfg(test)]
#[path = "route_diagnostic_corpus_tests.rs"]
mod route_diagnostic_corpus_tests;

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    thread_local! {
        pub(super) static BANK_TARGET_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn large_area_bank_zone_diagnosis_has_a_fixed_probe_bound() {
        let origin = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        let world = NavWorld::from_parts(
            nav_planner::collision::WorldCollision {
                origin,
                width: 209,
                height: 209,
                walk: vec![0; 209 * 209],
                blocked: vec![0; (209usize * 209).div_ceil(64)],
                flags: None,
            },
            nav_planner::transport::TransportGraph::default(),
            Vec::new(),
        );
        let mut request = ScriptRouteRequest {
            candidate_goals: None,
            admission: crate::admission::Admission::manual(
                FindOptions::default(),
                Default::default(),
                1,
                crate::WalkGlobals::default(),
            ),
            generation: 1,
            request_id: 1,
            world: Arc::new(world),
            from: origin,
            to: WorldTile {
                x: 104,
                z: 104,
                ..origin
            },
            radius: 104,
            loc_id: None,
            arrival: ArrivalKind::Area,
            opts: FindOptions {
                allow_bank_fetch: true,
                ..FindOptions::default()
            },
            state: None,
            prereq_catalog: None,
            bank_origin: Origin::Unknown,
            bank: Vec::new(),
            live_geometry: None,
            exclusions: None,
            compat_tile_intent: false,
            completion: Default::default(),
        };
        let targets = request.targets();
        assert_eq!(targets.len(), 209 * 209, "routing keeps every Area goal");
        // Sixteen actual near goals expose the old per-goal fanout without
        // exhausting tens of thousands of searches on the pre-fix code.
        let targets = &targets[..16];
        BANK_TARGET_PROBES.with(|count| count.set(0));
        assert!(request
            .blocking_zones(&RouteOutcome::NoPath, targets)
            .is_none());
        BANK_TARGET_PROBES.with(|count| {
            assert!(
                count.get() > 0 && count.get() <= 8,
                "optional diagnosis must probe at most eight goals, not {}",
                count.get()
            );
        });
        request.opts.allow_bank_fetch = false;
        BANK_TARGET_PROBES.with(|count| count.set(0));
        assert!(request
            .blocking_zones(&RouteOutcome::NoPath, targets)
            .is_none());
        BANK_TARGET_PROBES.with(|count| assert_eq!(count.get(), 0));
    }

    fn stage_identity(index: u64) -> WorkIdentity {
        WorkIdentity::new(
            RequestId::from_raw(RequestId::HOST_FIRST + index).unwrap(),
            api::identity::ProcessIncarnation::from_start_nonce(
                std::num::NonZeroU64::new(1).unwrap(),
            ),
            Scope::Slot(api::identity::SlotKey::from_incarnation(1).unwrap()),
            None,
        )
        .unwrap()
    }

    fn stage_request(world: Arc<NavWorld>) -> Box<ScriptRouteRequest> {
        let from = world.collision.origin;
        Box::new(ScriptRouteRequest {
            candidate_goals: None,
            admission: crate::admission::Admission::manual(
                FindOptions::default(),
                Default::default(),
                1,
                crate::WalkGlobals::default(),
            ),
            generation: 1,
            request_id: 1,
            world,
            from,
            to: WorldTile {
                x: from.x + 1,
                z: from.z + 1,
                ..from
            },
            radius: 0,
            loc_id: None,
            arrival: ArrivalKind::Reach,
            opts: FindOptions::default(),
            state: None,
            prereq_catalog: None,
            bank_origin: Origin::Unknown,
            bank: Vec::new(),
            live_geometry: None,
            exclusions: None,
            compat_tile_intent: false,
            completion: Default::default(),
        })
    }
    fn first_goal_probe_world(
        width: usize,
        height: usize,
        split_x: usize,
        target_x: i32,
        with_item_gate: bool,
    ) -> (Arc<NavWorld>, WorldTile, WorldTile) {
        let cells = width * height * 4;
        let mut blocked = vec![0u64; cells.div_ceil(64)];
        for level in 0..4 {
            for z in 0..height {
                let index = level * width * height + z * width + split_x;
                blocked[index >> 6] |= 1 << (index & 63);
            }
        }
        let from = WorldTile {
            x: (split_x / 2) as i32,
            z: (height / 2) as i32,
            level: 0,
        };
        let to = WorldTile {
            x: target_x,
            z: from.z,
            level: 0,
        };
        let mut graph = nav_planner::transport::TransportGraph::default();
        if with_item_gate {
            let edge = nav_planner::transport::TransportEdge {
                takeoff: None,
                worn_all_req: Vec::new(),
                forbidden_held: Vec::new(),
                forbidden_worn: Vec::new(),
                kind: nav_planner::transport::TransportKind::Ladder,
                player_delta: None,
                at: from,
                to,
                loc_id: 1,
                option: 1,
                ticks: 1,
                dir: None,
                open_loc_id: None,
                skill_req: Vec::new(),
                item_req: vec![(995, 1)],
                consumed_req: Vec::new(),
                item_returns: Vec::new(),
                quest_req: Vec::new(),
                varp_req: Vec::new(),
                worn_req: Vec::new(),
                members_req: false,
                wildy_cap: None,
                quest_gates: None,
            };
            let index = graph.edges.len();
            graph.at.entry(from).or_default().push(index);
            graph.edges.push(edge);
        }
        let world = Arc::new(NavWorld::from_parts(
            nav_planner::collision::WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width,
                height,
                walk: vec![0; cells],
                blocked,
                flags: None,
            },
            graph,
            Vec::new(),
        ));
        (world, from, to)
    }

    fn first_goal_area_request(
        world: Arc<NavWorld>,
        from: WorldTile,
        to: WorldTile,
    ) -> Box<ScriptRouteRequest> {
        let mut request = stage_request(world);
        request.from = from;
        request.to = to;
        request.arrival = ArrivalKind::Area;
        request.candidate_goals = Some(script::native::WalkGoals::new(vec![to]).unwrap());
        request.opts = FindOptions::default();
        request.admission = crate::admission::Admission::manual(
            request.opts,
            Default::default(),
            1,
            crate::WalkGlobals::default(),
        );
        request.state = Some(Arc::new(WorldState::empty()));
        request
    }

    #[test]
    fn budget_exhausted_strict_search_keeps_non_supply_diagnostics() {
        let mut job = RouteJob::prepare(
            "test",
            stage_request(Arc::new(NavWorld::from_parts(
                nav_planner::collision::WorldCollision {
                    origin: WorldTile {
                        x: 0,
                        z: 0,
                        level: 0,
                    },
                    width: 4,
                    height: 4,
                    walk: vec![0; 4 * 4 * 4],
                    blocked: vec![0; (4usize * 4 * 4).div_ceil(64)],
                    flags: None,
                },
                nav_planner::transport::TransportGraph::default(),
                Vec::new(),
            ))),
            RouteDemandKind::Walk { authority: None },
        )
        .unwrap();
        job.targets.push(job.request.to);

        job.handle_search(
            SearchOutcome::Single {
                route: Err(nav_planner::router::RouteError::BudgetExhausted),
            },
            524_288,
        )
        .unwrap();

        assert!(job.diagnostics_started);
        assert!(matches!(job.stage, RouteJobStage::BeginDiagnostics));
        assert_eq!(job.budget_settled, Some(524_288));
        assert!(job.refusal.is_none());
        job.missing.push(MissingCarry { id: 954, count: 1 });
        job.identity = Some(stage_identity(1));
        let output = job.finish();
        assert_eq!(
            output.search_result,
            Some(RouteSearchResultKind::BudgetExhausted {
                settled: Some(524_288)
            })
        );
        let detail = output.detail.expect("incomplete search has a detail");
        assert!(detail.contains("search_result=BudgetExhausted nodes_settled=524288"));
        assert!(!detail.contains("Insufficient route supplies"));
        assert!(
            output.missing.is_empty(),
            "an incomplete search publishes no missing-supply list"
        );
    }

    #[test]
    fn strict_budget_stop_stays_incomplete_when_diagnostics_reach_their_shared_limit() {
        let request = tiny_stage_request();
        let world = Arc::clone(&request.world);
        let mut job =
            RouteJob::prepare("test", request, RouteDemandKind::Walk { authority: None }).unwrap();
        job.targets.push(job.request.to);
        job.diagnostic_budget = 1;
        job.handle_search(
            SearchOutcome::Single {
                route: Err(RouteError::BudgetExhausted),
            },
            524_288,
        )
        .unwrap();
        job.identity = Some(stage_identity(0));

        let (output, _, _) = ScriptRouteRequest::drive_prepared_route_job_for_test(job, world);
        assert!(matches!(output.outcome, RouteJobOutcome::NoPath));
        assert_eq!(
            output.native_failure,
            Some(script::native::WalkEnd::Refused)
        );
        assert!(matches!(
            output.refusal,
            Some(script::native::WalkRefusal::SearchIncomplete {
                nodes_settled: Some(524_288)
            })
        ));
        assert!(output.missing.is_empty());
        assert!(output
            .detail
            .as_deref()
            .unwrap()
            .contains("route check stopped after"));
    }
    #[test]
    fn first_goal_area_complete_abandoned_flood_keeps_supply_diagnosis() {
        let (world, from, to) = first_goal_probe_world(1537, 769, 400, 1300, true);
        let search = nav_planner::router::find_first_with(
            &world.collision,
            &world.graph,
            from,
            &[to],
            FindOptions::default(),
            &WorldState::empty(),
        );
        assert!(matches!(search.route(), Err(RouteError::NoPath)));
        assert_eq!(search.proof(), nav_planner::router::ReverseProof::Abandoned);
        assert_eq!(search.settled(), 400 * 769);

        let request = first_goal_area_request(Arc::clone(&world), from, to);
        let (output, _, _) = (*request).drive_route_job_output_for_test();
        assert_eq!(output.search_result, Some(RouteSearchResultKind::Supply));
        assert_eq!(output.missing.len(), 1);
        assert!(
            output
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("Insufficient route supplies")),
            "a complete first-goal NoPath must keep its relaxed supply diagnosis",
        );
    }

    #[test]
    fn first_goal_area_budget_stop_does_not_publish_supply_diagnosis() {
        let (world, from, to) = first_goal_probe_world(2200, 2000, 2050, 2199, true);
        let search = nav_planner::router::find_first_with(
            &world.collision,
            &world.graph,
            from,
            &[to],
            FindOptions::default(),
            &WorldState::empty(),
        );
        assert!(matches!(search.route(), Err(RouteError::BudgetExhausted)));
        assert_eq!(search.proof(), nav_planner::router::ReverseProof::Abandoned);
        let settled = search.settled();

        let mut request = first_goal_area_request(Arc::clone(&world), from, to);
        request.opts.allow_bank_fetch = true;
        request.bank_origin = Origin::Hint;
        request.admission = crate::admission::Admission::manual(
            request.opts,
            Default::default(),
            1,
            crate::WalkGlobals::default(),
        );
        let mut job = RouteJob::prepare(
            "first-goal-budget-test",
            request,
            RouteDemandKind::Walk { authority: None },
        )
        .unwrap();
        job.identity = Some(stage_identity(0));
        assert!(matches!(job.search_mode, RouteSearchMode::First));
        job.handle_strict_first_search(Err(RouteError::BudgetExhausted), None, settled)
            .unwrap();
        assert_eq!(job.budget_settled, Some(settled));
        let (output, _, _) =
            ScriptRouteRequest::drive_prepared_route_job_for_test(job, Arc::clone(&world));

        assert_eq!(
            output.search_result,
            Some(RouteSearchResultKind::BudgetExhausted {
                settled: Some(settled),
            }),
        );
        assert_eq!(
            output.refusal,
            Some(script::native::WalkRefusal::SearchIncomplete {
                nodes_settled: Some(settled),
            }),
        );
        assert!(
            output.missing.is_empty(),
            "incomplete bank planning must not publish a relaxed supply list",
        );
        assert!(!output
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("Insufficient route supplies"),);

        let bot = NavBot {
            route_generation: output.generation,
            walk_request_id: output.request_id,
            walk_outcome_generation: output.generation,
            walk_outcome_request_id: output.request_id,
            requested_route: Some((to, 0, false, false, true, ZoneExempt::NONE)),
            ..NavBot::default()
        };
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "first-goal-budget-test".to_owned(),
            bot,
        )])));
        apply_route_output(&navs, "first-goal-budget-test", output, None);
        let navs = navs.lock().unwrap();
        assert!(
            navs["first-goal-budget-test"].walk_missing_carry.is_empty(),
            "budget-stopped route must not post walk_missing_carry",
        );
    }

    pub(super) fn tiny_stage_request() -> Box<ScriptRouteRequest> {
        let origin = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        stage_request(Arc::new(NavWorld::from_parts(
            nav_planner::collision::WorldCollision {
                origin,
                width: 4,
                height: 4,
                walk: vec![0; 4 * 4 * 4],
                blocked: vec![0; (4usize * 4 * 4).div_ceil(64)],
                flags: None,
            },
            nav_planner::transport::TransportGraph::default(),
            Vec::new(),
        )))
    }

    #[test]
    fn real_pack_radius_104_admission_keeps_full_goals_and_refuses_excess_input() {
        let Some(path) = api::test_support::local_asset_path(
            "NAV_PACK",
            "",
            "real 289 nav pack for full radius-104 host admission",
        ) else {
            return;
        };
        let world = Arc::new(NavWorld::load_pack(&path).expect("real nav pack"));
        const GOALS: usize = 209 * 209;
        assert_eq!(GOALS, 43_681);
        assert_eq!(NODE_BUDGET, 4_000_000);
        assert_eq!(ROUTE_WORK_LIMIT_BYTES, 192 * 1024 * 1024);
        for allow_teleports in [false, true] {
            let request = || {
                let mut request = stage_request(Arc::clone(&world));
                request.from = WorldTile {
                    x: 3222,
                    z: 3218,
                    level: 0,
                };
                request.to = request.from;
                request.radius = 104;
                request.arrival = ArrivalKind::Area;
                request.opts = FindOptions {
                    allow_teleports,
                    allow_bank_fetch: false,
                    ..FindOptions::default()
                };
                request.admission = crate::admission::Admission::manual(
                    request.opts,
                    Default::default(),
                    1,
                    crate::WalkGlobals::default(),
                );
                request
            };
            let job = RouteJob::prepare(
                "radius-104-admission",
                request(),
                RouteDemandKind::Walk { authority: None },
            )
            .expect("the full radius-104 host request must fit its unchanged limit");
            assert_eq!(RouteTargetCursor::capacity(&job.request).unwrap(), GOALS);
            assert_eq!(job.request.opts.allow_teleports, allow_teleports);
            let expected = SearchCursor::preflight(
                SearchRequestShape::First {
                    target_capacity: GOALS,
                    fallback_capacity: GOALS,
                    options: route_search_options(&job.request),
                    avoid_capacity: 0,
                    max_settles: NODE_BUDGET,
                },
                &job.lease,
                &job.state,
            )
            .expect("the full preferred and fallback search must fit");
            assert_eq!(job.preflights.first, expected);
            assert_eq!(
                expected.input_bytes(),
                2 * GOALS * std::mem::size_of::<WorldTile>(),
            );
            let charge = <RouteWorkSpec as api::work::WorkFamilySpec>::charge(job.submission())
                .total_bytes()
                .unwrap();
            assert!(charge <= ROUTE_WORK_LIMIT_BYTES);
            assert!(
                charge
                    >= expected.input_bytes()
                        + expected.reservation().peak_bytes
                        + expected.reservation().future_layout_bytes
                        + GOALS * std::mem::size_of::<WorldTile>(),
                "host admission must retain its candidate buffer and the complete search charge",
            );

            // Bank fetching stays disabled. This buffer adds only captured
            // input and exceeds the remaining whole-host allowance.
            let excess_rows =
                (ROUTE_WORK_LIMIT_BYTES - charge) / std::mem::size_of::<(i32, i32)>() + 1;
            let mut excessive = request();
            excessive.bank = Vec::with_capacity(excess_rows);
            assert!(matches!(
                RouteJob::prepare(
                    "radius-104-admission",
                    excessive,
                    RouteDemandKind::Walk { authority: None },
                ),
                Err(RouteJobError::Budget),
            ));
        }
    }

    #[test]
    fn route_admission_charges_bank_custody_only_when_bank_fetch_is_reachable() {
        let mut request = tiny_stage_request();
        let state = Arc::new(WorldState::empty());
        let lease = nav_lease(&request.world).unwrap();
        request.opts.allow_bank_fetch = false;
        let (without_bank, diagnostics) =
            RouteSubmission::prepare(&request, &state, &lease, 0, 0, 32).unwrap();
        // A captured buffer stays input in either mode. Its unreachable
        // planning/fetch/post-bank owners must not become scratch or output.
        // Leave room for the route and diagnostics already reserved above.
        let baseline_charge =
            without_bank.input_bytes + without_bank.scratch_bytes + without_bank.result_bytes;
        let bank_bytes = (ROUTE_WORK_LIMIT_BYTES - baseline_charge) / 2;
        assert!(bank_bytes > 0);
        request.bank = Vec::with_capacity(bank_bytes / std::mem::size_of::<(i32, i32)>());
        let (captured_bank, _) =
            RouteSubmission::prepare(&request, &state, &lease, request.bank.capacity(), 0, 32)
                .unwrap();
        assert!(captured_bank.input_bytes > without_bank.input_bytes);
        assert_eq!(captured_bank.scratch_bytes, without_bank.scratch_bytes);
        assert_eq!(captured_bank.result_bytes, without_bank.result_bytes);
        for diagnostic in [
            &diagnostics.missing,
            &diagnostics.missing_route,
            &diagnostics.entrance,
            &diagnostics.blocking,
            &diagnostics.unresolved,
        ] {
            assert!(
                captured_bank.scratch_bytes + captured_bank.result_bytes
                    >= diagnostic.input_bytes()
                        + diagnostic.reservation().peak_bytes
                        + diagnostic.reservation().future_layout_bytes,
                "bank-disabled requests retain each reachable diagnostic",
            );
        }
        request.opts.allow_bank_fetch = true;
        assert!(matches!(
            RouteSubmission::prepare(&request, &state, &lease, request.bank.capacity(), 0, 32),
            Err(RouteJobError::Budget),
        ));
        assert_eq!(ROUTE_WORK_LIMIT_BYTES, 192 * 1024 * 1024);
    }

    #[test]
    fn bank_enabled_route_admission_keeps_bank_plan_output_custody() {
        let (world, _, _) = first_goal_probe_world(8, 4, 4, 6, true);
        let mut request = stage_request(world);
        let state = Arc::new(WorldState::empty());
        let lease = nav_lease(&request.world).unwrap();
        request.opts.allow_bank_fetch = false;
        let (without_bank, _) =
            RouteSubmission::prepare(&request, &state, &lease, 0, 1, 32).unwrap();
        request.opts.allow_bank_fetch = true;
        let (with_bank, _) = RouteSubmission::prepare(&request, &state, &lease, 0, 1, 32).unwrap();
        assert!(with_bank.result_bytes > without_bank.result_bytes);
        assert!(
            with_bank.scratch_bytes + with_bank.result_bytes
                > without_bank.scratch_bytes + without_bank.result_bytes,
        );
    }

    #[test]
    fn compat_no_path_diagnoses_actionable_gates_before_reporting_failure() {
        let mut request = tiny_stage_request();
        request.state = Some(Arc::new(WorldState::empty().with_establishable(Arc::from(
            [api::selected::StageWindow {
                quest: api::selected::FactKey::new("quest"),
                signal: api::selected::FactKey::new("stage"),
                values: api::selected::InclusiveRange {
                    min: Some(1),
                    max: Some(1),
                },
            }],
        ))));
        let mut job =
            RouteJob::prepare("test", request, RouteDemandKind::Walk { authority: None }).unwrap();
        job.search_purpose = RouteSearchPurpose::Blocking;
        job.handle_search(SearchOutcome::BlockingZones(None), 0)
            .unwrap();
        assert_eq!(job.search_purpose, RouteSearchPurpose::Actionable);
        let crossing = api::prereq_steps::CrossingId {
            loc_id: 1,
            option: 0,
            at: WorldTile {
                x: 1,
                z: 1,
                level: 0,
            },
            takeoff: WorldTile {
                x: 1,
                z: 1,
                level: 0,
            },
            to: WorldTile {
                x: 2,
                z: 1,
                level: 0,
            },
            kind: api::prereq_steps::CrossingKind::Door,
        };
        let gate = api::prereq_steps::ActionableGate {
            gate: api::selected::QuestGate::Complete(api::selected::FactKey(Arc::from("quest"))),
            truth: api::selected::Truth::False,
            leg: 0,
            crossing,
        };
        let expected = gate.clone();

        job.handle_search(
            SearchOutcome::ActionableQuestGates(Some(Box::from([gate]))),
            0,
        )
        .unwrap();

        assert!(matches!(
            job.native_failure.as_ref(),
            Some(script::native::WalkEnd::NeedsStep(gates)) if gates.as_ref() == [expected]
        ));
        assert_eq!(job.search_purpose, RouteSearchPurpose::MissingCarry);
    }

    #[test]
    fn no_establishable_windows_skip_actionable_preflight_and_search() {
        // Revision 274 has no prerequisite catalog. An empty catalog also
        // cannot establish a gate, so neither case reserves or runs this pass.
        for establishable in [None, Some(Arc::from([]))] {
            let mut request = tiny_stage_request();
            request.state = Some(Arc::new(WorldState {
                establishable,
                ..WorldState::empty()
            }));
            let mut job =
                RouteJob::prepare("test", request, RouteDemandKind::Walk { authority: None })
                    .unwrap();
            assert!(job.preflights.actionable.is_none());
            job.search_purpose = RouteSearchPurpose::Blocking;
            job.handle_search(SearchOutcome::BlockingZones(None), 0)
                .unwrap();
            assert_eq!(job.search_purpose, RouteSearchPurpose::Unresolved);
            assert!(job.native_failure.is_none());
            job.handle_search(SearchOutcome::UnresolvedQuestGates(None), 0)
                .unwrap();
            assert_eq!(job.search_purpose, RouteSearchPurpose::Unresolved);
            assert!(matches!(job.stage, RouteJobStage::Diagnostics));
            assert_eq!(job.native_failure, Some(script::native::WalkEnd::NoPath));
        }
    }

    #[test]
    fn legacy_safety_walk_does_not_start_prerequisite_diagnosis() {
        let mut request = tiny_stage_request();
        request.request_id = 0;
        let mut job =
            RouteJob::prepare("test", request, RouteDemandKind::Walk { authority: None }).unwrap();
        job.search_purpose = RouteSearchPurpose::Blocking;
        job.handle_search(SearchOutcome::BlockingZones(None), 0)
            .unwrap();
        assert_eq!(job.search_purpose, RouteSearchPurpose::Blocking);
        assert!(matches!(job.stage, RouteJobStage::Diagnostics));
        assert!(job.native_failure.is_none());
    }

    #[test]
    fn candidate_search_keeps_reachable_components_without_per_goal_searches() {
        let tile = |x| WorldTile { x, z: 0, level: 0 };
        let world = Arc::new(NavWorld::from_parts(
            nav_planner::collision::WorldCollision {
                origin: tile(0),
                width: 3,
                height: 1,
                walk: vec![0; 3 * 4],
                blocked: vec![1 << 1],
                flags: None,
            },
            nav_planner::transport::TransportGraph::default(),
            Vec::new(),
        ));
        let mut old = stage_request(Arc::clone(&world));
        old.to = tile(2);
        old.arrival = ArrivalKind::Area;
        let (outcome, _) = old.drive_route_job_for_test();
        assert!(matches!(outcome, RouteOutcome::NoPath));
        for from in [tile(0), tile(2)] {
            let mut request = stage_request(Arc::clone(&world));
            request.from = from;
            request.to = tile(2);
            request.arrival = ArrivalKind::Area;
            request.candidate_goals = Some(
                script::native::WalkGoals::with_preferred(vec![tile(2), tile(0)], &[tile(2)])
                    .unwrap(),
            );
            let (output, _, _) = request.drive_route_job_output_for_test();
            let RouteJobOutcome::Route(route) = &output.outcome else {
                panic!("the bounded tier searches must find this component");
            };
            assert_eq!(route.to_route(&world.graph).dest, from);
            assert_eq!(
                output.detail.as_deref(),
                Some(if from == tile(2) {
                    "candidate_goal_tier=preferred"
                } else {
                    "candidate_goal_tier=fallback"
                }),
            );
        }
    }

    #[test]
    fn candidate_search_tries_the_distant_fallback_after_preferred_failure() {
        let tile = |x| WorldTile { x, z: 0, level: 0 };
        let mut blocked = vec![0u64; (1024usize * 4).div_ceil(64)];
        blocked[1022 >> 6] |= 1 << (1022 & 63);
        let world = Arc::new(NavWorld::from_parts(
            nav_planner::collision::WorldCollision {
                origin: tile(0),
                width: 1024,
                height: 1,
                walk: vec![0; 1024 * 4],
                blocked,
                flags: None,
            },
            nav_planner::transport::TransportGraph::default(),
            Vec::new(),
        ));
        let mut request = stage_request(world);
        request.to = tile(1023);
        request.arrival = ArrivalKind::Area;
        request.candidate_goals = Some(
            script::native::WalkGoals::with_preferred(vec![tile(1023), tile(1000)], &[tile(1023)])
                .unwrap(),
        );
        let (output, _, _) = request.drive_route_job_output_for_test();
        let RouteJobOutcome::Route(route) = &output.outcome else {
            panic!("fallback-only continuation must reach the distant alternative");
        };
        assert_eq!(route.destination(), tile(1000));
        assert_eq!(
            output.detail.as_deref(),
            Some("candidate_goal_tier=fallback")
        );
    }

    #[test]
    #[ignore = "requires the reviewed real R289 NAV_PACK"]
    fn candidate_preference_keeps_own_zone_completion_ahead_of_a_safe_fallback() {
        let pack = std::env::var_os("NAV_PACK").expect("set the real 289 navpack");
        let world = Arc::new(NavWorld::load_pack(std::path::Path::new(&pack)).unwrap());
        let tile = |x, z| WorldTile { x, z, level: 0 };
        let bank = tile(3092, 3242);
        let preferred = [tile(3086, 3230), tile(3086, 3231)];
        let mut state = WorldState::empty().with_map_members(true);
        state.stats = (0..=20).map(|skill| (skill, 99)).collect();
        let shared = nav_planner::router::find_first_with_fallback(
            &world.collision,
            &world.graph,
            bank,
            &preferred,
            &[bank],
            FindOptions::default(),
            &state,
        );
        assert!(
            shared.route().is_err(),
            "safe fallback closes before own-zone completion"
        );
        assert!(
            matches!(shared.fallback(), Some(nav_planner::router::FallbackRoute::Routed(route)) if route.dest == bank)
        );
        let old = nav_planner::router::find_first_with(
            &world.collision,
            &world.graph,
            bank,
            &preferred,
            FindOptions::default(),
            &state,
        )
        .into_route()
        .unwrap();
        let mut request = stage_request(Arc::clone(&world));
        request.from = bank;
        request.to = tile(3085, 3230);
        request.arrival = ArrivalKind::Area;
        request.state = Some(Arc::new(state));
        request.candidate_goals =
            Some(script::native::WalkGoals::with_preferred(vec![bank], &preferred).unwrap());
        let mut job = completed_search_stage(request);
        let Some(RouteJobOutcome::Route(route)) = job.outcome.as_ref() else {
            panic!("the preferred-only route must remain reachable");
        };
        assert_eq!(route.to_route(&world.graph), old);
        assert!(preferred.contains(&route.destination()));
        assert_eq!(
            job.finish().detail.as_deref(),
            Some("candidate_goal_tier=preferred")
        );
    }

    #[test]
    #[ignore = "bounded NODE_BUDGET failure-cost measurement"]
    fn candidate_preferred_failure_completes_within_the_node_budget() {
        // Both components exceed the backward proof's 262144-tile cap, so the
        // proof abandons. An abandoned proof no longer stops the forward search
        // at FIRST_TARGET_BUDGET: it settles the start component and returns a
        // complete NoPath, still within NODE_BUDGET.
        let (width, height) = (1537usize, 769usize);
        let cells = width * height * 4;
        let mut blocked = vec![0u64; cells.div_ceil(64)];
        for level in 0..4 {
            for z in 0..height {
                let index = level * width * height + z * width + 768;
                blocked[index >> 6] |= 1 << (index & 63);
            }
        }
        let from = WorldTile {
            x: 384,
            z: 384,
            level: 0,
        };
        let preferred = WorldTile { x: 1152, ..from };
        let collision = nav_planner::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width,
            height,
            walk: vec![0; cells],
            blocked,
            flags: None,
        };
        let graph = nav_planner::transport::TransportGraph::default();
        let started = std::time::Instant::now();
        let search = nav_planner::router::find_first_with(
            &collision,
            &graph,
            from,
            &[preferred],
            FindOptions::default(),
            &WorldState::empty(),
        );
        let elapsed = started.elapsed();
        assert_eq!(search.route(), Err(nav_planner::router::RouteError::NoPath));
        assert_eq!(search.proof(), nav_planner::router::ReverseProof::Abandoned);
        assert!(search.settled() > nav_planner::router::FIRST_TARGET_BUDGET);
        assert!(search.settled() <= nav_planner::router::NODE_BUDGET);
        let fallback = nav_planner::router::find_first_with(
            &collision,
            &graph,
            from,
            &[from],
            FindOptions::default(),
            &WorldState::empty(),
        )
        .into_route()
        .expect("the separate fallback tier must retain the zero-cost origin");
        assert_eq!(fallback.dest, from);
        let record = serde_json::json!({
            "preferred_failure": "NoPath",
            "proof": "Abandoned",
            "forward_settles": search.settled(),
            "settle_budget": nav_planner::router::NODE_BUDGET,
            "elapsed_ms": elapsed.as_millis(),
            "fallback": fallback.dest,
            "fallback_ticks": fallback.ticks,
        });
        eprintln!("{record}");
        if let Some(path) = std::env::var_os("BOT_FISHING_TIER_BUDGET_OUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        }
    }

    #[test]
    fn single_candidate_keeps_the_ordinary_exact_area_route() {
        let world = tiny_stage_request().world;
        let tile = |x, z| WorldTile { x, z, level: 0 };
        for (from, to) in [
            (tile(0, 0), tile(3, 3)),
            (tile(3, 0), tile(0, 3)),
            (tile(1, 1), tile(2, 1)),
            (tile(2, 2), tile(2, 2)),
        ] {
            let request = |candidate_goals| {
                let mut request = stage_request(Arc::clone(&world));
                request.from = from;
                request.to = to;
                request.arrival = ArrivalKind::Area;
                request.candidate_goals = candidate_goals;
                request
            };
            let ordinary = completed_search_stage(request(None));
            assert!(matches!(ordinary.search_mode, RouteSearchMode::InOrder));
            let candidate = completed_search_stage(request(Some(
                script::native::WalkGoals::new(vec![to]).unwrap(),
            )));
            assert!(matches!(candidate.search_mode, RouteSearchMode::First));
            let Some(RouteJobOutcome::Route(ordinary)) = ordinary.outcome else {
                panic!("ordinary exact-Area route must be reachable");
            };
            let Some(RouteJobOutcome::Route(candidate)) = candidate.outcome else {
                panic!("the equivalent single candidate must be reachable");
            };
            assert_eq!(
                candidate.to_route(&world.graph),
                ordinary.to_route(&world.graph),
            );
        }
    }

    #[test]
    fn cooperative_route_targets_preserve_compat_center_priority() {
        for compat_tile_intent in [false, true] {
            let mut request = tiny_stage_request();
            request.radius = 1;
            request.compat_tile_intent = compat_tile_intent;
            let expected = request.targets();
            let job = completed_search_stage(request);
            assert_eq!(
                job.targets, expected,
                "compat_tile_intent={compat_tile_intent}"
            );
        }
    }

    fn drive_stage(worker: &mut RouteWorkJob) -> (RouteStageResult, usize, std::time::Duration) {
        use api::identity::Deadline;
        use api::work::{MonotonicClock, SharedCancellation, SliceBudget, SystemClock};

        let clock = SystemClock;
        let cancellation = SharedCancellation::new();
        let mut largest = std::time::Duration::ZERO;
        for slices in 1..=250_000 {
            let budget = SliceBudget::new(
                std::num::NonZeroU32::new(64).unwrap(),
                Deadline::after(clock.now(), std::time::Duration::from_millis(10)),
            );
            let mut context = StepContext::new(&clock, budget, &cancellation);
            let started = Instant::now();
            let outcome = worker.step(&mut context);
            largest = largest.max(started.elapsed());
            match outcome {
                StepOutcome::Yielded => {}
                StepOutcome::Completed(result) => return (result, slices, largest),
                StepOutcome::Failed(error) => panic!("route stage failed: {error}"),
            }
        }
        panic!("route stage exceeded the bounded slice count");
    }

    fn completed_search_stage(request: Box<ScriptRouteRequest>) -> Box<RouteJob> {
        let mut job =
            RouteJob::prepare("test", request, RouteDemandKind::Walk { authority: None }).unwrap();
        job.identity = Some(stage_identity(0));
        let mut worker = RouteWorkJob::new(Box::new(job));
        let (result, _, _) = drive_stage(&mut worker);
        let RouteStageResult::Continue(job) = result else {
            panic!("the completed search must cross a private job boundary");
        };
        assert!(job.search_cursor.is_none());
        assert!(!job.boundary_ready);
        job
    }

    fn carry_diagnosis_request(
        requirements: usize,
        graph_requirements: usize,
        origin: Origin,
        first_goal: bool,
    ) -> Box<ScriptRouteRequest> {
        assert!(graph_requirements >= requirements);
        let (world, from, to) = first_goal_probe_world(8, 4, 4, 6, true);
        let mut world = Arc::try_unwrap(world).ok().unwrap();
        let rows: Vec<_> = (0..requirements)
            .map(|index| (i32::try_from(index).unwrap() + 1, 1))
            .collect();
        world.graph.edges[0].item_req = rows.clone();
        if graph_requirements > requirements {
            let mut unreachable = world.graph.edges[0].clone();
            unreachable.at = WorldTile {
                x: 99,
                z: 99,
                level: 0,
            };
            unreachable.item_req = (requirements..graph_requirements)
                .map(|index| (i32::try_from(index).unwrap() + 1, 1))
                .collect();
            let index = world.graph.edges.len();
            world
                .graph
                .at
                .entry(unreachable.at)
                .or_default()
                .push(index);
            world.graph.edges.push(unreachable);
        }
        let world = Arc::new(NavWorld::from_parts(
            world.collision,
            world.graph,
            vec![nav_planner::pack::BankStand {
                name: "Bank booth".into(),
                tile: from,
                access: nav_planner::pack::BankAccess::Booth { op: 2 },
            }],
        ));
        let mut request = stage_request(world);
        request.from = from;
        request.to = to;
        request.opts.allow_bank_fetch = true;
        request.opts.allow_wilderness = true;
        request.admission = crate::admission::Admission::manual(
            request.opts,
            Default::default(),
            1,
            crate::WalkGlobals::default(),
        );
        request.state = Some(Arc::new(WorldState::empty()));
        request.bank_origin = origin;
        request.bank = rows;
        if first_goal {
            request.arrival = ArrivalKind::Area;
            request.candidate_goals = Some(script::native::WalkGoals::new(vec![to]).unwrap());
        }
        request
    }

    fn diagnosis_derived_bank_bytes(job: &RouteJob) -> usize {
        job.planning_cursor
            .as_ref()
            .map_or(0, PlanningRowsCursor::retained_bytes)
            + job
                .bank_cursor
                .as_ref()
                .map_or(0, BankFetchCursor::retained_bytes)
            + job
                .bank_plan
                .as_ref()
                .map_or(0, BankFetchPlan::retained_bytes)
            + [job.planned_rows.as_ref(), job.fetchable_rows.as_ref()]
                .into_iter()
                .flatten()
                .map(|rows| route_vec_bytes(rows).unwrap())
                .sum::<usize>()
            + job
                .fetchable_cursor
                .as_ref()
                .map_or(0, |cursor| cursor.retained_bytes().unwrap())
            + route_vec_bytes(&job.bank_undo).unwrap()
    }

    fn readmit_route_continuation(job: Box<RouteJob>) -> Box<RouteJob> {
        let identity = job.identity.unwrap();
        let revision = job.revision;
        let ledger = OwnerResultLedger::with_limits(ROUTE_WORK_LIMIT_BYTES, 64 * 1024 * 1024);
        let bytes = job.owned_retained_bytes().unwrap()
            + std::mem::size_of::<RouteDemand>()
            + OwnerResultReservation::storage_bytes();
        let reservation = ledger.try_reserve_bytes(bytes).unwrap();
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "alice".to_owned(),
            NavBot {
                route_generation: job.generation(),
                walk_request_id: job.request_id(),
                requested_route: Some((job.request.to, 0, false, true, true, ZoneExempt::NONE)),
                ..NavBot::default()
            },
        )])));
        retain_route_continuation(&navs, "alice", job, identity, revision, reservation);
        let pending = navs
            .lock()
            .unwrap()
            .get_mut("alice")
            .unwrap()
            .pending_route
            .take();
        let RouteDemand::Prepared(job) = *pending.expect("production must retain the admitted job")
        else {
            panic!("the continuation must already be prepared");
        };
        assert_eq!(
            job.revision,
            revision.next().unwrap(),
            "production retention must prepare a new admission",
        );
        job
    }

    struct BankReservationBounds {
        charged: RouteBankCharges,
        inventory_input: usize,
        worn_input: usize,
    }

    impl BankReservationBounds {
        fn at_admission(job: &RouteJob) -> Self {
            let input = job.state.growth_bound(0, 0).unwrap();
            Self {
                charged: job.submission.bank_charges,
                inventory_input: input.inventory_old_bytes,
                worn_input: input.worn_old_bytes,
            }
        }

        fn assert_retained(&self, job: &RouteJob) {
            if let Some(cursor) = &job.planning_cursor {
                assert!(
                    cursor.peak_retained_bytes() <= self.charged.planning,
                    "planning rows"
                );
            }
            for rows in [job.planned_rows.as_ref(), job.fetchable_rows.as_ref()]
                .into_iter()
                .flatten()
            {
                assert!(
                    route_vec_bytes(rows).unwrap() <= self.charged.rows,
                    "retained rows"
                );
            }
            if let Some(cursor) = &job.bank_cursor {
                assert!(
                    cursor.peak_retained_bytes() <= self.charged.fetch,
                    "fetch cursor"
                );
            }
            if let Some(plan) = &job.bank_plan {
                assert!(plan.retained_bytes() <= self.charged.plan, "bank plan");
            }
            assert!(
                route_vec_bytes(&job.bank_undo).unwrap() <= self.charged.undo,
                "undo capacity exceeds its admitted component",
            );
            let state = job.state.growth_bound(0, 0).unwrap();
            if job.bank_apply_reserved {
                let retained = state.inventory_old_bytes + state.worn_old_bytes;
                let input = self.inventory_input + self.worn_input;
                assert!(
                    retained.saturating_sub(input) <= self.charged.post_bank_state,
                    "post-bank state capacity exceeds its admitted component",
                );
            }
            if let Some(cursor) = &job.fetchable_cursor {
                let growth = state
                    .inventory_old_bytes
                    .saturating_sub(self.inventory_input);
                assert!(
                    cursor.retained_bytes().unwrap() + growth <= self.charged.overlay,
                    "overlay capacity exceeds its admitted component",
                );
            }
        }
    }

    fn drive_stage_with_bank_bounds(
        worker: &mut RouteWorkJob,
    ) -> (RouteStageResult, usize, std::time::Duration, bool) {
        use api::identity::Deadline;
        use api::work::{MonotonicClock, SharedCancellation, SliceBudget, SystemClock};

        let bounds = BankReservationBounds::at_admission(worker.job.as_ref().unwrap());
        let clock = SystemClock;
        let cancellation = SharedCancellation::new();
        let mut largest = std::time::Duration::ZERO;
        let mut saw_apply = false;
        // Keep the original 250,000 * 64 unit horizon. One-unit slices expose
        // each reservation before a later search can transfer or release it.
        for slices in 1..=16_000_000 {
            let budget = SliceBudget::new(
                std::num::NonZeroU32::new(1).unwrap(),
                Deadline::after(clock.now(), std::time::Duration::from_millis(10)),
            );
            let mut context = StepContext::new(&clock, budget, &cancellation);
            let started = Instant::now();
            let outcome = worker.step(&mut context);
            largest = largest.max(started.elapsed());
            if let Some(job) = worker.job.as_ref() {
                bounds.assert_retained(job);
                saw_apply |= job.bank_apply_reserved;
            }
            match outcome {
                StepOutcome::Yielded => {}
                StepOutcome::Completed(result) => {
                    if let RouteStageResult::Continue(job) = &result {
                        bounds.assert_retained(job);
                        saw_apply |= job.bank_apply_reserved;
                    }
                    return (result, slices, largest, saw_apply);
                }
                StepOutcome::Failed(error) => panic!("route stage failed: {error}"),
            }
        }
        panic!("route stage exceeded its unchanged unit horizon");
    }

    fn next_diagnosis_boundary(
        request: Box<ScriptRouteRequest>,
    ) -> (Box<RouteJob>, RouteSubmission) {
        let mut job = Box::new(
            RouteJob::prepare(
                "diagnosis-boundary",
                request,
                RouteDemandKind::Walk { authority: None },
            )
            .unwrap(),
        );
        for index in 0..16 {
            job.identity = Some(stage_identity(index));
            let before = *job.submission();
            let previous_count = job
                .missing_requirements
                .as_ref()
                .map_or(0, MissingRequirements::len);
            let mut worker = RouteWorkJob::new(job);
            let (result, _, _) = drive_stage(&mut worker);
            let RouteStageResult::Continue(next) = result else {
                panic!("the missing-item diagnosis must cross a job boundary");
            };
            let next_count = next
                .missing_requirements
                .as_ref()
                .map_or(0, MissingRequirements::len);
            if next_count > previous_count {
                assert!(next.search_cursor.is_none());
                assert!(!next.boundary_ready);
                assert_eq!(
                    diagnosis_derived_bank_bytes(&next),
                    0,
                    "the larger diagnosis must not grow bank buffers in the old admission",
                );
                return (next, before);
            }
            job = readmit_route_continuation(next);
        }
        panic!("the bounded fixture did not publish a missing-item diagnosis");
    }

    #[test]
    fn a_larger_diagnosis_is_readmitted_before_bank_buffers_grow() {
        for (requirements, graph_requirements) in [(1, 1), (1024, 1024), (1, 4097)] {
            for (origin, first_goal) in [
                (Origin::Session, false),
                (Origin::Session, true),
                (Origin::Hint, true),
            ] {
                let (job, before) = next_diagnosis_boundary(carry_diagnosis_request(
                    requirements,
                    graph_requirements,
                    origin,
                    first_goal,
                ));
                assert_eq!(
                    job.missing_requirements.as_ref().unwrap().len(),
                    requirements
                );
                assert_eq!(job.plan_item_limit(), graph_requirements);
                let mut job = readmit_route_continuation(job);
                assert_eq!(job.submission.plan_item_rows, requirements);
                assert!(
                    job.submission.result_bytes
                        >= before.result_bytes + BankFetchPlan::heap_bound(requirements).unwrap(),
                    "the worst-case diagnosed plan must retain its complete output bound",
                );
                let charge = <RouteWorkSpec as api::work::WorkFamilySpec>::charge(job.submission())
                    .total_bytes()
                    .unwrap();
                assert!(charge <= ROUTE_WORK_LIMIT_BYTES);
                assert_eq!(diagnosis_derived_bank_bytes(&job), 0);
                let mut saw_apply = false;
                for index in 0..8 {
                    job.identity = Some(stage_identity(index + 16));
                    let mut worker = RouteWorkJob::new(job);
                    let (result, _, _, applied) = drive_stage_with_bank_bounds(&mut worker);
                    if applied {
                        saw_apply = true;
                        break;
                    }
                    let RouteStageResult::Continue(next) = result else {
                        panic!("the bank fixture must reach post-bank search");
                    };
                    job = readmit_route_continuation(next);
                }
                assert!(
                    saw_apply,
                    "the guard must check undo, inventory and worn reservations"
                );
            }
        }
    }

    #[test]
    fn worn_diagnosis_keeps_withdraw_and_wear_undo_charged() {
        let mut request = carry_diagnosis_request(1, 4097, Origin::Session, false);
        let mut world = Arc::try_unwrap(request.world).ok().unwrap();
        world.graph.edges[0].item_req.clear();
        world.graph.edges[0].worn_req = vec![1];
        let stands = world.banks().to_vec();
        request.world = Arc::new(NavWorld::from_parts(world.collision, world.graph, stands));
        let (job, _) = next_diagnosis_boundary(request);
        assert_eq!(job.missing_requirements.as_ref().unwrap().len(), 1);
        let mut job = readmit_route_continuation(job);
        for index in 0..8 {
            job.identity = Some(stage_identity(index + 48));
            let mut worker = RouteWorkJob::new(job);
            let (result, _, _, applied) = drive_stage_with_bank_bounds(&mut worker);
            if applied {
                return;
            }
            let RouteStageResult::Continue(next) = result else {
                panic!("the worn fixture must reach post-bank search");
            };
            job = readmit_route_continuation(next);
        }
        panic!("the worn guard must check withdrawal and wear reservations");
    }

    #[test]
    fn retained_hint_overlay_stays_charged_after_a_smaller_diagnosis() {
        let mut request = carry_diagnosis_request(64, 4097, Origin::Hint, true);
        request.bank = Vec::new();
        let (job, _) = next_diagnosis_boundary(request);
        let mut job = readmit_route_continuation(job);
        for index in 0..4 {
            if job.fetchable_rows.is_some() {
                break;
            }
            job.identity = Some(stage_identity(index + 32));
            let mut worker = RouteWorkJob::new(job);
            let (result, _, _, _) = drive_stage_with_bank_bounds(&mut worker);
            let RouteStageResult::Continue(next) = result else {
                panic!("the Hint overlay must return its planned rows");
            };
            job = readmit_route_continuation(next);
        }
        let retained_rows = job.fetchable_rows.as_ref().unwrap().len();
        assert_eq!(retained_rows, 64);
        assert_eq!(job.bank_rows_limit, 0);
        let (mut smaller, _) =
            next_diagnosis_boundary(carry_diagnosis_request(1, 4097, Origin::Session, false));
        job.missing_requirements = smaller.missing_requirements.take();
        job = readmit_route_continuation(job);
        assert_eq!(job.missing_requirements.as_ref().unwrap().len(), 1);
        assert_eq!(
            job.submission.plan_item_rows, 64,
            "a smaller diagnosis must retain the earlier overlay bound",
        );
        assert!(retained_rows > job.bank_rows_limit + 1);
        let bounds = BankReservationBounds::at_admission(&job);
        job.finish_bank_failure();
        assert!(job.fetchable_cursor.is_some());
        bounds.assert_retained(&job);
    }

    #[test]
    fn maximum_diagnosis_is_refused_before_unreserved_bank_buffers_grow() {
        let requirements = 50_000;
        let (job, _) = next_diagnosis_boundary(carry_diagnosis_request(
            requirements,
            requirements,
            Origin::Session,
            false,
        ));
        assert_eq!(
            job.missing_requirements.as_ref().unwrap().len(),
            requirements
        );
        assert_eq!(
            job.plan_item_limit(),
            requirements,
            "the diagnosis reaches the unchanged graph maximum",
        );
        let identity = job.identity.unwrap();
        let revision = job.revision;
        let generation = job.generation();
        let request_id = job.request_id();
        let target = job.request.to;
        let ledger = OwnerResultLedger::with_limits(ROUTE_WORK_LIMIT_BYTES, ROUTE_WORK_LIMIT_BYTES);
        let bytes = job.owned_retained_bytes().unwrap()
            + std::mem::size_of::<RouteDemand>()
            + OwnerResultReservation::storage_bytes();
        let reservation = ledger.try_reserve_bytes(bytes).unwrap();
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "alice".to_owned(),
            NavBot {
                route_generation: generation,
                walk_request_id: request_id,
                requested_route: Some((target, 0, false, true, true, ZoneExempt::NONE)),
                ..NavBot::default()
            },
        )])));
        retain_route_continuation(&navs, "alice", job, identity, revision, reservation);
        let all = navs.lock().unwrap();
        let bot = &all["alice"];
        assert!(bot.pending_route.is_none());
        assert!(bot.route_ticket.is_none());
        assert!(bot.bank_fetch.is_none());
        assert!(bot.walk_outcome_failed);
        assert!(bot
            .walk_outcome_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("checked WorkAdmission bounds")));
        assert_eq!(ledger.used_bytes(), 0);
        assert_eq!(ROUTE_WORK_LIMIT_BYTES, 192 * 1024 * 1024);
    }

    #[test]
    fn search_result_is_an_exact_charged_input_to_the_next_route_job() {
        let mut job = completed_search_stage(tiny_stage_request());
        assert!(matches!(job.outcome, Some(RouteJobOutcome::Route(_))));
        let exact = job.owned_retained_bytes().unwrap();
        let revision = job.revision;
        job.prepare_continuation().unwrap();
        assert_eq!(job.revision, revision.next().unwrap());
        assert!(
            job.submission.input_bytes >= exact + OwnerResultReservation::storage_bytes(),
            "the later job must charge the retained route and captured request as input",
        );
        assert!(
            <RouteWorkSpec as api::work::WorkFamilySpec>::charge(job.submission())
                .total_bytes()
                .unwrap()
                <= ROUTE_WORK_LIMIT_BYTES,
        );
    }

    #[test]
    fn manual_cancel_in_the_route_job_gap_releases_custody_and_signals_once() {
        let mut request = tiny_stage_request();
        let (completion, receiver) = RouteCompletion::channel();
        request.completion = completion;
        let job = completed_search_stage(request);
        assert!(matches!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty),
        ));
        let identity = job.identity.expect("executed stage identity");
        let revision = job.revision;
        let bytes = job.owned_retained_bytes().unwrap()
            + std::mem::size_of::<RouteDemand>()
            + OwnerResultReservation::storage_bytes();
        let ledger = OwnerResultLedger::with_limits(ROUTE_WORK_LIMIT_BYTES, ROUTE_WORK_LIMIT_BYTES);
        let reservation = ledger.try_reserve_bytes(bytes).unwrap();
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "alice".to_owned(),
            NavBot {
                route_generation: 1,
                walk_request_id: 1,
                ..NavBot::default()
            },
        )])));
        retain_route_continuation(&navs, "alice", job, identity, revision, reservation);
        assert_eq!(ledger.used_bytes(), bytes);
        {
            let mut all = navs.lock().unwrap();
            let bot = all.get_mut("alice").unwrap();
            assert!(matches!(
                bot.pending_route.as_deref(),
                Some(RouteDemand::Prepared(_))
            ));
            end_route_follow(bot);
            assert!(bot.pending_route.is_none());
            assert!(!bot.has_route());
        }
        assert_eq!(ledger.used_bytes(), 0);
        receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected),
        ));
    }

    #[test]
    fn stale_generation_identity_and_revision_do_not_publish_across_the_route_job_gap() {
        for stale in 0..3 {
            let job = completed_search_stage(tiny_stage_request());
            let identity = job.identity.expect("executed stage identity");
            let revision = job.revision;
            let ledger =
                OwnerResultLedger::with_limits(ROUTE_WORK_LIMIT_BYTES, ROUTE_WORK_LIMIT_BYTES);
            let reservation = ledger
                .try_reserve_bytes(job.owned_retained_bytes().unwrap())
                .unwrap();
            let navs = Arc::new(Mutex::new(HashMap::from([(
                "alice".to_owned(),
                NavBot {
                    route_generation: if stale == 0 { 2 } else { 1 },
                    walk_request_id: 1,
                    ..NavBot::default()
                },
            )])));
            retain_route_continuation(
                &navs,
                "alice",
                job,
                if stale == 1 {
                    stage_identity(1)
                } else {
                    identity
                },
                if stale == 2 {
                    revision.next().unwrap()
                } else {
                    revision
                },
                reservation,
            );
            let all = navs.lock().unwrap();
            let bot = &all["alice"];
            assert!(bot.pending_route.is_none(), "stale kind {stale}");
            assert!(!bot.has_route(), "stale kind {stale}");
            assert!(!bot.walk_outcome_failed, "stale kind {stale}");
            assert_eq!(ledger.used_bytes(), 0, "stale kind {stale}");
        }
    }

    #[test]
    #[ignore = "requires BOT_ROUTE_PARITY_NAVPACK and BOT_ROUTE_PARITY_OUT; real 17-case per-goal corpus"]
    fn ordinary_and_single_candidate_routes_match_across_the_real_corpus() {
        use sha2::Digest;
        use std::collections::{HashMap, HashSet};
        use std::io::Write as _;

        fn snapshot_for_state(state: &WorldState) -> GameSnapshot {
            fn item(
                id: i32,
                count: i32,
                container: api::snapshot::ItemContainer,
                slot: i32,
            ) -> api::snapshot::ItemView {
                api::snapshot::ItemView {
                    def: api::ItemDefView {
                        id,
                        name: None,
                        stackable: false,
                        members: false,
                        base_value: 0,
                        noted: false,
                        certificate_link: -1,
                        certificate_template: -1,
                    },
                    container,
                    action_family: api::snapshot::ItemActionFamily::Held,
                    slot,
                    count,
                    actions: Vec::new(),
                    component_id: 0,
                }
            }

            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(2);

            let mut inventory = state
                .inv
                .iter()
                .map(|(&id, &count)| (id, count))
                .collect::<Vec<_>>();
            inventory.sort_unstable_by_key(|&(id, _)| id);
            snapshot.seed_inventory(
                inventory
                    .into_iter()
                    .enumerate()
                    .map(|(slot, (id, count))| {
                        item(
                            id,
                            count,
                            api::snapshot::ItemContainer::Inventory,
                            i32::try_from(slot).unwrap(),
                        )
                    })
                    .collect(),
                28,
            );
            snapshot.seed_inventory_packet_generations(state.inventory_ready.then_some(1), None);

            let mut worn = state.worn.iter().copied().collect::<Vec<_>>();
            worn.sort_unstable();
            snapshot.seed_equipment(
                worn.into_iter()
                    .enumerate()
                    .map(|(slot, id)| {
                        item(
                            id,
                            1,
                            api::snapshot::ItemContainer::Equipment,
                            i32::try_from(slot).unwrap(),
                        )
                    })
                    .collect(),
            );
            snapshot.seed_equipment_packet_generation(state.equipment_ready.then_some(1));

            let combat_base = match state.combat_level {
                Some(126) => Some([99; 7]),
                Some(3) => Some([3; 7]),
                Some(50) => Some([40, 39, 40, 40, 40, 39, 40]),
                None => None,
                Some(level) => panic!("unexpected route fixture combat level {level}"),
            };
            let mut stat_indices = state.stats.keys().copied().collect::<Vec<_>>();
            if combat_base.is_some() {
                stat_indices.extend(0..7);
            }
            stat_indices.sort_unstable();
            stat_indices.dedup();
            snapshot.seed_stats(
                stat_indices
                    .into_iter()
                    .map(|index| {
                        let effective = state.stats.get(&index).copied().unwrap_or(0);
                        let base = combat_base
                            .and_then(|levels| {
                                usize::try_from(index)
                                    .ok()
                                    .and_then(|slot| levels.get(slot).copied())
                            })
                            .unwrap_or(effective);
                        api::snapshot::StatView {
                            index,
                            name: format!("skill-{index}"),
                            effective,
                            base,
                            xp: 0,
                            used: true,
                        }
                    })
                    .collect(),
            );

            let mut varps = state
                .varps
                .iter()
                .map(|(&index, &value)| (index, value))
                .collect::<Vec<_>>();
            varps.sort_unstable_by_key(|&(index, _)| index);
            snapshot.seed_varps(
                varps
                    .into_iter()
                    .map(|(index, value)| api::snapshot::VarpView { index, value })
                    .collect(),
            );

            let mut quests = state.quests.iter().cloned().collect::<Vec<_>>();
            quests.sort_unstable();
            snapshot.seed_quest_statuses(
                quests
                    .into_iter()
                    .enumerate()
                    .map(|(component_id, name)| api::snapshot::QuestStatusView {
                        component_id: i32::try_from(component_id).unwrap(),
                        name,
                        colour: nav_planner::world_state::QUEST_COMPLETE_COLOUR,
                    })
                    .collect(),
                true,
            );
            snapshot
        }

        let pack = std::path::PathBuf::from(
            std::env::var_os("BOT_ROUTE_PARITY_NAVPACK").expect("set the real 289 navpack"),
        );
        let output = std::path::PathBuf::from(
            std::env::var_os("BOT_ROUTE_PARITY_OUT").expect("set the parity evidence output"),
        );
        assert!(pack.is_absolute() && output.is_absolute());
        let stride: usize = std::env::var("BOT_ROUTE_PARITY_STRIDE")
            .map_or(1, |value| value.parse().expect("positive parity stride"));
        assert!(stride > 0);
        let tile = |x, z, level| WorldTile { x, z, level };
        let mut loaded = NavWorld::load_pack(&pack).unwrap();
        loaded
            .bind_named_bank_facts(
                &api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
            )
            .unwrap();
        let mut zones = loaded.graph.zones.take();
        let mut world = Arc::new(loaded);
        let empty = WorldState::empty();
        let rich = WorldState {
            map_members: true,
            combat_level: Some(126),
            quests: HashSet::from([
                "Prince Ali Rescue".into(),
                "Rune Mysteries".into(),
                "Lost City".into(),
                "Shilo Village".into(),
                "Tree Gnome Village".into(),
                "The Grand Tree".into(),
            ]),
            inv: HashMap::from([
                (995, 10_000),
                (554, 1_000),
                (555, 1_000),
                (556, 1_000),
                (557, 1_000),
                (558, 1_000),
                (561, 1_000),
                (563, 1_000),
            ]),
            stats: (0..21).map(|skill| (skill, 99)).collect(),
            ..WorldState::empty()
        };
        let low = WorldState {
            combat_level: Some(3),
            ..rich.clone()
        };
        let live = WorldState {
            map_members: true,
            combat_level: Some(50),
            inv: HashMap::from([(995, 60)]),
            stats: (0..7).map(|skill| (skill, 0)).collect(),
            ..WorldState::empty()
        };
        let legacy = FindOptions {
            zones: nav_planner::zones::ZoneExempt::all(),
            ..FindOptions::default()
        };
        let teleports = FindOptions {
            allow_teleports: true,
            ..legacy
        };
        let default = FindOptions::default();
        let bank_targets: Vec<_> = world
            .named_bank_facts()
            .unwrap()
            .banks()
            .iter()
            .filter(|bank| bank.routable)
            .map(|bank| bank.tile)
            .collect();
        let mut radius_targets = Vec::new();
        for x in 2543..=2563 {
            for z in 3396..=3416 {
                let goal = tile(x, z, 0);
                if world.collision.standable(goal) {
                    radius_targets.push(goal);
                }
            }
        }
        let lumbridge = tile(3222, 3218, 0);
        let falador = tile(2965, 3379, 0);
        let dungeon = tile(3016, 9840, 0);
        let radius_from = tile(2615, 3332, 0);
        // Per-goal projections of all 17 rows in nav/examples/perf_corpus.rs.
        // Many/First rows expand to their full goal lists, not a proxy center.
        let cases = [
            (
                "reachable_lumbridge_falador",
                lumbridge,
                vec![falador],
                legacy,
                empty.clone(),
            ),
            (
                "bank_pick_all_stands",
                dungeon,
                bank_targets.clone(),
                legacy,
                rich.clone(),
            ),
            (
                "radius_walk_candidates",
                radius_from,
                radius_targets.clone(),
                legacy,
                rich.clone(),
            ),
            (
                "teleports_on_real_state",
                lumbridge,
                vec![tile(2817, 3443, 0)],
                teleports,
                rich.clone(),
            ),
            (
                "unreachable_fail_closed",
                lumbridge,
                vec![tile(2817, 3443, 0)],
                legacy,
                empty.clone(),
            ),
            (
                "reachable_lumbridge_falador_zones_l126",
                lumbridge,
                vec![falador],
                default,
                rich.clone(),
            ),
            (
                "reachable_lumbridge_falador_zones_l3",
                lumbridge,
                vec![falador],
                default,
                low.clone(),
            ),
            (
                "bank_pick_all_stands_zones_l3",
                dungeon,
                bank_targets,
                default,
                low.clone(),
            ),
            (
                "radius_walk_candidates_zones_l3",
                radius_from,
                radius_targets,
                default,
                low.clone(),
            ),
            (
                "lumbridge_rellekka_refused_zones_l126",
                lumbridge,
                vec![tile(2664, 3664, 0)],
                default,
                rich.clone(),
            ),
            (
                "taverley_catherby_zones_l126",
                tile(2895, 3450, 0),
                vec![tile(2809, 3440, 0)],
                default,
                rich,
            ),
            (
                "danger_live_off",
                tile(2809, 3441, 0),
                vec![tile(3103, 3163, 2)],
                default,
                live.clone(),
            ),
            (
                "danger_live_grant",
                tile(2809, 3441, 0),
                vec![tile(3103, 3163, 2)],
                legacy,
                live,
            ),
            (
                "falador_lumbridge_zones_l3",
                falador,
                vec![lumbridge],
                default,
                low.clone(),
            ),
            (
                "deep_zone_goal_teles_off",
                tile(2895, 3450, 0),
                vec![tile(2852, 3493, 0)],
                default,
                low.clone(),
            ),
            (
                "deep_zone_goal_teles_on",
                tile(2895, 3450, 0),
                vec![tile(2852, 3493, 0)],
                FindOptions {
                    allow_teleports: true,
                    ..default
                },
                low,
            ),
            ("all_off_control", lumbridge, vec![falador], default, empty),
        ];
        let mut records = Vec::new();
        let cached_route_output = output.with_extension("cached.routes");
        let fresh_route_output = output.with_extension("fresh.routes");
        let mut cached_route_bytes = std::fs::File::create(&cached_route_output).unwrap();
        let mut fresh_route_bytes = std::fs::File::create(&fresh_route_output).unwrap();
        let started = Instant::now();
        for (index, (name, from, goals, opts, state)) in cases.into_iter().enumerate() {
            if index == 5 {
                Arc::get_mut(&mut world).unwrap().graph.zones = zones.take();
            }
            let all_goals = goals.len();
            let mut selected = if name.starts_with("radius_walk") && stride > 1 {
                let mut selected: Vec<_> = goals.iter().step_by(stride).copied().collect();
                let first = nav_planner::router::find_first_with(
                    &world.collision,
                    &world.graph,
                    from,
                    &goals,
                    opts,
                    &state,
                );
                if let Ok(route) = first.route() {
                    if !selected.contains(&route.dest) {
                        selected.push(route.dest);
                    }
                }
                selected
            } else {
                goals
            };
            let pair_count = selected.len();
            let mut routed = 0;
            let mut route_digest = sha2::Sha256::new();
            let snapshot = snapshot_for_state(&state);
            let mut cache = crate::script_runtime::NavWorldStateCache::default();
            let here = Some((from.x, from.z, from.level));
            let cached_state = crate::script_runtime::nav_world_state_for_observe(
                &mut cache,
                here,
                &snapshot,
                true,
                false,
                state.map_members,
                1,
            )
            .expect("active route corpus state");
            let cached_again = crate::script_runtime::nav_world_state_for_observe(
                &mut cache,
                here,
                &snapshot,
                true,
                false,
                state.map_members,
                1,
            )
            .expect("reused route corpus state");
            assert!(Arc::ptr_eq(&cached_state, &cached_again));
            let original_state = Arc::new(state.clone());
            let fresh_state =
                Arc::new(WorldState::from_snapshot(&snapshot).with_map_members(state.map_members));
            assert_eq!(cached_state.as_ref(), original_state.as_ref());
            assert_eq!(cached_state.as_ref(), fresh_state.as_ref());
            for goal in selected.drain(..) {
                let request = |route_state: Arc<WorldState>, candidate_goals| {
                    let mut request = stage_request(Arc::clone(&world));
                    request.from = from;
                    request.to = goal;
                    request.arrival = ArrivalKind::Area;
                    request.opts = opts;
                    request.admission = crate::admission::Admission::manual(
                        opts,
                        Default::default(),
                        1,
                        crate::WalkGlobals::default(),
                    );
                    request.state = Some(route_state);
                    request.candidate_goals = candidate_goals;
                    request
                };
                let (before, _, _) = request(Arc::clone(&original_state), None)
                    .drive_route_job_output_with_diagnostic_budget_for_test(usize::MAX);
                let before = ScriptRouteRequest::project_route_job_outcome(
                    before.outcome,
                    before.opts,
                    before.bank_avoid_copy,
                    &world,
                );
                let (ordinary, _) =
                    request(Arc::clone(&original_state), None).drive_route_job_for_test();
                let (candidate, _) = request(
                    Arc::clone(&original_state),
                    Some(script::native::WalkGoals::new(vec![goal]).unwrap()),
                )
                .drive_route_job_for_test();
                let (cached, _) =
                    request(Arc::clone(&cached_state), None).drive_route_job_for_test();
                let (fresh, _) = request(Arc::clone(&fresh_state), None).drive_route_job_for_test();
                route_digest.update(goal.x.to_le_bytes());
                route_digest.update(goal.z.to_le_bytes());
                route_digest.update(goal.level.to_le_bytes());
                match (before, ordinary, candidate, cached, fresh) {
                    (
                        RouteOutcome::Routed(before),
                        RouteOutcome::Routed(ordinary),
                        RouteOutcome::Routed(candidate),
                        RouteOutcome::Routed(cached),
                        RouteOutcome::Routed(fresh),
                    ) => {
                        assert_eq!(before, ordinary, "{name}: {from:?} -> {goal:?}");
                        assert_eq!(
                            ordinary, candidate,
                            "{name}: candidate route {from:?} -> {goal:?}"
                        );
                        assert_eq!(
                            ordinary, cached,
                            "{name}: cached route {from:?} -> {goal:?}"
                        );
                        assert_eq!(
                            ordinary, fresh,
                            "{name}: independent fresh-state route {from:?} -> {goal:?}"
                        );
                        route_digest.update(format!("{ordinary:?}").as_bytes());
                        cached_route_bytes
                            .write_all(
                                format!("{name}\t{from:?}\t{goal:?}\t{cached:?}\n").as_bytes(),
                            )
                            .unwrap();
                        fresh_route_bytes
                            .write_all(
                                format!("{name}\t{from:?}\t{goal:?}\t{fresh:?}\n").as_bytes(),
                            )
                            .unwrap();
                        routed += 1;
                    }
                    (
                        RouteOutcome::NoPath,
                        RouteOutcome::NoPath,
                        RouteOutcome::NoPath,
                        RouteOutcome::NoPath,
                        RouteOutcome::NoPath,
                    ) => {
                        route_digest.update(b"NoPath");
                        let line = format!("{name}\t{from:?}\t{goal:?}\tno-path\n");
                        cached_route_bytes.write_all(line.as_bytes()).unwrap();
                        fresh_route_bytes.write_all(line.as_bytes()).unwrap();
                    }
                    _ => panic!(
                        "{name}: diagnostic/original/candidate/cached/fresh route availability changed for {from:?} -> {goal:?}"
                    ),
                }
            }
            let record = serde_json::json!({
                "case": name, "from": [from.x, from.z, from.level],
                "corpus_goals": all_goals, "pairs": pair_count,
                "arrival": "Area", "radius": 0,
                "stride": if name.starts_with("radius_walk") { stride } else { 1 },
                "routed_pairs": routed, "no_path_pairs": pair_count - routed,
                "diagnostic_bound_routes_unchanged": true,
                "routes_sha256": format!("{:x}", route_digest.finalize()),
                "exact_path_and_cost_equal": true,
            });
            println!("{record}");
            records.push(record);
            std::fs::write(
                &output,
                serde_json::to_vec_pretty(&serde_json::json!({
                    "schema": "274bot-host-per-goal-parity-v1",
                    "complete": records.len() == 17,
                    "elapsed_ms": started.elapsed().as_millis(),
                    "rows": records,
                }))
                .unwrap(),
            )
            .unwrap();
        }
        assert_eq!(records.len(), 17);
        cached_route_bytes.flush().unwrap();
        fresh_route_bytes.flush().unwrap();
        assert_eq!(
            std::fs::metadata(&cached_route_output).unwrap().len(),
            std::fs::metadata(&fresh_route_output).unwrap().len(),
            "cached and fresh route corpus byte lengths differ"
        );
        println!(
            "cached/fresh route bytes written: {} and {}",
            cached_route_output.display(),
            fresh_route_output.display()
        );
    }

    #[test]
    #[ignore = "requires BOT_ROUTE_PLAN_NAVPACK and BOT_ROUTE_PLAN_CAPTURE_OUT"]
    fn qualifies_real_bank_route_stages_with_the_production_node_budget() {
        let pack = std::path::PathBuf::from(
            std::env::var_os("BOT_ROUTE_PLAN_NAVPACK").expect("set the real 289 navpack"),
        );
        let output = std::path::PathBuf::from(
            std::env::var_os("BOT_ROUTE_PLAN_CAPTURE_OUT").expect("set the evidence output"),
        );
        assert!(pack.is_absolute() && output.is_absolute());
        let loaded = NavWorld::load_pack(&pack).unwrap();
        loaded
            .bind_named_bank_facts(
                &api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
            )
            .unwrap();
        let world = Arc::new(loaded);
        assert_eq!(NODE_BUDGET, 4_000_000);
        let mut records = Vec::new();
        // Keep the exact-tile carried-knife case. The exact worn-only case
        // retains its baseline refusal. Qualify the worn bank branch through
        // the unchanged production Area path, with radius one.
        for (case, item, radius, arrival) in [
            ("carry-knife", 946, 0, ArrivalKind::Reach),
            ("exact-worn-blade", 1277, 0, ArrivalKind::Reach),
            ("worn-blade", 1277, 1, ArrivalKind::Area),
        ] {
            let mut request = stage_request(Arc::clone(&world));
            request.from = WorldTile {
                x: 3202,
                z: 9909,
                level: 0,
            };
            request.to = WorldTile {
                x: 3209,
                z: 9891,
                level: 0,
            };
            request.radius = radius;
            request.arrival = arrival;
            request.opts = FindOptions {
                allow_bank_fetch: true,
                allow_wilderness: true,
                ..FindOptions::default()
            };
            request.admission = crate::admission::Admission::manual(
                request.opts,
                Default::default(),
                1,
                crate::WalkGlobals::default(),
            );
            request.state = Some(Arc::new(WorldState {
                stats: (0..25).map(|skill| (skill, 99)).collect(),
                combat_level: Some(126),
                map_members: true,
                ..WorldState::empty()
            }));
            request.bank_origin = Origin::Session;
            request.bank = vec![(item, 1)];
            if case == "exact-worn-blade" {
                let state = request
                    .state
                    .as_ref()
                    .expect("qualification request has a captured state")
                    .clone();
                let bank = request.bank.clone();
                let from = request.from;
                let to = request.to;
                let opts = request.opts;
                let (outcome, _) = request.drive_route_job_for_test();
                assert!(matches!(outcome, RouteOutcome::NoPath));
                let missing = nav_planner::router::find_missing_item_reqs_with_avoid(
                    &world.collision,
                    &world.graph,
                    from,
                    to,
                    opts,
                    &state,
                    &[],
                )
                .unwrap();
                assert_eq!(
                    missing,
                    vec![nav_planner::router::MissingReq::Carry { id: 946, count: 1 }],
                );
                assert!(matches!(
                    nav_planner::bank_fetch::plan_bank_fetch(
                        &missing,
                        &state,
                        &bank,
                        world.banks(),
                        from,
                        &world.collision,
                        &world.graph,
                        opts,
                        &[],
                    ),
                    Err(nav_planner::bank_fetch::BankFetchError::Unavailable),
                ));
                records.push(serde_json::json!({
                    "case": case,
                    "radius": radius,
                    "arrival": "Reach",
                    "legacy_result": "NoPath",
                    "missing": [{"kind": "Carry", "id": 946, "count": 1}],
                    "bank_rows": bank,
                    "bank_plan_accepted": false,
                    "qualification": "unchanged exact-tile refusal; baseline comparison is external",
                }));
                continue;
            }
            let mut captured = None;
            let capture = allocation_counter::measure(|| {
                let mut job =
                    RouteJob::prepare("test", request, RouteDemandKind::Walk { authority: None })
                        .unwrap();
                job.identity = Some(stage_identity(0));
                captured = Some(job);
            });
            let mut next = Some(Box::new(captured.unwrap()));
            let ledger = OwnerResultLedger::with_limits(ROUTE_WORK_LIMIT_BYTES, 64 * 1024 * 1024);
            let mut completed = false;
            let mut saw_bank_access = false;
            let mut saw_post_bank = false;
            for index in 0..64 {
                let job = next
                    .take()
                    .expect("the prior stage retained its continuation");
                let phase = format!("{:?}", job.stage);
                let purpose = match job.search_purpose {
                    RouteSearchPurpose::Strict => "Strict",
                    RouteSearchPurpose::StrictFirst => "StrictFirst",
                    RouteSearchPurpose::CandidateFallback => "CandidateFallback",
                    RouteSearchPurpose::MissingTarget => "MissingTarget",
                    RouteSearchPurpose::MissingTargets => "MissingTargets",
                    RouteSearchPurpose::MissingRoute => "MissingRoute",
                    RouteSearchPurpose::BankAccess => "BankAccess",
                    RouteSearchPurpose::PostBank => "PostBank",
                    RouteSearchPurpose::FetchableFirst => "FetchableFirst",
                    RouteSearchPurpose::Blocking => "Blocking",
                    RouteSearchPurpose::Unresolved => "Unresolved",
                    RouteSearchPurpose::Actionable => "Actionable",
                    RouteSearchPurpose::MissingCarry => "MissingCarry",
                    RouteSearchPurpose::Entrance => "Entrance",
                };
                saw_bank_access |= job.search_purpose == RouteSearchPurpose::BankAccess;
                saw_post_bank |= job.search_purpose == RouteSearchPurpose::PostBank;
                let input = *job.submission();
                let charge = <RouteWorkSpec as api::work::WorkFamilySpec>::charge(&input)
                    .total_bytes()
                    .unwrap();
                assert!(charge <= ROUTE_WORK_LIMIT_BYTES, "{case} job {index}");
                assert!(capture.bytes_max <= charge as u64);
                let held =
                    job.owned_retained_bytes().unwrap() + OwnerResultReservation::storage_bytes();
                let retained_routes: usize = [
                    job.strict_fallback.as_ref(),
                    job.selected_route.as_ref(),
                    job.bank_access_route.as_ref(),
                ]
                .into_iter()
                .flatten()
                .map(|route| route.try_retained_bytes().unwrap())
                .sum();
                let mut measured = None;
                let allocations = allocation_counter::measure(|| {
                    let mut worker = RouteWorkJob::new(job);
                    measured = Some(drive_stage_with_bank_bounds(&mut worker));
                });
                let peak = held
                    .checked_add(usize::try_from(allocations.bytes_max).unwrap())
                    .unwrap();
                assert!(
                    peak <= charge,
                    "{case} job {index}: peak={peak}, charge={charge}"
                );
                let (result, slices, largest, _) = measured.unwrap();
                let result_bytes = route_stage_retained_bytes(&result).unwrap().unwrap_or_else(|| {
                    if let RouteStageResult::Complete(output) = &result {
                        panic!(
                            "{case} job {index} {phase}/{purpose}: no route custody; detail={:?}, missing={:?}, blocked={:?}, refusal={:?}",
                            output.detail, output.missing, output.blocked, output.refusal,
                        );
                    }
                    panic!("{case} job {index}: continuation has no custody")
                });
                let reservation = ledger.try_reserve_bytes(result_bytes).unwrap();
                records.push(serde_json::json!({
                    "case": case,
                    "radius": radius,
                    "arrival": format!("{arrival:?}"),
                    "job": index,
                    "phase_at_admission": phase,
                    "next_search_at_admission": purpose,
                    "input_bytes": input.input_bytes,
                    "scratch_bytes": input.scratch_bytes,
                    "admitted_plan_item_rows": input.plan_item_rows,
                    "result_bound_bytes": input.result_bytes,
                    "charge_bytes": charge,
                    "exact_initial_owned_bytes": held,
                    "retained_earlier_route_bytes": retained_routes,
                    "new_allocation_peak_bytes": allocations.bytes_max,
                    "whole_job_peak_bytes": peak,
                    "exact_terminal_bytes": result_bytes,
                    "slices": slices,
                    "largest_slice_ns": largest.as_nanos(),
                }));
                match result {
                    RouteStageResult::Continue(mut job) => {
                        assert!(job.search_cursor.is_none());
                        job.prepare_continuation().unwrap();
                        job.identity = Some(stage_identity(index + 1));
                        job.gap_custody = Some(reservation);
                        next = Some(job);
                    }
                    RouteStageResult::Complete(result) => {
                        let RouteJobOutcome::Bank {
                            route,
                            final_route,
                            steps,
                        } = result.outcome
                        else {
                            panic!("{case} must execute the real bank branch, not Path or NoPath");
                        };
                        assert!(route.as_ref().unwrap().try_retained_bytes().unwrap() > 0);
                        assert!(final_route.try_retained_bytes().unwrap() > 0);
                        assert!(!steps.is_empty());
                        if case == "worn-blade" {
                            assert!(steps
                                .iter()
                                .any(|step| matches!(step, BankStep::Wear { id: 1277 })));
                        }
                        drop(reservation);
                        completed = true;
                        break;
                    }
                }
            }
            assert!(completed && saw_bank_access && saw_post_bank, "{case}");
            assert_eq!(ledger.used_bytes(), 0);
        }
        std::fs::write(
            output,
            serde_json::to_vec_pretty(&serde_json::json!({
                "navpack": pack,
                "node_budget": NODE_BUDGET,
                "job_limit_bytes": ROUTE_WORK_LIMIT_BYTES,
                "result_limit_bytes": 64 * 1024 * 1024,
                "cases": records,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
#[cfg(test)]
mod route_quest_provider_tests {
    use super::*;
    use api::quest_progress::{EvidenceProvider, EvidenceStamp, QuestListEvidenceProvider};
    use api::selected::{QuestGate, RunKey, Truth};
    use api::snapshot::{QuestStatusView, SnapshotView};
    use std::sync::LazyLock;

    fn selected_r289() -> Option<Arc<api::game_data::SelectedGameData>> {
        for (path, description) in [
            (
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../api/data/game-data/289.json"
                ),
                "revision 289 game data",
            ),
            (
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../api/data/game-data/289/quests.json"
                ),
                "revision 289 quest facts",
            ),
            (
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../api/data/game-data/289/prereq_steps.json"
                ),
                "revision 289 prerequisite facts",
            ),
        ] {
            api::test_support::local_asset_path("", path, description)?;
        }
        static SELECTED: LazyLock<Arc<api::game_data::SelectedGameData>> = LazyLock::new(|| {
            let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
                .expect("selected R289 game data");
            let worker_data = Arc::clone(&selected);
            let input =
                script::preparation::PreparationInput::new(1, 0, 8 * 1024 * 1024, 2 * 1024 * 1024)
                    .expect("quest preparation budget");
            script::preparation::submit(
                script::preparation::PreparationKind::Profile,
                Scope::Process,
                input,
                move |worker| {
                    worker_data.prepare_quests(worker)?;
                    worker_data.prepare_prereq_steps(worker)
                },
            )
            .expect("submit production quest preparation")
            .join()
            .expect("quest preparation worker")
            .expect("prepare selected quest family");
            selected
        });
        Some(Arc::clone(&SELECTED))
    }

    fn selected_quest(
        selected: &api::game_data::SelectedGameData,
    ) -> (api::selected::FactKey, String) {
        let catalog = selected.try_quests().expect("selected quest catalog");
        let facts = catalog
            .iter()
            .next()
            .expect("selected quest catalog has rows");
        (facts.id.clone(), facts.display.to_string())
    }

    fn run() -> RunKey {
        RunKey {
            slot: 1,
            run: 1,
            session: 1,
        }
    }

    fn stamp(run: RunKey, tick: u64, sequence: u64) -> EvidenceStamp {
        EvidenceStamp {
            run,
            tick,
            sequence,
        }
    }

    fn snapshot(name: &str, colour: i32, available: bool) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                component_id: 1,
                name: name.to_owned(),
                colour,
            }],
            available,
        );
        snapshot
    }

    fn real_list_provider(
        selected: &api::game_data::SelectedGameData,
        name: &str,
        colour: i32,
        available: bool,
        observed: EvidenceStamp,
    ) -> Arc<dyn EvidenceProvider> {
        let quests = selected.try_quests().expect("prepared quest catalog");
        Arc::new(QuestListEvidenceProvider::from_snapshot(
            &quests,
            SnapshotView::new(Some(&snapshot(name, colour, available)), observed),
        ))
    }

    fn context(
        selected: Arc<api::game_data::SelectedGameData>,
        stamp: EvidenceStamp,
    ) -> RouteQuestContext {
        let prereq_catalog = selected.try_prereq_steps();
        RouteQuestContext {
            selected,
            prereq_catalog,
            stamp,
            local_state: None,
        }
    }
    struct ColdStatePort;

    impl api::local_state::LocalStatePort for ColdStatePort {
        fn lease(
            &self,
            identity: &api::local_state::DomainIdentity,
        ) -> Result<api::local_state::DomainLease, api::local_state::ErrorCode> {
            Err(api::local_state::ErrorCode::NotReady {
                domain: identity.domain(),
            })
        }

        fn submit(&self, edit: api::local_state::DomainEdit) -> api::local_state::SubmitReceipt {
            api::local_state::SubmitReceipt::Refused(api::local_state::ErrorCode::NotReady {
                domain: edit.identity().domain(),
            })
        }

        fn observe(
            &self,
            _request: api::identity::RequestId,
        ) -> Option<api::local_state::WriteSettlement> {
            None
        }
    }

    impl script::local_state::StatePort for ColdStatePort {
        fn lease_version(
            &self,
            identity: &api::local_state::DomainIdentity,
        ) -> Result<api::local_state::StoreVersion, api::local_state::ErrorCode> {
            Err(api::local_state::ErrorCode::NotReady {
                domain: identity.domain(),
            })
        }
    }

    fn state_with_fact(fact: api::prereq_knowledge::Fact) -> Arc<script::local_state::SlotState> {
        let state = Arc::new(script::local_state::SlotState::new(
            Arc::new(ColdStatePort),
            api::local_state::AccountKey::try_new("test", "alice").expect("valid test account"),
            api::local_state::ContentScope::try_new(client::io::ClientRevision::R289, "test")
                .expect("valid test content"),
        ));
        assert!(state.record_prereq_fact(fact));
        state
    }

    #[test]
    fn compat_route_context_keeps_actual_session_after_scene_only_down_observation() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let mut slot = script::SlotScript::new();
        slot.bind_incarnation(44);
        let factory = Arc::new(script::surface::test_support::StubSurfaceFactory::new());
        slot.bind_surface_factory(factory.clone());
        slot.start_surface(script::surface::SurfaceStart {
            card: Arc::new(script::surface::test_support::StubSurfaceCard),
            settings_bag: None,
            game_data: Some(selected),
            named_banks: Arc::default(),
            loadouts: Arc::default(),
        })
        .expect("start the real compatibility surface owner");
        factory
            .last_run()
            .expect("stub surface run")
            .set_readiness(script::surface::SurfaceReady::Ready);
        assert_eq!(
            slot.poll_start(),
            script::StartPoll::Settled(script::StartOutcome::Ready)
        );
        slot.on_is_up(true);
        let before = route_quest_context(&slot, 1)
            .expect("active compatibility route context")
            .stamp
            .run;
        slot.on_is_up(false);
        slot.on_is_up(false);
        slot.on_is_up(true);
        assert_ne!(slot.work_epoch(), before.session);
        assert_eq!(
            route_quest_context(&slot, 2)
                .expect("same live owner")
                .stamp
                .run,
            before
        );
        slot.reset_session_work();
        assert_ne!(
            route_quest_context(&slot, 3).map(|context| context.stamp.run),
            Some(before),
            "an actual connection boundary must revoke the old route identity"
        );
        slot.stop();
    }

    #[test]
    fn idle_manual_route_uses_profile_data_and_same_run_memory() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let mut slot = script::SlotScript::new();
        slot.bind_incarnation(41);
        let run = RunKey {
            slot: 41,
            run: slot.runtime_generation(),
            session: slot.work_epoch(),
        };
        let quest = api::selected::FactKey::new("priestperil");
        let values = api::selected::InclusiveRange {
            min: Some(61),
            max: Some(61),
        };
        slot.bind_local_state(state_with_fact(api::prereq_knowledge::Fact {
            quest: quest.clone(),
            signal: quest.clone(),
            possible: values,
            after: values,
            refused: None,
            source: api::prereq_knowledge::FactSource::Page { row: 0, line: 0 },
            observed: stamp(run, 0, 0),
        }));
        assert!(slot.compiled_game_data().is_none());
        assert!(route_quest_context(&slot, 0).is_none());
        let snapshot = GameSnapshot::new();
        let (captured_run, catalog, route_state) =
            manual_route_state(&mut slot, &snapshot, &WorldState::empty(), Some(&selected))
                .expect("idle manual request uses the bound profile capability");
        assert_eq!(captured_run, run);
        assert!(catalog.is_some());
        assert!(route_state.establishable.is_some());
        assert_eq!(
            route_state.quest_evidence.unwrap().test(&QuestGate::Window(
                api::selected::StageWindow {
                    quest: quest.clone(),
                    signal: quest,
                    values,
                }
            )),
            Truth::True
        );
    }

    #[test]
    fn idle_manual_route_without_profile_data_stays_unavailable() {
        let mut slot = script::SlotScript::new();
        slot.bind_incarnation(42);
        assert!(
            manual_route_state(&mut slot, &GameSnapshot::new(), &WorldState::empty(), None,)
                .is_none()
        );
    }

    #[test]
    fn manual_route_refusal_survives_scene_reload_but_not_owner_boundaries() {
        use api::prereq_knowledge::{CatalogRef, Fact, FactSource, RefusalObservation};
        use api::prereq_steps::ActionableGate;

        let Some(selected) = selected_r289() else {
            return;
        };
        let catalog = selected.try_prereq_steps().expect("prepared prerequisites");
        let (row_index, row) = catalog
            .iter()
            .find(|(_, row)| {
                row.establishes.quest.0.as_ref() == "priestperil"
                    && row.establishes.values.min == Some(61)
                    && row.establishes.values.max == Some(61)
            })
            .expect("holy barrier prerequisite");
        let crossing = row.crossings[0].crossing;

        for boundary in ["reset", "reconnect", "stop"] {
            let mut slot = script::SlotScript::new();
            slot.bind_incarnation(43);
            let run = RunKey {
                slot: 43,
                run: slot.runtime_generation(),
                session: slot.work_epoch(),
            };
            let state = state_with_fact(Fact {
                quest: row.establishes.quest.clone(),
                signal: row.establishes.signal.clone(),
                possible: row.establishes.values,
                after: row.establishes.values,
                refused: None,
                source: FactSource::Page { row: 0, line: 0 },
                observed: stamp(run, 0, 0),
            });
            slot.bind_local_state(Arc::clone(&state));
            let snapshot = GameSnapshot::new();
            let (captured_run, _, _) =
                manual_route_state(&mut slot, &snapshot, &WorldState::empty(), Some(&selected))
                    .expect("capture idle manual route");
            assert_eq!(captured_run, run);

            slot.on_is_up(false);
            slot.on_is_up(false);
            slot.on_is_up(true);
            assert_ne!(slot.work_epoch(), run.session);
            let refusal = row.probe.refuse.first().expect("barrier refusal");
            let mut observation = RefusalObservation {
                crossing,
                leg: 7,
                gates: vec![ActionableGate {
                    gate: QuestGate::Window(row.establishes.clone()),
                    truth: Truth::True,
                    leg: 7,
                    crossing,
                }]
                .into(),
                matched: CatalogRef {
                    row: row_index,
                    entry: 0,
                },
                fact: Fact {
                    quest: row.establishes.quest.clone(),
                    signal: row.establishes.signal.clone(),
                    possible: refusal.proves,
                    after: row.after_for(refusal.proves).unwrap(),
                    refused: Some(row.establishes.values),
                    source: FactSource::Refusal {
                        row: row_index,
                        entry: 0,
                    },
                    observed: stamp(captured_run, 61, 12),
                },
            };
            assert!(
                slot.record_gate_refusal(captured_run, &observation),
                "a scene reload must not revoke the route's real refusal"
            );
            let before = state.prereq_snapshot();
            let (later_run, _, later_state) =
                manual_route_state(&mut slot, &snapshot, &WorldState::empty(), Some(&selected))
                    .expect("same manual route session");
            assert_eq!(later_run, captured_run);
            assert_eq!(
                later_state
                    .quest_evidence
                    .unwrap()
                    .test(&QuestGate::Window(row.establishes.clone())),
                Truth::False
            );

            match boundary {
                "reset" => slot.reset_session_work(),
                "reconnect" => {
                    slot.reconnect_session_work();
                }
                "stop" => slot.stop(),
                _ => unreachable!(),
            }
            observation.fact.observed.tick += 1;
            assert!(
                !slot.record_gate_refusal(captured_run, &observation),
                "{boundary} must revoke the captured route"
            );
            assert_eq!(state.prereq_snapshot(), before);
        }
    }

    #[test]
    fn observed_red_quest_rows_gate_windows_only_with_colour_authority() {
        use api::selected::{InclusiveRange, StageWindow};

        let Some(selected) = selected_r289() else {
            return;
        };

        let observed = stamp(run(), 126, 126);
        let context = context(Arc::clone(&selected), observed);
        let quests = selected.try_quests().expect("prepared quest catalog");
        // elena.rs2:118,126-132 is conservatively analysed without its outer
        // guard. Keep real authoritative positive and disjoint-window controls.
        for (quest, authoritative, min, window_truth) in [
            ("biohazard", false, None, Truth::Unknown),
            ("druidspirit", true, None, Truth::True),
            ("druidspirit", true, Some(1), Truth::False),
        ] {
            let facts = quests.quest(quest).expect("selected quest facts");
            assert_eq!(facts.colour_authoritative, authoritative, "{quest}");
            let snapshot = snapshot(&facts.display, 0xF80000, true);
            let evidence = captured_route_quests(Some(&context), &snapshot, None, observed)
                .expect("route captures the observed quest-list row");
            assert_eq!(
                (
                    evidence.test(&QuestGate::Complete(facts.id.clone())),
                    evidence.test(&QuestGate::Window(StageWindow {
                        quest: facts.id.clone(),
                        signal: facts.varp.clone(),
                        values: InclusiveRange { min, max: Some(9) },
                    })),
                ),
                (Truth::False, window_truth),
                "{quest}",
            );
        }
    }

    #[test]
    fn explicit_real_evidence_wins_when_known_and_unknown_falls_back() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let (quest, name) = selected_quest(&selected);

        let run = run();
        let floor = stamp(run, 9, 9);
        let complete = nav_planner::world_state::QUEST_COMPLETE_COLOUR;
        let not_started = 0xF80000;
        let gate = QuestGate::Complete(quest);
        let context = context(Arc::clone(&selected), floor);

        let explicit_false = real_list_provider(&selected, &name, not_started, true, floor);
        let ordinary_true = snapshot(&name, complete, true);
        let evidence =
            captured_route_quests(Some(&context), &ordinary_true, Some(explicit_false), floor)
                .expect("ordinary evidence and explicit provider");
        assert_eq!(
            evidence.test(&gate),
            Truth::False,
            "known explicit False must override ordinary Complete"
        );

        let explicit_true = real_list_provider(&selected, &name, complete, true, floor);
        let ordinary_false = snapshot(&name, not_started, true);
        let evidence =
            captured_route_quests(Some(&context), &ordinary_false, Some(explicit_true), floor)
                .expect("ordinary evidence and explicit provider");
        assert_eq!(
            evidence.test(&gate),
            Truth::True,
            "known explicit True must override ordinary NotStarted"
        );

        let explicit_unknown = real_list_provider(&selected, &name, complete, false, floor);
        let evidence = captured_route_quests(
            Some(&context),
            &ordinary_true,
            Some(explicit_unknown),
            floor,
        )
        .expect("ordinary evidence and explicit provider");
        assert_eq!(
            evidence.test(&gate),
            Truth::True,
            "Unknown explicit evidence must fall back to ordinary Complete"
        );
    }
    #[test]
    fn current_refusal_memory_overrides_ordinary_quest_colour_and_is_charged() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let (_, name) = selected_quest(&selected);

        let observed = stamp(run(), 9, 9);
        let gate = QuestGate::Window(api::selected::StageWindow {
            quest: api::selected::FactKey::new("unlisted-quest"),
            signal: api::selected::FactKey::new("unlisted-signal"),
            values: api::selected::InclusiveRange {
                min: Some(61),
                max: Some(61),
            },
        });
        let snapshot = snapshot(&name, nav_planner::world_state::QUEST_COMPLETE_COLOUR, true);
        let without_memory_context = context(Arc::clone(&selected), observed);
        let without_memory =
            captured_route_quests(Some(&without_memory_context), &snapshot, None, observed)
                .expect("ordinary route evidence");
        let without_memory_bytes = without_memory.owned_provider_bytes();
        let state = state_with_fact(api::prereq_knowledge::Fact {
            quest: api::selected::FactKey::new("unlisted-quest"),
            signal: api::selected::FactKey::new("unlisted-signal"),
            possible: api::selected::InclusiveRange {
                min: Some(60),
                max: Some(60),
            },
            after: api::selected::InclusiveRange {
                min: Some(60),
                max: Some(61),
            },
            refused: Some(api::selected::InclusiveRange {
                min: Some(61),
                max: Some(61),
            }),
            source: api::prereq_knowledge::FactSource::Crossing {
                loc_id: 3443,
                option: 1,
                passed: false,
            },
            observed,
        });
        let with_memory_context = RouteQuestContext {
            prereq_catalog: selected.try_prereq_steps(),
            selected,
            stamp: observed,
            local_state: Some(state),
        };
        let evidence = captured_route_quests(Some(&with_memory_context), &snapshot, None, observed)
            .expect("memory and ordinary route evidence");
        assert_eq!(evidence.test(&gate), Truth::False);
        assert_eq!(
            evidence.owned_provider_bytes() - without_memory_bytes,
            std::mem::size_of::<api::prereq_knowledge::Fact>()
        );
        let service_evidence =
            captured_service_quests(Some(&with_memory_context), &snapshot, Some(observed))
                .expect("service selection captures the same immutable memory");
        assert_eq!(service_evidence.test(&gate), Truth::False);
        assert_eq!(
            service_evidence.owned_provider_bytes(),
            evidence.owned_provider_bytes()
        );
    }

    #[test]
    fn captured_rows_fail_closed_after_the_run_or_causal_floor_changes() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let (quest, name) = selected_quest(&selected);

        let observed = stamp(run(), 5, 5);
        let complete = snapshot(&name, nav_planner::world_state::QUEST_COMPLETE_COLOUR, true);
        let gate = QuestGate::Complete(quest);

        let later_floor = stamp(run(), 6, 6);
        let context = context(Arc::clone(&selected), observed);
        let evidence = captured_route_quests(Some(&context), &complete, None, later_floor)
            .expect("prepared ordinary provider");
        assert_eq!(evidence.test(&gate), Truth::Unknown);

        let changed_run = RunKey { run: 2, ..run() };
        let changed_run_floor = stamp(changed_run, 5, 5);
        let evidence = captured_route_quests(Some(&context), &complete, None, changed_run_floor)
            .expect("prepared ordinary provider");
        assert_eq!(evidence.test(&gate), Truth::Unknown);
    }

    fn tiny_world() -> Arc<NavWorld> {
        let origin = WorldTile {
            x: 0,
            z: 0,
            level: 0,
        };
        Arc::new(NavWorld::from_parts(
            nav_planner::collision::WorldCollision {
                origin,
                width: 4,
                height: 4,
                walk: vec![0; 16],
                blocked: vec![0; 1],
                flags: None,
            },
            nav_planner::transport::TransportGraph::default(),
            Vec::new(),
        ))
    }

    fn request(world: Arc<NavWorld>) -> ScriptRouteRequest {
        let (completion, _receiver) = RouteCompletion::channel();
        let origin = world.collision.origin;
        ScriptRouteRequest {
            candidate_goals: None,
            generation: 1,
            request_id: 1,
            world,
            from: origin,
            to: WorldTile {
                x: origin.x + 1,
                z: origin.z + 1,
                level: origin.level,
            },
            radius: 0,
            loc_id: None,
            arrival: ArrivalKind::Reach,
            opts: FindOptions::default(),
            state: None,
            admission: crate::admission::Admission::manual(
                FindOptions::default(),
                Default::default(),
                1,
                crate::WalkGlobals::default(),
            ),
            bank_origin: Origin::Unknown,
            bank: Vec::new(),
            live_geometry: None,
            exclusions: None,
            compat_tile_intent: false,
            prereq_catalog: None,
            completion,
        }
    }

    #[test]
    fn ordinary_provider_bytes_are_charged_to_route_input_and_output() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let (quest, name) = selected_quest(&selected);

        let stamp = stamp(run(), 12, 12);
        let context = context(Arc::clone(&selected), stamp);
        let snapshot = snapshot(&name, nav_planner::world_state::QUEST_COMPLETE_COLOUR, true);
        let evidence = captured_route_quests(Some(&context), &snapshot, None, stamp)
            .expect("real ordinary quest-list evidence");
        assert_eq!(evidence.test(&QuestGate::Complete(quest)), Truth::True);
        let owned_bytes = evidence.owned_provider_bytes();
        assert!(
            owned_bytes > 0,
            "the provider owns its retained status table"
        );

        let world = tiny_world();
        let request = request(Arc::clone(&world));
        let lease = nav_lease(&world).expect("navigation lease");
        let without_evidence = Arc::new(WorldState::default());
        let with_evidence = Arc::new(WorldState::default().with_quest_evidence(evidence));
        let base_state_bytes = without_evidence
            .try_shared_retained_bytes()
            .expect("baseline state byte bound");
        let evidence_state_bytes = with_evidence
            .try_shared_retained_bytes()
            .expect("evidence state byte bound");
        assert_eq!(evidence_state_bytes - base_state_bytes, owned_bytes);

        let (baseline, _) = RouteSubmission::prepare(&request, &without_evidence, &lease, 0, 0, 0)
            .expect("baseline route budget");
        let (charged, _) = RouteSubmission::prepare(&request, &with_evidence, &lease, 0, 0, 0)
            .expect("evidence route budget");
        assert_eq!(charged.input_bytes - baseline.input_bytes, owned_bytes);
        assert_eq!(charged.result_bytes - baseline.result_bytes, owned_bytes);
    }

    fn compat_bot(selected: Arc<api::game_data::SelectedGameData>, request_id: u64) -> NavBot {
        let target = WorldTile {
            x: 3423,
            z: 3485,
            level: 0,
        };
        let exclusions = ScriptRouteExclusions {
            avoid_wire: vec![
                script::interact::InspectAvoidWire::Rect {
                    min_x: 3400,
                    max_x: 3401,
                    min_z: 3480,
                    max_z: 3481,
                    level: Some(0),
                },
                script::interact::InspectAvoidWire::Catalog("named-avoid".to_owned()),
            ],
            cross: vec![Arc::from("named-cross")],
            ..Default::default()
        };
        let mut bot = NavBot {
            quest_context: Some(context(selected, stamp(run(), 100, 100))),
            route_generation: 7,
            walk_request_id: request_id,
            requested_route: Some((target, 3, false, false, true, ZoneExempt::NONE)),
            requested_exclusions: Some(Arc::new(exclusions)),
            admission: Some(Box::new(crate::admission::Admission::manual(
                FindOptions {
                    allow_bank_fetch: true,
                    ..Default::default()
                },
                Default::default(),
                7,
                crate::WalkGlobals::default(),
            ))),
            ..Default::default()
        };
        assert!(bot.retain_compat_walk(
            request_id,
            script::native::WalkOptions {
                allow_quest_start: script::native::WalkBit::Forbid,
                ..Default::default()
            },
            script::native::PrereqAllow {
                dialogue: true,
                quest_start: false,
            },
        ));
        bot
    }

    fn compat_gates(
        selected: &api::game_data::SelectedGameData,
    ) -> (
        Arc<api::prereq_steps::PrereqCatalog>,
        Arc<[api::prereq_steps::ActionableGate]>,
    ) {
        let catalog = selected.try_prereq_steps().expect("prepared prerequisites");
        let (_, row) = catalog
            .iter()
            .find(|(_, row)| row.id.contains("drezel-advice"))
            .expect("released A1 row");
        let gates = Arc::from([api::prereq_steps::ActionableGate {
            gate: QuestGate::Window(row.establishes.clone()),
            truth: Truth::False,
            leg: 0,
            crossing: row.crossings[0].crossing,
        }]);
        (catalog, gates)
    }

    #[test]
    fn journal_prerequisite_metadata_does_not_invent_crossing_messages() {
        use api::prereq_steps::{ActionableGate, CrossingId, CrossingKind};
        use api::selected::{FactKey, InclusiveRange, StageWindow};
        let Some(selected) = selected_r289() else {
            return;
        };
        let catalog = selected.try_prereq_steps().expect("prepared prerequisites");
        let gate = ActionableGate {
            gate: QuestGate::Window(StageWindow {
                quest: FactKey::new("blackarmgang"),
                signal: FactKey::new("phoenixgang"),
                values: InclusiveRange {
                    min: Some(10),
                    max: Some(10),
                },
            }),
            truth: Truth::Unknown,
            leg: 0,
            crossing: CrossingId {
                loc_id: 2628,
                option: 1,
                at: WorldTile {
                    x: 2788,
                    z: 3190,
                    level: 0,
                },
                takeoff: WorldTile {
                    x: 2788,
                    z: 3190,
                    level: 0,
                },
                to: WorldTile {
                    x: 2788,
                    z: 3189,
                    level: 0,
                },
                kind: CrossingKind::Door,
            },
        };
        let content = route_prereq_content(&catalog, run(), Arc::from([gate.clone()]))
            .expect("an unknown journal-backed gate must reach the native prerequisite owner");
        assert_eq!(content.gates.as_ref(), &[gate]);
        assert_eq!(content.gate_groups.len(), 1);
        assert!(content.crossing_passes.is_empty());
        assert!(content.refusal_texts.is_empty());
        assert!(content.refusal_candidates.is_empty());
    }

    #[test]
    fn registered_refusal_content_survives_preceding_proved_ladder_gates() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let (catalog, gates) = compat_gates(&selected);
        let mut barrier = gates[0].clone();
        barrier.truth = Truth::True;
        barrier.leg = 7;
        let mut route_gates = Vec::new();
        for (leg, minimum, loc_id) in [(3, 5, 3444), (5, 8, 3445)] {
            let mut ladder = barrier.clone();
            let QuestGate::Window(window) = &mut ladder.gate else {
                panic!("released A1 stage window");
            };
            window.values = api::selected::InclusiveRange {
                min: Some(minimum),
                max: Some(61),
            };
            ladder.leg = leg;
            ladder.crossing.loc_id = loc_id;
            ladder.crossing.kind = api::prereq_steps::CrossingKind::Ladder;
            assert!(catalog.for_gate(&ladder.gate, &ladder.crossing).is_none());
            assert!(catalog.for_journal(&ladder.gate).is_some());
            route_gates.push(ladder);
        }
        route_gates.push(barrier.clone());
        let route_gates: Arc<[_]> = route_gates.into();
        let content = route_prereq_content(&catalog, run(), Arc::clone(&route_gates))
            .expect("proved ladders must not discard the registered barrier's refusal content");
        assert_eq!(content.gates.as_ref(), route_gates.as_ref());
        assert_eq!(content.gate_groups.len(), 3);
        assert!(content
            .gate_groups
            .iter()
            .map(|group| group.leg)
            .eq([3, 5, 7]));
        assert_eq!(content.crossing_passes.len(), 1);
        assert_eq!(content.crossing_passes[0].crossing, barrier.crossing);
        assert!(!content.refusal_texts.is_empty());
        assert!(content
            .refusal_texts
            .iter()
            .all(|(crossing, _)| *crossing == barrier.crossing));
    }

    #[test]
    fn unproved_unregistered_crossings_do_not_get_partial_repair_content() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let (catalog, gates) = compat_gates(&selected);
        for truth in [Truth::False, Truth::Unknown] {
            let mut unregistered = gates[0].clone();
            unregistered.truth = truth;
            unregistered.crossing.loc_id = 3444;
            unregistered.crossing.kind = api::prereq_steps::CrossingKind::Ladder;
            assert!(catalog
                .for_gate(&unregistered.gate, &unregistered.crossing)
                .is_none());
            assert!(route_prereq_content(
                &catalog,
                run(),
                Arc::from([unregistered, gates[0].clone()]),
            )
            .is_none());
        }
    }

    fn fail_compat(bot: &mut NavBot, end: script::native::WalkEnd) {
        let (target, radius, teleports, ..) = bot.requested_route.expect("direct route key");
        bot.note_failure(
            bot.route_generation,
            bot.walk_request_id,
            target,
            radius,
            teleports,
        );
        bot.native_walk_failure = Some((bot.walk_request_id, end));
    }

    #[test]
    fn compat_handover_moves_original_policy_and_preserves_rectangular_exclusions() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let mut bot = compat_bot(Arc::clone(&selected), 11);
        let (_, gates) = compat_gates(&selected);
        fail_compat(
            &mut bot,
            script::native::WalkEnd::NeedsStep(Arc::clone(&gates)),
        );
        let handover = bot
            .take_compat_walk_handover(WorldTile {
                x: 3408,
                z: 3489,
                level: 0,
            })
            .expect("owned no-path handover");
        assert_eq!(
            handover.outer.identity,
            script::native::OuterIdentity::Compat { request_id: 11 }
        );
        assert_eq!(handover.outer.radius, 3);
        assert_eq!(
            handover.outer.options.allow_teleports,
            script::native::WalkBit::Forbid
        );
        assert_eq!(
            handover.outer.options.allow_wilderness,
            script::native::WalkBit::Forbid
        );
        assert_eq!(
            handover.outer.options.allow_quest_start,
            script::native::WalkBit::Forbid
        );
        assert!(handover.outer.allow_bank_fetch);
        assert!(!handover.outer.prereq_allow.quest_start);
        let script::native::HandoverEntry::NeedsStep(found) = handover.entry else {
            panic!("no-path entry");
        };
        assert!(Arc::ptr_eq(&found, &gates));
        assert_eq!(bot.walk_request_id, 0);
        assert_eq!(
            bot.walk_outcome_request_id, 0,
            "direct failure is consumed, not posted"
        );
        assert!(bot.ordinary_movement_owned(0));
        let (options, frozen) = bot.compat_walk_native_policy(run()).expect("child policy");
        assert!(!options.allow_teleports && !options.allow_wilderness);
        assert!(options.allow_bank_fetch);
        let exclusions = native_route_exclusions(
            &handover.outer.avoid,
            &handover.outer.cross,
            frozen.as_deref(),
        );
        assert!(
            exclusions.avoid.is_empty(),
            "derived rectangles must not accumulate"
        );
        assert_eq!(exclusions.avoid_wire.len(), 2);
        assert!(matches!(
            exclusions.avoid_wire[0],
            script::interact::InspectAvoidWire::Rect { .. }
        ));
        assert_eq!(exclusions.cross.len(), 1);
        assert!(bot.take_compat_walk_handover(handover.here).is_none());
    }

    #[test]
    fn compat_refusal_keeps_the_acknowledged_page_stamp_and_same_observation() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let mut bot = compat_bot(Arc::clone(&selected), 12);
        let (catalog, gates) = compat_gates(&selected);
        let content = route_prereq_content(&catalog, run(), Arc::clone(&gates))
            .expect("canonical route content");
        let observation = content
            .refusal_observation(
                0,
                gates[0].crossing,
                &[gates[0].gate.clone()],
                nav_executor::RefusalMatch {
                    text: 0,
                    tick: 17,
                    sequence: 21,
                },
            )
            .expect("content-witnessed refusal");
        assert_eq!(observation.fact.observed, stamp(run(), 17, 21));
        fail_compat(
            &mut bot,
            script::native::WalkEnd::GateRefused(Arc::clone(&observation)),
        );
        assert!(Arc::ptr_eq(
            bot.compat_walk_refusal().expect("record first"),
            &observation
        ));
        let handover = bot
            .take_compat_walk_handover(WorldTile {
                x: 3408,
                z: 3489,
                level: 0,
            })
            .expect("refusal handover");
        let script::native::HandoverEntry::GateRefused(found) = handover.entry else {
            panic!("refusal entry");
        };
        assert!(Arc::ptr_eq(&found, &observation));
        assert_eq!(found.fact.observed, stamp(run(), 17, 21));
    }

    #[test]
    fn compat_handover_rejects_a_refusal_from_each_stale_run_component() {
        let Some(selected) = selected_r289() else {
            return;
        };
        for stale in [
            RunKey { slot: 2, ..run() },
            RunKey { run: 2, ..run() },
            RunKey {
                session: 2,
                ..run()
            },
        ] {
            let mut bot = compat_bot(Arc::clone(&selected), 16);
            let (catalog, gates) = compat_gates(&selected);
            let content = route_prereq_content(&catalog, stale, Arc::clone(&gates))
                .expect("canonical route content");
            let observation = content
                .refusal_observation(
                    0,
                    gates[0].crossing,
                    &[gates[0].gate.clone()],
                    nav_executor::RefusalMatch {
                        text: 0,
                        tick: 17,
                        sequence: 21,
                    },
                )
                .expect("content-witnessed refusal");
            fail_compat(&mut bot, script::native::WalkEnd::GateRefused(observation));
            assert!(bot
                .take_compat_walk_handover(WorldTile {
                    x: 3408,
                    z: 3489,
                    level: 0
                })
                .is_none());
            assert_eq!(bot.compat_walk_request_id(), Some(16));
            assert!(!bot.compat_walk_handed_over());
        }
    }

    #[test]
    fn compat_handover_and_native_children_reject_each_stale_run_component() {
        let Some(selected) = selected_r289() else {
            return;
        };
        for stale in [
            RunKey { slot: 2, ..run() },
            RunKey { run: 2, ..run() },
            RunKey {
                session: 2,
                ..run()
            },
        ] {
            let mut bot = compat_bot(Arc::clone(&selected), 13);
            let (_, gates) = compat_gates(&selected);
            fail_compat(&mut bot, script::native::WalkEnd::NeedsStep(gates));
            bot.quest_context.as_mut().expect("context").stamp.run = stale;
            assert!(!bot.compat_walk_context_current());
            assert!(bot
                .take_compat_walk_handover(WorldTile {
                    x: 3408,
                    z: 3489,
                    level: 0
                })
                .is_none());
            assert!(bot.compat_walk_native_policy(run()).is_none());
            assert_eq!(
                bot.compat_walk_request_id(),
                Some(13),
                "stale take does not consume owner"
            );
        }
    }

    #[test]
    fn compat_completion_reports_only_the_outer_id_and_only_once() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let mut bot = compat_bot(Arc::clone(&selected), 14);
        let (_, gates) = compat_gates(&selected);
        fail_compat(&mut bot, script::native::WalkEnd::NeedsStep(gates));
        let handover = bot
            .take_compat_walk_handover(WorldTile {
                x: 3408,
                z: 3489,
                level: 0,
            })
            .expect("handover");
        bot.walk_request_id = 91;
        assert!(!bot.complete_compat_walk(13, script::native::WalkEnd::Cancelled, None));
        assert!(bot.complete_compat_walk(14, script::native::WalkEnd::Arrived, None));
        assert_eq!(bot.walk_outcome_request_id, 14);
        assert_eq!(
            (bot.walk_outcome_x, bot.walk_outcome_z),
            (handover.outer.target.x, handover.outer.target.z)
        );
        assert_eq!(bot.walk_outcome_radius, 3);
        assert!(!bot.walk_outcome_failed);
        assert!(!bot.script_walk_armed());
        let sequence = bot.walk_outcome_seq;
        assert!(!bot.complete_compat_walk(14, script::native::WalkEnd::Arrived, None));
        assert_eq!(bot.walk_outcome_seq, sequence);
    }

    #[test]
    fn rejected_compat_handover_keeps_outer_terminal_for_seat_observer() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let mut bot = compat_bot(Arc::clone(&selected), 17);
        let (_, gates) = compat_gates(&selected);
        fail_compat(&mut bot, script::native::WalkEnd::NeedsStep(gates));
        let mut slot = script::SlotScript::new();
        assert!(!super::super::script_walk::observe_compat_walk(
            &mut slot,
            &mut bot,
            Some(WorldTile {
                x: 3408,
                z: 3489,
                level: 0,
            }),
            "rejected-compat-test",
        ));
        assert_eq!(bot.compat_walk_request_id(), None);
        assert_eq!(bot.walk_outcome_request_id, 17);
        assert!(bot.walk_outcome_failed);
        let sequence = bot.walk_outcome_seq;
        assert!(!super::super::script_walk::observe_compat_walk(
            &mut slot,
            &mut bot,
            None,
            "rejected-compat-test",
        ));
        assert_eq!(bot.walk_outcome_seq, sequence);
    }

    #[test]
    fn compat_reissue_preserves_a_false_bank_fetch_policy() {
        let Some(selected) = selected_r289() else {
            return;
        };
        let mut bot = compat_bot(Arc::clone(&selected), 15);
        let walk = bot.compat_walk.as_mut().expect("retained owner");
        walk.key.4 = false;
        walk.outer.as_mut().expect("original walk").allow_bank_fetch = false;
        let (_, gates) = compat_gates(&selected);
        fail_compat(&mut bot, script::native::WalkEnd::NeedsStep(gates));
        let handover = bot
            .take_compat_walk_handover(WorldTile {
                x: 3408,
                z: 3489,
                level: 0,
            })
            .expect("handover");
        assert!(!handover.outer.allow_bank_fetch);
        assert!(
            !bot.compat_walk_native_policy(run())
                .expect("child policy")
                .0
                .allow_bank_fetch
        );
    }

    #[test]
    fn native_walk_exclusions_keep_request_avoid_names_and_deduplicate_crosses() {
        let avoid = [Arc::from("named-avoid")];
        let cross = [Arc::from("named-cross"), Arc::from("named-cross")];
        let exclusions = native_route_exclusions(&avoid, &cross, None);
        assert_eq!(
            exclusions.avoid_wire,
            vec![script::interact::InspectAvoidWire::Catalog(
                "named-avoid".to_owned()
            )],
        );
        assert_eq!(exclusions.cross, vec![Arc::<str>::from("named-cross")]);
    }

    #[test]
    fn compat_owner_idle_storage_is_one_pointer_and_live_metadata_is_bounded() {
        assert_eq!(
            std::mem::size_of::<Option<Box<CompatWalk>>>(),
            std::mem::size_of::<usize>(),
        );
        assert!(std::mem::size_of::<CompatWalk>() <= 384);
        println!(
            "prereq host layout NavBot={} CompatWalk={} idle_compat={}",
            std::mem::size_of::<NavBot>(),
            std::mem::size_of::<CompatWalk>(),
            std::mem::size_of::<Option<Box<CompatWalk>>>(),
        );
    }
}
