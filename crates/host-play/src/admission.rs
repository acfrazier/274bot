//! One host route-admission seam. Proceed keeps the bound informational.
//! The runtime net stays gated until its fire accounting is qualified.
//!
//! This is the owner boundary for script's risk module (T5.1): the route,
//! zone table, wilderness and exemptions it judges are served here as
//! `nav_planner::contract` views over the storage host-play retains. Script's risk
//! facts are keyed by NPC id, never by the pack's kind rows.
use std::future::Future;
use std::sync::{Arc, OnceLock};

use crate::walk_plan::RoutePath;
use api::condition::PoisonState;
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, SnapshotView};
use nav_planner::contract::BoundZoneKey;
use nav_planner::producer;
use nav_planner::router::{AvoidRect, FindOptions, Route};
use nav_planner::world::NavWorld;
use nav_planner::zones::{ZoneExempt, ZoneKey};
use script::combat::frame::Frame;
pub use script::combat::risk::RiskInput;
use script::combat::risk::{self, Estimate, RiskTables, RouteAssessment, UnknownWhy, Verdict};
use script::combat::{CombatTables, ThreatSet};
pub use script::native::{RiskPolicy, WalkAllow, WalkRefusal};

/// Immutable movement authority and tile basis, retained together with the
/// estimate. A worker refresh cannot erase the original fallback corridor or
/// broaden teleport/wilderness/avoid/quest authority for a later escape.
#[derive(Clone)]
pub struct RouteBasis {
    /// Retain the frozen route and its result custody for the estimate's lifetime.
    pub(crate) _route: RoutePath,
    pub options: FindOptions,
    pub avoid: Arc<[AvoidRect]>,
    pub quest_evidence: Option<nav_planner::quest_gates::QuestEvidence>,
}

#[derive(Clone, Copy, Debug)]
pub struct Admission {
    pub policy: RiskPolicy,
    /// False only for inherited admission while activation is held: retain the
    /// assessment, but preserve the pre-S2b router's admit/refuse decision.
    pub enforce: bool,
    pub grants: ZoneExempt,
    pub allow: WalkAllow,
    pub input: RiskInput,
    pub generation: u64,
    pub compat_v1: bool,
    /// Slot-owned movement obligation. S2b only enforces admission against an
    /// existing obligation; the observer that creates one arrives in S2c.
    pub escape: Option<script::combat::guard::Escape>,
}

impl Admission {
    pub fn manual(
        options: FindOptions,
        input: RiskInput,
        generation: u64,
        globals: crate::WalkGlobals,
    ) -> Self {
        Self {
            policy: if options.zones.is_all() {
                RiskPolicy::Proceed
            } else {
                globals.risk_policy(Default::default(), Default::default())
            },
            enforce: crate::NET_AVAILABLE || options.zones != ZoneExempt::NONE,
            grants: options.zones,
            allow: WalkAllow::default(),
            input,
            generation,
            compat_v1: false,
            escape: None,
        }
    }
}

pub(crate) struct RouteYieldUnit(bool);

impl Future for RouteYieldUnit {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.0 {
            std::task::Poll::Ready(())
        } else {
            self.0 = true;
            std::task::Poll::Pending
        }
    }
}

pub(crate) fn route_yield_unit() -> RouteYieldUnit {
    RouteYieldUnit(false)
}

pub(crate) type RouteYieldCallback = fn() -> RouteYieldUnit;

pub struct RouteAdmission {
    pub(crate) outcome: crate::RouteOutcome,
    pub assessment: Option<Arc<RouteAssessment>>,
    pub refusal: Option<WalkRefusal>,
    pub blocked: Box<[ZoneKey]>,
    pub tried: u8,
}

