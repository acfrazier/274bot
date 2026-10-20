//! Cooperative route inspection. One accepted WorkAdmission job plus one
//! bounded latest owner demand per profile. Pause does not cancel inspection,
//! and inspection results never arm Traveller.

use super::NavBot;
use api::bank_memory::Origin;
use api::identity::{Cancellation, Scope, Settlement, WorkIdentity};
use api::obj_names::ObjNames;
use api::process_condition::ProcessConditions;
use api::snapshot::WorldTile;
use api::work::{
    Checkpoint, CooperativeJob, InputRevision, RetainedState, StepContext, StepOutcome,
    SubmissionRefusal, WorkAdmission, WorkTicket,
};
use client::config::Cache;
#[cfg(test)]
use nav::bank_fetch::{plan_bank_fetch, planning_rows, BankFetchError, BankStep};
use nav::bank_fetch::{BankFetchCursor, BankFetchPlan, BankRows, PlanningRowsCursor};
#[cfg(test)]
use nav::router::{
    find_missing_item_reqs_with_avoid, find_missing_item_reqs_with_avoid_bounded, find_with_avoid,
    find_with_avoid_bounded, Leg, Route, RouteError,
};
use nav::router::{AvoidRect, FindOptions};
use nav::transport::TransportKind;
use nav::world::NavWorld;
use nav::world_state::WorldState;
use nav::zones::{ZoneExempt, ZoneFilterOwned};
use script::work_admission::{cancellation::WorkCancellation, WorkService};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

#[cfg(test)]
std::thread_local! {
    static INSPECTION_WORK_SERVICE_OVERRIDE: std::cell::RefCell<
        Option<Arc<parking_lot::Mutex<WorkService>>>,
    > = const { std::cell::RefCell::new(None) };
}

#[inline]
fn inspection_work_service(
) -> Result<Arc<parking_lot::Mutex<WorkService>>, script::work_admission::WorkStartError> {
    #[cfg(test)]
    if let Some(service) = INSPECTION_WORK_SERVICE_OVERRIDE.with(|service| service.borrow().clone())
    {
        return Ok(service);
    }
    WorkService::shared()
}

#[cfg(test)]
struct InspectionWorkServiceOverride;

#[cfg(test)]
impl InspectionWorkServiceOverride {
    fn install(service: Arc<parking_lot::Mutex<WorkService>>) -> Self {
        INSPECTION_WORK_SERVICE_OVERRIDE.with(|current| {
            let mut current = current.borrow_mut();
            assert!(current.is_none(), "nested inspection service override");
            *current = Some(service);
        });
        Self
    }
}

#[cfg(test)]
impl Drop for InspectionWorkServiceOverride {
    fn drop(&mut self) {
        INSPECTION_WORK_SERVICE_OVERRIDE.with(|service| {
            let _ = service.borrow_mut().take();
        });
    }
}

pub(super) const MAX_AVOID: usize = 16;
const MAX_INSPECT_CHARGE: usize = 192 * 1024 * 1024;
/// Isolate unsettled-waiter cap. Host published storage plus admission
/// equal this bound: RING(2)+HELD(1)=3. Registered overflow posts a
/// refuse identity; it does not accept a fourth terminal.
const RING: usize = 2;
const HELD: usize = 1;
const CAPACITY: usize = RING + HELD;
#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InspectHop {
    pub kind: String,
    pub loc_id: i32,
    pub loc_name: String,
    pub action: String,
    pub option: i32,
    pub from_x: i32,
    pub from_z: i32,
    pub from_level: i32,
    pub to_x: i32,
    pub to_z: i32,
    pub to_level: i32,
    pub ticks: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InspectHopView<'a> {
    pub kind: &'a str,
    pub loc_id: i32,
    pub loc_name: &'a str,
    pub action: &'a str,
    pub option: i32,
    pub from_x: i32,
    pub from_z: i32,
    pub from_level: i32,
    pub to_x: i32,
    pub to_z: i32,
    pub to_level: i32,
    pub ticks: i32,
}

pub(crate) struct InspectStored {
    ok: bool,
    reason: InspectMessage,
    bank_planned: bool,
    ticks: f64,
    route: Option<nav::contract::RoutePlanLease>,
    cache: Option<Arc<Cache>>,
    obj_names: Option<Arc<ObjNames>>,
}
impl std::fmt::Debug for InspectStored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectStored")
            .field("ok", &self.ok)
            .field("reason", &self.reason)
            .field("bank_planned", &self.bank_planned)
            .field("ticks", &self.ticks)
            .field("route", &self.route)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
enum InspectMessage {
    Static(&'static str),
    Owned(String),
}

struct InspectJobOutput {
    request_id: u64,
    generation: u64,
    revision: InputRevision,
    ok: bool,
    reason: InspectMessage,
    bank_planned: bool,
    ticks: f64,
    route: Option<nav::router::CompactRoute>,
    lease: Option<nav::contract::NavLease>,
    cache: Option<Arc<Cache>>,
    obj_names: Option<Arc<ObjNames>>,
}
impl InspectJobOutput {
    fn cancelled(request_id: u64, generation: u64, revision: InputRevision) -> Self {
        Self {
            request_id,
            generation,
            revision,
            ok: false,
            reason: InspectMessage::Static(""),
            bank_planned: false,
            ticks: 0.0,
            route: None,
            lease: None,
            cache: None,
            obj_names: None,
        }
    }

    fn retained_heap_bytes(&self) -> Option<usize> {
        let mut bytes = 0;
        if let InspectMessage::Owned(reason) = &self.reason {
            add_bytes(&mut bytes, reason.capacity())?;
        }
        if let Some(route) = &self.route {
            add_bytes(&mut bytes, route.try_retained_bytes()?)?;
        }
        Some(bytes)
    }
}
struct InspectWorkFamily;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InspectTicketRef {
    identity: WorkIdentity,
    revision: InputRevision,
}

impl InspectTicketRef {
    fn new(ticket: WorkTicket<InspectWorkFamily>) -> Self {
        Self {
            identity: ticket.identity(),
            revision: ticket.revision(),
        }
    }

