use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use api::bank_memory::Origin;
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, SnapshotView, WorldTile};
use nav::arrival::ArrivalKind;
use nav::bank_fetch::{plan_bank_fetch, planning_rows, BankRows, BankStep};
use nav::router::{
    find_first_with_avoid, find_first_with_fallback_avoid, find_missing_item_reqs_with_avoid,
    AvoidRect, FallbackRoute, FindOptions, MissingReq, Route,
};
use nav::traveller::Traveller;
use nav::world::NavWorld;
use nav::zones::{ZoneExempt, ZoneKey};
use nav::WorldState;

use super::{
    debug_enabled, fetch_stand, fetch_tile, fetchable_facts, route_inspect, route_or_bank_fetch,
    PendingBankFetch, RouteOutcome, StandFetch,
};
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
        #[cfg(test)]
        if let Some(sender) = self.sender {
            let _ = sender.send(());
        }
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

/// Per-uid nav state: the whole-world traveller plus the route it is
/// following. `ctx.walk` stores the route (found off-pump over the shared
/// [`NavWorld`]); the slot pump polls [`Traveller::follow`] with a clone of
/// it one step per player-info tick. `route` being set is the "armed"
/// gate the walk hook and the busy flag read. A pending BankBudget
/// session freezes follow until its steps finish.
#[derive(Default)]
pub(crate) struct NavBot {
    /// Shared session globals, attached before a production slot starts.
    pub(crate) walk_globals: Option<Arc<Mutex<super::WalkGlobals>>>,
    pub(crate) walk_globals_store: Option<Arc<std::path::PathBuf>>,
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
    pub(crate) assess_worker: Option<Arc<()>>,
    pub(crate) manual_arm: Option<std::sync::Weak<Mutex<crate::WalkArm>>>,
    pub(crate) route_generation: u64,
    pub(crate) map_route_generation: u64,
    pub(crate) route_worker: Option<Arc<()>>,
    pub(crate) pending_route: Option<Box<ScriptRouteRequest>>,
    pub(crate) pending_assess: Option<Box<PendingAdvisoryAssess>>,
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
    pub(crate) route_quest_evidence: Option<nav::quest_gates::QuestEvidence>,
    /// Dest, radius, FindOptions bits, and the per-walk zone exemptions.
    pub(crate) requested_route: Option<(WorldTile, i32, bool, bool, bool, ZoneExempt)>,
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
    /// Distinct from `route_generation`, which remains the worker / retained-route token.
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
    /// ([`nav::traveller::HopFailure::EndBlocked`]).
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

/// The shared arm for queued script walks and host bank-stand walks.
/// Routing stays off-pump; the slot pump owns follow and dispatch.
#[derive(Clone)]
pub(crate) struct ScriptWalkArm {
    pub(crate) here: Option<(i32, i32, i32)>,
    pub(crate) world: Option<Arc<NavWorld>>,
    pub(crate) navs: Arc<Mutex<HashMap<String, NavBot>>>,
    pub(crate) name: String,
    /// The slot's gating facts at arm time (from its live snapshot);
    /// `None` when no player is decoded — the worker then routes with
    /// the fail-closed empty [`WorldState`].
    pub(crate) state: Option<WorldState>,
    /// The account's bank memory rows at arm time
    /// ([`super::slot_bank_memory::planner_rows`]): what a BankBudget
    /// session is planned over ([`nav::bank_fetch::planning_rows`]).
    pub(crate) bank: BankRows,
}

fn walk_arm_outcome_tag(outcome: &RouteOutcome) -> &'static str {
    match outcome {
        RouteOutcome::Routed(_) => "Routed",
        RouteOutcome::BankSession { .. } => "BankSession",
        RouteOutcome::NoPath => "NoPath",
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

fn walk_arm_worker_slot() -> String {
    thread::current()
        .name()
        .and_then(|n| n.strip_prefix("nav-find-").map(str::to_string))
        .unwrap_or_else(|| "?".to_string())
}

pub(super) fn log_walk_arm_bot(build: impl FnOnce() -> String) {
    api::host_log!(
        api::hostlog::Category::NavTrace,
        api::hostlog::Level::Debug,
        "walk-arm {}",
        build()
    );
}
fn format_zone_witness(table: &nav::zones::ZoneTable, keys: &[ZoneKey]) -> String {
    keys.iter()
        .map(|&key| format!("{} [{}]", table.label(key), table.name(key)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn blocked_zone_detail(table: &nav::zones::ZoneTable, keys: &[ZoneKey]) -> String {
    format!(
        "blocked by danger zones: {}",
        format_zone_witness(table, keys)
    )
}

/// How a user lets a script walk cross the zones its refusal names. WalkTo
/// carries its own one-walk hint (`walk_map/actions.rs`); compat scripts get
/// the `crossZones` hint from [`compat_zone_no_route_line`].
const SCRIPT_DANGER_HINT: &str =
    "to allow it, set Danger routing to Always in Nav config, or allow danger zones in the script's own settings if it has that option";

pub(crate) fn compat_zone_no_route_line(table: &nav::zones::ZoneTable, keys: &[ZoneKey]) -> String {
    let names = keys
        .iter()
        .map(|&key| format!("{:?}", table.name(key)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "no route: {} — pass crossZones: [{names}] to cross",
        blocked_zone_detail(table, keys)
    )
}

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
                if !nav::zones::AVOID_CATALOG_IDS.contains(&id.as_str()) {
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
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
            if let Some(pending) = bot.pending_assess.take() {
                pending.request.completion.signal();
            }
            bot.assess_worker = None;
            bot.assess_result = Some(Box::new((authority.clone(), receipt)));
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
        if let (Some(provider), Some(family)) = (request.evidence, world.graph.quest_family) {
            self.state
                .get_or_insert_with(Default::default)
                .quest_evidence = Some(nav::quest_gates::QuestEvidence::new(
                provider,
                family,
                request.required_after,
            ));
        }
        let from = WorldTile { x, z, level };
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
        let mut input = crate::admission::capture(
            snapshot,
            state.map_members,
            script::combat::risk::PoisonState::default(),
            false,
        );
        let admission = {
            let navs = self.navs.lock().unwrap();
            let bot = navs.get(&self.name);
            input.off_debt = bot.is_some_and(|bot| bot.walk_guard_off.is_some());
            crate::admission::Admission {
                policy,
                enforce: enforce || !request.cross.is_empty(),
                grants: options.zones,
                allow: request.allow,
                input,
                generation: authority.request_id().get(),
                compat_v1: false,
                escape: None,
            }
        };
        let radius = i32::from(request.radius);
        let live_candidates = if radius > 0 {
            request
                .loc_id
                .and_then(|id| loc_target_approach_tiles(snapshot, request.target, radius, id))
        } else {
            None
        };
        let request = ScriptRouteRequest {
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
            bank_origin: self.bank.origin,
            bank: self.bank.rows,
            live_candidates,
            exclusions: (!exclusions.is_empty()).then(|| Arc::new(exclusions)),
            completion: RouteCompletion::default(),
        };
        let token = Arc::new(());
        let worker_token = {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.entry(self.name.clone()).or_default();
            bot.assess_worker = Some(Arc::clone(&token));
            bot.assess_result = None;
            bot.pending_assess = Some(Box::new(PendingAdvisoryAssess {
                request: Box::new(request),
                authority: authority.clone(),
                evidence,
                token: Arc::clone(&token),
            }));
            if bot.route_worker.is_some() {
                None
            } else {
                let worker_token = Arc::new(());
                bot.route_worker = Some(Arc::clone(&worker_token));
                Some(worker_token)
            }
        };
        let Some(worker_token) = worker_token else {
            return;
        };
        if spawn_route_worker(
            Arc::clone(&self.navs),
            self.name.clone(),
            Arc::clone(&worker_token),
        ) {
            return;
        }
        let mut navs = self.navs.lock().unwrap();
        let Some(bot) = navs.get_mut(&self.name) else {
            return;
        };
        if !bot
            .route_worker
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &worker_token))
        {
            return;
        }
        bot.route_worker = None;
        if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
            bot.note_failure(
                bot.route_generation,
                bot.walk_request_id,
                to,
                radius,
                allow_teleports,
            );
        }
        if let Some(request) = bot.pending_route.take() {
            request.completion.signal();
        }
        bot.requested_route = None;
        if let Some(pending) = bot.pending_assess.take() {
            pending.request.completion.signal();
            if pending.authority.live()
                && bot
                    .assess_worker
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &pending.token))
            {
                bot.assess_worker = None;
                bot.assess_result = Some(Box::new((
                    pending.authority.clone(),
                    advisory_refusal(
                        &pending.authority,
                        pending.evidence,
                        "Advisory route worker could not start",
                    ),
                )));
            }
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
        let off_debt = self
            .navs
            .lock()
            .unwrap()
            .get(&self.name)
            .is_some_and(|bot| bot.walk_guard_off.is_some());
        let admission = crate::admission::Admission {
            policy,
            enforce: enforce || !request.cross.is_empty(),
            grants: options.zones,
            allow: request.allow,
            input: crate::admission::capture(
                snapshot,
                self.state.as_ref().is_some_and(|state| state.map_members),
                script::combat::risk::PoisonState::default(),
                off_debt,
            ),
            generation: 0,
            compat_v1: false,
            escape: None,
        };
        if let (Some(provider), Some(family)) = (
            request.evidence,
            self.world
                .as_ref()
                .and_then(|world| world.graph.quest_family),
        ) {
            self.state
                .get_or_insert_with(Default::default)
                .quest_evidence = Some(nav::quest_gates::QuestEvidence::new(
                provider,
                family,
                request.required_after,
            ));
        }
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
        &self,
        snapshot: &GameSnapshot,
        to: WorldTile,
        radius: i32,
        opts: FindOptions,
        request_id: u64,
        exclusions: ScriptRouteExclusions,
        authority: Option<script::native::HostAuthority>,
        loc_id: Option<i32>,
        arrival: ArrivalKind,
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
        self.queue_route_impl(
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
        )
    }

    /// Queue one walk toward `(x, z, level)` with `opts`, routing off-pump
    /// on a short-lived worker (`find_with` over the shared [`NavWorld`]).
    /// When `allow_bank_fetch` is on and the strict find fails only on
    /// missing item/worn reqs, latches a [`PendingBankFetch`] session and
    /// the post-session route. Refuses synchronously only when there is
    /// no player tile, no nav world, or a route/session already queued;
    /// the worker stores the outcome on the uid's nav bot. Returns whether
    /// the worker was spawned — not whether a path exists.
    #[allow(clippy::too_many_arguments)] // plus the borrowed arm-time scene
    pub(crate) fn queue_route_in_snapshot(
        &self,
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
        )
    }

