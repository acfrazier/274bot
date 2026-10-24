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
    MissingReqView, MissingRequirements, Route, SearchCursor, SearchOutcome, SearchPreflight,
    SearchRequest, SearchRequestShape, NODE_BUDGET,
};
use nav_planner::world::NavWorld;
use nav_planner::zones::{ZoneExempt, ZoneFilterOwned, ZoneKey};
use nav_planner::WorldState;
use script::work_admission::{cancellation::WorkCancellation, WorkService};

use crate::walk_plan::{PendingBankFetch, RouteOutcome, RoutePath};

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
            Self::Preparing { .. } => None,
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

#[derive(Clone, Copy)]
struct RouteSubmission {
    revision: InputRevision,
    input_bytes: usize,
    scratch_bytes: usize,
    result_bytes: usize,
    assessment_future_bytes: usize,
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
            entrance,
        };

        let plan_item_rows = preflights
            .missing
            .reservation()
            .max_plan_item_rows
            .max(preflights.missing_route.reservation().max_plan_item_rows);
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
        let bank_plan_bytes =
            BankFetchPlan::heap_bound(plan_item_rows).ok_or(RouteJobError::Budget)?;
        let planner_bytes = PlanningRowsCursor::heap_bound(bank_rows, plan_item_rows)
            .ok_or(RouteJobError::Budget)?;
        let planned_rows_bytes = PlanningRowsCursor::rows_heap_bound(bank_rows, plan_item_rows)
            .ok_or(RouteJobError::Budget)?;
        let planned_rows = bank_rows
            .checked_add(plan_item_rows)
            .ok_or(RouteJobError::Budget)?;
        let bank_direct_bytes = BankFetchCursor::heap_bound(bank_rows, plan_item_rows, stand_rows)
            .ok_or(RouteJobError::Budget)?;
        let bank_planned_bytes =
            BankFetchCursor::heap_bound(planned_rows, plan_item_rows, stand_rows)
                .ok_or(RouteJobError::Budget)?;
        let has_stands = !request.world.banks().is_empty();
        let fetchable_state_bytes =
            FetchableStateCursor::heap_bound(state, planned_rows, has_stands)
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
            &preflights.single,
            &preflights.post_bank_single,
            &preflights.first,
            &preflights.missing,
            &preflights.missing_route,
            &preflights.blocking,
            &preflights.unresolved,
            &preflights.entrance,
        ] {
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
                            bytes.checked_add(preflights.first.reservation().future_layout_bytes)
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
            .checked_mul(std::mem::size_of::<script::shim::InspectAvoidWire>())?,
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
        "native walk route failed: {:?} arrival from {:?} to {:?} within radius {}",
        request.arrival, request.from, request.to, request.radius
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
    let detail_len = blocked_with_context.checked_add(if supply_bytes == 0 {
        0
    } else {
        supply_bytes.checked_add(2)?
    })?;
    let detail_len = detail_len.max("candidate_goal_tier=preferred".len());
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
        let state = Arc::new(request.state.take().unwrap_or_else(WorldState::empty));
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
        let max_settles = NODE_BUDGET;
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

    fn begin_diagnostics(&mut self) {
        self.outcome = Some(RouteJobOutcome::NoPath);
        self.diagnostics_started = true;
        self.stage = RouteJobStage::BeginDiagnostics;
    }

    fn handle_search(&mut self, search: SearchOutcome) -> Result<(), RouteJobError> {
        match (self.search_purpose, search) {
            (RouteSearchPurpose::Strict, SearchOutcome::Single { route }) => {
                if let Ok(route) = route {
                    self.outcome = Some(RouteJobOutcome::Route(route));
                    self.stage = RouteJobStage::Assess;
                } else if self.request.opts.allow_bank_fetch {
                    self.schedule_search(RouteSearchPurpose::MissingTarget);
                } else {
                    self.finish_strict_searches();
                }
            }
            (RouteSearchPurpose::StrictFirst, SearchOutcome::First { search }) => {
                let (preferred, fallback) = search.into_routes();
                if let Ok(route) = preferred {
                    self.outcome = Some(RouteJobOutcome::Route(route));
                    self.stage = RouteJobStage::Assess;
                } else {
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
                    if let Some(CompactFallbackRoute::Routed(route)) = fallback {
                        self.strict_fallback = Some(route);
                    }
                    self.begin_fetchable_first()?;
                }
            }
            (RouteSearchPurpose::CandidateFallback, SearchOutcome::First { search }) => {
                self.strict_fallback = search.into_routes().0.ok();
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
                if self.kind.as_ref().is_some_and(|kind| {
                    matches!(kind, RouteDemandKind::Walk { authority: Some(_) })
                }) {
                    if self.blocked.is_empty() {
                        self.prepare_goal_copy(
                            RouteGoalCopy::Diagnostics,
                            RouteSearchPurpose::Unresolved,
                        )?;
                    } else {
                        self.native_failure = Some(script::native::WalkEnd::Refused);
                        self.schedule_search(RouteSearchPurpose::MissingCarry);
                    }
                } else {
                    self.schedule_search(RouteSearchPurpose::MissingCarry);
                }
            }
            (RouteSearchPurpose::Unresolved, SearchOutcome::UnresolvedQuestGates(result)) => {
                self.native_failure = Some(match result {
                    Ok(Some(gates)) if !gates.is_empty() => {
                        script::native::WalkEnd::NeedsEvidence(Arc::from(gates))
                    }
                    Ok(_) => script::native::WalkEnd::Failed,
                    Err(_) => script::native::WalkEnd::Refused,
                });
                self.schedule_search(RouteSearchPurpose::MissingCarry);
            }
            (RouteSearchPurpose::MissingCarry, SearchOutcome::MissingItemReqs(missing)) => {
                self.missing_requirements = missing;
                self.missing_diag_index = 0;
                self.prepare_goal_copy(RouteGoalCopy::Diagnostics, RouteSearchPurpose::Entrance)?;
            }
            (RouteSearchPurpose::Entrance, SearchOutcome::EntranceRequirements(requirement)) => {
                if let Some(requirement) = requirement {
                    self.native_failure = Some(script::native::WalkEnd::Blocked);
                    self.detail = Some(requirement.to_string());
                }
                self.stage = RouteJobStage::Diagnostics;
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

    fn finish(&mut self) -> RouteJobOutput {
        let outcome = self.outcome.take().unwrap_or(RouteJobOutcome::NoPath);
        let mut detail = self.detail.take();
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
                    "native walk route failed: {:?} arrival from {:?} to {:?} within radius {}",
                    self.request.arrival, self.request.from, self.request.to, self.request.radius
                );
                route_detail = Some(match route_detail {
                    Some(detail) => format!("{detail}; {context}"),
                    None => context,
                });
            }
            if !self.missing.is_empty() {
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
            outcome,
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
                        let missing_capacity = self.plan_item_limit();
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
                        let missing_capacity = self.plan_item_limit();
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
                    if let Err(error) = self
                        .prepare_goal_copy(RouteGoalCopy::Diagnostics, RouteSearchPurpose::Blocking)
                    {
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
                        CursorStep::Yielded | CursorStep::Cancelled(_) => {
                            return StepOutcome::Yielded;
                        }
                        CursorStep::Failed(_) => return StepOutcome::Failed(RouteJobError::Search),
                        CursorStep::Complete(completion) => {
                            #[cfg(test)]
                            route_work_tests::record_search_event(
                                &self.thread_name,
                                self.search_purpose,
                                false,
                            );
                            self.search_cursor = None;
                            let (search, _state) = completion.into_parts();
                            if let Err(error) = self.handle_search(search) {
                                return StepOutcome::Failed(error);
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
    outcome: RouteJobOutcome,
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
    let retained = match &output.outcome {
        RouteJobOutcome::NoPath => return Ok(None),
        RouteJobOutcome::Route(route) => nav_planner::producer::compact_route_storage_bytes(route)
            .ok_or(nav_planner::producer::OwnerResultRefusal::SizeOverflow)?
            .checked_add(std::mem::size_of::<RoutePath>())
            .and_then(|bytes| bytes.checked_add(custody_bytes))
            .and_then(|bytes| bytes.checked_add(basis_bytes))
            .and_then(|bytes| bytes.checked_add(route_basis_layout))
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
    let detail = refusal.map_or_else(|| error.to_string(), |refusal| refusal.to_string());
    publish_accepted_route_refusal(navs, name, generation, request_id, kind, &detail, refusal);
}

fn make_compact_route_path(
    route: CompactRoute,
    lease: &NavLease,
    identity: WorkIdentity,
    revision: InputRevision,
    reservation: &OwnerResultReservation,
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
        RoutePath::compact(plan, lease.clone(), Arc::clone(&custody_any)),
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
                if bot.advisory_request_id == authority.request_id().get() && authority.live() {
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
                    );
                    let (path, custody) = match route {
                        Some(route) => make_compact_route_path(
                            route,
                            &lease,
                            output.identity,
                            output.revision,
                            reservation,
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
    pub(crate) cancel_reason: script::isolate_fb::WalkCancelReason,
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
    pub(crate) avoid_wire: Vec<script::shim::InspectAvoidWire>,
    pub(crate) cross: Vec<Arc<str>>,
}
impl ScriptRouteExclusions {
    fn is_empty(&self) -> bool {
        self.avoid.is_empty() && self.avoid_wire.is_empty() && self.cross.is_empty()
    }
}

#[derive(Clone)]
pub(crate) struct RouteQuestContext {
    selected: Arc<api::game_data::SelectedGameData>,
    stamp: EvidenceStamp,
}

/// Retain only immutable catalog ownership and the observation's actual
/// causal stamp. Status rows are copied only when a route is requested.
pub(crate) fn route_quest_context(
    slot: &script::SlotScript,
    tick: u64,
) -> Option<RouteQuestContext> {
    let api::identity::Scope::Slot(key) = slot.work_scope()? else {
        return None;
    };
    let run = slot.native_run().unwrap_or(api::selected::RunKey {
        slot: key.incarnation().get(),
        run: slot.runtime_generation(),
        session: slot.work_epoch(),
    });
    Some(RouteQuestContext {
        selected: slot.compiled_game_data()?,
        stamp: EvidenceStamp {
            run,
            tick,
            sequence: tick,
        },
    })
}

struct RouteQuestProvider {
    explicit: Option<Arc<dyn api::quest_progress::EvidenceProvider>>,
    ordinary: api::quest_progress::QuestListEvidenceProvider,
}

impl api::quest_progress::EvidenceProvider for RouteQuestProvider {
    fn test_gate(
        &self,
        gate: &api::selected::QuestGate,
        required_after: EvidenceStamp,
    ) -> api::selected::Truth {
        if let Some(explicit) = self.explicit.as_ref() {
            let truth = explicit.test_gate(gate, required_after);
            if truth != api::selected::Truth::Unknown {
                return truth;
            }
        }
        self.ordinary.test_gate(gate, required_after)
    }
}

fn captured_route_quests(
    family: nav_planner::quest_gates::QuestFamilyId,
    context: Option<&RouteQuestContext>,
    snapshot: &GameSnapshot,
    explicit: Option<Arc<dyn api::quest_progress::EvidenceProvider>>,
    required_after: EvidenceStamp,
) -> Option<nav_planner::quest_gates::QuestEvidence> {
    let prepared = context.and_then(|context| {
        let identity = context.selected.quest_family_identity()?;
        if nav_planner::quest_gates::QuestFamilyId::new(identity.sha256, identity.schema) != family
        {
            return None;
        }
        let quests = context.selected.try_quests()?;
        Some(
            api::quest_progress::QuestListEvidenceProvider::from_snapshot(
                &quests,
                Some(identity),
                Some(identity),
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
        let bytes = layout
            .size()
            .checked_add(ordinary.retained_status_bytes())?;
        let provider = Arc::new(RouteQuestProvider { explicit, ordinary });
        Some(
            nav_planner::quest_gates::QuestEvidence::new(provider, family, required_after)
                .with_owned_provider_bytes(bytes),
        )
    } else {
        explicit.map(|provider| {
            nav_planner::quest_gates::QuestEvidence::new(provider, family, required_after)
        })
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
    pub(crate) walk_outcome_cancel_reason: script::isolate_fb::WalkCancelReason,
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
    request: script::shim::InteractReq,
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
    pub(crate) state: Option<WorldState>,
    /// The account's bank memory rows at arm time
    /// ([`super::slot_bank_memory::planner_rows`]): what a BankBudget
    /// session is planned over ([`nav_planner::bank_fetch::planning_rows`]).
    pub(crate) bank: BankRows,
}

fn walk_arm_outcome_tag(outcome: &RouteOutcome) -> &'static str {
    match outcome {
        RouteOutcome::Routed(_) => "Routed",
        RouteOutcome::BankSession { .. } => "BankSession",
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
pub(crate) fn route_world_state(snapshot: &GameSnapshot, armed: Option<WorldState>) -> WorldState {
    match armed {
        Some(state) => state,
        None => WorldState::from_snapshot(snapshot),
    }
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
            script::shim::InspectAvoidWire::Rect {
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
            script::shim::InspectAvoidWire::Catalog(id) => {
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
            script::shim::InspectAvoidWire::Unsupported => {
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
    ) {
        if !authority.live() {
            return;
        }
        let refusal = |detail: &str| {
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
        self.state = Some(route_world_state(snapshot, self.state.take()));
        let context = self
            .navs
            .lock()
            .unwrap()
            .get(&self.name)
            .and_then(|bot| bot.quest_context.clone());
        self.state
            .as_mut()
            .expect("request state was captured")
            .quest_evidence = world.graph.quest_family.and_then(|family| {
            captured_route_quests(
                family,
                context.as_ref(),
                snapshot,
                request.evidence,
                request.required_after,
            )
        });
        let from = WorldTile { x, z, level };
        let radius = i32::from(request.radius);
        let (options, policy, enforce) = super::walk_permissions::native_admission(
            &self.navs,
            &self.name,
            request.options,
            session_permissions,
        );
        let empty = WorldState::empty();
        let state = self.state.as_ref().unwrap_or(&empty);
        let (options, exclusions) = match resolve_route_exclusions(
            options,
            world,
            from,
            request.target,
            state,
            ScriptRouteExclusions {
                cross: request.cross.to_vec(),
                ..Default::default()
            },
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
            compat_tile_intent: false,
            bank_origin: self.bank.origin,
            bank: self.bank.rows,
            live_geometry,
            exclusions: (!exclusions.is_empty()).then(|| Arc::new(exclusions)),
            completion: RouteCompletion::default(),
        };
        let request_id = request.request_id;
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
        if let Some(displaced) = displaced {
            (*displaced).signal_completion();
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
        let guard = if protect || request.food_guard {
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
        let (options, policy, enforce) = super::walk_permissions::native_admission(
            &self.navs,
            &self.name,
            request.options,
            session_permissions,
        );
        // A v1 card keeps frozen danger routing on the machine-owned native
        // walks too (Banking.open's stand walk), as on its compiled walks.
        let (off_debt, compat_v1, condition) =
            self.navs
                .lock()
                .unwrap()
                .get(&self.name)
                .map_or((false, false, None), |bot| {
                    (
                        bot.walk_guard_off.is_some(),
                        bot.compat_v1,
                        bot.condition.current().copied(),
                    )
                });
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
        self.state
            .as_mut()
            .expect("request state was captured")
            .quest_evidence = self
            .world
            .as_ref()
            .and_then(|world| world.graph.quest_family)
            .and_then(|family| {
                captured_route_quests(
                    family,
                    context.as_ref(),
                    snapshot,
                    request.evidence,
                    request.required_after,
                )
            });
        let exclusions = ScriptRouteExclusions {
            cross: request.cross.to_vec(),
            ..Default::default()
        };
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
            if let (Some(snapshot), Some(family)) = (snapshot, world.graph.quest_family) {
                let context = self
                    .navs
                    .lock()
                    .unwrap()
                    .get(&self.name)
                    .and_then(|bot| bot.quest_context.clone());
                if let Some(context) = context.as_ref() {
                    state
                        .get_or_insert_with(|| WorldState::from_snapshot(snapshot))
                        .quest_evidence =
                        captured_route_quests(family, Some(context), snapshot, None, context.stamp);
                }
            }
        }
        let (mut opts, exclusions) = if let Some((basis, exclusions)) = frozen {
            let state = state.get_or_insert_with(WorldState::empty);
            state.quest_evidence = basis.quest_evidence.clone();
            (basis.options, exclusions)
        } else {
            let empty = WorldState::empty();
            let route_state = state.as_ref().unwrap_or(&empty);
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
                bot.supersede_route_work();
            }
            let displaced = bot.pending_route.take();
            let displaced_kind = displaced.as_ref().and_then(|demand| demand.kind());
            admission.input.off_debt = bot.walk_guard_off.is_some();
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
    pub(crate) state: Option<WorldState>,
    pub(crate) admission: crate::admission::Admission,
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
        let state = self.state.as_ref().unwrap_or(&empty);
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

    fn drive_route_job_output_for_test(self) -> (RouteJobOutput, Vec<WorldTile>, Arc<NavWorld>) {
        fn identity(index: u64) -> WorkIdentity {
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

        fn drive_stage(worker: &mut RouteWorkJob) -> RouteStageResult {
            use api::identity::Deadline;
            use api::work::{MonotonicClock, SharedCancellation, SliceBudget, SystemClock};

            let clock = SystemClock;
            let cancellation = SharedCancellation::new();
            for _ in 0..250_000 {
                let budget = SliceBudget::new(
                    std::num::NonZeroU32::new(64).expect("slice budget is nonzero"),
                    Deadline::after(clock.now(), std::time::Duration::from_millis(10)),
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

        let world = Arc::clone(&self.world);
        let mut job = RouteJob::prepare(
            "route-rule-test",
            Box::new(self),
            RouteDemandKind::Walk { authority: None },
        )
        .expect("test route request prepares");
        job.identity = Some(identity(0));
        let mut next = Some(Box::new(job));
        for stage_index in 0..64u64 {
            let job = next.take().expect("the prior route stage continued");
            let mut worker = RouteWorkJob::new(job);
            match drive_stage(&mut worker) {
                RouteStageResult::Continue(mut job) => {
                    assert!(job.search_cursor.is_none());
                    job.prepare_continuation()
                        .expect("test route continuation prepares");
                    job.identity = Some(identity(stage_index + 1));
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
        }));
    }

    /// The gating facts of the route request still waiting for a pump turn.
    #[cfg(test)]
    pub(crate) fn pending_route_state_for_test(&self) -> Option<&WorldState> {
        match self.pending_route.as_deref()? {
            RouteDemand::Pending { request, .. } => request.state.as_ref(),
            RouteDemand::Preparing { .. } | RouteDemand::Prepared(_) => None,
        }
    }

    /// Abandon the accepted route when bank work takes ownership of the nav entry.
    pub(crate) fn supersede_route_work(&mut self) {
        if let Some(mut owner) = self.route_ticket.take() {
            owner.abandon(Cancellation::Superseded);
        }
        self.pending_route = None;
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
    }

    /// A plain increment from 1. It never wraps in practice: see the 63-bit
    /// lifetime note at `script::native::ledger::ObservedWalkOutcome`.
    fn bump_walk_outcome_seq(&mut self) {
        self.manual_walk_cancelled_detail = false;
        self.walk_outcome_seq += 1;
    }

    pub(crate) fn ordinary_movement_owned(&self, tick: u16) -> bool {
        self.native_walk.as_ref().is_some_and(|owner| owner.live())
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
                        .map(|(_, refusal)| *refusal),
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
        self.walk_outcome_cancel_reason = script::isolate_fb::WalkCancelReason::None;
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
        self.walk_outcome_cancel_reason = script::isolate_fb::WalkCancelReason::None;
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
        if self.walk_outcome_cancel_reason == script::isolate_fb::WalkCancelReason::UserInput {
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
        self.walk_outcome_cancel_reason = script::isolate_fb::WalkCancelReason::None;
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
            || self.walk_outcome_cancel_reason == script::isolate_fb::WalkCancelReason::UserInput
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
                script::shim::InteractReq::Walk {
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
                script::shim::InteractReq::WalkNear {
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
            self.walk_outcome_cancel_reason = script::isolate_fb::WalkCancelReason::UserInput;
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
        self.has_route()
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
            RouteOutcome::NoPath => {
                log_walk_arm_bot(|| {
                    format!(
                        "publish_route NoPath request_id={request_id} walk_request_id={} \
                         may_publish={}",
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
/// present, the same lifecycle receipt resets this state only once.
pub(crate) fn reset_script_nav(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    terminal_generation: Option<u64>,
) {
    if let Some(nav) = navs.lock().unwrap().get_mut(name) {
        if terminal_generation.is_some_and(|generation| {
            nav.terminal_nav_reset_generation
                .is_some_and(|last| last >= generation)
        }) {
            return;
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
    }
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
        request: script::shim::InteractReq::WalkNear {
            x: to.x,
            z: to.z,
            level: to.level,
            radius,
            allow_teleports,
            allow_wilderness,
            compat_tile_intent: bot.route_compat_tile_intent,
            allow_bank_fetch,
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
                request: script::shim::InteractReq::WalkNearestBank,
            }));
        } else if let (Some(runtime_generation), true, Some(requested), None) =
            (carry, armed, nav.requested_route, &nav.native_walk)
        {
            let (to, radius, allow_teleports, allow_wilderness, allow_bank_fetch, _) = requested;
            let request_id = nav.walk_request_id;
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
                script::shim::InteractReq::WalkNear {
                    x: to.x,
                    z: to.z,
                    level: to.level,
                    radius,
                    compat_tile_intent: nav.route_compat_tile_intent,
                    allow_teleports,
                    allow_wilderness,
                    allow_bank_fetch,
                    request_id,
                    avoid,
                    cross,
                }
            } else {
                script::shim::InteractReq::Walk {
                    x: to.x,
                    z: to.z,
                    level: to.level,
                    allow_teleports,
                    allow_wilderness,
                    allow_bank_fetch,
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
) -> Option<script::shim::InteractReq> {
    let carried = navs.lock().unwrap().get_mut(name)?.carried_walk.take()?;
    (carried.runtime_generation == Some(runtime_generation)).then_some(carried.request)
}

fn end_route_follow(nav: &mut NavBot) {
    if nav.bank_pick.walking(nav.route_generation) {
        nav.bank_pick.reset();
    }
    nav.supersede_route_work();
    nav.route_generation = nav.route_generation.wrapping_add(1);
    nav.pending_route = None;
    nav.requested_route = None;
    nav.traveller.clear();
    nav.clear_route();
    nav.bank_fetch = None;
    nav.walk_request_id = 0;
    nav.route_quest_evidence = None;
    super::script_walk::owe_walk_guard_off(nav);
    nav.end_native_walk(script::native::WalkEnd::Cancelled);
    nav.admission_pending = false;
    nav.assess_result = None;
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
            bank_origin: Origin::Unknown,
            bank: Vec::new(),
            live_geometry: None,
            exclusions: None,
            compat_tile_intent: false,
            completion: Default::default(),
        })
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
        request.state = Some(state);
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
    #[ignore = "bounded 524288-settle failure-cost measurement"]
    fn candidate_preferred_failure_cost_stays_within_the_existing_first_goal_budget() {
        // Both components exceed the backward proof's 262144-tile cap.
        // The preferred proof must abandon, rather than cheaply close.
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
        assert_eq!(
            search.route(),
            Err(nav_planner::router::RouteError::BudgetExhausted)
        );
        assert_eq!(search.proof(), nav_planner::router::ReverseProof::Abandoned);
        assert!(search.settled() <= nav_planner::router::FIRST_TARGET_BUDGET + 1);
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
            "preferred_failure": "BudgetExhausted",
            "proof": "Abandoned",
            "forward_settles": search.settled(),
            "unproven_settle_budget": nav_planner::router::FIRST_TARGET_BUDGET,
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
        use std::collections::{HashMap, HashSet};

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
            for goal in selected.drain(..) {
                let request = |candidate_goals| {
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
                    request.state = Some(state.clone());
                    request.candidate_goals = candidate_goals;
                    request
                };
                let (ordinary, _) = request(None).drive_route_job_for_test();
                let (candidate, _) =
                    request(Some(script::native::WalkGoals::new(vec![goal]).unwrap()))
                        .drive_route_job_for_test();
                match (ordinary, candidate) {
                    (RouteOutcome::Routed(ordinary), RouteOutcome::Routed(candidate)) => {
                        assert_eq!(ordinary, candidate, "{name}: {from:?} -> {goal:?}");
                        routed += 1;
                    }
                    (RouteOutcome::NoPath, RouteOutcome::NoPath) => {}
                    _ => panic!("{name}: route availability changed for {from:?} -> {goal:?}"),
                }
            }
            let record = serde_json::json!({
                "case": name, "from": [from.x, from.z, from.level],
                "corpus_goals": all_goals, "pairs": pair_count,
                "arrival": "Area", "radius": 0,
                "stride": if name.starts_with("radius_walk") { stride } else { 1 },
                "routed_pairs": routed, "no_path_pairs": pair_count - routed,
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
            request.state = Some(WorldState {
                stats: (0..25).map(|skill| (skill, 99)).collect(),
                combat_level: Some(126),
                map_members: true,
                ..WorldState::empty()
            });
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
                    measured = Some(drive_stage(&mut worker));
                });
                let peak = held
                    .checked_add(usize::try_from(allocations.bytes_max).unwrap())
                    .unwrap();
                assert!(
                    peak <= charge,
                    "{case} job {index}: peak={peak}, charge={charge}"
                );
                let (result, slices, largest) = measured.unwrap();
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

    fn selected_r289() -> Arc<api::game_data::SelectedGameData> {
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
                move |worker| worker_data.prepare_quests(worker),
            )
            .expect("submit production quest preparation")
            .join()
            .expect("quest preparation worker")
            .expect("prepare selected quest family");
            selected
        });
        Arc::clone(&SELECTED)
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

    fn family(
        selected: &api::game_data::SelectedGameData,
    ) -> nav_planner::quest_gates::QuestFamilyId {
        let identity = selected
            .quest_family_identity()
            .expect("selected quest family identity");
        nav_planner::quest_gates::QuestFamilyId::new(identity.sha256, identity.schema)
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
        let identity = selected
            .quest_family_identity()
            .expect("selected quest family identity");
        Arc::new(QuestListEvidenceProvider::from_snapshot(
            &quests,
            Some(identity),
            Some(identity),
            SnapshotView::new(Some(&snapshot(name, colour, available)), observed),
        ))
    }

    fn context(
        selected: Arc<api::game_data::SelectedGameData>,
        stamp: EvidenceStamp,
    ) -> RouteQuestContext {
        RouteQuestContext { selected, stamp }
    }

    #[test]
    fn explicit_real_evidence_wins_when_known_and_unknown_falls_back() {
        let selected = selected_r289();
        let (quest, name) = selected_quest(&selected);
        let family = family(&selected);
        let run = run();
        let floor = stamp(run, 9, 9);
        let complete = nav_planner::world_state::QUEST_COMPLETE_COLOUR;
        let not_started = 0xF80000;
        let gate = QuestGate::Complete(quest);
        let context = context(Arc::clone(&selected), floor);

        let explicit_false = real_list_provider(&selected, &name, not_started, true, floor);
        let ordinary_true = snapshot(&name, complete, true);
        let evidence = captured_route_quests(
            family,
            Some(&context),
            &ordinary_true,
            Some(explicit_false),
            floor,
        )
        .expect("ordinary evidence and explicit provider");
        assert_eq!(
            evidence.test(&gate),
            Truth::False,
            "known explicit False must override ordinary Complete"
        );

        let explicit_true = real_list_provider(&selected, &name, complete, true, floor);
        let ordinary_false = snapshot(&name, not_started, true);
        let evidence = captured_route_quests(
            family,
            Some(&context),
            &ordinary_false,
            Some(explicit_true),
            floor,
        )
        .expect("ordinary evidence and explicit provider");
        assert_eq!(
            evidence.test(&gate),
            Truth::True,
            "known explicit True must override ordinary NotStarted"
        );

        let explicit_unknown = real_list_provider(&selected, &name, complete, false, floor);
        let evidence = captured_route_quests(
            family,
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
    fn captured_rows_fail_closed_after_the_run_or_causal_floor_changes() {
        let selected = selected_r289();
        let (quest, name) = selected_quest(&selected);
        let family = family(&selected);
        let observed = stamp(run(), 5, 5);
        let complete = snapshot(&name, nav_planner::world_state::QUEST_COMPLETE_COLOUR, true);
        let gate = QuestGate::Complete(quest);

        let later_floor = stamp(run(), 6, 6);
        let context = context(Arc::clone(&selected), observed);
        let evidence = captured_route_quests(family, Some(&context), &complete, None, later_floor)
            .expect("matching-family ordinary provider");
        assert_eq!(evidence.test(&gate), Truth::Unknown);

        let changed_run = RunKey { run: 2, ..run() };
        let changed_run_floor = stamp(changed_run, 5, 5);
        let evidence =
            captured_route_quests(family, Some(&context), &complete, None, changed_run_floor)
                .expect("matching-family ordinary provider");
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
            completion,
        }
    }

    #[test]
    fn ordinary_provider_bytes_are_charged_to_route_input_and_output() {
        let selected = selected_r289();
        let (quest, name) = selected_quest(&selected);
        let family = family(&selected);
        let stamp = stamp(run(), 12, 12);
        let context = context(Arc::clone(&selected), stamp);
        let snapshot = snapshot(&name, nav_planner::world_state::QUEST_COMPLETE_COLOUR, true);
        let evidence = captured_route_quests(family, Some(&context), &snapshot, None, stamp)
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

        let (baseline, _) = RouteSubmission::prepare(&request, &without_evidence, &lease, 0, 0)
            .expect("baseline route budget");
        let (charged, _) = RouteSubmission::prepare(&request, &with_evidence, &lease, 0, 0)
            .expect("evidence route budget");
        assert_eq!(charged.input_bytes - baseline.input_bytes, owned_bytes);
        assert_eq!(charged.result_bytes - baseline.result_bytes, owned_bytes);
    }
}