    fn ticket(self) -> WorkTicket<InspectWorkFamily> {
        WorkTicket::new(self.identity, self.revision)
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    let _ = assert_send_sync::<WorkCancellation<InspectWorkFamily>>;
};

#[derive(Clone, Copy)]
struct InspectSubmission {
    revision: api::work::InputRevision,
    input_bytes: usize,
    phase_peak_bytes: usize,
    result_bytes: usize,
}

#[derive(Default)]
struct InspectSearchPreflights {
    strict: Option<nav::router::SearchPreflight>,
    missing: Option<nav::router::SearchPreflight>,
    stand: Option<nav::router::SearchPreflight>,
    stand_fallback: Option<nav::router::SearchPreflight>,
    post: Option<nav::router::SearchPreflight>,
    blockers_with_bank: Option<nav::router::SearchPreflight>,
    blockers_without_bank: Option<nav::router::SearchPreflight>,
    max_plan_item_rows: usize,
    missing_result_bytes: usize,
}

fn strict_bank_access_options(options: FindOptions) -> FindOptions {
    FindOptions {
        allow_bank_fetch: false,
        prefer_target_order: true,
        ..options
    }
}

impl InspectSubmission {
    fn for_capture(
        name: &str,
        capture: &InspectCapture,
    ) -> Result<(Self, InspectSearchPreflights), InspectJobError> {
        let graph = &capture.world.graph;
        let avoid_capacity = capture.avoid.len();
        let stand_rows = capture.world.banks().len();
        let mut bounds = InspectSearchBounds::default();
        let mut preflights = InspectSearchPreflights::default();

        let strict = preflight_inspection_search(
            capture,
            nav::router::SearchRequestShape::Single {
                options: capture.opts,
                avoid_capacity,
                max_settles: strict_budget(capture),
            },
        )?;
        bounds.include(&strict, InspectSearchKind::FinalRoute)?;
        preflights.strict = Some(strict);

        if capture.opts.allow_bank_fetch {
            let missing = preflight_inspection_search(
                capture,
                nav::router::SearchRequestShape::MissingItemReqs {
                    target_capacity: 1,
                    fallback_capacity: 0,
                    options: capture.opts,
                    avoid_capacity,
                    max_settles: strict_budget(capture),
                },
            )?;
            bounds.include(&missing, InspectSearchKind::Missing)?;
            preflights.missing = Some(missing);

            let options = strict_bank_access_options(capture.opts);
            let stand_target_capacity = stand_rows
                .checked_mul(4)
                .ok_or(InspectJobError::ChargeOverflow)?;
            // Search every access tile in air-distance order. A single query
            // below preserves the first candidate's failure for diagnosis.
            let stand = preflight_inspection_search(
                capture,
                nav::router::SearchRequestShape::First {
                    target_capacity: stand_target_capacity,
                    fallback_capacity: 0,
                    options,
                    avoid_capacity,
                    max_settles: stand_budget(capture),
                },
            )?;
            bounds.include(&stand, InspectSearchKind::Stand)?;
            preflights.stand = Some(stand);

            let stand_fallback = preflight_inspection_search(
                capture,
                nav::router::SearchRequestShape::Single {
                    options,
                    avoid_capacity,
                    max_settles: stand_budget(capture),
                },
            )?;
            bounds.include(&stand_fallback, InspectSearchKind::Stand)?;
            preflights.stand_fallback = Some(stand_fallback);
            let post = preflight_inspection_search(
                capture,
                nav::router::SearchRequestShape::Single {
                    options: FindOptions {
                        allow_bank_fetch: false,
                        ..capture.opts
                    },
                    avoid_capacity,
                    max_settles: post_budget(capture),
                },
            )?;
            bounds.include(&post, InspectSearchKind::PostRoute)?;
            preflights.post = Some(post);
        }

        if !capture.opts.zones.is_all() {
            if let Some(table) = graph.zones.as_ref() {
                let zone_filter_bytes =
                    zone_filter_input_bytes(table.zones().len(), capture.opts.zones)?;
                let blocker_options = if capture.opts.allow_bank_fetch {
                    capture.opts
                } else {
                    FindOptions {
                        allow_bank_fetch: false,
                        ..capture.opts
                    }
                };
                let blockers_with_bank = preflight_inspection_search(
                    capture,
                    nav::router::SearchRequestShape::BlockingZones {
                        target_capacity: 1,
                        diagnosis: nav::router::BlockerDiagnosis::SingleTarget,
                        options: blocker_options,
                        avoid_capacity,
                        zone_filter_bytes,
                        max_settles: nav::router::NODE_BUDGET,
                    },
                )?;
                bounds.include(&blockers_with_bank, InspectSearchKind::Blockers)?;
                if capture.opts.allow_bank_fetch {
                    preflights.blockers_with_bank = Some(blockers_with_bank);
                    let blockers_without_bank = preflight_inspection_search(
                        capture,
                        nav::router::SearchRequestShape::BlockingZones {
                            target_capacity: 1,
                            diagnosis: nav::router::BlockerDiagnosis::SingleTarget,
                            options: FindOptions {
                                allow_bank_fetch: false,
                                ..capture.opts
                            },
                            avoid_capacity,
                            zone_filter_bytes,
                            max_settles: nav::router::NODE_BUDGET,
                        },
                    )?;
                    bounds.include(&blockers_without_bank, InspectSearchKind::Blockers)?;
                    preflights.blockers_without_bank = Some(blockers_without_bank);
                } else {
                    preflights.blockers_without_bank = Some(blockers_with_bank);
                }
            }
        }

        preflights.missing_result_bytes = bounds.missing_result_bytes;
        let missing_reservation = preflights
            .missing
            .as_ref()
            .map(|preflight| preflight.reservation());
        let plan_item_rows =
            missing_reservation.map_or(0, |reservation| reservation.max_plan_item_rows);
        preflights.max_plan_item_rows = plan_item_rows;
        let mut input_bytes = capture
            .input_bytes()
            .ok_or(InspectJobError::ChargeOverflow)?;
        add_bytes(&mut input_bytes, std::mem::size_of::<InspectJob>())
            .ok_or(InspectJobError::ChargeOverflow)?;
        add_bytes(&mut input_bytes, "nav-inspect-".len())
            .and_then(|()| add_bytes(&mut input_bytes, name.len()))
            .ok_or(InspectJobError::ChargeOverflow)?;
        // InspectJob already charges its phase union. mem::replace also holds
        // the moved phase locally beside the replacement Start variant.
        add_bytes(&mut input_bytes, std::mem::size_of::<InspectJobPhase>())
            .ok_or(InspectJobError::ChargeOverflow)?;

        let planning_peak_bytes =
            if capture.opts.allow_bank_fetch && capture.bank_origin == Origin::Hint {
                PlanningRowsCursor::heap_bound(capture.bank.len(), plan_item_rows)
                    .ok_or(InspectJobError::ChargeOverflow)?
            } else {
                0
            };
        let planning_rows_bytes =
            if capture.opts.allow_bank_fetch && capture.bank_origin == Origin::Hint {
                PlanningRowsCursor::rows_heap_bound(capture.bank.len(), plan_item_rows)
                    .ok_or(InspectJobError::ChargeOverflow)?
            } else {
                0
            };
        let bank_rows = if capture.bank_origin == Origin::Hint {
            capture
                .bank
                .len()
                .checked_add(plan_item_rows)
                .ok_or(InspectJobError::ChargeOverflow)?
        } else {
            capture.bank.len()
        };
        let bank_cursor_bytes = if capture.opts.allow_bank_fetch {
            BankFetchCursor::heap_bound(bank_rows, plan_item_rows, stand_rows)
                .ok_or(InspectJobError::ChargeOverflow)?
        } else {
            0
        };
        let bank_plan_bytes = if capture.opts.allow_bank_fetch {
            BankFetchPlan::heap_bound(plan_item_rows).ok_or(InspectJobError::ChargeOverflow)?
        } else {
            0
        };

        // Search peaks include their own terminal payloads. The missing
        // diagnosis is the only search result retained by later planning phases.
        let planning_phase_bytes = planning_peak_bytes
            .checked_add(bounds.missing_result_bytes)
            .ok_or(InspectJobError::ChargeOverflow)?;
        let bank_plan_phase_bytes = planning_rows_bytes
            .checked_add(bank_cursor_bytes)
            .and_then(|bytes| bytes.checked_add(bounds.missing_result_bytes))
            .ok_or(InspectJobError::ChargeOverflow)?;
        let mut state_growth_bytes = 0usize;
        let mut apply_phase_bytes = bank_plan_bytes;
        if capture.opts.allow_bank_fetch && plan_item_rows != 0 {
            let wear_choices = plan_item_rows
                .min(missing_reservation.map_or(0, |reservation| reservation.max_missing_worn_ids));
            let inventory_new = plan_item_rows;
            let growth = capture
                .state
                .growth_bound(inventory_new, wear_choices)
                .ok_or(InspectJobError::ChargeOverflow)?;
            add_bytes(&mut state_growth_bytes, growth.inventory_new_bytes)
                .ok_or(InspectJobError::ChargeOverflow)?;
            add_bytes(&mut state_growth_bytes, growth.worn_new_bytes)
                .ok_or(InspectJobError::ChargeOverflow)?;
            add_bytes(&mut apply_phase_bytes, state_growth_bytes)
                .ok_or(InspectJobError::ChargeOverflow)?;
        }
        let phase_b_bytes = planning_phase_bytes
            .max(bank_plan_phase_bytes)
            .max(apply_phase_bytes);
        let detail_bytes = if bounds.blocker_search_bytes == 0 {
            0
        } else {
            graph
                .zones
                .as_ref()
                .map_or(0, |table| table.blocking_detail_bytes_bound())
        };
        let blocker_search_phase_bytes = if bounds.blocker_search_bytes == 0 {
            0
        } else if capture.opts.allow_bank_fetch {
            bounds
                .blocker_search_bytes
                .checked_add(state_growth_bytes)
                .ok_or(InspectJobError::ChargeOverflow)?
        } else {
            bounds.blocker_search_bytes
        };
        let blocker_detail_phase_bytes = if bounds.blocker_result_bytes == 0 {
            0
        } else {
            let mut bytes = bounds
                .blocker_result_bytes
                .checked_add(detail_bytes)
                .ok_or(InspectJobError::ChargeOverflow)?;
            if capture.opts.allow_bank_fetch {
                add_bytes(&mut bytes, state_growth_bytes).ok_or(InspectJobError::ChargeOverflow)?;
            }
            bytes
        };
        let phase_a_bytes = bounds
            .phase_a_bytes
            .max(blocker_search_phase_bytes)
            .max(blocker_detail_phase_bytes);
        let post_phase_bytes = bounds
            .post_search_bytes
            .checked_add(state_growth_bytes)
            .ok_or(InspectJobError::ChargeOverflow)?;
        let phase_c_bytes = bounds
            .stand_search_bytes
            .checked_add(bank_cursor_bytes)
            .ok_or(InspectJobError::ChargeOverflow)?
            .max(post_phase_bytes);
        let phase_peak_bytes = phase_a_bytes.max(phase_b_bytes).max(phase_c_bytes);

        // Production route searches allow 4,000,000 settled transitions.
        // CompactRoute's reservation covers every selected edge ID,
        // checkpoint and walk tile, so this also bounds transport hops.
        // Published hop views borrow that route; the job makes no per-hop
        // strings or copied hop records.

        // Static failure text is inline in the settlement; only owned detail
        // strings and compact routes contribute retained payload bytes.
        let final_output_payload_bytes = bounds.final_route_result_bytes.max(detail_bytes);
        // R is the final retained payload plus the settlement header. The
        // compact route and any owned detail string move into terminal storage.
        let result_bytes = std::mem::size_of::<api::work::JobSettlement<InspectWorkFamily>>()
            .checked_add(final_output_payload_bytes)
            .ok_or(InspectJobError::ChargeOverflow)?;
        // With P as the largest simultaneous phase and R as the final result,
        // scratch=P-R (saturating) makes input+scratch+result=input+max(P,R).
        let total_bytes = input_bytes
            .checked_add(phase_peak_bytes.max(result_bytes))
            .ok_or(InspectJobError::ChargeOverflow)?;
        if total_bytes > MAX_INSPECT_CHARGE {
            return Err(InspectJobError::ChargeLimit);
        }

        Ok((
            Self {
                revision: capture.revision,
                input_bytes,
                phase_peak_bytes,
                result_bytes,
            },
            preflights,
        ))
    }
}

fn preflight_inspection_search(
    capture: &InspectCapture,
    shape: nav::router::SearchRequestShape,
) -> Result<nav::router::SearchPreflight, InspectJobError> {
    nav::router::SearchCursor::preflight(shape, &capture.lease, &capture.state)
        .ok_or(InspectJobError::ChargeOverflow)
}

fn zone_filter_input_bytes(zone_count: usize, zones: ZoneExempt) -> Result<usize, InspectJobError> {
    if zones.is_all() || zones == ZoneExempt::NONE {
        Ok(std::mem::size_of::<ZoneFilterOwned>())
    } else {
        ZoneFilterOwned::worst_case_bytes(zone_count).ok_or(InspectJobError::ChargeOverflow)
    }
}

#[derive(Clone, Copy)]
enum InspectSearchKind {
    FinalRoute,
    Missing,
    Stand,
    PostRoute,
    Blockers,
}

#[derive(Default)]
struct InspectSearchBounds {
    phase_a_bytes: usize,
    blocker_search_bytes: usize,
    blocker_result_bytes: usize,
    stand_search_bytes: usize,
    post_search_bytes: usize,
    missing_result_bytes: usize,
    final_route_result_bytes: usize,
}

impl InspectSearchBounds {
    fn include(
        &mut self,
        preflight: &nav::router::SearchPreflight,
        kind: InspectSearchKind,
    ) -> Result<(), InspectJobError> {
        let reservation = preflight.reservation();
        // peak_bytes already includes result_bytes; do not add it again.
        let search_bytes = preflight
            .input_bytes()
            .checked_add(reservation.future_layout_bytes)
            .and_then(|bytes| bytes.checked_add(reservation.peak_bytes))
            .ok_or(InspectJobError::ChargeOverflow)?;
        let result_payload_bytes = search_result_payload_bytes(reservation.result_bytes)?;
        match kind {
            InspectSearchKind::FinalRoute => {
                self.phase_a_bytes = self.phase_a_bytes.max(search_bytes);
                self.final_route_result_bytes =
                    self.final_route_result_bytes.max(result_payload_bytes);
            }
            InspectSearchKind::Missing => {
                self.phase_a_bytes = self.phase_a_bytes.max(search_bytes);
                self.missing_result_bytes = self.missing_result_bytes.max(result_payload_bytes);
            }
            InspectSearchKind::Stand => {
                self.stand_search_bytes = self.stand_search_bytes.max(search_bytes);
            }
            InspectSearchKind::PostRoute => {
                self.post_search_bytes = self.post_search_bytes.max(search_bytes);
                self.final_route_result_bytes =
                    self.final_route_result_bytes.max(result_payload_bytes);
            }
            InspectSearchKind::Blockers => {
                self.blocker_search_bytes = self.blocker_search_bytes.max(search_bytes);
                self.blocker_result_bytes = self.blocker_result_bytes.max(result_payload_bytes);
            }
        }
        Ok(())
    }
}

// Search wrappers are inline during the cursor phase and disappear before
// publication. Their sizes stay in peak_bytes, not in later retained payloads.
fn search_result_payload_bytes(result_bytes: usize) -> Result<usize, InspectJobError> {
    let inline_bytes = std::mem::size_of::<nav::router::SearchOutcome>()
        .checked_add(std::mem::size_of::<nav::router::SearchCompletion>())
        .ok_or(InspectJobError::ChargeOverflow)?;
    result_bytes
        .checked_sub(inline_bytes)
        .ok_or(InspectJobError::ChargeOverflow)
}
#[derive(Debug, thiserror::Error)]
enum InspectJobError {
    #[error("inspection retained-byte arithmetic overflowed")]
    ChargeOverflow,
    #[error("inspection retained-byte charge exceeds the 192 MiB endpoint bound")]
    ChargeLimit,
    #[error("inspection allocation was refused")]
    AllocationRefused,
    #[error("inspection search cursor rejected its bounded request")]
    SearchCursor,
    #[error("inspection search returned an unexpected outcome")]
    WrongOutcome,
}

impl From<std::collections::TryReserveError> for InspectJobError {
    fn from(_: std::collections::TryReserveError) -> Self {
        Self::AllocationRefused
    }
}

impl api::work::WorkFamilySpec for InspectWorkFamily {
    const FAMILY: api::work::WorkFamily = api::work::WorkFamily::Inspection;
    type Input = InspectSubmission;
    type Output = InspectJobOutput;
    type Error = InspectJobError;
    type Phase = api::work::InspectionPhase;

    fn revision(input: &Self::Input) -> api::work::InputRevision {
        input.revision
    }

    fn charge(input: &Self::Input) -> api::work::WorkCharge {
        api::work::WorkCharge::new(
            input.input_bytes,
            input.phase_peak_bytes.saturating_sub(input.result_bytes),
            input.result_bytes,
            0,
        )
    }

    fn map_cancellation(reason: api::identity::Cancellation) -> api::identity::Cancellation {
        reason
    }
}
struct InspectBankPlanning {
    missing: nav::router::MissingRequirements,
    rows: BankRowsInput,
    cursor: BankFetchCursor,
}

impl InspectBankPlanning {
    fn retained_bytes(&self, missing_result_bytes: usize) -> Option<usize> {
        let mut bytes = missing_result_bytes;
        if let BankRowsInput::Overlay(rows) = &self.rows {
            add_bytes(
                &mut bytes,
                rows.capacity()
                    .checked_mul(std::mem::size_of::<(i32, i32)>())?,
            )?;
        }
        add_bytes(&mut bytes, self.cursor.retained_bytes())?;
        Some(bytes)
    }
}

struct InspectBankAccess {
    cursor: BankFetchCursor,
}

enum InspectSearchPurpose {
    Strict,
    Missing,
    BankAccessFirst {
        planning: InspectBankAccess,
    },
    BankAccessFailure {
        planning: InspectBankAccess,
        access: WorldTile,
    },
    Post {
        bank_planned: bool,
    },
    Blockers {
        bank_planned: bool,
    },
}

enum InspectSearchInput {
    Single,
    First(Box<[WorldTile]>),
    Missing,
    Blocking(ZoneFilterOwned),
}

struct InspectSearchBuilder {
    from: WorldTile,
    to: WorldTile,
    options: FindOptions,
    max_settles: usize,
    input: InspectSearchInput,
    target: Option<WorldTile>,
    targets: Vec<WorldTile>,
    target_at: usize,
    avoid: Vec<AvoidRect>,
    avoid_at: usize,
    reserved: bool,
    purpose: InspectSearchPurpose,
    preflight: nav::router::SearchPreflight,
}

impl InspectSearchBuilder {
    /// Missing and blocker requests take `to` as their one target. Single
    /// requests use `to` directly and first-goal requests own their targets.
    fn new(
        from: WorldTile,
        to: WorldTile,
        options: FindOptions,
        max_settles: usize,
        input: InspectSearchInput,
        purpose: InspectSearchPurpose,
        preflight: nav::router::SearchPreflight,
    ) -> Self {
        let target = match input {
            InspectSearchInput::Missing | InspectSearchInput::Blocking(_) => Some(to),
            InspectSearchInput::Single | InspectSearchInput::First(_) => None,
        };
        Self {
            from,
            to,
            options,
            max_settles,
            input,
            target,
            targets: Vec::new(),
            target_at: 0,
            avoid: Vec::new(),
            avoid_at: 0,
            reserved: false,
            purpose,
            preflight,
        }
    }

    fn retained_bytes(&self) -> Option<usize> {
        let mut bytes = self
            .targets
            .capacity()
            .checked_mul(std::mem::size_of::<WorldTile>())?;
        if let InspectSearchInput::First(targets) = &self.input {
            add_bytes(
                &mut bytes,
                targets
                    .len()
                    .checked_mul(std::mem::size_of::<WorldTile>())?,
            )?;
        }
        add_bytes(
            &mut bytes,
            self.avoid
                .capacity()
                .checked_mul(std::mem::size_of::<AvoidRect>())?,
        )?;
        if let InspectSearchInput::Blocking(filter) = &self.input {
            add_bytes(&mut bytes, filter.retained_bytes()?)?;
        }
        Some(bytes)
    }