    /// Queue a walk with the request's frozen avoid rectangles and zone names.
    #[allow(clippy::too_many_arguments)] // route queue plus the walk's exclusions
    pub(crate) fn queue_fixed_route_in_snapshot_avoiding(
        &self,
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
        )
    }

    /// [`Self::queue_route_in_snapshot`] for a walk with request exclusions.
    #[allow(clippy::too_many_arguments)] // plus the borrowed arm-time scene and exclusions
    pub(crate) fn queue_route_in_snapshot_avoiding(
        &self,
        snapshot: &GameSnapshot,
        x: i32,
        z: i32,
        level: i32,
        opts: FindOptions,
        radius: i32,
        request_id: u64,
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
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)] // deterministic worker seam for route tests
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
        self.queue_route_impl(
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
        if bot.bank_fetch.is_some()
            || (!retarget && (bot.route.is_some() || bot.route_worker.is_some()))
        {
            log_walk_arm(&self.name, || {
                format!(
                    "queue_route refused-in-flight dest={to:?} r={radius} request_id={request_id} \
                     bank_fetch={} retarget={retarget} route={} worker={}",
                    bot.bank_fetch.is_some(),
                    bot.route.is_some(),
                    bot.route_worker.is_some()
                )
            });
            refuse(bot);
            return Some(false);
        }
        if bot.requested_route == Some(key)
            && bot.requested_exclusions.as_deref() == exclusions.as_deref()
            && bot.route_loc_id == loc_id
            && bot.route_arrival == arrival
            && (bot.route_worker.is_some() || bot.route.is_some() || bot.pending_route.is_some())
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

    #[allow(clippy::too_many_arguments)] // queue state plus borrowed arm-time scene
    fn queue_route_impl(
        &self,
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
        let mut state = self.state.clone();
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
                        self.state.as_ref().is_some_and(|state| state.map_members),
                        Default::default(),
                        false,
                    )
                })
            });
        // Resolve live-scene geometry only after this request passes the
        // refusal/coalescing gates, but outside the process-wide nav mutex.
        // The second gate below closes the race with another arming thread.
        let live_candidates = if radius > 0 {
            snapshot.and_then(|snapshot| {
                loc_id.and_then(|id| loc_target_approach_tiles(snapshot, to, radius, id))
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
        let token = {
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
            ) {
                return result;
            }
            if let Some(ess) = bot.traveller.essence() {
                opts.essence = Some(ess);
            }
            // Refresh retains its original policy and corridor; a new request
            // derives its own policy instead of borrowing the previous override.
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
            admission.input.off_debt = bot.walk_guard_off.is_some();
            admission.grants = opts.zones;
            admission.generation = bot.route_generation.wrapping_add(1);
            match bot.admission.as_deref_mut() {
                Some(pending) => *pending = admission,
                None => bot.admission = Some(Box::new(admission)),
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
            bot.native_walk = authority;
            bot.native_walk_failure = None;
            bot.pending_route = Some(Box::new(ScriptRouteRequest {
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
                bank_origin: self.bank.origin,
                bank: self.bank.rows.clone(),
                live_candidates,
                exclusions,
                completion,
            }));
            if bot.route_worker.is_some() {
                log_walk_arm(&self.name, || {
                    format!(
                        "queue_route handed-pending dest={to:?} r={radius} request_id={request_id} \
                         generation={} from={from:?}{}",
                        bot.route_generation,
                        if world.graph.zones.is_none() {
                            " zones: unavailable (legacy grid pack)"
                        } else {
                            ""
                        }
                    )
                });
                return true;
            }
            let token = Arc::new(());
            bot.route_worker = Some(Arc::clone(&token));
            token
        };
        log_walk_arm(&self.name, || {
            format!(
                "queue_route spawned dest={to:?} r={radius} request_id={request_id} from={from:?} \
                 allow_teleports={} allow_wilderness={} allow_bank_fetch={}{}",
                opts.allow_teleports,
                opts.allow_wilderness,
                opts.allow_bank_fetch,
                if world.graph.zones.is_none() {
                    " zones: unavailable (legacy grid pack)"
                } else {
                    ""
                }
            )
        });
        let spawned = spawn_route_worker(
            Arc::clone(&self.navs),
            self.name.clone(),
            Arc::clone(&token),
        );
        if !spawned {
            log_walk_arm(&self.name, || {
                format!("queue_route spawn-failed dest={to:?} r={radius} request_id={request_id}")
            });
            if let Some(bot) = self.navs.lock().unwrap().get_mut(&self.name) {
                if bot
                    .route_worker
                    .as_ref()
                    .is_some_and(|t| Arc::ptr_eq(t, &token))
                {
                    if let Some((to, radius, allow_teleports, ..)) = bot.requested_route {
                        bot.note_failure(
                            bot.route_generation,
                            bot.walk_request_id,
                            to,
                            radius,
                            allow_teleports,
                        );
                    }
                    bot.route_worker = None;
                    if let Some(request) = bot.pending_route.take() {
                        request.completion.signal();
                    }
                    bot.requested_route = None;
                    if let Some(pending) = bot.pending_assess.take() {
                        pending.request.completion.signal();
                        if bot
                            .assess_worker
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &pending.token))
                        {
                            bot.assess_worker = None;
                            if pending.authority.live() {
                                bot.assess_result = Some(Box::new((
                                    pending.authority.clone(),
                                    advisory_refusal(
                                        &pending.authority,
                                        pending.evidence,
                                        "Shared route worker could not start",
                                    ),
                                )));
                            }
                        }
                    }
                }
            }
        }
        spawned
    }
}

