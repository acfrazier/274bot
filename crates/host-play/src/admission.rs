//! One host route-admission seam. The estimate is advisory under Proceed;
//! no runtime net or poison-clear observer is claimed before S2c integration.
use std::sync::Arc;

use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, SnapshotView};
use nav::router::{AvoidRect, FindOptions, Route};
use nav::world::NavWorld;
use nav::zones::{ZoneExempt, ZoneKey};
use script::combat::frame::Frame;
use script::combat::risk::{self, Estimate, RiskTables, RouteAssessment, UnknownWhy, Verdict};
pub use script::combat::risk::{PoisonState, RiskInput};
use script::combat::{CombatTables, ThreatSet};
use script::native::{RiskPolicy, WalkAllow, WalkRefusal};

/// Immutable movement authority and tile basis, retained together with the
/// estimate. A worker refresh cannot erase the original fallback corridor or
/// broaden teleport/wilderness/avoid/quest authority for a later escape.
#[derive(Clone)]
pub struct RouteBasis {
    pub route: Arc<Route>,
    pub options: FindOptions,
    pub avoid: Arc<[AvoidRect]>,
    pub quest_evidence: Option<nav::quest_gates::QuestEvidence>,
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
    let blocked = if matches!(outcome, crate::RouteOutcome::NoPath)
        && !admission.compat_v1
        && admission.policy != RiskPolicy::Proceed
    {
        witness().unwrap_or_default()
    } else {
        Vec::new()
    };
    if matches!(outcome, crate::RouteOutcome::NoPath)
        && admission.policy == RiskPolicy::Inherit
        && witness_admissible(world, &admission.input, &blocked)
    {
        if let Ok(zones) = ZoneExempt::named(&blocked).and_then(|named| options.zones.union(named))
        {
            tried = 1;
            outcome = search(FindOptions { zones, ..options });
        }
    }
    let route = match &outcome {
        crate::RouteOutcome::Routed(route) | crate::RouteOutcome::BankSession { route, .. } => {
            Some(route)
        }
        crate::RouteOutcome::NoPath => None,
    };
    let assessment = route.map(|route| assess(route, world, admission));
    let refusal = match route.zip(assessment.as_ref()) {
        Some((route, assessment)) if !permits(route, world, admission, assessment) => {
            Some(verdict_refusal(assessment.verdict))
        }
        None if !blocked.is_empty() || admission.policy == RiskPolicy::Inherit => {
            Some(WalkRefusal::NoRouteWithinBounds { tried, last: None })
        }
        _ => None,
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
        Verdict::FixableWith => WalkRefusal::FixableWith,
        Verdict::Unsurvivable => WalkRefusal::Unsurvivable,
        Verdict::Unknown(why) => WalkRefusal::Unknown(why),
    }
}

struct Tables {
    combat: Arc<CombatTables>,
    risks: RiskTables,
}

fn tables(world: &NavWorld) -> Option<&Tables> {
    world
        .risk_facts(|| {
            let combat = script::combat::guard::shared_tables().ok()?;
            let risks = match world.graph.zones.as_ref() {
                Some(zones) => RiskTables::build(zones, &combat),
                None => RiskTables::without_zones(&combat),
            };
            Some(Tables { combat, risks })
        })
        .as_ref()
}

/// Absent observations are missing facts, not witnessed overflow or damage.
pub fn unavailable(map_members: bool) -> RiskInput {
    RiskInput {
        map_members,
        overflow: false,
        unattributed: false,
        ..RiskInput::default()
    }
}