    fn advance(&mut self, capture: &InspectCapture) -> Result<bool, InspectJobError> {
        if !self.reserved {
            if self.target.is_some() {
                self.targets.try_reserve_exact(1)?;
            }
            self.avoid.try_reserve_exact(capture.avoid.len())?;
            self.reserved = true;
            return Ok(false);
        }
        if let Some(target) = self.target {
            if self.target_at == 0 {
                self.targets.push(target);
                self.target_at = 1;
                return Ok(false);
            }
        }
        if let Some(rect) = capture.avoid.get(self.avoid_at).copied() {
            self.avoid.push(rect);
            self.avoid_at += 1;
            return Ok(false);
        }
        Ok(true)
    }

    fn into_request(
        self,
    ) -> (
        nav::router::SearchRequest,
        InspectSearchPurpose,
        nav::router::SearchPreflight,
    ) {
        let Self {
            from,
            to,
            options,
            max_settles,
            input,
            targets,
            avoid,
            purpose,
            preflight,
            ..
        } = self;
        let avoid = avoid.into_boxed_slice();
        let request = match input {
            InspectSearchInput::Single => nav::router::SearchRequest::Single {
                from,
                to,
                options,
                avoid,
                max_settles,
            },
            InspectSearchInput::First(targets) => nav::router::SearchRequest::First {
                from,
                targets,
                fallback: Box::default(),
                options,
                avoid,
                max_settles,
            },
            InspectSearchInput::Missing => nav::router::SearchRequest::MissingItemReqs {
                from,
                targets: targets.into_boxed_slice(),
                fallback: Box::new([]),
                options,
                avoid,
                max_settles,
            },
            InspectSearchInput::Blocking(zone_filter) => {
                nav::router::SearchRequest::BlockingZones {
                    from,
                    targets: targets.into_boxed_slice(),
                    diagnosis: nav::router::BlockerDiagnosis::SingleTarget,
                    options,
                    avoid,
                    max_settles,
                    zone_filter,
                }
            }
        };
        (request, purpose, preflight)
    }
}

enum BankRowsInput {
    Captured,
    Overlay(Vec<(i32, i32)>),
}

#[expect(
    clippy::large_enum_variant,
    reason = "The admitted job charges the inline phase union and moves cursors without a per-phase allocation."
)]
enum InspectJobPhase {
    Start,
    BuildSearch {
        builder: InspectSearchBuilder,
    },
    Search {
        cursor: nav::router::SearchCursor,
        purpose: InspectSearchPurpose,
    },
    PlanningRows {
        missing: nav::router::MissingRequirements,
        cursor: PlanningRowsCursor,
    },
    BankPlan(InspectBankPlanning),
    ApplyBank {
        plan: BankFetchPlan,
    },
    BuildZoneFilter {
        filter: ZoneFilterOwned,
        from: WorldTile,
        to: WorldTile,
        options: FindOptions,
        bank_planned: bool,
        preflight: nav::router::SearchPreflight,
    },
    BlockerDetail {
        cursor: BlockingDetailCursor,
        bank_planned: bool,
    },
    Ready(InspectJobOutput),
}

struct InspectJob {
    thread_name: String,
    capture: Option<InspectCapture>,
    phase: InspectJobPhase,
    preflights: InspectSearchPreflights,
}

impl InspectJob {
    fn new(name: &str, capture: InspectCapture, preflights: InspectSearchPreflights) -> Self {
        let mut thread_name = String::with_capacity("nav-inspect-".len() + name.len());
        thread_name.push_str("nav-inspect-");
        thread_name.push_str(name);
        Self {
            thread_name,
            capture: Some(capture),
            phase: InspectJobPhase::Start,
            preflights,
        }
    }

    fn retained_bytes(&self) -> Option<usize> {
        let mut bytes = std::mem::size_of::<Self>();
        add_bytes(&mut bytes, self.thread_name.capacity())?;
        if let Some(capture) = &self.capture {
            add_bytes(&mut bytes, capture.input_bytes()?)?;
        }
        match &self.phase {
            InspectJobPhase::Start => {}
            InspectJobPhase::BuildSearch { builder } => {
                add_bytes(&mut bytes, builder.retained_bytes()?)?;
                if let InspectSearchPurpose::BankAccessFirst { planning }
                | InspectSearchPurpose::BankAccessFailure { planning, .. } = &builder.purpose
                {
                    add_bytes(&mut bytes, planning.cursor.retained_bytes())?;
                }
            }
            InspectJobPhase::Search { cursor, purpose } => {
                add_bytes(&mut bytes, cursor.input_bytes())?;
                add_bytes(&mut bytes, cursor.future_layout_bytes())?;
                add_bytes(&mut bytes, cursor.retained_bytes())?;
                if let InspectSearchPurpose::BankAccessFirst { planning }
                | InspectSearchPurpose::BankAccessFailure { planning, .. } = purpose
                {
                    add_bytes(&mut bytes, planning.cursor.retained_bytes())?;
                }
            }
            InspectJobPhase::PlanningRows { cursor, .. } => {
                add_bytes(&mut bytes, self.preflights.missing_result_bytes)?;
                add_bytes(&mut bytes, cursor.retained_bytes())?;
            }
            InspectJobPhase::BankPlan(planning) => {
                add_bytes(
                    &mut bytes,
                    planning.retained_bytes(self.preflights.missing_result_bytes)?,
                )?;
            }
            InspectJobPhase::ApplyBank { plan } => {
                add_bytes(&mut bytes, plan.retained_bytes())?;
            }
            InspectJobPhase::BuildZoneFilter { filter, .. } => {
                add_bytes(&mut bytes, filter.retained_bytes()?)?;
            }
            InspectJobPhase::BlockerDetail { cursor, .. } => {
                add_bytes(&mut bytes, cursor.retained_heap_bytes()?)?;
            }
            InspectJobPhase::Ready(output) => {
                add_bytes(&mut bytes, output.retained_heap_bytes()?)?;
            }
        }
        Some(bytes)
    }
}
impl InspectJob {
    fn capture_ref(&self) -> &InspectCapture {
        self.capture
            .as_ref()
            .expect("inspection state remains owned until the terminal")
    }

    fn set_ready(
        &mut self,
        ok: bool,
        reason: InspectMessage,
        bank_planned: bool,
        route: Option<nav::router::CompactRoute>,
    ) {
        let capture = self
            .capture
            .take()
            .expect("inspection result consumes its captured state once");
        let InspectCapture {
            request_id,
            generation,
            revision,
            lease,
            state,
            cache,
            obj_names,
            ..
        } = capture;
        let ticks = route.as_ref().map_or(0.0, |route| route.ticks());
        let (lease, cache, obj_names) = if route.is_some() {
            (Some(lease), cache, obj_names)
        } else {
            drop(lease);
            drop(cache);
            drop(obj_names);
            (None, None, None)
        };
        drop(state);
        self.preflights = InspectSearchPreflights::default();
        self.phase = InspectJobPhase::Ready(InspectJobOutput {
            request_id,
            generation,
            revision,
            ok,
            reason,
            bank_planned,
            ticks,
            route,
            lease,
            cache,
            obj_names,
        });
    }

    fn set_failure(&mut self, reason: &'static str, bank_planned: bool) {
        self.set_ready(false, InspectMessage::Static(reason), bank_planned, None);
    }

    fn set_route(&mut self, route: nav::router::CompactRoute, bank_planned: bool) {
        self.set_ready(true, InspectMessage::Static(""), bank_planned, Some(route));
    }

    fn cancelled_output(&mut self) -> InspectJobOutput {
        let metadata = self
            .capture
            .as_ref()
            .map(|capture| (capture.request_id, capture.generation, capture.revision));
        let previous = std::mem::replace(&mut self.phase, InspectJobPhase::Start);
        let output = match (metadata, previous) {
            (Some((request_id, generation, revision)), _) => {
                InspectJobOutput::cancelled(request_id, generation, revision)
            }
            (None, InspectJobPhase::Ready(output)) => {
                InspectJobOutput::cancelled(output.request_id, output.generation, output.revision)
            }
            (None, _) => unreachable!("inspection job has captured or terminal metadata"),
        };
        self.capture = None;
        self.preflights = InspectSearchPreflights::default();
        output
    }

    fn start_search(
        &mut self,
        request: nav::router::SearchRequest,
        purpose: InspectSearchPurpose,
        preflight: nav::router::SearchPreflight,
    ) -> Result<(), InspectJobError> {
        let (lease, state) = {
            let capture = self.capture_ref();
            (capture.lease.clone(), Arc::clone(&capture.state))
        };
        let cursor = nav::router::SearchCursor::new_preflighted(request, lease, state, preflight)
            .map_err(|_| InspectJobError::SearchCursor)?;
        self.phase = InspectJobPhase::Search { cursor, purpose };
        Ok(())
    }

    fn start_single(
        &mut self,
        from: WorldTile,
        to: WorldTile,
        options: FindOptions,
        max_settles: usize,
        purpose: InspectSearchPurpose,
        preflight: nav::router::SearchPreflight,
    ) {
        self.phase = InspectJobPhase::BuildSearch {
            builder: InspectSearchBuilder::new(
                from,
                to,
                options,
                max_settles,
                InspectSearchInput::Single,
                purpose,
                preflight,
            ),
        };
    }

    fn start_first(
        &mut self,
        from: WorldTile,
        targets: Box<[WorldTile]>,
        options: FindOptions,
        max_settles: usize,
        purpose: InspectSearchPurpose,
        preflight: nav::router::SearchPreflight,
    ) -> Result<(), InspectJobError> {
        let to = targets
            .first()
            .copied()
            .ok_or(InspectJobError::SearchCursor)?;
        self.phase = InspectJobPhase::BuildSearch {
            builder: InspectSearchBuilder::new(
                from,
                to,
                options,
                max_settles,
                InspectSearchInput::First(targets),
                purpose,
                preflight,
            ),
        };
        Ok(())
    }

    fn start_bank_access(
        &mut self,
        mut planning: InspectBankAccess,
    ) -> Result<(), InspectJobError> {
        let (from, options, budget) = {
            let capture = self.capture_ref();
            (
                capture.from,
                strict_bank_access_options(capture.opts),
                stand_budget(capture),
            )
        };
        let preflight = self
            .preflights
            .stand
            .take()
            .ok_or(InspectJobError::SearchCursor)?;
        let targets = planning.cursor.take_access_targets();
        self.start_first(
            from,
            targets,
            options,
            budget,
            InspectSearchPurpose::BankAccessFirst { planning },
            preflight,
        )
    }

    fn start_bank_access_failure(
        &mut self,
        planning: InspectBankAccess,
        access: WorldTile,
    ) -> Result<(), InspectJobError> {
        let (from, options, budget) = {
            let capture = self.capture_ref();
            (
                capture.from,
                strict_bank_access_options(capture.opts),
                stand_budget(capture),
            )
        };
        let preflight = self
            .preflights
            .stand_fallback
            .take()
            .ok_or(InspectJobError::SearchCursor)?;
        self.start_single(
            from,
            access,
            options,
            budget,
            InspectSearchPurpose::BankAccessFailure { planning, access },
            preflight,
        );
        Ok(())
    }

    fn finish_bank_access(
        &mut self,
        mut planning: InspectBankAccess,
        route: nav::router::CompactRoute,
    ) -> Result<(), InspectJobError> {
        if !planning.cursor.select_access(route.destination()) {
            return Err(InspectJobError::SearchCursor);
        }
        let plan = planning
            .cursor
            .take_plan()
            .ok_or(InspectJobError::SearchCursor)?;
        self.phase = InspectJobPhase::ApplyBank { plan };
        Ok(())
    }

    fn start_missing(&mut self) -> Result<(), InspectJobError> {
        let preflight = self
            .preflights
            .missing
            .take()
            .ok_or(InspectJobError::SearchCursor)?;
        let capture = self.capture_ref();
        self.phase = InspectJobPhase::BuildSearch {
            builder: InspectSearchBuilder::new(
                capture.from,
                capture.to,
                capture.opts,
                strict_budget(capture),
                InspectSearchInput::Missing,
                InspectSearchPurpose::Missing,
                preflight,
            ),
        };
        Ok(())
    }

    fn start_blockers(
        &mut self,
        from: WorldTile,
        to: WorldTile,
        options: FindOptions,
        bank_planned: bool,
    ) -> Result<(), InspectJobError> {
        if options.zones.is_all() {
            self.set_failure("NoPath", bank_planned);
            return Ok(());
        }
        let zone_parameters = {
            let capture = self.capture_ref();
            capture
                .world
                .graph
                .zones
                .as_ref()
                .map(|table| (table.zones().len(), capture.state.combat_level))
        };
        let Some((zone_count, combat_level)) = zone_parameters else {
            self.set_failure("NoPath", bank_planned);
            return Ok(());
        };
        let preflight = if options.allow_bank_fetch {
            self.preflights.blockers_with_bank.take()
        } else {
            self.preflights.blockers_without_bank.take()
        }
        .ok_or(InspectJobError::SearchCursor)?;
        let endpoints = [from, to];
        let filter = ZoneFilterOwned::new(zone_count, combat_level, &endpoints, options.zones)?;
        self.phase = InspectJobPhase::BuildZoneFilter {
            filter,
            from,
            to,
            options,
            bank_planned,
            preflight,
        };
        Ok(())
    }