enum RouteWorkerTask {
    Route(
        Box<ScriptRouteRequest>,
        Option<script::native::HostAuthority>,
    ),
    Assess(Box<PendingAdvisoryAssess>),
}
fn spawn_route_worker(
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    name: String,
    token: Arc<()>,
) -> bool {
    let worker_token = Arc::clone(&token);
    let spawned = thread::Builder::new()
        .name(format!("nav-find-{name}"))
        .spawn(move || loop {
            let task = {
                let mut all = navs.lock().unwrap();
                let Some(bot) = all.get_mut(&name) else {
                    log_walk_arm(&name, || "worker exit bot-gone".to_string());
                    return;
                };
                if !bot
                    .route_worker
                    .as_ref()
                    .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
                {
                    log_walk_arm(&name, || {
                        "worker discard stale-token before dequeue".to_string()
                    });
                    return;
                }
                if let Some(request) = bot.pending_route.take() {
                    RouteWorkerTask::Route(request, bot.native_walk.clone())
                } else if let Some(pending) = bot.pending_assess.take() {
                    RouteWorkerTask::Assess(pending)
                } else {
                    bot.route_worker = None;
                    log_walk_arm(&name, || "worker exit no-pending-work".to_string());
                    return;
                }
            };
            match task {
                RouteWorkerTask::Route(request, authority) => {
                if authority.as_ref().is_some_and(|owner| !owner.live()) {
                    request.completion.signal();
                    continue;
                }
                let debug = debug_enabled();
                if debug {
                    log_walk_arm(&name, || {
                        format!(
                            "worker calculate begin generation={} request_id={} from={:?} dest={:?} r={}",
                            request.generation,
                            request.request_id,
                            request.from,
                            request.to,
                            request.radius
                        )
                    });
                }
                let started = debug.then(Instant::now);
                let (admitted, targets) = request.calculate_admitted();
                let crate::admission::RouteAdmission { outcome, assessment, refusal, blocked, tried: _ } = admitted;
                let outcome = if refusal.is_some() { RouteOutcome::NoPath } else { outcome };
                if debug {
                    let elapsed_ms = started.unwrap().elapsed().as_millis();
                    log_walk_arm(&name, || {
                        format!(
                            "worker calculate end generation={} request_id={} elapsed_ms={elapsed_ms} \
                             outcome={}",
                            request.generation,
                            request.request_id,
                            walk_arm_outcome_tag(&outcome)
                        )
                    });
                }
                let blocked = (!blocked.is_empty()).then(|| blocked.into_vec());
                let route_detail = blocked
                    .as_ref()
                    .and_then(|keys| {
                        request
                            .world
                            .graph
                            .zones
                            .as_ref()
                            .map(|table| format!("{}; {SCRIPT_DANGER_HINT}", blocked_zone_detail(table, keys)))
                    })
                    .or_else(|| {
                        request
                            .world
                            .graph
                            .zones
                            .is_none()
                            .then(|| crate::walk_map::LEGACY_ZONES_DETAIL.to_owned())
                    });
                let route_detail = assessment.as_ref().filter(|_| refusal.is_some())
                    .map(|assessment| assessment.reason.to_string()).or(route_detail);
                let mut route_detail = if authority.is_some()
                    && matches!(&outcome, &RouteOutcome::NoPath)
                {
                    let context = format!(
                        "native walk route failed: {:?} arrival from {:?} to {:?} within radius {}",
                        request.arrival, request.from, request.to, request.radius
                    );
                    Some(match route_detail {
                        Some(detail) => format!("{detail}; {context}"),
                        None => context,
                    })
                } else {
                    route_detail
                };
                if let (Some(keys), Some(table)) =
                    (blocked.as_ref(), request.world.graph.zones.as_ref())
                {
                    api::host_log!(
                        api::hostlog::Category::NavTrace,
                        api::hostlog::Level::Warn,
                        slot = &name,
                        "{}",
                        compat_zone_no_route_line(table, keys)
                    );
                }
                let native_failure = if refusal.is_some() {
                    Some(script::native::WalkEnd::Refused)
                } else { request.native_failure(&outcome, blocked.as_deref(), &targets) };
                let native_refused = native_failure == Some(script::native::WalkEnd::Refused);
                let mut all = navs.lock().unwrap();
                let Some(bot) = all.get_mut(&name) else {
                    log_walk_arm(&name, || "worker exit bot-gone after calculate".to_string());
                    return;
                };
                if !bot
                    .route_worker
                    .as_ref()
                    .is_some_and(|t| Arc::ptr_eq(t, &worker_token))
                {
                    log_walk_arm(&name, || {
                        format!(
                            "worker discard stale-token after calculate generation={} \
                             live_generation={}",
                            request.generation, bot.route_generation
                        )
                    });
                    return;
                }
                if authority.as_ref().is_some_and(|owner| !owner.live()) {
                    request.completion.signal();
                    continue;
                }
                let missing = if refusal.is_some() { Vec::new() } else { request.missing_carry(&outcome) };
                if !missing.is_empty() {
                    let empty_state = WorldState::empty();
                    let state = request.state.as_ref().unwrap_or(&empty_state);
                    if let Some(detail) = crate::walk_map::route_supply_shortfall_detail(
                        &request.world,
                        state,
                        missing.iter().map(|row| (row.id, row.count)),
                    ) {
                        let supply_detail =
                            crate::walk_map::ActionError::InsufficientItems { detail }.to_string();
                        route_detail = Some(match route_detail.take() {
                            Some(nav_detail) => format!("{supply_detail}; {nav_detail}"),
                            None => supply_detail,
                        });
                    }
                }

                if bot.route_generation == request.generation && bot.walk_request_id == request.request_id {
                    bot.risk_refusal = refusal.map(|refusal| Box::new((request.request_id, refusal)));
                    bot.assessment = assessment;
                    if let Some(assessment) = bot.assessment.as_ref() {
                        crate::admission::log(Some(&name), assessment, request.admission.compat_v1);
                        if bot.risk_refusal.is_some() {
                            bot.last_assessment = Some(Arc::clone(assessment));
                        }
                    }
                }
                // Only a route this publish installs carries its evidence; a
                // newer request's NoPath leaves the followed route's own.
                if bot.route_generation == request.generation
                    && bot.walk_request_id == request.request_id
                    && !matches!(outcome, RouteOutcome::NoPath)
                {
                    bot.route_quest_evidence = request
                        .state
                        .as_ref()
                        .and_then(|state| state.quest_evidence.clone());
                }
                let has_route = matches!(
                    &outcome,
                    RouteOutcome::Routed(_) | RouteOutcome::BankSession { .. }
                );
                bot.publish_route(
                    request.generation,
                    request.request_id,
                    request.opts.allow_teleports,
                    outcome,
                );
                if bot.route_generation == request.generation
                    && bot.walk_request_id == request.request_id
                    && has_route
                {
                    if bot.route_basis.is_none() {
                        if let Some(route) = bot.route.as_ref() {
                            bot.route_basis = Some(Arc::new(crate::admission::RouteBasis {
                                route: Arc::clone(route),
                                options: request.opts,
                                avoid: Arc::from(request.avoid()),
                                quest_evidence: request
                                    .state
                                    .as_ref()
                                    .and_then(|state| state.quest_evidence.clone()),
                            }));
                        }
                    }
                    bot.admission_pending = true;
                }
                // The diagnosis lands under the same lock as the outcome it
                // belongs to, so the packer can never read a list beside
                // another failure. A discarded publish (stale generation)
                // leaves the list it did not name cleared.
                bot.note_missing_carry(request.generation, request.request_id, missing);
                if bot.walk_outcome_generation == request.generation
                    && bot.walk_outcome_request_id == request.request_id
                {
                    bot.native_walk_failure =
                        native_failure.map(|end| (request.request_id, end));
                    bot.walk_outcome_detail = route_detail.map(Arc::from);
                    bot.native_walk_blocked = if native_refused {
                        blocked.map(|keys| (request.request_id, Arc::<[ZoneKey]>::from(keys)))
                    } else {
                        None
                    };
                }
                request.completion.signal();
                }
                RouteWorkerTask::Assess(pending) => {
                    if !pending.authority.live() {
                        let mut all = navs.lock().unwrap();
                        if let Some(bot) = all.get_mut(&name) {
                            if bot
                                .assess_worker
                                .as_ref()
                                .is_some_and(|current| Arc::ptr_eq(current, &pending.token))
                            {
                                bot.assess_worker = None;
                            }
                        }
                        drop(all);
                        pending.request.completion.signal();
                        continue;
                    }
                    let (admitted, _) = pending.request.calculate_admitted();
                    let route_ticks = match &admitted.outcome {
                        RouteOutcome::Routed(route) | RouteOutcome::BankSession { route, .. } => {
                            script::combat::risk::RoutePath::new(route)
                                .ok()
                                .and_then(|path| u32::try_from(path.ticks()).ok())
                                .unwrap_or(0)
                        }
                        RouteOutcome::NoPath => 0,
                    };
                    let detail = admitted
                        .assessment
                        .as_ref()
                        .map(|assessment| Arc::clone(&assessment.reason))
                        .or_else(|| {
                            matches!(&admitted.outcome, RouteOutcome::NoPath).then(|| {
                                Arc::from("No route within the request's hard constraints")
                            })
                        });
                    let receipt = script::native::AssessReceipt {
                        request_id: pending.authority.request_id().get(),
                        evidence: pending.evidence,
                        assessment: admitted.assessment,
                        refusal: admitted.refusal.or_else(|| {
                            matches!(&admitted.outcome, RouteOutcome::NoPath).then_some(
                                script::native::WalkRefusal::NoRouteWithinBounds {
                                    tried: admitted.tried,
                                    last: None,
                                },
                            )
                        }),
                        detail,
                        route_ticks,
                    };
                    let mut all = navs.lock().unwrap();
                    let Some(bot) = all.get_mut(&name) else {
                        pending.request.completion.signal();
                        return;
                    };
                    if !bot
                        .route_worker
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &worker_token))
                    {
                        pending.request.completion.signal();
                        return;
                    }
                    if !pending.authority.live() {
                        if bot
                            .assess_worker
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &pending.token))
                        {
                            bot.assess_worker = None;
                        }
                        pending.request.completion.signal();
                        continue;
                    }
                    if bot
                        .assess_worker
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &pending.token))
                    {
                        bot.assess_worker = None;
                        bot.assess_result = Some(Box::new((
                            pending.authority.clone(),
                            receipt,
                        )));
                    }
                    pending.request.completion.signal();
                }
            }
            })
        .is_ok();
    spawned
}
/// Legal full-footprint stands for the caller's explicitly identified live loc.
/// Do not filter by the scene flood: the baked graph can cross a shut door.
fn loc_target_approach_tiles(
    snapshot: &GameSnapshot,
    to: WorldTile,
    radius: i32,
    loc_id: i32,
) -> Option<Vec<WorldTile>> {
    let query = api::query::SceneQuery::new(snapshot.scene(), None);
    if !query.contains(to) {
        return None;
    }
    let loc = snapshot
        .locs()
        .iter()
        .find(|loc| loc.tile == to && loc.id == loc_id)?;
    api::query::loc_approach::distance_from(loc, to)?;
    let mut tiles = query.operable_tiles(loc)?;
    tiles.retain(|stand| {
        api::query::loc_approach::distance_from(loc, *stand)
            .is_some_and(|distance| distance <= radius.clamp(0, 104) as u32)
    });
    Some(tiles)
}

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
            let t = WorldTile {
                x: to.x + dx,
                z: to.z + dz,
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
            (t.x - from.x).abs().max((t.z - from.z).abs()),
            (t.x - to.x).abs().max((t.z - to.z).abs()),
            t.x,
            t.z,
        )
    });
    tiles
}