/// Strict/endpoint search, one witness diagnosis, at most one named retry.
/// The closures reuse each owner's existing area/live-loc/bank search; all
/// hard authority stays in `options`, `state` and the caller's frozen avoids.
pub(crate) fn route(
    world: &NavWorld,
    admission: &Admission,
    options: FindOptions,
    mut search: impl FnMut(FindOptions) -> crate::RouteOutcome,
    witness: impl FnOnce() -> Option<Vec<ZoneKey>>,
) -> RouteAdmission {
    if admission.escape.is_some_and(escape_live) {
        return RouteAdmission {
            outcome: crate::RouteOutcome::NoPath,
            assessment: None,
            refusal: Some(WalkRefusal::EscapeInProgress),
            blocked: Box::new([]),
            tried: 0,
        };
    }
    let options = FindOptions {
        zones: if admission.compat_v1 || admission.policy == RiskPolicy::Proceed {
            ZoneExempt::all()
        } else {
            options.zones
        },
        ..options
    };
    let mut outcome = search(options);
    let mut tried = 0;
    let blocked = if matches!(
        &outcome,
        crate::RouteOutcome::NoPath | crate::RouteOutcome::IncompleteSearch { .. }
    ) && !admission.compat_v1
        && admission.policy != RiskPolicy::Proceed
    {
        witness().unwrap_or_default()
    } else {
        Vec::new()
    };
    if matches!(
        &outcome,
        crate::RouteOutcome::NoPath | crate::RouteOutcome::IncompleteSearch { .. }
    ) && admission.policy == RiskPolicy::Inherit
        && witness_admissible(world, &admission.input, &blocked)
    {
        if let Ok(zones) = ZoneExempt::named(&blocked).and_then(|named| options.zones.union(named))
        {
            tried = 1;
            let retry = search(FindOptions { zones, ..options });
            outcome = retry;
        }
    }
    let route = match &outcome {
        crate::RouteOutcome::Routed(route) | crate::RouteOutcome::BankSession { route, .. } => {
            Some(route)
        }
        crate::RouteOutcome::IncompleteSearch { .. } | crate::RouteOutcome::NoPath => None,
    };
    let assessment = route.map(|route| assess(route, world, admission));
    let refusal = match &outcome {
        crate::RouteOutcome::IncompleteSearch { search_result } => {
            let nodes_settled = match search_result {
                crate::walk_plan::RouteSearchResultKind::BudgetExhausted { settled } => *settled,
                _ => None,
            };
            Some(WalkRefusal::SearchIncomplete { nodes_settled })
        }
        _ => match route.zip(assessment.as_ref()) {
            Some((route, assessment)) if !permits(route, world, admission, assessment) => {
                Some(verdict_refusal(assessment.verdict))
            }
            None if !blocked.is_empty() || admission.policy == RiskPolicy::Inherit => {
                Some(WalkRefusal::NoRouteWithinBounds { tried, last: None })
            }
            _ => None,
        },
    };
    let blocked = if blocked.is_empty() && refusal.is_some() {
        assessment
            .as_ref()
            .map(|assessment| assessment_keys(world, assessment))
            .unwrap_or_default()
    } else {
        blocked
    };
    RouteAdmission {
        outcome,
        assessment,
        refusal,
        blocked: blocked.into_boxed_slice(),
        tried,
    }
}

pub fn verdict_refusal(verdict: Verdict) -> WalkRefusal {
    match verdict {
        Verdict::Survivable => WalkRefusal::Unsurvivable,
        Verdict::Unsurvivable => WalkRefusal::Unsurvivable,
        Verdict::Unknown(why) => WalkRefusal::Unknown(why),
    }
}

struct Tables {
    combat: Arc<CombatTables>,
    risks: RiskTables,
}

fn tables_cell(world: &NavWorld) -> &OnceLock<Option<Tables>> {
    world.risk_facts(OnceLock::new)
}

fn build_tables_for(selected: Arc<api::game_data::SelectedGameData>) -> Option<Tables> {
    let combat = CombatTables::build(selected).ok()?;
    let risks = RiskTables::build(&combat);
    Some(Tables { combat, risks })
}

fn build_tables() -> Option<Tables> {
    let combat = script::combat::guard::shared_tables().ok()?;
    let risks = RiskTables::build(&combat);
    Some(Tables { combat, risks })
}