    fn handle_search(
        &mut self,
        purpose: InspectSearchPurpose,
        outcome: nav::router::SearchOutcome,
    ) -> Result<(), InspectJobError> {
        use nav::router::{RouteError, SearchOutcome};

        match (purpose, outcome) {
            (InspectSearchPurpose::Strict, SearchOutcome::Single { route: Ok(route) }) => {
                self.set_route(route, false);
            }
            (
                InspectSearchPurpose::Strict,
                SearchOutcome::Single {
                    route: Err(RouteError::BudgetExhausted),
                },
            ) => {
                self.set_failure("BudgetExhausted", false);
            }
            (
                InspectSearchPurpose::Strict,
                SearchOutcome::Single {
                    route: Err(RouteError::NoPath),
                },
            ) => {
                let capture = self.capture_ref();
                if capture.opts.allow_bank_fetch {
                    self.start_missing()?;
                } else {
                    self.start_blockers(capture.from, capture.to, capture.opts, false)?;
                }
            }
            (InspectSearchPurpose::Missing, SearchOutcome::MissingItemReqs(Some(missing))) => {
                let (hint_bank, bank_rows, stand_rows) = {
                    let capture = self.capture_ref();
                    (
                        capture.bank_origin == Origin::Hint,
                        capture.bank.len(),
                        capture.world.banks().len(),
                    )
                };
                let plan_item_rows = self.preflights.max_plan_item_rows;
                if hint_bank {
                    let cursor = PlanningRowsCursor::with_limits(bank_rows, plan_item_rows)
                        .ok_or(InspectJobError::ChargeOverflow)?;
                    self.phase = InspectJobPhase::PlanningRows { missing, cursor };
                } else {
                    let cursor =
                        BankFetchCursor::with_limits(bank_rows, plan_item_rows, stand_rows)
                            .ok_or(InspectJobError::ChargeOverflow)?;
                    self.phase = InspectJobPhase::BankPlan(InspectBankPlanning {
                        missing,
                        rows: BankRowsInput::Captured,
                        cursor,
                    });
                }
            }
            (InspectSearchPurpose::Missing, SearchOutcome::MissingItemReqs(None)) => {
                let capture = self.capture_ref();
                self.start_blockers(capture.from, capture.to, capture.opts, false)?;
            }
            (
                InspectSearchPurpose::BankAccessFirst { planning },
                SearchOutcome::First { search },
            ) => {
                let (route, _) = search.into_routes();
                match route {
                    Ok(route) => self.finish_bank_access(planning, route)?,
                    Err(_) => {
                        let access = planning
                            .cursor
                            .planned_access()
                            .ok_or(InspectJobError::SearchCursor)?;
                        self.start_bank_access_failure(planning, access)?;
                    }
                }
            }
            (
                InspectSearchPurpose::BankAccessFailure { planning, .. },
                SearchOutcome::Single { route: Ok(route) },
            ) => self.finish_bank_access(planning, route)?,
            (
                InspectSearchPurpose::BankAccessFailure { access, .. },
                SearchOutcome::Single { route: Err(error) },
            ) => match error {
                RouteError::NoPath => {
                    let capture = self.capture_ref();
                    self.start_blockers(
                        capture.from,
                        access,
                        FindOptions {
                            allow_bank_fetch: false,
                            ..capture.opts
                        },
                        false,
                    )?;
                }
                RouteError::BudgetExhausted => self.set_failure("BudgetExhausted", false),
            },
            (
                InspectSearchPurpose::Post { bank_planned },
                SearchOutcome::Single { route: Ok(route) },
            ) => {
                self.set_route(route, bank_planned);
            }
            (
                InspectSearchPurpose::Post { bank_planned },
                SearchOutcome::Single {
                    route: Err(RouteError::BudgetExhausted),
                },
            ) => {
                self.set_failure("BudgetExhausted", bank_planned);
            }
            (
                InspectSearchPurpose::Post { bank_planned },
                SearchOutcome::Single {
                    route: Err(RouteError::NoPath),
                },
            ) => {
                let capture = self.capture_ref();
                let options = FindOptions {
                    allow_bank_fetch: false,
                    ..capture.opts
                };
                self.start_blockers(capture.from, capture.to, options, bank_planned)?;
            }
            (
                InspectSearchPurpose::Blockers { bank_planned },
                SearchOutcome::BlockingZones(keys),
            ) => {
                if let Some(keys) = keys.filter(|keys| !keys.is_empty()) {
                    self.phase = InspectJobPhase::BlockerDetail {
                        cursor: BlockingDetailCursor::new(keys),
                        bank_planned,
                    };
                } else {
                    self.set_failure("NoPath", bank_planned);
                }
            }
            _ => return Err(InspectJobError::WrongOutcome),
        }
        Ok(())
    }
}

#[cfg(test)]
fn strict_budget(capture: &InspectCapture) -> usize {
    capture
        .search_budget_strict
        .unwrap_or(nav::router::NODE_BUDGET)
}

#[cfg(not(test))]
fn strict_budget(_capture: &InspectCapture) -> usize {
    nav::router::NODE_BUDGET
}

#[cfg(test)]
fn stand_budget(capture: &InspectCapture) -> usize {
    capture
        .search_budget_stand
        .unwrap_or(nav::router::NODE_BUDGET)
}

#[cfg(not(test))]
fn stand_budget(_capture: &InspectCapture) -> usize {
    nav::router::NODE_BUDGET
}

#[cfg(test)]
fn post_budget(capture: &InspectCapture) -> usize {
    capture
        .search_budget_post
        .unwrap_or(nav::router::NODE_BUDGET)
}

#[cfg(not(test))]
fn post_budget(_capture: &InspectCapture) -> usize {
    nav::router::NODE_BUDGET
}
impl InspectJob {
    fn after_unit(
        &mut self,
        context: &mut StepContext<'_, api::work::InspectionPhase>,
    ) -> Option<StepOutcome<InspectJobOutput, InspectJobError>> {
        match context.consume_unit() {
            Checkpoint::Continue => None,
            Checkpoint::Yield => Some(StepOutcome::Yielded),
            Checkpoint::Cancel(_) => Some(StepOutcome::Completed(self.cancelled_output())),
        }
    }
}

impl CooperativeJob for InspectJob {
    type Output = InspectJobOutput;
    type Error = InspectJobError;
    type Phase = api::work::InspectionPhase;

    fn request_thread(&self) -> Option<api::work::RequestThread<'_>> {
        Some(api::work::RequestThread::new(&self.thread_name))
    }

    fn step(
        &mut self,
        context: &mut StepContext<'_, Self::Phase>,
    ) -> StepOutcome<Self::Output, Self::Error> {
        loop {
            match context.checkpoint() {
                Checkpoint::Continue => {}
                Checkpoint::Yield => return StepOutcome::Yielded,
                Checkpoint::Cancel(_) => {
                    return StepOutcome::Completed(self.cancelled_output());
                }
            }

            let phase = std::mem::replace(&mut self.phase, InspectJobPhase::Start);
            match phase {
                InspectJobPhase::Start => {
                    let (from, to, options, budget) = {
                        let capture = self.capture_ref();
                        (
                            capture.from,
                            capture.to,
                            capture.opts,
                            strict_budget(capture),
                        )
                    };
                    let preflight = self
                        .preflights
                        .strict
                        .take()
                        .ok_or(InspectJobError::SearchCursor);
                    let preflight = match preflight {
                        Ok(preflight) => preflight,
                        Err(error) => return StepOutcome::Failed(error),
                    };
                    self.start_single(
                        from,
                        to,
                        options,
                        budget,
                        InspectSearchPurpose::Strict,
                        preflight,
                    );
                }
                InspectJobPhase::BuildSearch { mut builder } => {
                    let ready = {
                        let capture = self.capture_ref();
                        match builder.advance(capture) {
                            Ok(ready) => ready,
                            Err(error) => return StepOutcome::Failed(error),
                        }
                    };
                    if ready {
                        let (request, purpose, preflight) = builder.into_request();
                        if let Err(error) = self.start_search(request, purpose, preflight) {
                            return StepOutcome::Failed(error);
                        }
                    } else {
                        self.phase = InspectJobPhase::BuildSearch { builder };
                    }
                    if let Some(outcome) = self.after_unit(context) {
                        return outcome;
                    }
                }
                InspectJobPhase::Search {
                    mut cursor,
                    purpose,
                } => match cursor.advance(context) {
                    nav::router::CursorStep::Yielded => {
                        self.phase = InspectJobPhase::Search { cursor, purpose };
                        return StepOutcome::Yielded;
                    }
                    nav::router::CursorStep::Cancelled(_) => {
                        return StepOutcome::Completed(self.cancelled_output());
                    }
                    nav::router::CursorStep::Failed(_) => {
                        return StepOutcome::Failed(InspectJobError::SearchCursor);
                    }
                    nav::router::CursorStep::Complete(completion) => {
                        let (outcome, state) = completion.into_parts();
                        let capture = self
                            .capture
                            .as_ref()
                            .expect("search completion returns to its inspection capture");
                        debug_assert!(Arc::ptr_eq(&capture.state, &state));
                        drop(state);
                        drop(cursor);
                        if let Err(error) = self.handle_search(purpose, outcome) {
                            return StepOutcome::Failed(error);
                        }
                    }
                },
                InspectJobPhase::PlanningRows {
                    missing,
                    mut cursor,
                } => {
                    let complete = {
                        let capture = self.capture_ref();
                        cursor.advance(&capture.bank, &missing)
                    };
                    if complete {
                        let Some(rows) = cursor.take_rows() else {
                            return StepOutcome::Failed(InspectJobError::WrongOutcome);
                        };
                        let Some(cursor) = BankFetchCursor::with_limits(
                            rows.len(),
                            self.preflights.max_plan_item_rows,
                            self.capture_ref().world.banks().len(),
                        ) else {
                            return StepOutcome::Failed(InspectJobError::ChargeOverflow);
                        };
                        self.phase = InspectJobPhase::BankPlan(InspectBankPlanning {
                            missing,
                            rows: BankRowsInput::Overlay(rows),
                            cursor,
                        });
                    } else {
                        self.phase = InspectJobPhase::PlanningRows { missing, cursor };
                    }
                    if let Some(outcome) = self.after_unit(context) {
                        return outcome;
                    }
                }
                InspectJobPhase::BankPlan(mut planning) => {
                    let complete = {
                        let capture = self.capture_ref();
                        let bank_rows = match &planning.rows {
                            BankRowsInput::Captured => capture.bank.as_slice(),
                            BankRowsInput::Overlay(rows) => rows.as_slice(),
                        };
                        planning.cursor.advance(
                            &planning.missing,
                            &capture.state,
                            bank_rows,
                            capture.world.banks(),
                            capture.from,
                            &capture.world.collision,
                        )
                    };
                    if complete {
                        if planning.cursor.planned_access().is_some() {
                            let InspectBankPlanning {
                                missing,
                                rows,
                                cursor,
                            } = planning;
                            // Selection owns item targets. Release the missing
                            // diagnosis and hint overlay before search.
                            drop(missing);
                            drop(rows);
                            if let Err(error) = self.start_bank_access(InspectBankAccess { cursor })
                            {
                                return StepOutcome::Failed(error);
                            }
                        } else if let Some(plan) = planning.cursor.take_plan() {
                            self.phase = InspectJobPhase::ApplyBank { plan };
                        } else {
                            self.set_failure("NoPath", false);
                        }
                    } else {
                        self.phase = InspectJobPhase::BankPlan(planning);
                    }
                    if let Some(outcome) = self.after_unit(context) {
                        return outcome;
                    }
                }
                InspectJobPhase::ApplyBank { mut plan } => {
                    let complete = {
                        let capture = self
                            .capture
                            .as_mut()
                            .expect("bank deltas retain the same owned world state");
                        let Some(state) = Arc::get_mut(&mut capture.state) else {
                            return StepOutcome::Failed(InspectJobError::WrongOutcome);
                        };
                        plan.apply_next(state)
                    };
                    if complete {
                        drop(plan);
                        let (from, to, options, budget) = {
                            let capture = self.capture_ref();
                            (
                                capture.from,
                                capture.to,
                                FindOptions {
                                    allow_bank_fetch: false,
                                    ..capture.opts
                                },
                                post_budget(capture),
                            )
                        };
                        let preflight = match self.preflights.post.take() {
                            Some(preflight) => preflight,
                            None => return StepOutcome::Failed(InspectJobError::SearchCursor),
                        };
                        self.start_single(
                            from,
                            to,
                            options,
                            budget,
                            InspectSearchPurpose::Post { bank_planned: true },
                            preflight,
                        );
                    } else {
                        self.phase = InspectJobPhase::ApplyBank { plan };
                    }
                    if let Some(outcome) = self.after_unit(context) {
                        return outcome;
                    }
                }
                InspectJobPhase::BuildZoneFilter {
                    mut filter,
                    from,
                    to,
                    options,
                    bank_planned,
                    preflight,
                } => {
                    let ready = {
                        let capture = self.capture_ref();
                        let Some(table) = capture.world.graph.zones.as_ref() else {
                            self.set_failure("NoPath", bank_planned);
                            continue;
                        };
                        filter.advance(table, 1)
                    };
                    if ready {
                        if !filter.is_ready() {
                            return StepOutcome::Failed(InspectJobError::WrongOutcome);
                        }
                        filter.select_destination(Some(to));
                        self.phase = InspectJobPhase::BuildSearch {
                            builder: InspectSearchBuilder::new(
                                from,
                                to,
                                options,
                                nav::router::NODE_BUDGET,
                                InspectSearchInput::Blocking(filter),
                                InspectSearchPurpose::Blockers { bank_planned },
                                preflight,
                            ),
                        };
                    } else {
                        self.phase = InspectJobPhase::BuildZoneFilter {
                            filter,
                            from,
                            to,
                            options,
                            bank_planned,
                            preflight,
                        };
                    }
                    if let Some(outcome) = self.after_unit(context) {
                        return outcome;
                    }
                }
                InspectJobPhase::BlockerDetail {
                    mut cursor,
                    bank_planned,
                } => {
                    let complete = {
                        let capture = self.capture_ref();
                        let Some(table) = capture.world.graph.zones.as_ref() else {
                            self.set_failure("NoPath", bank_planned);
                            continue;
                        };
                        match cursor.advance(table) {
                            Ok(complete) => complete,
                            Err(error) => return StepOutcome::Failed(error),
                        }
                    };
                    if complete {
                        self.set_ready(
                            false,
                            InspectMessage::Owned(cursor.into_reason()),
                            bank_planned,
                            None,
                        );
                    } else {
                        self.phase = InspectJobPhase::BlockerDetail {
                            cursor,
                            bank_planned,
                        };
                    }
                    if let Some(outcome) = self.after_unit(context) {
                        return outcome;
                    }
                }
                InspectJobPhase::Ready(output) => return StepOutcome::Completed(output),
            }
        }
    }

    fn retained(&self) -> RetainedState {
        let bytes = self.retained_bytes().unwrap_or(usize::MAX);
        RetainedState::clean(bytes)
    }
}