pub(crate) struct PendingAdvisoryAssess {
    request: Box<ScriptRouteRequest>,
    authority: script::native::HostAuthority,
    evidence: api::quest_progress::EvidenceStamp,
    token: Arc<()>,
}

fn advisory_refusal(
    authority: &script::native::HostAuthority,
    evidence: api::quest_progress::EvidenceStamp,
    detail: &'static str,
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
    /// Explicit live-loc footprint goals; otherwise use the request's tile-area rule.
    pub(crate) live_candidates: Option<Vec<WorldTile>>,
    /// Frozen request exclusions: every search keeps out of rects and carries
    /// the original wire entries across a resume for arm-time re-resolution.
    pub(crate) exclusions: Option<Arc<ScriptRouteExclusions>>,
    pub(crate) completion: RouteCompletion,
}

// Refusal hints must not fan out into a separate full-world diagnosis for
// every Area goal. Routing still searches the complete goal set; only these
// optional bank-fetch attribution probes have a fixed candidate budget.
const BANK_ZONE_DIAGNOSTIC_TARGETS: usize = 8;
impl ScriptRouteRequest {
    fn targets(&self) -> Vec<WorldTile> {
        if self.radius <= 0 {
            vec![self.to]
        } else if let Some(stands) = self.live_candidates.as_ref() {
            stands.clone()
        } else {
            approach_tiles(
                &self.world,
                self.from,
                self.to,
                self.radius,
                self.loc_id.is_some() || self.arrival == ArrivalKind::Area,
            )
        }
    }