/// Pin revision 377's combat and risk facts on its world before any slot can
/// observe it. A failed build remains unavailable in this world's one cache;
/// it must not fall through to the legacy shared revision-289 tables.
pub(crate) fn bind_377_tables(
    world: &NavWorld,
    selected: Option<Arc<api::game_data::SelectedGameData>>,
) -> Result<(), script::combat::guard::GuardRefusal> {
    let cached = tables_cell(world).get_or_init(|| {
        let selected = selected.filter(|data| data.revision() == 377)?;
        build_tables_for(selected)
    });
    if cached
        .as_ref()
        .is_some_and(|tables| tables.combat.selected().revision() != 377)
    {
        return Err(script::combat::guard::GuardRefusal::Tables);
    }
    Ok(())
}

/// The route-job charge uses the world's selected combat rows when bound.
/// Before a synthetic/legacy world is initialized, retain its R289 estimate;
/// an explicitly unavailable world stays unavailable.
pub(crate) fn combat_tables_for_estimate(world: &NavWorld) -> Option<Arc<CombatTables>> {
    match tables_cell(world).get() {
        Some(Some(tables)) => Some(Arc::clone(&tables.combat)),
        Some(None) => None,
        None => script::combat::guard::shared_tables().ok(),
    }
}

fn tables(world: &NavWorld) -> Option<&Tables> {
    let cell = tables_cell(world);
    if cell.get().is_none() {
        let _ = cell.set(build_tables());
    }
    cell.get().and_then(Option::as_ref)
}

/// The same shared selection the admission model uses, read at session start.
pub(crate) fn condition_tables(world: &NavWorld) -> Option<(Arc<CombatTables>, u8)> {
    let tables = tables(world)?;
    Some((Arc::clone(&tables.combat), tables.risks.poison_unknown_v))
}

/// Reuse warm facts, but never hold the world's opaque cache initializer
/// while a cold selected-NPC table is built cooperatively.
async fn tables_cooperative<'a, F, U>(world: &'a NavWorld, yield_unit: &mut F) -> Option<&'a Tables>
where
    F: FnMut() -> U + Send,
    U: Future<Output = ()> + Send,
{
    let cell = tables_cell(world);
    if let Some(tables) = cell.get() {
        return tables.as_ref();
    }
    let combat = match script::combat::guard::shared_tables() {
        Ok(combat) => combat,
        Err(_) => {
            let _ = cell.set(None);
            return cell.get().and_then(Option::as_ref);
        }
    };
    let risks = RiskTables::build_cooperative(&combat, yield_unit)
        .await
        .ok()?;
    let _ = cell.set(Some(Tables { combat, risks }));
    cell.get().and_then(Option::as_ref)
}

#[cfg(test)]
fn prime_tables(world: &NavWorld, tables: Option<Tables>) {
    let _ = tables_cell(world).set(tables);
}

fn unknown_assessment(
    why: UnknownWhy,
    reason: &'static str,
    admission: &Admission,
) -> Arc<RouteAssessment> {
    Arc::new(RouteAssessment {
        verdict: Verdict::Unknown(why),
        bound: 1.0,
        poison_test: risk::PoisonTest::NotNeeded,
        plan: Default::default(),
        crossings: Box::new([]),
        more: 0,
        supplies: Box::new([]),
        hp_after: admission.input.hp,
        volley: u8::MAX,
        input: admission.input,
        generation: admission.generation,
        reason: Arc::from(reason),
    })
}

/// Absent observations are missing facts, not witnessed overflow or damage.
pub fn unavailable(map_members: bool) -> RiskInput {
    let mut input = RiskInput::default();
    input.set_map_members(map_members);
    input.set_overflow(false);
    input.set_unattributed(false);
    input
}