impl InspectMessage {
    fn as_str(&self) -> &str {
        match self {
            Self::Static(reason) => reason,
            Self::Owned(reason) => reason,
        }
    }
}

struct BlockingDetailCursor {
    keys: Box<[nav::zones::ZoneKey]>,
    measure_at: usize,
    write_at: usize,
    required_bytes: usize,
    output: Option<String>,
}

impl BlockingDetailCursor {
    fn new(keys: Box<[nav::zones::ZoneKey]>) -> Self {
        Self {
            keys,
            measure_at: 0,
            write_at: 0,
            required_bytes: "blocked by danger zones: ".len(),
            output: None,
        }
    }

    /// One unit measures or writes one key, or allocates the exact output.
    fn advance(&mut self, table: &nav::zones::ZoneTable) -> Result<bool, InspectJobError> {
        if self.output.is_none() {
            if let Some(&key) = self.keys.get(self.measure_at) {
                let separator = if self.measure_at == 0 { 0 } else { 2 };
                self.required_bytes = self
                    .required_bytes
                    .checked_add(separator)
                    .and_then(|bytes| bytes.checked_add(table.blocking_detail_item_bytes(key)))
                    .ok_or(InspectJobError::ChargeOverflow)?;
                self.measure_at += 1;
                return Ok(false);
            }
            let mut output = String::new();
            output.try_reserve_exact(self.required_bytes)?;
            output.push_str("blocked by danger zones: ");
            self.output = Some(output);
            return Ok(false);
        }
        let Some(&key) = self.keys.get(self.write_at) else {
            return Ok(true);
        };
        let output = self
            .output
            .as_mut()
            .expect("diagnostic output exists while keys are written");
        if self.write_at != 0 {
            output.push_str(", ");
        }
        table
            .write_blocking_detail_item(key, output)
            .expect("reserved String formatting cannot fail");
        self.write_at += 1;
        Ok(false)
    }

    fn retained_heap_bytes(&self) -> Option<usize> {
        self.keys
            .len()
            .checked_mul(std::mem::size_of::<nav::zones::ZoneKey>())?
            .checked_add(self.output.as_ref().map_or(0, String::capacity))
    }

    fn into_reason(self) -> String {
        self.output.expect("completed blocker detail has output")
    }
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InspectLegacy {
    ok: bool,
    reason: String,
    bank_planned: bool,
    ticks: f64,
    hops: Arc<[InspectHop]>,
}

#[derive(Clone, Debug)]
pub(crate) enum InspectReason {
    Static(&'static str),
    Owned(Arc<nav::producer::OwnerResult<InspectStored>>),
    #[cfg(test)]
    Legacy(Arc<InspectLegacy>),
}

impl InspectReason {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Static(reason) => reason,
            Self::Owned(result) => result.value().reason.as_str(),
            #[cfg(test)]
            Self::Legacy(result) => &result.reason,
        }
    }
}

impl std::ops::Deref for InspectReason {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl std::fmt::Display for InspectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq for InspectReason {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<&str> for InspectReason {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InspectHops {
    owner: Option<Arc<nav::producer::OwnerResult<InspectStored>>>,
    #[cfg(test)]
    legacy: Option<Arc<InspectLegacy>>,
}

impl InspectHops {
    fn owner(result: Arc<nav::producer::OwnerResult<InspectStored>>) -> Self {
        Self {
            owner: Some(result),
            #[cfg(test)]
            legacy: None,
        }
    }

    #[cfg(test)]
    fn legacy(result: Arc<InspectLegacy>) -> Self {
        Self {
            legacy: Some(result),
            ..Self::default()
        }
    }