    fn avoid(&self) -> &[AvoidRect] {
        self.exclusions
            .as_deref()
            .map_or(&[][..], |exclusions| exclusions.avoid.as_slice())
    }
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
            self.live_candidates
                .as_ref()
                .map_or(&[][..], |stands| stands.as_slice())
        };
        let direct = if self.radius <= 0 {
            nav::router::find_blocking_zones(
                &self.world.collision,
                &self.world.graph,
                self.from,
                self.to,
                self.opts,
                state,
                self.avoid(),
            )
        } else {
            nav::router::find_first_blocking_zones(
                &self.world.collision,
                &self.world.graph,
                self.from,
                targets,
                self.opts,
                state,
                self.avoid(),
            )
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
            let Some(plan) = plan_bank_fetch(
                &missing,
                state,
                &planning_rows(self.bank_origin, &self.bank, &missing),
                self.world.banks(),
                self.from,
                &self.world.collision,
            ) else {
                continue;
            };
            let access = plan.steps.iter().find_map(|step| match step {
                BankStep::Walk { x, z, level } => Some(WorldTile {
                    x: *x,
                    z: *z,
                    level: *level,
                }),
                _ => None,
            });
            if let Some(access) = access {
                if let Some(keys) = nav::router::find_blocking_zones(
                    &self.world.collision,
                    &self.world.graph,
                    self.from,
                    access,
                    strict_opts,
                    state,
                    self.avoid(),
                )
                .filter(|keys| !keys.is_empty())
                {
                    return Some(keys);
                }
            }
            if let Some(keys) = nav::router::find_blocking_zones(
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

    fn native_failure(
        &self,
        outcome: &RouteOutcome,
        blocked: Option<&[ZoneKey]>,
        targets: &[WorldTile],
    ) -> Option<script::native::WalkEnd> {
        if !matches!(outcome, RouteOutcome::NoPath) {
            return None;
        }
        if blocked.is_some_and(|keys| !keys.is_empty()) {
            return Some(script::native::WalkEnd::Refused);
        }
        let empty = WorldState::empty();
        let state = self.state.as_ref().unwrap_or(&empty);
        Some(
            match nav::router::find_unresolved_quest_gates(
                &self.world.collision,
                &self.world.graph,
                self.from,
                targets,
                self.opts,
                state,
                self.avoid(),
            ) {
                Ok(Some(gates)) => script::native::WalkEnd::NeedsEvidence(gates),
                Ok(None) => script::native::WalkEnd::Failed,
                Err(_) => script::native::WalkEnd::Refused,
            },
        )
    }
}
impl ScriptRouteRequest {
    /// The navigator-named gate shorts of a failed walk: the strict find's own
    /// diagnosis, re-run with only the `item_req`/`worn_req` gates ignored.
    /// A routed outcome names none, and neither does a failure the relaxed
    /// re-run cannot diagnose (budget, off-graph, a skill/quest/varp gate) —
    /// a `NoPath` is never read into a shopping list from its dest geometry.
    /// A `worn_req` any-of alternative is not a carry and is never posted as
    /// one.
    fn missing_carry(&self, outcome: &RouteOutcome) -> Vec<MissingCarry> {
        if !matches!(outcome, RouteOutcome::NoPath) {
            return Vec::new();
        }
        let empty = WorldState::empty();
        let state = self.state.as_ref().unwrap_or(&empty);
        let Some(missing) = find_missing_item_reqs_with_avoid(
            &self.world.collision,
            &self.world.graph,
            self.from,
            self.to,
            self.opts,
            state,
            self.avoid(),
        ) else {
            return Vec::new();
        };
        missing
            .into_iter()
            .filter_map(|req| match req {
                MissingReq::Carry { id, count }
                    if state.inv.get(&id).copied().unwrap_or(0) < count =>
                {
                    Some(MissingCarry { id, count })
                }
                MissingReq::Carry { .. } => None,
                MissingReq::WearAny { .. } => None,
            })
            .collect()
    }

    /// A modeled live loc or an unmodeled solid: legal stands first, then an
    /// optional wall-aware radius fallback for unmodeled solids only. In order: a strict route to a
    /// stand, a BankBudget session to a stand, a strict route to a radius tile, a
    /// session to a radius tile. The strict search for the stands also
    /// answers for the tiles as a search over them alone would, where that
    /// needs no more settles ([`find_first_with_fallback`]: the tiles keep
    /// their own proof and budget), and so does the search under what a
    /// session can fetch; a search over the tiles alone runs only when its
    /// stand search left them undecided. Of each such pair at most one stops
    /// at the unproven budget, so the arm spends at most two budget-limited
    /// searches, one strict and one fetchable, plus one fetchable search for
    /// each stand or tile whose session is refused (no session covers its
    /// missing facts, or the post-state re-find still misses a gate; a
    /// session never deposits). A `Hint` bank memory adds one relaxed
    /// goal-set diagnosis of the same shape before the fetchable search
    /// ([`fetchable_facts`]), so its stock never rejects a goal whose one
    /// verifying trip would decide it.
    fn calculate_solid(
        &self,
        stands: &[WorldTile],
        tiles: &[WorldTile],
        state: &WorldState,
        opts: FindOptions,
    ) -> RouteOutcome {
        let debug = debug_enabled();
        let slot = walk_arm_worker_slot();
        let started = debug.then(Instant::now);
        let search = find_first_with_fallback_avoid(
            &self.world.collision,
            &self.world.graph,
            self.from,
            stands,
            tiles,
            opts,
            state,
            self.avoid(),
        );
        if debug {
            let elapsed_ms = started.unwrap().elapsed().as_millis();
            let scratch = search.scratch_capacities();
            log_walk_arm(&slot, || {
                format!(
                    "approach solid dest={:?} r={} stands={} tiles={} settled={} \
                     scratch={}/{}/{}/{} reverse={}/{} proof={:?} elapsed_ms={elapsed_ms} \
                     stand={} tile={:?}",
                    self.to,
                    self.radius,
                    stands.len(),
                    tiles.len(),
                    search.settled(),
                    scratch.distances,
                    scratch.predecessors,
                    scratch.settled,
                    scratch.heap,
                    scratch.reverse,
                    scratch.reverse_queue,
                    search.proof(),
                    search.route().is_ok(),
                    search.fallback().map(|tile| match tile {
                        FallbackRoute::Routed(route) => Ok(route.dest),
                        other => Err(other.clone()),
                    })
                )
            });
        }
        let strict_tile = match search.into_routes() {
            (Ok(route), _) => return RouteOutcome::Routed(route),
            (Err(_), tile) => tile,
        };

        // A routed strict tile beats a session to one, so the stands are
        // searched alone under the fetchable facts (and diagnosed alone).
        let stand_tiles = match strict_tile {
            Some(FallbackRoute::Routed(_)) => &[][..],
            _ => tiles,
        };
        let fetchable = fetchable_facts(
            &self.world,
            self.from,
            stands,
            stand_tiles,
            opts,
            state,
            self.bank_origin,
            &self.bank,
            self.avoid(),
        );
        let mut fetch_tiles = None;
        if let Some(fetchable) = &fetchable {
            match fetch_stand(
                &self.world,
                self.from,
                stands,
                stand_tiles,
                opts,
                state,
                fetchable,
                self.bank_origin,
                &self.bank,
                self.avoid(),
            ) {
                StandFetch::Outcome(outcome) => return outcome,
                StandFetch::Tiles(known) => fetch_tiles = known,
            }
        }

        let strict_tile = match strict_tile {
            Some(FallbackRoute::Routed(route)) => Some(route),
            Some(FallbackRoute::Undecided) => find_first_with_avoid(
                &self.world.collision,
                &self.world.graph,
                self.from,
                tiles,
                opts,
                state,
                self.avoid(),
            )
            .into_route()
            .ok(),
            Some(FallbackRoute::Failed(_)) | None => None,
        };
        if let Some(route) = strict_tile {
            return RouteOutcome::Routed(route);
        }

        let outcome = match &fetchable {
            Some(fetchable) => fetch_tile(
                &self.world,
                self.from,
                tiles,
                fetch_tiles,
                opts,
                state,
                fetchable,
                self.bank_origin,
                &self.bank,
                self.avoid(),
            ),
            None => RouteOutcome::NoPath,
        };
        if debug {
            let elapsed_ms = started.unwrap().elapsed().as_millis();
            log_walk_arm(&slot, || {
                format!(
                    "approach solid end elapsed_ms={elapsed_ms} fetchable={} outcome={}",
                    fetchable.is_some(),
                    walk_arm_outcome_tag(&outcome)
                )
            });
        }
        outcome
    }

    fn calculate_in_order(
        &self,
        candidates: &[WorldTile],
        state: &WorldState,
        label: &str,
        opts: FindOptions,
    ) -> RouteOutcome {
        let debug = debug_enabled();
        let slot = walk_arm_worker_slot();
        for (idx, &target) in candidates.iter().enumerate() {
            if debug {
                log_walk_arm(&slot, || {
                    format!(
                        "{label} begin idx={idx} target={target:?} generation={} request_id={}",
                        self.generation, self.request_id
                    )
                });
            }
            let started = debug.then(Instant::now);
            let outcome = route_or_bank_fetch(
                &self.world,
                self.from,
                target,
                opts,
                state,
                self.bank_origin,
                &self.bank,
                self.avoid(),
            );
            if debug {
                let elapsed_ms = started.unwrap().elapsed().as_millis();
                log_walk_arm(&slot, || {
                    format!(
                        "{label} end idx={idx} target={target:?} elapsed_ms={elapsed_ms} outcome={}",
                        walk_arm_outcome_tag(&outcome)
                    )
                });
            }
            if !matches!(outcome, RouteOutcome::NoPath) {
                return outcome;
            }
        }
        RouteOutcome::NoPath
    }

    pub(crate) fn calculate_admitted(&self) -> (crate::admission::RouteAdmission, Vec<WorldTile>) {
        let targets = self.targets();
        let result = crate::admission::route(
            &self.world,
            &self.admission,
            self.opts,
            |opts| self.calculate_targets_with_options(&targets, opts),
            || self.blocking_zones(&RouteOutcome::NoPath, &targets),
        );
        (result, targets)
    }

    #[cfg(test)]
    pub(crate) fn calculate(&self) -> (RouteOutcome, Vec<WorldTile>) {
        let targets = self.targets();
        (self.calculate_targets(&targets), targets)
    }

    #[cfg(test)]
    fn calculate_targets(&self, targets: &[WorldTile]) -> RouteOutcome {
        self.calculate_targets_with_options(targets, self.opts)
    }

    fn calculate_targets_with_options(
        &self,
        targets: &[WorldTile],
        opts: FindOptions,
    ) -> RouteOutcome {
        let empty = WorldState::empty();
        let state = self.state.as_ref().unwrap_or(&empty);
        if self.radius <= 0 {
            return route_or_bank_fetch(
                &self.world,
                self.from,
                self.to,
                opts,
                state,
                self.bank_origin,
                &self.bank,
                self.avoid(),
            );
        }
        if self.arrival == ArrivalKind::Area {
            return self.calculate_solid(targets, &[], state, opts);
        }

        // Only explicitly identified live footprints use operable stands.
        // Off-scene/unknown targets retain the ordinary anchor-radius estimate.
        if let Some(stands) = self.live_candidates.as_ref() {
            return self.calculate_solid(stands.as_slice(), &[], state, opts);
        }
        if self.loc_id.is_none() && !self.world.collision.standable(self.to) {
            let mut stands = [self.to; 4];
            let mut len = 0;
            for stand in api::query::arrival_stands(
                self.to,
                |stand| self.world.collision.standable(stand),
                |stand| {
                    Some(
                        self.world
                            .collision
                            .walkable_word(stand.x, stand.z, stand.level)
                            as i32,
                    )
                },
            ) {
                stands[len] = stand;
                len += 1;
            }
            return self.calculate_solid(&stands[..len], targets, state, opts);
        }
        self.calculate_in_order(targets, state, "approach estimate", opts)
    }
}
impl NavBot {
    fn bump_walk_outcome_seq(&mut self) {
        self.manual_walk_cancelled_detail = false;
        self.walk_outcome_seq = self.walk_outcome_seq.wrapping_add(1);
        if self.walk_outcome_seq == 0 {
            self.walk_outcome_seq = 1;
        }
    }

    pub(crate) fn ordinary_movement_owned(&self, tick: u16) -> bool {
        self.native_walk.as_ref().is_some_and(|owner| owner.live())
            || self.route.is_some()
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
    pub(crate) fn note_native_follow_failure(&mut self, outcome: &nav::traveller::TravelOutcome) {
        if self.walk_request_id == 0
            || self.route_request_id != self.walk_request_id
            || self.walk_outcome_request_id != self.walk_request_id
            || self.walk_outcome_generation != self.route_generation
        {
            return;
        }
        use api::selected::Truth;
        let end = match outcome {
            nav::traveller::TravelOutcome::Refused { .. } => Some(script::native::WalkEnd::Refused),
            nav::traveller::TravelOutcome::Blocked { .. } => Some(script::native::WalkEnd::Blocked),
            nav::traveller::TravelOutcome::EvidenceUnproven {
                verdict: Truth::Unknown,
                unresolved,
                ..
            } if !unresolved.is_empty() => Some(script::native::WalkEnd::NeedsEvidence(
                Arc::clone(unresolved),
            )),
            nav::traveller::TravelOutcome::EvidenceUnproven { .. } => {
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
            self.walk_outcome_detail =
                Some(Arc::from(format!("native follow failed: {outcome:?}")));
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
        self.walk_outcome_detail = None;
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
        self.route.is_some()
            || self.bank_fetch.is_some()
            || self.route_worker.is_some()
            || self.pending_route.is_some()
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
        self.route = Some(Arc::clone(&route));
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
        let armed = nav.route.is_some()
            || nav.route_worker.is_some()
            || nav.pending_route.is_some()
            || nav.bank_fetch.is_some();
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
    nav.route_generation = nav.route_generation.wrapping_add(1);
    nav.route_worker = None;
    nav.pending_route = None;
    nav.pending_assess = None;
    nav.requested_route = None;
    nav.traveller.clear();
    nav.route = None;
    nav.bank_fetch = None;
    nav.walk_request_id = 0;
    nav.route_quest_evidence = None;
    super::script_walk::owe_walk_guard_off(nav);
    nav.end_native_walk(script::native::WalkEnd::Cancelled);
    nav.admission_pending = false;
    nav.assess_worker = None;
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
            nav::collision::WorldCollision {
                origin,
                width: 209,
                height: 209,
                walk: vec![0; 209 * 209],
                blocked: vec![0; (209usize * 209).div_ceil(64)],
                flags: None,
            },
            nav::transport::TransportGraph::default(),
            Vec::new(),
        );
        let mut request = ScriptRouteRequest {
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
            live_candidates: None,
            exclusions: None,
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
}