/// Capture one admission from the host's current condition, never a retained
/// assessment's poison input. Missing or stale conditions fail closed.
/// Use the world's selected facts when present. An unavailable binding stays
/// unavailable; callers without a world retain the legacy table selection.
pub fn capture(
    snapshot: &GameSnapshot,
    map_members: bool,
    condition: Option<&api::condition::BotCondition>,
    off_debt: bool,
    world: Option<&NavWorld>,
) -> RiskInput {
    let poison = condition
        .filter(|fact| {
            fact.tick == snapshot.tick() && fact.session == api::condition::SessionMark::Live
        })
        .map_or(
            PoisonState::Unknown {
                since: snapshot.tick() as u16,
            },
            |fact| fact.poison,
        );
    let mut input = unavailable(map_members);
    input.poison = poison;
    input.set_off_debt(off_debt);
    let combat = match world {
        Some(world) => condition_tables(world).map(|(combat, _)| combat),
        None => script::combat::guard::shared_tables().ok(),
    };
    if let Some(tables) = combat {
        let stamp = EvidenceStamp {
            run: api::selected::RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            tick: u64::from(snapshot.tick()),
            sequence: u64::from(snapshot.tick()),
        };
        if let Some(frame) = Frame::borrow(SnapshotView::new(Some(snapshot), stamp)) {
            let mut threats = ThreatSet::default();
            threats.observe(&frame, &tables, frame.tick);
            input = RiskInput::capture(&frame, &tables, &threats, poison, off_debt);
        }
    }
    input.set_map_members(map_members);
    input
}

pub fn assess(route: &Route, world: &NavWorld, admission: &Admission) -> Arc<RouteAssessment> {
    let Ok(route) = producer::route_view(world, route) else {
        return unknown_assessment(
            UnknownWhy::NavIdentity,
            "Route assessment unknown: navigation lease identity unavailable",
            admission,
        );
    };
    assess_view(route, world, admission)
}

pub(crate) fn assess_route_path(
    route: &RoutePath,
    world: &NavWorld,
    admission: &Admission,
) -> Arc<RouteAssessment> {
    let Some(route) = route.view_in(world) else {
        return unknown_assessment(
            UnknownWhy::NavIdentity,
            "Route assessment unknown: navigation lease identity unavailable",
            admission,
        );
    };
    assess_view(route, world, admission)
}

fn assess_view(
    route: nav_planner::contract::RouteView<'_>,
    world: &NavWorld,
    admission: &Admission,
) -> Arc<RouteAssessment> {
    let Some(tables) = tables(world) else {
        return unknown_assessment(
            UnknownWhy::MissingFacts,
            "Route assessment unknown: selected combat facts unavailable",
            admission,
        );
    };
    let (Ok(Some(zones)), Ok(wilderness), Ok(multiway)) = (
        producer::zones_view(world),
        producer::wilderness_view(world),
        producer::multiway_view(world),
    ) else {
        return unknown_assessment(
            UnknownWhy::NavIdentity,
            "Route assessment unknown: navigation lease identity unavailable",
            admission,
        );
    };
    let Ok(named) = producer::bind_exempt(zones, admission.grants) else {
        return unknown_assessment(
            UnknownWhy::NavIdentity,
            "Route assessment unknown: navigation grant identity unavailable",
            admission,
        );
    };
    let assessment = risk::assess(
        route,
        &Estimate {
            zones: Some(zones),
            wilderness,
            multiway,
            grants: Some(risk::Grants::request(named, wilderness)),
            risks: &tables.risks,
            combat: &tables.combat,
            allow: admission.allow,
            generation: admission.generation,
        },
        admission.input,
    );
    informational_assessment(assessment, admission)
}

fn informational_assessment(
    mut assessment: Arc<RouteAssessment>,
    admission: &Admission,
) -> Arc<RouteAssessment> {
    let mode = if admission.compat_v1 {
        Some("rs2b0t compatibility")
    } else if admission.policy == RiskPolicy::Proceed {
        Some("Always")
    } else {
        None
    };
    if let Some(mode) = mode {
        let assessment = Arc::get_mut(&mut assessment).expect("new assessment has one owner");
        assessment.reason = Arc::from(format!(
            "walk proceeds without the check ({mode}); the model's bound is {:.1}% for information; {}",
            assessment.bound * 100.0,
            assessment.reason,
        ));
    }
    assessment
}

pub fn permits(
    route: &Route,
    world: &NavWorld,
    admission: &Admission,
    assessment: &RouteAssessment,
) -> bool {
    let Ok(route) = producer::route_view(world, route) else {
        return false;
    };
    permits_view(route, world, admission, assessment)
}