    pub(crate) fn iter(&self) -> InspectHopIter<'_> {
        let owner = self.owner.as_deref().map(|result| result.value());
        let route = owner.and_then(|owner| owner.route.as_ref()).map(|route| {
            nav::producer::route_plan_cursor(route)
                .expect("published inspection plan retains its navigation lease")
        });
        InspectHopIter {
            route,
            owner,
            #[cfg(test)]
            legacy: self.legacy.as_deref().map(|legacy| legacy.hops.iter()),
        }
    }
}

impl PartialEq for InspectHops {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

pub(crate) struct InspectHopIter<'a> {
    route: Option<nav::producer::RoutePlanCursor<'a>>,
    owner: Option<&'a InspectStored>,
    #[cfg(test)]
    legacy: Option<std::slice::Iter<'a, InspectHop>>,
}

impl<'a> Iterator for InspectHopIter<'a> {
    type Item = InspectHopView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if let (Some(route), Some(owner)) = (self.route.as_mut(), self.owner) {
            while let Some(leg) = route.next_leg() {
                let nav::producer::RoutePlanLeg::Transport(step) = leg else {
                    continue;
                };
                let kind = step.kind();
                let (loc_name, action) = hop_texts(
                    kind,
                    step.loc_id(),
                    step.option(),
                    owner.cache.as_deref(),
                    owner.obj_names.as_deref(),
                );
                let from = step.anchor();
                let to = step.actual_to();
                return Some(InspectHopView {
                    kind: kind_name(kind),
                    loc_id: step.loc_id(),
                    loc_name,
                    action,
                    option: step.option(),
                    from_x: from.x,
                    from_z: from.z,
                    from_level: from.level,
                    to_x: to.x,
                    to_z: to.z,
                    to_level: to.level,
                    ticks: step.ticks(),
                });
            }
            self.route = None;
            self.owner = None;
        }
        #[cfg(test)]
        if let Some(hop) = self.legacy.as_mut()?.next() {
            return Some(InspectHopView {
                kind: hop.kind.as_str(),
                loc_id: hop.loc_id,
                loc_name: hop.loc_name.as_str(),
                action: hop.action.as_str(),
                option: hop.option,
                from_x: hop.from_x,
                from_z: hop.from_z,
                from_level: hop.from_level,
                to_x: hop.to_x,
                to_z: hop.to_z,
                to_level: hop.to_level,
                ticks: hop.ticks,
            });
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InspectTerminal {
    pub seq: u64,
    pub generation: u64,
    pub request_id: u64,
    pub ok: bool,
    pub reason: InspectReason,
    pub bank_planned: bool,
    pub ticks: f64,
    pub hops: InspectHops,
}

impl InspectTerminal {
    fn refusal(request_id: u64, generation: u64, reason: &'static str) -> Self {
        Self {
            seq: 0,
            generation,
            request_id,
            ok: false,
            reason: InspectReason::Static(reason),
            bank_planned: false,
            ticks: 0.0,
            hops: InspectHops::default(),
        }
    }

    fn owned(
        request_id: u64,
        generation: u64,
        result: Arc<nav::producer::OwnerResult<InspectStored>>,
    ) -> Self {
        let value = result.value();
        Self {
            seq: 0,
            generation,
            request_id,
            ok: value.ok,
            reason: InspectReason::Owned(Arc::clone(&result)),
            bank_planned: value.bank_planned,
            ticks: value.ticks,
            hops: InspectHops::owner(result),
        }
    }

    #[cfg(test)]
    fn oracle(
        request_id: u64,
        generation: u64,
        ok: bool,
        reason: String,
        bank_planned: bool,
        ticks: f64,
        hops: Vec<InspectHop>,
    ) -> Self {
        let result = Arc::new(InspectLegacy {
            ok,
            reason,
            bank_planned,
            ticks,
            hops: hops.into(),
        });
        Self {
            seq: 0,
            generation,
            request_id,
            ok: result.ok,
            reason: InspectReason::Legacy(Arc::clone(&result)),
            bank_planned: result.bank_planned,
            ticks: result.ticks,
            hops: InspectHops::legacy(result),
        }
    }
    #[cfg(test)]
    fn oracle_refusal(request_id: u64, generation: u64, reason: &str) -> Self {
        Self::oracle(
            request_id,
            generation,
            false,
            reason.to_owned(),
            false,
            0.0,
            Vec::new(),
        )
    }
}

pub(crate) struct InspectCapture {
    pub request_id: u64,
    pub generation: u64,
    pub revision: api::work::InputRevision,
    pub world: Arc<NavWorld>,
    pub lease: nav::contract::NavLease,
    pub state: Arc<WorldState>,
    /// The bank memory's origin beside its rows ([`BankRows`] flattened, so
    /// the byte packs into this capture's own padding).
    pub bank_origin: Origin,
    pub bank: Vec<(i32, i32)>,
    pub from: WorldTile,
    pub to: WorldTile,
    pub opts: FindOptions,
    pub avoid: Vec<AvoidRect>,
    pub cache: Option<Arc<Cache>>,
    pub obj_names: Option<Arc<ObjNames>>,
    /// Injectable node budgets for the cfg(test) reference calculation.
    #[cfg(test)]
    pub search_budget_strict: Option<usize>,
    #[cfg(test)]
    pub search_budget_stand: Option<usize>,
    #[cfg(test)]
    pub search_budget_post: Option<usize>,
}

impl InspectCapture {
    fn input_bytes(&self) -> Option<usize> {
        let mut bytes = 0;
        add_bytes(
            &mut bytes,
            self.bank
                .capacity()
                .checked_mul(std::mem::size_of::<(i32, i32)>())?,
        )?;
        add_bytes(
            &mut bytes,
            self.avoid
                .capacity()
                .checked_mul(std::mem::size_of::<AvoidRect>())?,
        )?;
        add_bytes(&mut bytes, self.state_heap_bytes()?)?;
        Some(bytes)
    }

    fn state_heap_bytes(&self) -> Option<usize> {
        self.state.try_shared_retained_bytes()
    }
}

fn add_bytes(total: &mut usize, bytes: usize) -> Option<()> {
    *total = total.checked_add(bytes)?;
    Some(())
}

struct PendingInspect {
    // Taken only during submission, then restored on refusal. Keeping the
    // allocation across Full retries avoids allocating on every slot turn.
    job: Option<InspectJob>,
    submission: InspectSubmission,
}

impl PendingInspect {
    fn capture_ref(&self) -> &InspectCapture {
        self.job
            .as_ref()
            .expect("pending inspection retains its unsubmitted job")
            .capture_ref()
    }
}

#[derive(Default)]
pub(crate) struct InspectNav {
    pub generation: u64,
    active_ticket: Option<InspectTicketRef>,
    cancellation: Option<WorkCancellation<InspectWorkFamily>>,
    pub active_world: Option<Weak<NavWorld>>,
    pub active_generation: Option<u64>,
    pub polling: bool,
    pub submitting: bool,
    pub submitting_id: u64,
    // The phase union and preflights belong to a request, not every idle bot.
    pending: Option<Box<PendingInspect>>,
    pub running_id: u64,
    pub pending_id: u64,
    pub accepted_id: u64,
    pub replaced_id: u64,
    pub replaced_prev_id: u64,
    pub refused: [u64; 3],
    pub latest: Option<InspectTerminal>,
    pub prev: Option<InspectTerminal>,
    pub held: Option<InspectTerminal>,
    pub observed_seq: u64,
    pub executing: bool,
    pub live_world: Option<Arc<NavWorld>>,
    next_revision: u64,
}

impl InspectNav {
    fn next_revision(&mut self) -> Option<api::work::InputRevision> {
        let revision = api::work::InputRevision::new(self.next_revision);
        self.next_revision = self.next_revision.checked_add(1)?;
        Some(revision)
    }
}
#[derive(Clone, Default)]
pub(crate) struct PostedInspect {
    pub(super) latest: Option<InspectTerminal>,
    pub(super) prev: Option<InspectTerminal>,
    pub(super) running_id: u64,
    pub(super) pending_id: u64,
    pub(super) accepted_id: u64,
    pub(super) replaced_id: u64,
    pub(super) replaced_prev_id: u64,
    pub(super) refused_id: u64,
    pub(super) refused_id_2: u64,
    pub(super) refused_id_3: u64,
    pub(super) unobserved: u64,
}

impl InspectNav {
    /// Read-only copy of the slot inspect generation and, when present, the
    /// already-published latest terminal. Callers invoke this only for the
    /// active inspect case. `reset_inspect` bumps generation and clears
    /// terminals; a missing terminal must not be treated as generation 0.
    pub(crate) fn published_core_facts(&self) -> crate::catalog_core::RouteInspectPublished {
        match self.latest.as_ref() {
            Some(latest) => crate::catalog_core::RouteInspectPublished {
                live_generation: self.generation,
                has_terminal: true,
                seq: latest.seq,
                generation: latest.generation,
                request_id: latest.request_id,
                ok: latest.ok,
                reason: latest.reason.as_str().to_owned(),
                hop_loc_names: latest
                    .hops
                    .iter()
                    .map(|hop| hop.loc_name.to_owned())
                    .collect(),
            },
            None => crate::catalog_core::RouteInspectPublished {
                live_generation: self.generation,
                has_terminal: false,
                ..Default::default()
            },
        }
    }

    fn next_seq(&self) -> u64 {
        let cur = self.latest.as_ref().map(|t| t.seq).unwrap_or(0);
        if cur == u64::MAX {
            1
        } else {
            cur + 1
        }
    }

    fn note_replaced(&mut self, id: u64) {
        if id == 0 {
            return;
        }
        self.replaced_prev_id = self.replaced_id;
        self.replaced_id = id;
    }

    fn slot_unobserved(term: Option<&InspectTerminal>, observed_seq: u64) -> bool {
        term.is_some_and(|t| t.seq > observed_seq)
    }

    fn unobserved_count(&self) -> usize {
        usize::from(Self::slot_unobserved(
            self.latest.as_ref(),
            self.observed_seq,
        )) + usize::from(Self::slot_unobserved(self.prev.as_ref(), self.observed_seq))
            + usize::from(self.held.is_some())
    }

    /// Count only unobserved retention plus the jobs that will still
    /// produce a terminal. Observed ring history is not capacity.
    fn can_admit(&self, replacing_pending: bool) -> bool {
        let reserved = usize::from(self.executing) + usize::from(replacing_pending) + 1;
        self.unobserved_count() + reserved <= CAPACITY
    }

    fn is_duplicate(&self, request_id: u64) -> bool {
        request_id != 0
            && (self.running_id == request_id
                || self.submitting_id == request_id
                || self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.capture_ref().request_id == request_id))
    }

    fn note_refused(&mut self, id: u64) {
        if id == 0 || self.refused.contains(&id) {
            return;
        }
        if let Some(slot) = self.refused.iter_mut().find(|slot| **slot == 0) {
            *slot = id;
            return;
        }
        self.refused[0] = self.refused[1];
        self.refused[1] = self.refused[2];
        self.refused[2] = id;
    }

    /// Registered token that cannot reserve storage: identity-only refuse.
    /// Does not overwrite unobserved ring/held terminals. Snapshot-only 0
    /// has no waiter and leaves the previous published latest in place.
    fn refuse_unreserved(&mut self, request_id: u64) {
        self.note_refused(request_id);
    }

    fn max_ackable_seq(&self) -> u64 {
        self.latest
            .as_ref()
            .map(|t| t.seq)
            .unwrap_or(0)
            .max(self.prev.as_ref().map(|t| t.seq).unwrap_or(0))
    }

    fn publish(&mut self, mut term: InspectTerminal) {
        if term.seq == 0 {
            term.seq = self.next_seq();
        }
        if self.latest.is_none() {
            self.latest = Some(term);
            return;
        }
        if self.prev.is_none() {
            self.prev = self.latest.take();
            self.latest = Some(term);
            return;
        }
        if !self.can_wrap() {
            assert!(
                self.held.is_none(),
                "inspect publish overflow: accepted terminal with wrap blocked and held occupied"
            );
            self.held = Some(term);
            return;
        }
        self.prev = self.latest.take();
        self.latest = Some(term);
    }

    fn can_wrap(&self) -> bool {
        let Some(prev) = self.prev.as_ref() else {
            return true;
        };
        self.observed_seq >= prev.seq
    }

    pub(super) fn apply_ack(&mut self, seq: u64, generation: u64) {
        if generation != self.generation || seq == 0 {
            return;
        }
        if seq > self.max_ackable_seq() {
            return;
        }
        if seq > self.observed_seq {
            self.observed_seq = seq;
        }
        if self.held.is_some() && self.can_wrap() {
            if let Some(held) = self.held.take() {
                self.publish(held);
            }
        }
    }

    pub(super) fn posted(&self) -> PostedInspect {
        PostedInspect {
            latest: self.latest.clone(),
            prev: self.prev.clone(),
            running_id: self.running_id,
            pending_id: self.pending_id,
            accepted_id: self.accepted_id,
            replaced_id: self.replaced_id,
            replaced_prev_id: self.replaced_prev_id,
            refused_id: self.refused[0],
            refused_id_2: self.refused[1],
            refused_id_3: self.refused[2],
            unobserved: self.unobserved_count() as u64,
        }
    }

    pub(super) fn clear_published(&mut self) {
        self.latest = None;
        self.prev = None;
        self.held = None;
        self.running_id = 0;
        self.pending_id = 0;
        self.accepted_id = 0;
        self.replaced_id = 0;
        self.replaced_prev_id = 0;
        self.refused = [0; 3];
        self.observed_seq = 0;
    }
}

#[derive(Clone, Debug)]
pub(super) struct InspectRequest {
    pub from: WorldTile,
    pub to: WorldTile,
    pub allow_teleports: bool,
    pub allow_wilderness: bool,
    pub allow_bank_fetch: bool,
    pub zones: ZoneExempt,
    pub avoid: Vec<AvoidRect>,
    pub request_id: u64,
    pub invalid_args: bool,
}

pub(super) fn validate_request(
    from: WorldTile,
    to: WorldTile,
    avoid: &[AvoidRect],
) -> Result<(), &'static str> {
    if !(0..=3).contains(&from.level) || !(0..=3).contains(&to.level) {
        return Err("invalid-args");
    }
    if avoid.len() > MAX_AVOID {
        return Err("invalid-args");
    }
    for rect in avoid {
        if rect.min_x > rect.max_x || rect.min_z > rect.max_z {
            return Err("invalid-args");
        }
        if let Some(level) = rect.level {
            if !(0..=3).contains(&level) {
                return Err("invalid-args");
            }
        }
    }
    Ok(())
}

/// `state` borrows the observe frame's gating facts. An admitted inspection
/// copies them once into its capture, so it keeps that frame's facts.
#[expect(
    clippy::too_many_arguments,
    reason = "The one production call site hands over the slot's navigation table, name, world, state, bank rows, cache and object names from separate owners, plus the request."
)]
pub(super) fn queue_inspect(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    world: &Option<Arc<NavWorld>>,
    state: Option<&WorldState>,
    bank: BankRows,
    cache: Option<Arc<Cache>>,
    obj_names: Option<Arc<ObjNames>>,
    req: InspectRequest,
) {
    let Some(world) = world.clone() else {
        let mut all = navs
            .lock()
            .expect("inspection navigation state mutex poisoned");
        let bot = all.entry(name.to_string()).or_default();
        if bot.inspect.is_duplicate(req.request_id) {
            return;
        }
        if !bot.inspect.can_admit(bot.inspect.pending.is_some()) {
            bot.inspect.refuse_unreserved(req.request_id);
            return;
        }
        if req.invalid_args || validate_request(req.from, req.to, &req.avoid).is_err() {
            refuse_locked(bot, req.request_id, "invalid-args");
        } else {
            refuse_locked(bot, req.request_id, "missing-graph");
        }
        return;
    };

    let lease = nav::producer::lease(&world);
    let (generation, revision, opts, lease) = {
        let mut all = navs
            .lock()
            .expect("inspection navigation state mutex poisoned");
        let bot = all.entry(name.to_string()).or_default();
        if bot.inspect.is_duplicate(req.request_id) {
            return;
        }
        if !bot.inspect.can_admit(bot.inspect.pending.is_some()) {
            bot.inspect.refuse_unreserved(req.request_id);
            return;
        }
        if req.invalid_args || validate_request(req.from, req.to, &req.avoid).is_err() {
            refuse_locked(bot, req.request_id, "invalid-args");
            return;
        }
        let lease = match lease {
            Ok(lease) => lease,
            Err(_) => {
                refuse_locked(bot, req.request_id, "missing-graph");
                return;
            }
        };
        let generation = bot.inspect.generation;
        let Some(revision) = bot.inspect.next_revision() else {
            refuse_locked(bot, req.request_id, "spawn-failed");
            return;
        };
        let mut opts = FindOptions {
            allow_teleports: req.allow_teleports,
            allow_wilderness: req.allow_wilderness,
            allow_bank_fetch: req.allow_bank_fetch,
            zones: req.zones,
            ..FindOptions::default()
        };
        if let Some(essence) = bot.traveller.essence() {
            opts.essence = Some(essence);
        }
        (generation, revision, opts, lease)
    };

    let capture = InspectCapture {
        request_id: req.request_id,
        generation,
        revision,
        world,
        lease,
        state: Arc::new(state.map_or_else(WorldState::empty, WorldState::clone)),
        bank_origin: bank.origin,
        bank: bank.rows,
        from: req.from,
        to: req.to,
        opts,
        avoid: req.avoid,
        cache,
        obj_names,
        #[cfg(test)]
        search_budget_strict: None,
        #[cfg(test)]
        search_budget_stand: None,
        #[cfg(test)]
        search_budget_post: None,
    };
    let (submission, preflights) = match InspectSubmission::for_capture(name, &capture) {
        Ok(preflight) => preflight,
        Err(_) => {
            refuse_current(navs, name, req.request_id, generation, "spawn-failed");
            return;
        }
    };
    let pending = Box::new(PendingInspect {
        job: Some(InspectJob::new(name, capture, preflights)),
        submission,
    });
    let replaced = {
        let mut all = navs
            .lock()
            .expect("inspection navigation state mutex poisoned");
        let bot = all.entry(name.to_string()).or_default();
        if bot.inspect.generation != generation || bot.inspect.is_duplicate(req.request_id) {
            return;
        }
        if !bot.inspect.can_admit(bot.inspect.pending.is_some()) {
            bot.inspect.refuse_unreserved(req.request_id);
            return;
        }
        let replaced = bot.inspect.pending.take();
        if let Some(old) = replaced.as_ref() {
            let old_capture = old.capture_ref();
            bot.inspect.note_replaced(old_capture.request_id);
            bot.inspect.publish(InspectTerminal::refusal(
                old_capture.request_id,
                old_capture.generation,
                "stale",
            ));
        }
        if req.request_id != 0 {
            bot.inspect.accepted_id = req.request_id;
        }
        bot.inspect.live_world = Some(Arc::clone(&pending.capture_ref().world));
        bot.inspect.pending_id = req.request_id;
        bot.inspect.pending = Some(pending);
        replaced
    };
    drop(replaced);
    drive_pending(navs, name);
}

enum SubmitAttempt {
    Accepted {
        ticket: WorkTicket<InspectWorkFamily>,
        cancellation: Option<WorkCancellation<InspectWorkFamily>>,
    },
    Full(Box<PendingInspect>),
    Failed(Box<PendingInspect>),
}

fn refuse_current(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    request_id: u64,
    generation: u64,
    reason: &'static str,
) {
    let mut all = navs
        .lock()
        .expect("inspection navigation state mutex poisoned");
    let Some(bot) = all.get_mut(name) else {
        return;
    };
    if bot.inspect.generation != generation || bot.inspect.is_duplicate(request_id) {
        return;
    }
    if bot.inspect.can_admit(bot.inspect.pending.is_some()) {
        refuse_locked(bot, request_id, reason);
    } else {
        bot.inspect.refuse_unreserved(request_id);
    }
}