/// Copy a snapshot once at request admission. Poison remains Unknown until
/// the later observer supplies positive evidence; silence is not a cure.
pub fn capture(
    snapshot: &GameSnapshot,
    map_members: bool,
    poison: PoisonState,
    off_debt: bool,
) -> RiskInput {
    let mut input = RiskInput {
        poison,
        off_debt,
        ..unavailable(map_members)
    };
    if let Ok(tables) = script::combat::guard::shared_tables() {
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
    input.map_members = map_members;
    input
}

pub fn assess(route: &Route, world: &NavWorld, admission: &Admission) -> Arc<RouteAssessment> {
    let Some(tables) = tables(world) else {
        return Arc::new(RouteAssessment {
            verdict: Verdict::Unknown(UnknownWhy::MissingFacts),
            plan: Default::default(),
            crossings: Box::new([]),
            more: 0,
            supplies: Box::new([]),
            hp_after: admission.input.hp,
            volley: u8::MAX,
            input: admission.input,
            generation: admission.generation,
            reason: Arc::from("Route assessment unknown: selected combat facts unavailable"),
        });
    };
    risk::assess(
        route,
        &Estimate {
            zones: world.graph.zones.as_ref(),
            wilderness: &world.graph.wilderness,
            risks: &tables.risks,
            combat: &tables.combat,
            allow: admission.allow,
            generation: admission.generation,
        },
        admission.input,
    )
}

pub fn permits(
    route: &Route,
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
    if assessment.verdict == Verdict::Survivable {
        return true;
    }
    // Fleeing cannot be refused because an attacker or snapshot is unknown.
    if assessment.plan.crossings.is_empty() {
        return true;
    }
    let (Some(zones), Some(tables)) = (world.graph.zones.as_ref(), tables(world)) else {
        return false;
    };
    risk::admission_passes(
        route,
        assessment,
        zones,
        &tables.combat,
        admission.allow,
        risk::Grants::request(admission.grants, &world.graph.wilderness),
    )
    .unwrap_or(false)
}

/// Check every member of a witness group; one unsupported or lethal member
/// refuses the one-round expansion. This never relaxes any hard constraint.
pub fn witness_admissible(world: &NavWorld, input: &RiskInput, keys: &[ZoneKey]) -> bool {
    if keys.is_empty() || keys.len() > 8 || matches!(input.poison, PoisonState::Unknown { .. }) {
        return false;
    }
    let (Some(zones), Some(tables)) = (world.graph.zones.as_ref(), tables(world)) else {
        return false;
    };
    let acceptable = |index: u16| {
        zones
            .zones()
            .get(usize::from(index))
            .and_then(|zone| tables.risks.kind(zone.kind))
            .is_some_and(|kind| {
                kind.unknown_for(input.map_members).is_none() && kind.max_hit < input.hp
            })
    };
    keys.iter().all(|key| match *key {
        ZoneKey::Zone(index) => acceptable(index),
        ZoneKey::Group(index) => zones.groups().get(usize::from(index)).is_some_and(|group| {
            !group.members.is_empty() && group.members.iter().copied().all(acceptable)
        }),
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
        "risk={:?} worst={} volley={} floor={} hp={}/{} food={:?} prayer={} poison={:?} compat_v1={} reason={}",
        assessment.verdict,
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
    if !std::mem::take(&mut bot.admission_pending) {
        return;
    }
    let (Some(route), Some(mut admission), Some(assessment)) = (
        bot.route.as_ref(),
        bot.admission.as_deref().copied(),
        bot.assessment.as_ref(),
    ) else {
        return;
    };
    if let Some(escape) = bot.slot_escape() {
        let destination = bot.requested_route.map_or(route.dest, |request| request.0);
        refuse(bot, world, destination, WalkRefusal::EscapeInProgress);
        if let Some(end) = bot.native_end_risk.as_deref_mut() {
            end.escape = Some(escape);
        }
        return;
    }
    let input = capture(
        snapshot,
        map_members,
        admission.input.poison,
        bot.walk_guard_off.is_some(),
    );
    if !fresh(assessment, input) {
        admission.input = input;
        *bot.admission.as_deref_mut().expect("pending admission") = admission;
        bot.assessment = Some(assess(route, world, &admission));
    }
    let assessment = bot
        .assessment
        .as_ref()
        .expect("publication retains assessment");
    log(slot, assessment, admission.compat_v1);
    if permits(route, world, &admission, assessment) {
        return;
    }
    let refusal = verdict_refusal(assessment.verdict);
    let destination = bot.requested_route.map_or(route.dest, |request| request.0);
    refuse(bot, world, destination, refusal);
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
    let Some(zones) = world.graph.zones.as_ref() else {
        return Vec::new();
    };
    let mut keys: Vec<_> = assessment
        .plan
        .intervals
        .iter()
        .map(|interval| interval.key(zones))
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
    world: &NavWorld,
    destination: api::snapshot::WorldTile,
    refusal: WalkRefusal,
) {
    let request_id = bot.walk_request_id;
    let detail = if refusal == WalkRefusal::EscapeInProgress {
        bot.slot_escape()
            .map(|escape| Arc::from(escape_reason(escape)))
    } else {
        bot.assessment
            .as_ref()
            .map(|assessment| Arc::clone(&assessment.reason))
    }
    .unwrap_or_else(|| Arc::from(refusal.to_string()));
    if let Some(assessment) = bot.assessment.as_ref() {
        bot.last_assessment = Some(Arc::clone(assessment));
        let keys = assessment_keys(world, assessment);
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
    bot.route = None;
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