fn permits_view(
    route: nav_planner::contract::RouteView<'_>,
    world: &NavWorld,
    admission: &Admission,
    assessment: &RouteAssessment,
) -> bool {
    if !admission.enforce || admission.compat_v1 || admission.policy == RiskPolicy::Proceed {
        return true;
    }
    // A failed or unavailable assessment (for example `Unknown(Overflow)`)
    // carries an empty, incomplete plan. That is not evidence of a safe-only
    // route, and replaying its empty plan would certify nothing.
    if !assessment.plan.complete {
        return false;
    }
    let Ok(Some(zones)) = producer::zones_view(world) else {
        return false;
    };
    if assessment.plan.pack != Some(zones.pack()) || route.pack() != zones.pack() {
        return false;
    }
    assessment.verdict == Verdict::Survivable || assessment.plan.crossings.is_empty()
}

fn retained_bot_route_view<'a>(
    legacy: Option<&'a Route>,
    compact: Option<&'a RoutePath>,
    world: &'a NavWorld,
) -> Option<nav_planner::contract::RouteView<'a>> {
    compact
        .and_then(|path| path.view_in(world))
        .or_else(|| legacy.and_then(|route| producer::route_view(world, route).ok()))
}

/// Assess one borrowed compact route without synchronously initializing risk tables.
pub(crate) async fn assess_view_cooperative<F, U>(
    route: nav_planner::contract::RouteView<'_>,
    world: &NavWorld,
    admission: &Admission,
    yield_unit: &mut F,
) -> Arc<RouteAssessment>
where
    F: FnMut() -> U + Send,
    U: Future<Output = ()> + Send,
{
    let Some(tables) = tables_cooperative(world, yield_unit).await else {
        return unknown_assessment(
            UnknownWhy::MissingFacts,
            "Route assessment unknown: selected combat facts unavailable",
            admission,
        );
    };
    let (Ok(Some(zones)), Ok(wilderness), Ok(multiway)) = (
        producer::zones_view(world),
        producer::wilderness_view(world),
        producer::multiway_view(world),
    ) else {
        return unknown_assessment(
            UnknownWhy::NavIdentity,
            "Route assessment unknown: navigation lease identity unavailable",
            admission,
        );
    };
    let Ok(named) = producer::bind_exempt(zones, admission.grants) else {
        return unknown_assessment(
            UnknownWhy::NavIdentity,
            "Route assessment unknown: navigation grant identity unavailable",
            admission,
        );
    };
    let assessment = risk::assess_cooperative(
        route,
        &Estimate {
            zones: Some(zones),
            wilderness,
            multiway,
            grants: Some(risk::Grants::request(named, wilderness)),
            risks: &tables.risks,
            combat: &tables.combat,
            allow: admission.allow,
            generation: admission.generation,
        },
        admission.input,
        yield_unit,
    )
    .await;
    informational_assessment(assessment, admission)
}

/// Cooperative counterpart of [`permits`] over the same borrowed route view.
pub(crate) async fn permits_view_cooperative<F, U>(
    route: nav_planner::contract::RouteView<'_>,
    world: &NavWorld,
    admission: &Admission,
    assessment: &RouteAssessment,
    _yield_unit: &mut F,
) -> bool
where
    F: FnMut() -> U + Send,
    U: Future<Output = ()> + Send,
{
    permits_view(route, world, admission, assessment)
}

/// Check every member of a witness group for supported combat facts.
/// The full route assessment, not this filter, decides whether damage is safe.
/// Each raw witness key is bound to the world's zone view (the identity its
/// lease serves) and judged through the facade kind view.
pub fn witness_admissible(world: &NavWorld, _input: &RiskInput, keys: &[ZoneKey]) -> bool {
    if keys.is_empty() || keys.len() > 8 {
        return false;
    }
    let (Ok(Some(zones)), Some(tables)) = (producer::zones_view(world), tables(world)) else {
        return false;
    };
    let acceptable = |bound: BoundZoneKey| {
        zones.zone(bound).is_ok_and(|zone| {
            let kind = tables.risks.zone_kind(zone.kind());
            kind.unknown_for().is_none()
        })
    };
    keys.iter().all(|&key| {
        let Ok(bound) = producer::bind_key(zones, key) else {
            return false;
        };
        match key {
            ZoneKey::Zone(_) => acceptable(bound),
            // An empty group refuses; every member must be acceptable.
            ZoneKey::Group(_) => producer::group_members(zones, bound).is_ok_and(|members| {
                let mut members = members.peekable();
                members.peek().is_some() && members.all(acceptable)
            }),
        }
    })
}