fn drive_pending(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    let (mut pending, generation, request_id, world) = {
        let mut all = navs
            .lock()
            .expect("inspection navigation state mutex poisoned");
        let Some(bot) = all.get_mut(name) else {
            return;
        };
        if bot.inspect.active_ticket.is_some() || bot.inspect.polling || bot.inspect.submitting {
            return;
        }
        let Some(pending) = bot.inspect.pending.take() else {
            return;
        };
        let capture = pending.capture_ref();
        let request_id = capture.request_id;
        let generation = capture.generation;
        let world = Arc::downgrade(&capture.world);
        bot.inspect.pending_id = 0;
        bot.inspect.submitting_id = request_id;
        bot.inspect.submitting = true;
        bot.inspect.executing = true;
        (pending, generation, request_id, world)
    };

    let attempt = match inspection_work_service() {
        Ok(service) => {
            let mut admitted = service.lock();
            match admitted.mint_identity(Scope::Process, None) {
                Ok(identity) => {
                    let conditions = ProcessConditions::new(admitted.incarnation());
                    let submission = pending.submission;
                    let job = pending
                        .job
                        .take()
                        .expect("pending inspection submits its job once");
                    match admitted.try_submit::<InspectWorkFamily, _>(
                        identity,
                        &submission,
                        job,
                        &conditions,
                    ) {
                        Ok(ticket) => {
                            let cancellation = admitted.cancellation_handle(ticket).ok();
                            if cancellation.is_none() {
                                let _ = admitted.abandon(ticket, Cancellation::ScopeInvalidated);
                            }
                            SubmitAttempt::Accepted {
                                ticket,
                                cancellation,
                            }
                        }
                        Err(refused) => {
                            let (refusal, job) = refused.into_parts();
                            pending.job = Some(job);
                            if refusal == SubmissionRefusal::Full {
                                SubmitAttempt::Full(pending)
                            } else {
                                SubmitAttempt::Failed(pending)
                            }
                        }
                    }
                }
                Err(_) => SubmitAttempt::Failed(pending),
            }
        }
        Err(_) => SubmitAttempt::Failed(pending),
    };

    let mut discard = None;
    let late_cancellation = {
        let mut attempt = Some(attempt);
        let mut all = navs
            .lock()
            .expect("inspection navigation state mutex poisoned");
        if let Some(bot) = all.get_mut(name) {
            if bot.inspect.generation != generation || bot.inspect.submitting_id != request_id {
                if bot.inspect.submitting_id == request_id {
                    bot.inspect.submitting = false;
                    bot.inspect.submitting_id = 0;
                    bot.inspect.executing = bot.inspect.active_ticket.is_some();
                }
                match attempt.take().expect("submit result is consumed once") {
                    SubmitAttempt::Accepted { cancellation, .. } => cancellation,
                    SubmitAttempt::Full(pending) | SubmitAttempt::Failed(pending) => {
                        discard = Some(pending);
                        None
                    }
                }
            } else {
                bot.inspect.submitting = false;
                bot.inspect.submitting_id = 0;
                match attempt.take().expect("submit result is consumed once") {
                    SubmitAttempt::Accepted {
                        ticket,
                        cancellation,
                    } => {
                        bot.inspect.active_ticket = Some(InspectTicketRef::new(ticket));
                        bot.inspect.cancellation = cancellation;
                        bot.inspect.active_world = Some(world);
                        bot.inspect.active_generation = Some(generation);
                        bot.inspect.running_id = request_id;
                        bot.inspect.executing = true;
                        None
                    }
                    SubmitAttempt::Full(pending) => {
                        if bot.inspect.pending.is_none() {
                            bot.inspect.pending_id = pending.capture_ref().request_id;
                            bot.inspect.pending = Some(pending);
                        } else {
                            let old = pending.capture_ref();
                            bot.inspect.note_replaced(old.request_id);
                            bot.inspect.publish(InspectTerminal::refusal(
                                old.request_id,
                                old.generation,
                                "stale",
                            ));
                            discard = Some(pending);
                        }
                        bot.inspect.executing = bot.inspect.active_ticket.is_some();
                        None
                    }
                    SubmitAttempt::Failed(pending) => {
                        let old = pending.capture_ref();
                        if bot.inspect.pending.is_some() {
                            bot.inspect.note_replaced(old.request_id);
                            bot.inspect.publish(InspectTerminal::refusal(
                                old.request_id,
                                old.generation,
                                "stale",
                            ));
                        } else if old.request_id != 0 {
                            bot.inspect.publish(InspectTerminal::refusal(
                                old.request_id,
                                old.generation,
                                "spawn-failed",
                            ));
                        }
                        discard = Some(pending);
                        bot.inspect.executing = bot.inspect.active_ticket.is_some();
                        None
                    }
                }
            }
        } else {
            match attempt.take().expect("submit result is consumed once") {
                SubmitAttempt::Accepted { cancellation, .. } => cancellation,
                SubmitAttempt::Full(pending) | SubmitAttempt::Failed(pending) => {
                    discard = Some(pending);
                    None
                }
            }
        }
    };
    drop((late_cancellation, discard));
}

enum InspectPollAttempt {
    NotReady,
    OwnerFull,
    Finished { terminal: Option<InspectTerminal> },
    Unavailable,
}

fn reserve_inspect_owner_result(
    output: &InspectJobOutput,
    ledger: &nav::producer::OwnerResultLedger,
) -> Result<nav::producer::OwnerResultReservation, nav::producer::OwnerResultRefusal> {
    let mut retained = match &output.reason {
        InspectMessage::Static(_) => 0,
        InspectMessage::Owned(reason) => reason.capacity(),
    };
    if let Some(route) = &output.route {
        let bytes = nav::producer::compact_route_storage_bytes(route)
            .ok_or(nav::producer::OwnerResultRefusal::SizeOverflow)?;
        add_bytes(&mut retained, bytes).ok_or(nav::producer::OwnerResultRefusal::SizeOverflow)?;
    }
    let bytes = nav::producer::OwnerResultLedger::owner_result_bytes::<InspectStored>(retained)
        .ok_or(nav::producer::OwnerResultRefusal::SizeOverflow)?;
    ledger.try_reserve_bytes(bytes)
}

fn own_inspect_output(
    output: InspectJobOutput,
    ticket: InspectTicketRef,
    reservation: nav::producer::OwnerResultReservation,
) -> Option<InspectTerminal> {
    let InspectJobOutput {
        request_id,
        generation,
        revision: _,
        ok,
        reason,
        bank_planned,
        ticks,
        route,
        lease,
        cache,
        obj_names,
    } = output;
    let route = match route {
        Some(route) => {
            let lease = lease.as_ref()?;
            Some(nav::producer::publish_compact_route(
                lease,
                ticket.identity,
                ticket.revision,
                route,
                reservation.clone(),
            ))
        }
        None => None,
    };
    let stored = InspectStored {
        ok,
        reason,
        bank_planned,
        ticks,
        route,
        cache,
        obj_names,
    };
    Some(InspectTerminal::owned(
        request_id,
        generation,
        reservation.hold(stored),
    ))
}

fn poll_admitted_inspection(
    ticket: InspectTicketRef,
    request_id: u64,
    generation: u64,
    ledger: &nav::producer::OwnerResultLedger,
) -> InspectPollAttempt {
    let Ok(shared) = inspection_work_service() else {
        return InspectPollAttempt::Unavailable;
    };
    let mut service = shared.lock();
    let exact_ticket = ticket.ticket();
    let mut route_too_large = false;
    let reservation = match service.borrow_terminal(&exact_ticket) {
        Ok(Some(Settlement::Completed(output))) => {
            match reserve_inspect_owner_result(output, ledger) {
                Ok(reservation) => Some(reservation),
                Err(nav::producer::OwnerResultRefusal::Full { .. }) => {
                    return InspectPollAttempt::OwnerFull;
                }
                Err(nav::producer::OwnerResultRefusal::TooLarge { .. }) => {
                    route_too_large = true;
                    None
                }
                Err(_) => None,
            }
        }
        Ok(Some(_)) => None,
        Ok(None) => return InspectPollAttempt::NotReady,
        Err(_) => return InspectPollAttempt::Unavailable,
    };
    let settlement = match service.consume(&exact_ticket) {
        Ok(settlement) => settlement,
        Err(_) => return InspectPollAttempt::Unavailable,
    };
    drop(service);
    match settlement {
        Settlement::Completed(output) => {
            if output.request_id != request_id
                || output.generation != generation
                || output.revision != ticket.revision
            {
                return InspectPollAttempt::Finished {
                    terminal: Some(InspectTerminal::refusal(request_id, generation, "stale")),
                };
            }
            if route_too_large {
                drop(output);
                return InspectPollAttempt::Finished {
                    terminal: Some(InspectTerminal::refusal(
                        request_id,
                        generation,
                        "route-too-large",
                    )),
                };
            }
            let Some(reservation) = reservation else {
                return InspectPollAttempt::Finished {
                    terminal: Some(InspectTerminal::refusal(
                        request_id,
                        generation,
                        "spawn-failed",
                    )),
                };
            };
            let terminal = own_inspect_output(output, ticket, reservation);
            InspectPollAttempt::Finished {
                terminal: terminal.or_else(|| {
                    Some(InspectTerminal::refusal(
                        request_id,
                        generation,
                        "spawn-failed",
                    ))
                }),
            }
        }
        Settlement::Failed(_) => InspectPollAttempt::Finished {
            terminal: Some(InspectTerminal::refusal(
                request_id,
                generation,
                "spawn-failed",
            )),
        },
        Settlement::Cancelled(_) => InspectPollAttempt::Finished { terminal: None },
    }
}

pub(super) fn poll_inspection(navs: &Arc<Mutex<HashMap<String, NavBot>>>, name: &str) {
    poll_inspection_with_ledger(navs, name, nav::producer::OwnerResultLedger::shared());
}

fn poll_inspection_with_ledger(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    name: &str,
    ledger: &nav::producer::OwnerResultLedger,
) {
    let (ticket, generation, request_id, active_world) = {
        let mut all = navs
            .lock()
            .expect("inspection navigation state mutex poisoned");
        let Some(bot) = all.get_mut(name) else {
            return;
        };
        if bot.inspect.polling {
            return;
        }
        let Some(ticket) = bot.inspect.active_ticket else {
            drop(all);
            drive_pending(navs, name);
            return;
        };
        let Some(generation) = bot.inspect.active_generation else {
            return;
        };
        let request_id = bot.inspect.running_id;
        bot.inspect.polling = true;
        (
            ticket,
            generation,
            request_id,
            bot.inspect.active_world.clone(),
        )
    };

    let attempt = poll_admitted_inspection(ticket, request_id, generation, ledger);
    let mut cancellation = None;
    let mut discarded = None;
    let mut drive = false;
    {
        let mut all = navs
            .lock()
            .expect("inspection navigation state mutex poisoned");
        if let Some(bot) = all.get_mut(name) {
            let active_matches = bot.inspect.active_ticket == Some(ticket)
                && bot.inspect.active_generation == Some(generation)
                && bot.inspect.running_id == request_id;
            if active_matches {
                bot.inspect.polling = false;
                match attempt {
                    InspectPollAttempt::NotReady | InspectPollAttempt::OwnerFull => {}
                    InspectPollAttempt::Unavailable => {
                        bot.inspect.active_ticket = None;
                        bot.inspect.active_world = None;
                        bot.inspect.active_generation = None;
                        bot.inspect.running_id = 0;
                        bot.inspect.executing = bot.inspect.submitting;
                        cancellation = bot.inspect.cancellation.take();
                        if bot.inspect.generation == generation {
                            bot.inspect.publish(InspectTerminal::refusal(
                                request_id,
                                generation,
                                "spawn-failed",
                            ));
                        }
                        drive = true;
                    }
                    InspectPollAttempt::Finished { mut terminal } => {
                        let world_matches = active_world
                            .as_ref()
                            .and_then(Weak::upgrade)
                            .zip(bot.inspect.live_world.as_ref())
                            .is_some_and(|(active, live)| Arc::ptr_eq(&active, live));
                        bot.inspect.active_ticket = None;
                        bot.inspect.active_world = None;
                        bot.inspect.active_generation = None;
                        bot.inspect.running_id = 0;
                        bot.inspect.executing = bot.inspect.submitting;
                        cancellation = bot.inspect.cancellation.take();
                        if bot.inspect.generation == generation {
                            if !world_matches {
                                discarded = terminal.take();
                                terminal =
                                    Some(InspectTerminal::refusal(request_id, generation, "stale"));
                            } else if terminal.is_none() {
                                terminal =
                                    Some(InspectTerminal::refusal(request_id, generation, "stale"));
                            }
                            if let Some(terminal) = terminal.take() {
                                bot.inspect.publish(terminal);
                            }
                        } else {
                            discarded = terminal.take();
                        }
                        drive = true;
                    }
                }
            } else if let InspectPollAttempt::Finished { terminal } = attempt {
                discarded = terminal;
            }
        } else if let InspectPollAttempt::Finished { terminal } = attempt {
            discarded = terminal;
        }
    }
    drop((cancellation, discarded));
    if drive {
        drive_pending(navs, name);
    }
}

fn refuse_locked(bot: &mut NavBot, request_id: u64, reason: &'static str) {
    if request_id != 0 {
        bot.inspect.accepted_id = request_id;
    }
    bot.inspect.publish(InspectTerminal::refusal(
        request_id,
        bot.inspect.generation,
        reason,
    ));
}