pub fn fresh(assessment: &RouteAssessment, input: RiskInput) -> bool {
    let mut assessed = assessment.input;
    // Clock age alone is not a changed loadout. Position, actor/due identity,
    // poison, carried counts, locks and off-debt are all compared exactly.
    assessed.tick = input.tick;
    assessed == input
}

pub fn log(slot: Option<&str>, assessment: &RouteAssessment, compat_v1: bool) {
    api::host_log!(api::hostlog::Category::NavEvent, api::hostlog::Level::Info,
        slot = slot.unwrap_or("?"),
        "risk={:?} bound={:.12} poison_test={:?} worst={} volley={} floor={} hp={}/{} food={:?} prayer={} poison={:?} compat_v1={} reason={}",
        assessment.verdict,
        assessment.bound, assessment.poison_test,
        assessment.crossings.iter().map(|row| u32::from(row.worst)).sum::<u32>(),
        assessment.volley, assessment.crossings.iter().map(|row| row.floor_deep).max().unwrap_or(0),
        assessment.input.hp, assessment.input.hp_max,
        assessment.input.food_counts,
        if assessment.crossings.iter().any(|row| row.protect_credited) { "credited" } else { "no" },
        assessment.input.poison, compat_v1, assessment.reason,
    );
}

/// Pump-thread publication freshness gate, before cleanup debts and outer hold
/// gates. A pending route is never followed using a worker's stale loadout.
pub(crate) fn publish(
    bot: &mut crate::script_runtime::NavBot,
    world: &NavWorld,
    snapshot: &GameSnapshot,
    map_members: bool,
    slot: Option<&str>,
) {
    // A poison fact takes precedence over the hold deadline on the same edge.
    if bot.poison_hold.is_some()
        && bot.condition.current().is_some_and(|fact| {
            fact.tick == snapshot.tick() && !matches!(fact.poison, PoisonState::Unknown { .. })
        })
    {
        let hold = bot.poison_hold.take().expect("live poison hold");
        if let Some(wait) = hold.wait {
            api::host_log!(api::hostlog::Category::NavEvent, api::hostlog::Level::Info,
                slot = slot.unwrap_or("?"),
                "poison_hold=end until_index={} started={} deadline={} remaining={} waited={} fact={:?}",
                hold.until_index, wait.started, wait.deadline,
                wait.deadline.wrapping_sub(wait.started),
                (snapshot.tick() as u16).wrapping_sub(wait.started),
                bot.condition.current().expect("current poison fact").poison,
            );
        } else {
            api::host_log!(
                api::hostlog::Category::NavEvent,
                api::hostlog::Level::Info,
                slot = slot.unwrap_or("?"),
                "poison_hold=end until_index={} reason=fact_before_retreat fact={:?}",
                hold.until_index,
                bot.condition.current().expect("current poison fact").poison,
            );
        }
        bot.admission_pending = true;
    }
    if !std::mem::take(&mut bot.admission_pending) {
        return;
    }
    let (Some(mut admission), Some(assessment)) =
        (bot.admission.as_deref().copied(), bot.assessment.as_ref())
    else {
        return;
    };
    let Some(route) =
        retained_bot_route_view(bot.route.as_deref(), bot.compact_route.as_ref(), world)
    else {
        return;
    };
    if let Some(escape) = bot.slot_escape() {
        let destination = bot
            .requested_route
            .map_or(route.destination(), |request| request.0);
        refuse(bot, Some(world), destination, WalkRefusal::EscapeInProgress);
        if let Some(end) = bot.native_end_risk.as_deref_mut() {
            end.escape = Some(escape);
        }
        return;
    }
    let input = capture(
        snapshot,
        map_members,
        bot.condition.current(),
        bot.walk_guard_off.is_some(),
        Some(world),
    );
    if !fresh(assessment, input) {
        admission.input = input;
        *bot.admission.as_deref_mut().expect("pending admission") = admission;
        bot.assessment = Some(assess_view(route, world, &admission));
    }
    let assessment = bot
        .assessment
        .as_ref()
        .expect("publication retains assessment");
    log(slot, assessment, admission.compat_v1);
    let destination = bot
        .requested_route
        .map_or(route.destination(), |request| request.0);
    if permits_view(route, world, &admission, assessment) {
        let enforced =
            admission.enforce && !admission.compat_v1 && admission.policy != RiskPolicy::Proceed;
        if let risk::PoisonTest::WaitForClear { until_index, .. } = assessment.poison_test {
            if enforced
                && bot
                    .poison_hold
                    .as_ref()
                    .is_none_or(|hold| hold.until_index != until_index)
            {
                bot.poison_hold = Some(crate::poison_hold::PoisonHold {
                    until_index,
                    wait: None,
                    stand_sent: false,
                });
            }
        } else {
            bot.poison_hold = None;
        }
        return;
    }
    let refusal = verdict_refusal(assessment.verdict);
    refuse(bot, Some(world), destination, refusal);
}

pub fn escape_live(escape: script::combat::guard::Escape) -> bool {
    use script::combat::guard::EscapeState;
    matches!(
        escape.state,
        EscapeState::Running | EscapeState::Suspended | EscapeState::Unresolved
    )
}

pub fn escape_reason(escape: script::combat::guard::Escape) -> String {
    format!(
        "escape in progress (HP {} ≤ floor {}): wait",
        escape.hp, escape.floor
    )
}

pub(crate) fn assessment_keys(world: &NavWorld, assessment: &RouteAssessment) -> Vec<ZoneKey> {
    let Ok(Some(zones)) = producer::zones_view(world) else {
        return Vec::new();
    };
    // A key from another pack cannot name a row here; it is not reported.
    let mut keys: Vec<_> = assessment
        .plan
        .intervals
        .iter()
        .filter_map(|interval| interval.key(zones).ok())
        .collect();
    keys.sort_unstable_by_key(|key| match *key {
        ZoneKey::Zone(index) => u32::from(index),
        ZoneKey::Group(index) => (1 << 16) | u32::from(index),
    });
    keys.dedup();
    keys
}

pub(crate) fn refuse(
    bot: &mut crate::script_runtime::NavBot,
    world: Option<&NavWorld>,
    destination: api::snapshot::WorldTile,
    refusal: WalkRefusal,
) {
    let request_id = bot.walk_request_id;
    let detail = if refusal == WalkRefusal::EscapeInProgress {
        bot.slot_escape()
            .map(|escape| Arc::from(escape_reason(escape)))
    } else if matches!(refusal, WalkRefusal::PoisonUnsettled { .. }) {
        Some(Arc::from(refusal.to_string()))
    } else {
        bot.assessment
            .as_ref()
            .map(|assessment| Arc::clone(&assessment.reason))
    }
    .unwrap_or_else(|| Arc::from(refusal.to_string()));
    if let Some(assessment) = bot.assessment.as_ref() {
        bot.last_assessment = Some(Arc::clone(assessment));
        let keys = world.map_or_else(Vec::new, |world| assessment_keys(world, assessment));
        bot.native_walk_blocked = (!keys.is_empty()).then(|| (request_id, Arc::from(keys)));
    }
    let (radius, teleports) = bot
        .requested_route
        .map_or((0, false), |request| (request.1, request.2));
    bot.note_failure(
        bot.route_generation,
        request_id,
        destination,
        radius,
        teleports,
    );
    bot.clear_route();
    bot.bank_fetch = None;
    bot.traveller.clear();
    bot.risk_refusal = Some(Box::new((request_id, refusal)));
    bot.end_native_walk(script::native::WalkEnd::Refused);
    bot.walk_outcome_request_id = request_id;
    bot.walk_outcome_detail = Some(detail);
}

#[cfg(test)]
#[path = "admission_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "survivable_probability_live_tests.rs"]
mod probability_live_tests;