pub(super) fn reset_inspect(bot: &mut NavBot) {
    let submitting = bot.inspect.submitting;
    let cancellation = bot.inspect.cancellation.take();
    bot.inspect.generation = bot.inspect.generation.saturating_add(1);
    bot.inspect.pending = None;
    bot.inspect.pending_id = 0;
    bot.inspect.active_ticket = None;
    bot.inspect.active_world = None;
    bot.inspect.live_world = None;
    bot.inspect.active_generation = None;
    bot.inspect.polling = false;
    bot.inspect.running_id = 0;
    bot.inspect.clear_published();
    bot.inspect.submitting = submitting;
    if !submitting {
        bot.inspect.submitting_id = 0;
    }
    bot.inspect.executing = submitting;
    drop(cancellation);
}

#[cfg(test)]
pub(super) fn calculate(capture: &InspectCapture) -> InspectTerminal {
    let pre = &capture.state;
    let avoid = capture.avoid.as_slice();
    let strict = search_path(
        capture,
        capture.from,
        capture.to,
        capture.opts,
        pre,
        avoid,
        capture.search_budget_strict,
    );
    match strict {
        Ok(route) => ok_terminal(capture, route, false),
        Err(RouteError::BudgetExhausted) => InspectTerminal::oracle_refusal(
            capture.request_id,
            capture.generation,
            "BudgetExhausted",
        ),
        Err(RouteError::NoPath) if !capture.opts.allow_bank_fetch => {
            let reason =
                no_path_reason(capture, capture.from, capture.to, capture.opts, pre, avoid);
            InspectTerminal::oracle_refusal(capture.request_id, capture.generation, &reason)
        }
        Err(RouteError::NoPath) => calculate_bank(capture, pre, avoid),
    }
}

#[cfg(test)]
fn calculate_bank(
    capture: &InspectCapture,
    pre: &WorldState,
    avoid: &[AvoidRect],
) -> InspectTerminal {
    let Some(missing) = search_missing(capture, pre, avoid) else {
        let reason = no_path_reason(capture, capture.from, capture.to, capture.opts, pre, avoid);
        return InspectTerminal::oracle_refusal(capture.request_id, capture.generation, &reason);
    };
    let plan = match plan_bank_fetch(
        &missing,
        pre,
        &planning_rows(capture.bank_origin, &capture.bank, &missing),
        capture.world.banks(),
        capture.from,
        &capture.world.collision,
        &capture.world.graph,
        capture.opts,
        avoid,
    ) {
        Ok(plan) => plan,
        Err(failure) => {
            let reason = match failure {
                BankFetchError::Access {
                    error: RouteError::BudgetExhausted,
                    ..
                } => "BudgetExhausted".into(),
                BankFetchError::Access { blocked_zones, .. } if !blocked_zones.is_empty() => {
                    capture
                        .world
                        .graph
                        .zones
                        .as_ref()
                        .map(|table| crate::blocked_zone_detail(table, &blocked_zones))
                        .unwrap_or_else(|| "NoPath".into())
                }
                _ => "NoPath".into(),
            };
            return InspectTerminal::oracle_refusal(
                capture.request_id,
                capture.generation,
                &reason,
            );
        }
    };
    let bank_planned = true;
    if let Some(stand) = plan.steps.iter().find_map(|step| match step {
        BankStep::Walk { x, z, level } => Some(WorldTile {
            x: *x,
            z: *z,
            level: *level,
        }),
        _ => None,
    }) {
        match search_path(
            capture,
            capture.from,
            stand,
            FindOptions {
                allow_bank_fetch: false,
                ..capture.opts
            },
            pre,
            avoid,
            capture.search_budget_stand,
        ) {
            Ok(_) => {}
            Err(RouteError::BudgetExhausted) => {
                return InspectTerminal::oracle_refusal(
                    capture.request_id,
                    capture.generation,
                    "BudgetExhausted",
                );
            }
            Err(RouteError::NoPath) => {
                let opts = FindOptions {
                    allow_bank_fetch: false,
                    ..capture.opts
                };
                let reason = no_path_reason(capture, capture.from, stand, opts, pre, avoid);
                return InspectTerminal {
                    bank_planned: false,
                    ..InspectTerminal::oracle_refusal(
                        capture.request_id,
                        capture.generation,
                        &reason,
                    )
                };
            }
        }
    }
    let post = search_path(
        capture,
        capture.from,
        capture.to,
        FindOptions {
            allow_bank_fetch: false,
            ..capture.opts
        },
        &plan.state,
        avoid,
        capture.search_budget_post,
    );
    match post {
        Ok(route) => ok_terminal(capture, route, bank_planned),
        Err(RouteError::BudgetExhausted) => InspectTerminal {
            bank_planned,
            ..InspectTerminal::oracle_refusal(
                capture.request_id,
                capture.generation,
                "BudgetExhausted",
            )
        },
        Err(RouteError::NoPath) => {
            let opts = FindOptions {
                allow_bank_fetch: false,
                ..capture.opts
            };
            let reason =
                no_path_reason(capture, capture.from, capture.to, opts, &plan.state, avoid);
            InspectTerminal {
                bank_planned,
                ..InspectTerminal::oracle_refusal(capture.request_id, capture.generation, &reason)
            }
        }
    }
}

#[cfg(test)]
fn no_path_reason(
    capture: &InspectCapture,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
) -> String {
    nav::router::find_blocking_zones(
        &capture.world.collision,
        &capture.world.graph,
        from,
        to,
        opts,
        state,
        avoid,
    )
    .filter(|keys| !keys.is_empty())
    .and_then(|keys| {
        capture
            .world
            .graph
            .zones
            .as_ref()
            .map(|table| crate::blocked_zone_detail(table, &keys))
    })
    .unwrap_or_else(|| "NoPath".into())
}

#[cfg(test)]
fn search_path(
    capture: &InspectCapture,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    avoid: &[AvoidRect],
    budget: Option<usize>,
) -> Result<Route, RouteError> {
    match budget {
        Some(n) => find_with_avoid_bounded(
            &capture.world.collision,
            &capture.world.graph,
            from,
            to,
            opts,
            state,
            avoid,
            n,
        ),
        None => find_with_avoid(
            &capture.world.collision,
            &capture.world.graph,
            from,
            to,
            opts,
            state,
            avoid,
        ),
    }
}

#[cfg(test)]
fn search_missing(
    capture: &InspectCapture,
    pre: &WorldState,
    avoid: &[AvoidRect],
) -> Option<Vec<nav::router::MissingReq>> {
    match capture.search_budget_strict {
        Some(n) => find_missing_item_reqs_with_avoid_bounded(
            &capture.world.collision,
            &capture.world.graph,
            capture.from,
            capture.to,
            capture.opts,
            pre,
            avoid,
            n,
        ),
        None => find_missing_item_reqs_with_avoid(
            &capture.world.collision,
            &capture.world.graph,
            capture.from,
            capture.to,
            capture.opts,
            pre,
            avoid,
        ),
    }
}

#[cfg(test)]
fn ok_terminal(capture: &InspectCapture, route: Route, bank_planned: bool) -> InspectTerminal {
    let hops = route
        .legs
        .iter()
        .filter_map(|leg| match leg {
            Leg::Transport { edge } => Some(project_hop(
                edge,
                capture.cache.as_deref(),
                capture.obj_names.as_deref(),
            )),
            Leg::Walk { .. } => None,
        })
        .collect();
    InspectTerminal::oracle(
        capture.request_id,
        capture.generation,
        true,
        String::new(),
        bank_planned,
        route.ticks,
        hops,
    )
}
fn hop_texts<'a>(
    kind: TransportKind,
    loc_id: i32,
    option: i32,
    cache: Option<&'a Cache>,
    obj_names: Option<&'a ObjNames>,
) -> (&'a str, &'a str) {
    match kind {
        TransportKind::Boat | TransportKind::Npc | TransportKind::Glider => cache
            .and_then(|cache| cache.npcs.get(loc_id as usize))
            .map(|npc| (npc.name.as_str(), op_at_ref(&npc.op, option)))
            .unwrap_or(("", "")),
        TransportKind::Door
        | TransportKind::Ladder
        | TransportKind::Stairs
        | TransportKind::AgilityShortcut
        | TransportKind::SpiritTree
        | TransportKind::EssenceExit => cache
            .and_then(|cache| cache.locs.get(loc_id as usize))
            .map(|loc| (loc.name.as_str(), op_at_ref(&loc.op, option)))
            .unwrap_or(("", "")),
        TransportKind::Teleport if loc_id > 0 => (
            obj_names.and_then(|names| names.name(loc_id)).unwrap_or(""),
            cache
                .and_then(|cache| cache.objs.get(loc_id as usize))
                .map(|object| op_at_ref(&object.op, option))
                .unwrap_or(""),
        ),
        TransportKind::Teleport => ("", ""),
    }
}

fn op_at_ref(ops: &[Option<String>], option: i32) -> &str {
    if option <= 0 {
        return "";
    }
    ops.get((option as usize).saturating_sub(1))
        .and_then(Option::as_deref)
        .unwrap_or("")
}

#[cfg(test)]
fn project_hop(
    edge: &nav::transport::TransportEdge,
    cache: Option<&Cache>,
    obj_names: Option<&ObjNames>,
) -> InspectHop {
    let kind = kind_name(edge.kind);
    let (loc_name, action) = match edge.kind {
        TransportKind::Boat | TransportKind::Npc | TransportKind::Glider => {
            let npc = cache.and_then(|c| c.npcs.get(edge.loc_id as usize));
            (
                npc.map(|n| n.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_default(),
                npc.map(|n| op_at(&n.op, edge.option)).unwrap_or_default(),
            )
        }
        TransportKind::Door
        | TransportKind::Ladder
        | TransportKind::Stairs
        | TransportKind::AgilityShortcut
        | TransportKind::SpiritTree
        | TransportKind::EssenceExit => {
            let loc = cache.and_then(|c| c.locs.get(edge.loc_id as usize));
            (
                loc.map(|l| l.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_default(),
                loc.map(|l| op_at(&l.op, edge.option)).unwrap_or_default(),
            )
        }
        TransportKind::Teleport if edge.loc_id > 0 => {
            let name = obj_names
                .and_then(|names| names.name(edge.loc_id))
                .unwrap_or("")
                .to_string();
            let action = cache
                .and_then(|c| c.objs.get(edge.loc_id as usize))
                .map(|o| op_at_obj(&o.op, edge.option))
                .unwrap_or_default();
            (name, action)
        }
        TransportKind::Teleport => (String::new(), String::new()),
    };
    InspectHop {
        kind: kind.into(),
        loc_id: edge.loc_id,
        loc_name,
        action,
        option: edge.option,
        from_x: edge.at.x,
        from_z: edge.at.z,
        from_level: edge.at.level,
        to_x: edge.to.x,
        to_z: edge.to.z,
        to_level: edge.to.level,
        ticks: edge.ticks,
    }
}

fn kind_name(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::Door => "door",
        TransportKind::Ladder => "ladder",
        TransportKind::Stairs => "stairs",
        TransportKind::Boat => "boat",
        TransportKind::Teleport => "teleport",
        TransportKind::AgilityShortcut => "agilityshortcut",
        TransportKind::Glider => "glider",
        TransportKind::SpiritTree => "spirittree",
        TransportKind::Npc => "npc",
        TransportKind::EssenceExit => "essenceexit",
    }
}

#[cfg(test)]
fn op_at(ops: &[Option<String>], option: i32) -> String {
    if option <= 0 {
        return String::new();
    }
    ops.get((option as usize).saturating_sub(1))
        .and_then(|op| op.clone())
        .unwrap_or_default()
}

#[cfg(test)]
fn op_at_obj(ops: &[Option<String>; 5], option: i32) -> String {
    op_at(ops.as_slice(), option)
}

pub(super) fn terminal_to_input<'a>(
    term: &'a InspectTerminal,
    hops: &'a [script::isolate_fb::InspectHopInput<'a>],
) -> script::isolate_fb::RouteInspectTerminalInput<'a> {
    script::isolate_fb::RouteInspectTerminalInput {
        seq: term.seq,
        generation: term.generation,
        request_id: term.request_id,
        ok: term.ok,
        reason: Some(term.reason.as_str()),
        bank_planned: term.bank_planned,
        ticks: term.ticks,
        hops,
    }
}

pub(super) fn hop_inputs(hops: &InspectHops) -> Vec<script::isolate_fb::InspectHopInput<'_>> {
    hops.iter()
        .map(|hop| script::isolate_fb::InspectHopInput {
            kind: hop.kind,
            loc_id: hop.loc_id,
            loc_name: hop.loc_name,
            action: hop.action,
            option: hop.option,
            from_x: hop.from_x,
            from_z: hop.from_z,
            from_level: hop.from_level,
            to_x: hop.to_x,
            to_z: hop.to_z,
            to_level: hop.to_level,
            ticks: hop.ticks,
        })
        .collect()
}

#[cfg(test)]
#[path = "route_inspect_tests.rs"]
mod tests;
