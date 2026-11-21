//! Operator decision D8: the Quester is poisoned at low HP and holds no food
//! and no cure it may use. It cures first when it can (the walk guard and
//! [`crate::survival::SurvivalStep`] drink a held antipoison through
//! [`crate::survival::cure`]); otherwise it walks to a tile outside the
//! stage's danger area and stops there, with the reason in its status.
//!
//! "Low HP" is the shared survival need [`SurvivalNeed::Cure`]: HP at or below
//! the poison-adjusted emergency line, or a poison outlook whose remaining
//! damage reaches HP ([`crate::survival::math::survival_need`]). Both come from
//! the content poison script: each `poison` timer fire queues
//! `(%poison + 4) / 5` damage and lowers `%poison` by one
//! (`poison.rs2:20-30`), and the timer runs every 30 ticks
//! (`poison.rs2:72-75`); `api::condition::poison_math` sums that series.
//!
//! "Safe" is a tile outside every zone that the stage and the walk step
//! declare in `cross`: the danger area the Path accepted for this stage.
//! Routing keeps a walk out of every other zone. A remembered safe tile
//! belongs to one danger area ([`Scope`]): a stage or walk-step change makes
//! it unusable, and the runner checks it against the current zones again
//! before it walks there.
//!
//! The stop is server-visible. Cancelling host work ends the host's route
//! follow, not the server's queued waypoints, so the stop sends one ordinary
//! scene walk click on the player's own tile ([`StandStill`]). The engine
//! takes a client path in place of the queued one (`MoveClickHandler.ts:41-54`,
//! `PathingEntity.ts:268-279`), and a click on the start tile is a path of
//! that tile alone (`MoveClickDecoder.ts:11-28`), so the player stays there.
//! The engine refuses a click while the player is delayed and keeps the old
//! path (`MoveClickHandler.ts:12-15`), so the click itself proves nothing: the
//! stop is reported only after the observed tile stays the same across two
//! observed ticks after the click took effect, and that tile is outside the
//! danger area. A later tile change ends that claim and the runner plans
//! again.
//!
//! The stop is the survival park (design A6.5). The park ends when fresh
//! evidence shows the survival need is gone ([`crate::survival::SurvivalStep`]
//! reassesses it on every eligible tick). Poison itself ends when `%poison`
//! reaches 0 (`poison.rs2:21-23`), and damage that reaches 0 HP queues
//! `player_death` (`poison.rs2:59-61`), which resets the stop. A food need can
//! outlast the poison, and timers can defer while the player is inaccessible
//! (`Player.ts:945-959`), so the park has no fixed tick bound. The host
//! suppresses the client idle timer by default, so no idle logout ends a park
//! early (`Client::set_host_idle_timer`).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};
use std::task::Poll;

use api::condition::FoodSummary;
use api::WorldTile;
use nav_planner::contract::{ZoneTableView, ZoneView};

use super::compile::StepPlan;
use crate::interact::InteractReq;
use crate::native::walk::Walk;
use crate::native::{
    ActionContext, ActionError, ActionHandle, InteractionDispatch, NativeMachine, WalkRetryState,
};
use crate::survival::math::SurvivalNeed;

/// Whether the survival need is the D8 state: a poison cure is needed and
/// the inventory holds no food. A [`SurvivalNeed::Cure`] reaches the runner
/// only when no allowed cure is held or clickable
/// (`crate::survival::step::SurvivalStep::want`).
pub(super) fn stranded(need: SurvivalNeed, food: &FoodSummary) -> bool {
    matches!(need, SurvivalNeed::Cure { .. }) && food.ordinary_count == 0 && food.combo_count == 0
}

/// What the runner does for a D8 need on this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopPlan {
    /// Stop here: the tile is outside the danger area.
    Park,
    /// The current walk step leaves the danger area: keep following it.
    KeepWalking,
    /// Walk back to this last observed safe tile, then stop.
    Retreat(WorldTile),
    /// Stop inside the danger area, because no safe walk is known.
    ParkInDanger(InDanger),
}

/// Why a stop is inside the danger area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InDanger {
    /// The zone table or the player's tile is not available.
    AreaUnknown,
    /// No tile outside the danger area was observed in this danger area.
    NoSafeTile,
    /// The walk out ended without arrival.
    WalkOutFailed,
}

/// Choose the D8 response. `here` and `walk_target` say whether the player's
/// tile and the running walk step's target are inside the danger area
/// (`None`: unknown, or no running walk step). `last_safe` is the last
/// observed tile outside this danger area, already checked against it.
pub(super) fn plan(
    here: Option<bool>,
    walk_target: Option<bool>,
    last_safe: Option<WorldTile>,
) -> StopPlan {
    match here {
        None => StopPlan::ParkInDanger(InDanger::AreaUnknown),
        Some(false) => StopPlan::Park,
        Some(true) if walk_target == Some(false) => StopPlan::KeepWalking,
        Some(true) => last_safe.map_or(
            StopPlan::ParkInDanger(InDanger::NoSafeTile),
            StopPlan::Retreat,
        ),
    }
}

/// The zones of the stage's danger area: the stage `cross` ids and the
/// walk step's `cross` ids.
pub(super) struct DangerArea<'a> {
    pub(super) stage: &'a [Arc<str>],
    pub(super) step: &'a [Arc<str>],
}

/// A walk step's target and its declared `cross` ids
/// (`StepPlan::walk_target`).
pub(super) type WalkTarget<'a> = (WorldTile, &'a [Arc<str>]);

impl DangerArea<'_> {
    /// Whether `tile` is inside the area. `None` when the zone table is
    /// unavailable and the area is not empty.
    pub(super) fn contains(
        &self,
        zones: Option<ZoneTableView<'_>>,
        tile: WorldTile,
    ) -> Option<bool> {
        if self.stage.is_empty() && self.step.is_empty() {
            return Some(false);
        }
        let zones = zones?;
        Some(zones.containing(tile).any(|zone| {
            self.stage
                .iter()
                .chain(self.step)
                .any(|id| names_zone(zones, zone, id))
        }))
    }

    /// The area's identity: the stage's sequence and every declared id.
    pub(super) fn scope(&self, sequence: Option<usize>) -> Scope {
        // SipHash with fixed zero keys: the same area gives the same value
        // in every process, and hashing borrows the ids (no allocation).
        let mut hasher = std::hash::DefaultHasher::new();
        sequence.hash(&mut hasher);
        self.stage.hash(&mut hasher);
        self.step.hash(&mut hasher);
        Scope(hasher.finish())
    }
}

/// Whether a `cross` id names `zone`: its hazard kind, its exact spawn
/// identity, or its group (`ZoneTable::resolve`).
fn names_zone(zones: ZoneTableView<'_>, zone: ZoneView<'_>, id: &str) -> bool {
    zone.kind().id() == id
        || zones
            .resolve(id)
            .is_some_and(|key| key == zone.key() || zone.group() == Some(key))
}

/// The identity of one danger area ([`DangerArea::scope`]). A safe tile is
/// valid only for the scope it was observed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Scope(u64);

/// The D8 stop's reason, built once for the status line and the log.
#[derive(Debug, Clone)]
pub(super) struct Stopped {
    pub(super) reason: Arc<str>,
    /// The tile the stop judged; a later tile change ends the stop's claim.
    tile: Option<WorldTile>,
    /// The runner may still send a stand click if the player moves.
    watch: bool,
}

impl Stopped {
    /// `tile` is where the player stands; `danger` says why that tile is
    /// inside the danger area, if it is; `detail` is the last failure.
    pub(super) fn new(
        tile: Option<WorldTile>,
        danger: Option<InDanger>,
        detail: Option<&str>,
    ) -> Self {
        Self {
            reason: Arc::from(
                StopText {
                    tile,
                    danger,
                    detail,
                    unconfirmed: false,
                }
                .to_string(),
            ),
            tile,
            watch: true,
        }
    }

    /// The stand click budget is spent and the player was not seen standing
    /// still: the stop names the tile and the failure and claims no safety.
    pub(super) fn unconfirmed(tile: Option<WorldTile>, detail: Option<&str>) -> Self {
        Self {
            reason: Arc::from(
                StopText {
                    tile,
                    danger: None,
                    detail,
                    unconfirmed: true,
                }
                .to_string(),
            ),
            tile,
            watch: false,
        }
    }
}

struct StopText<'a> {
    tile: Option<WorldTile>,
    danger: Option<InDanger>,
    detail: Option<&'a str>,
    unconfirmed: bool,
}

impl fmt::Display for StopText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("poisoned at low HP with no food and no antipoison: stopped")?;
        if let Some(tile) = self.tile {
            write!(f, " at {},{},{}", tile.x, tile.z, tile.level)?;
        }
        if self.unconfirmed {
            f.write_str(", but the player was not seen standing still")?;
            if let Some(detail) = self.detail {
                write!(f, " ({detail})")?;
            }
            return Ok(());
        }
        match self.danger {
            None => f.write_str(" outside the danger area"),
            Some(InDanger::AreaUnknown) => {
                f.write_str(" where the danger area is unknown (no zone table or tile)")
            }
            Some(InDanger::NoSafeTile) => {
                f.write_str(" inside the danger area (no safe tile seen in this danger area)")
            }
            Some(InDanger::WalkOutFailed) => match self.detail {
                Some(detail) => write!(
                    f,
                    " inside the danger area (the walk out did not arrive: {detail})"
                ),
                None => f.write_str(" inside the danger area (the walk out did not arrive)"),
            },
        }
    }
}

/// Stand clicks one D8 episode may send: the shared Quester walk bound,
/// one request and two fresh retries (`native::QUEST_WALK_MAX_ATTEMPTS`).
pub(super) const MAX_STAND_CLICKS: u8 = crate::native::QUEST_WALK_MAX_ATTEMPTS;

/// The host dispatches the click in the observation pass after it is
/// queued; a click it still holds after this many observed ticks is lost.
const STAND_SEND_TICKS: u64 = 4;
/// The host sends the click after its observation at the receipt tick. The
/// engine reads client input at the start of a server tick and moves the
/// player in the same tick (`World.ts:611-640`, `World.ts:690-720`), so the
/// first observation that surely includes the click is two ticks later.
const STAND_SETTLE_TICKS: u64 = 2;
/// A player with queued waypoints moves on every server tick
/// (`PathingEntity.ts:135-152`); after the click it steps at most once back
/// to the clicked tile. Four observed ticks give two pairs to compare.
const STAND_WINDOW_TICKS: u64 = 4;

/// The server-visible stop: one scene walk click on the player's own tile,
/// then two observations of the same tile after it took effect. The output
/// is the tile the player stands on.
pub(super) struct StandStill {
    request_id: u64,
    deadline: u64,
    settle_from: Option<u64>,
    last: Option<(u64, WorldTile)>,
}

impl NativeMachine for StandStill {
    type Args = WorldTile;
    type Output = WorldTile;

    fn begin(tile: WorldTile, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let request_id = cx.emit(InteractReq::WalkTo {
            x: tile.x,
            z: tile.z,
            level: tile.level,
        })?;
        Ok(Self {
            request_id,
            deadline: cx.evidence().tick.saturating_add(STAND_SEND_TICKS),
            settle_from: None,
            last: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<WorldTile, ActionError>> {
        static REFUSED: LazyLock<Arc<str>> =
            LazyLock::new(|| Arc::from("the client refused the stand click"));
        static DROPPED: LazyLock<Arc<str>> =
            LazyLock::new(|| Arc::from("the host did not send the stand click"));
        let now = cx.evidence().tick;
        let settle_from = match self.settle_from {
            Some(from) => from,
            None => match cx.interaction_dispatch(self.request_id) {
                InteractionDispatch::Settled(receipt) if !receipt.accepted => {
                    return Poll::Ready(Err(ActionError::Failed(Arc::clone(&REFUSED))));
                }
                InteractionDispatch::Settled(receipt) => {
                    let from = receipt.evidence.tick.saturating_add(STAND_SETTLE_TICKS);
                    self.settle_from = Some(from);
                    self.deadline = from.saturating_add(STAND_WINDOW_TICKS);
                    from
                }
                InteractionDispatch::Queued if now < self.deadline => return Poll::Pending,
                InteractionDispatch::Queued | InteractionDispatch::Unsent => {
                    return Poll::Ready(Err(ActionError::Failed(Arc::clone(&DROPPED))));
                }
            },
        };
        if now >= settle_from {
            if let Some(here) = cx.snapshot().here().map(|here| here.value) {
                match self.last {
                    Some((tick, tile)) if tick < now && tile == here => {
                        return Poll::Ready(Ok(here));
                    }
                    Some((tick, _)) if tick == now => {}
                    _ => self.last = Some((now, here)),
                }
            }
        }
        if now >= self.deadline {
            let detail = match self.last {
                Some((_, tile)) => format!(
                    "the player still moved after the stand click (last at {},{},{})",
                    tile.x, tile.z, tile.level
                ),
                None => "the player's tile was not observed after the stand click".to_owned(),
            };
            return Poll::Ready(Err(ActionError::Failed(Arc::from(detail))));
        }
        Poll::Pending
    }
}

/// A walk back to the last safe tile, with the shared bounded recovery.
pub(super) struct Retreat {
    pub(super) handle: Option<ActionHandle<Walk>>,
    pub(super) target: WorldTile,
    pub(super) need: SurvivalNeed,
    pub(super) retry: WalkRetryState,
    /// The last walk failure, kept for the stop reason.
    pub(super) failure: Option<Arc<str>>,
}

/// The stand-still in flight, before the stop is reported.
pub(super) struct Standing {
    pub(super) handle: Option<ActionHandle<StandStill>>,
    pub(super) need: SurvivalNeed,
    /// Why the planned stop is inside the danger area, if it is.
    pub(super) expect: Option<InDanger>,
    /// The last walk or stand click failure.
    pub(super) failure: Option<Arc<str>>,
}

enum Phase {
    /// The running walk step is kept as the walk out; its log line is out.
    WalkingOut,
    Retreat(Retreat),
    Standing(Standing),
    /// The stop, while the survival park holds.
    Stopped(Stopped),
    /// The player moved after a stop, or stood still inside the area: the
    /// next survival poll plans again from the fresh tile.
    Replan,
}

/// One D8 episode: from the first decision until the need ends.
struct Episode {
    /// The walk step whose `cross` ids were part of the danger area when the
    /// episode began; it stays part of the area after the step is cancelled.
    step: Option<Arc<dyn StepPlan>>,
    phase: Phase,
}

/// The runner's D8 state. The safe tile and the click count are inline; the
/// episode is behind one box, allocated at its first decision and reused by
/// every later phase. A D8 state is rare, and the Quester's per-bot layout is
/// bounded (`quester_struct_fits_the_per_bot_budget`).
#[derive(Default)]
pub(super) struct PoisonStop {
    /// Last observed tile outside the danger area, with that area's scope.
    safe: Option<(WorldTile, Scope)>,
    /// Stand clicks sent in this episode.
    clicks: u8,
    episode: Option<Box<Episode>>,
}

impl PoisonStop {
    /// Retained heap of one episode: the box. The reason and failure texts
    /// are `Arc<str>` and counted by the caller.
    #[cfg(test)]
    pub(super) const EPISODE_BYTES: usize = std::mem::size_of::<Episode>();

    pub(super) fn record_safe(&mut self, tile: WorldTile, scope: Scope) {
        self.safe = Some((tile, scope));
    }

    /// The last safe tile observed in this danger area.
    pub(super) fn safe_for(&self, scope: Scope) -> Option<WorldTile> {
        self.safe
            .filter(|(_, seen)| *seen == scope)
            .map(|(tile, _)| tile)
    }

    /// While an episode runs, its own walk step (or `Some(None)` for an
    /// episode that began without one).
    pub(super) fn episode_step(&self) -> Option<Option<&dyn StepPlan>> {
        self.episode
            .as_deref()
            .map(|episode| episode.step.as_deref())
    }

    /// Start an episode with `step` as its walk step, or keep the running
    /// one. The box is allocated once per episode.
    pub(super) fn begin_episode(&mut self, step: Option<Arc<dyn StepPlan>>) {
        if self.episode.is_none() {
            self.clicks = 0;
            self.episode = Some(Box::new(Episode {
                step,
                phase: Phase::Replan,
            }));
        }
    }

    fn set_phase(&mut self, phase: Phase) {
        match self.episode.as_deref_mut() {
            Some(episode) => episode.phase = phase,
            None => {
                self.clicks = 0;
                self.episode = Some(Box::new(Episode { step: None, phase }));
            }
        }
    }

    fn phase(&self) -> Option<&Phase> {
        self.episode.as_deref().map(|episode| &episode.phase)
    }

    fn phase_mut(&mut self) -> Option<&mut Phase> {
        self.episode
            .as_deref_mut()
            .map(|episode| &mut episode.phase)
    }

    /// A walk out or stand-still runs; the runner does no other work.
    pub(super) fn busy(&self) -> bool {
        matches!(self.phase(), Some(Phase::Retreat(_) | Phase::Standing(_)))
    }

    pub(super) fn retreat_mut(&mut self) -> Option<&mut Retreat> {
        match self.phase_mut() {
            Some(Phase::Retreat(retreat)) => Some(retreat),
            _ => None,
        }
    }

    /// Take the walk out when it ends; the phase becomes `Replan` until the
    /// caller stands still or plans again. Dropping its handle revokes the
    /// host walk.
    pub(super) fn end_retreat(&mut self) -> Option<Retreat> {
        let phase = self.phase_mut()?;
        if !matches!(phase, Phase::Retreat(_)) {
            return None;
        }
        match std::mem::replace(phase, Phase::Replan) {
            Phase::Retreat(retreat) => Some(retreat),
            _ => None,
        }
    }

    pub(super) fn start_retreat(&mut self, target: WorldTile, need: SurvivalNeed) {
        self.set_phase(Phase::Retreat(Retreat {
            handle: None,
            target,
            need,
            retry: WalkRetryState::default(),
            failure: None,
        }));
    }

    pub(super) fn standing_mut(&mut self) -> Option<&mut Standing> {
        match self.phase_mut() {
            Some(Phase::Standing(standing)) => Some(standing),
            _ => None,
        }
    }

    pub(super) fn stand(
        &mut self,
        need: SurvivalNeed,
        expect: Option<InDanger>,
        failure: Option<Arc<str>>,
    ) {
        self.set_phase(Phase::Standing(Standing {
            handle: None,
            need,
            expect,
            failure,
        }));
    }

    /// Take the stand-still's plan when it ends; the phase becomes `Replan`
    /// until the caller stops or plans again.
    pub(super) fn end_standing(&mut self) -> Option<Standing> {
        let phase = self.phase_mut()?;
        if !matches!(phase, Phase::Standing(_)) {
            return None;
        }
        match std::mem::replace(phase, Phase::Replan) {
            Phase::Standing(standing) => Some(standing),
            _ => None,
        }
    }

    /// Spend one stand click of this episode's budget. False when it is
    /// spent.
    pub(super) fn take_click(&mut self) -> bool {
        if self.clicks >= MAX_STAND_CLICKS {
            return false;
        }
        self.clicks += 1;
        true
    }

    pub(super) fn clicks(&self) -> u8 {
        self.clicks
    }

    /// The stop, while the survival park holds.
    pub(super) fn stopped(&self) -> Option<&Stopped> {
        match self.phase() {
            Some(Phase::Stopped(stopped)) => Some(stopped),
            _ => None,
        }
    }

    pub(super) fn stop(&mut self, stopped: Stopped) {
        self.set_phase(Phase::Stopped(stopped));
    }

    /// The tile a watched stop stands on.
    pub(super) fn watched_tile(&self) -> Option<WorldTile> {
        self.stopped()
            .filter(|stopped| stopped.watch)
            .and_then(|stopped| stopped.tile)
    }

    /// The player moved after the stop: plan again from the fresh tile.
    pub(super) fn replan(&mut self) {
        self.set_phase(Phase::Replan);
    }

    /// End the episode at the un-park and return its stop. The box is freed.
    pub(super) fn end_stop(&mut self) -> Option<Stopped> {
        self.stopped()?;
        self.clicks = 0;
        match self.episode.take().map(|episode| episode.phase) {
            Some(Phase::Stopped(stopped)) => Some(stopped),
            _ => None,
        }
    }

    /// Keep the running walk step as the walk out. True the first time, so
    /// the caller logs it once.
    pub(super) fn walk_out(&mut self, step: Option<Arc<dyn StepPlan>>) -> bool {
        if self.episode.is_some() {
            return false;
        }
        self.begin_episode(step);
        self.set_phase(Phase::WalkingOut);
        true
    }

    #[cfg(test)]
    pub(super) fn walking_out(&self) -> bool {
        matches!(self.phase(), Some(Phase::WalkingOut))
    }

    /// The need is gone before any stop: end the episode. Returns true when
    /// a walk out or stand-still was dropped, so the caller logs it.
    pub(super) fn need_gone(&mut self) -> bool {
        match self.phase() {
            Some(Phase::Stopped(_)) | None => false,
            Some(phase) => {
                let dropped = matches!(phase, Phase::Retreat(_) | Phase::Standing(_));
                self.clicks = 0;
                self.episode = None;
                dropped
            }
        }
    }

    /// Drop a walk out or stand-still in flight; keep a stop and the safe
    /// tile (an operator takeover that preserves the survival park).
    pub(super) fn cancel_pending(&mut self) {
        if !matches!(self.phase(), Some(Phase::Stopped(_)) | None) {
            self.clicks = 0;
            self.episode = None;
        }
    }

    /// Forget the episode. The last safe tile stays valid for its scope.
    pub(super) fn clear(&mut self) {
        self.clicks = 0;
        self.episode = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: i32) -> WorldTile {
        WorldTile { x, z: 0, level: 0 }
    }

    #[test]
    fn stranded_needs_a_cure_need_and_no_food() {
        let cure = SurvivalNeed::Cure {
            or_food: 3,
            ref_heal: 12,
            why: crate::survival::math::NeedWhy::Lethal,
        };
        let food_need = SurvivalNeed::Food {
            count: 2,
            ref_heal: 12,
        };
        let none = FoodSummary::default();
        assert!(stranded(cure, &none));
        assert!(!stranded(food_need, &none), "a food need is not poison");
        let ordinary = FoodSummary {
            ordinary_count: 1,
            ..FoodSummary::default()
        };
        assert!(!stranded(cure, &ordinary), "held food: the eater acts");
        let combo = FoodSummary {
            combo_count: 1,
            ..FoodSummary::default()
        };
        assert!(!stranded(cure, &combo), "held combo food: the eater acts");
    }

    #[test]
    fn plan_stops_only_outside_the_danger_area() {
        let safe = Some(tile(1));
        assert_eq!(plan(Some(false), None, safe), StopPlan::Park);
        assert_eq!(plan(Some(false), Some(true), safe), StopPlan::Park);
        assert_eq!(plan(Some(true), Some(false), safe), StopPlan::KeepWalking);
        assert_eq!(plan(Some(true), Some(false), None), StopPlan::KeepWalking);
        assert_eq!(
            plan(Some(true), Some(true), safe),
            StopPlan::Retreat(tile(1))
        );
        assert_eq!(plan(Some(true), None, safe), StopPlan::Retreat(tile(1)));
        assert_eq!(
            plan(Some(true), None, None),
            StopPlan::ParkInDanger(InDanger::NoSafeTile)
        );
        assert_eq!(
            plan(None, Some(false), safe),
            StopPlan::ParkInDanger(InDanger::AreaUnknown)
        );
    }

    #[test]
    fn stop_text_names_the_reason_tile_safety_and_failure() {
        let at = WorldTile {
            x: 2880,
            z: 3600,
            level: 0,
        };
        let outside = Stopped::new(Some(at), None, None);
        assert_eq!(
            outside.reason.as_ref(),
            "poisoned at low HP with no food and no antipoison: stopped at 2880,3600,0 outside the danger area"
        );
        let inside = Stopped::new(None, Some(InDanger::NoSafeTile), None);
        assert_eq!(
            inside.reason.as_ref(),
            "poisoned at low HP with no food and no antipoison: stopped inside the danger area (no safe tile seen in this danger area)"
        );
        let failed = Stopped::new(
            Some(at),
            Some(InDanger::WalkOutFailed),
            Some("walk has no route"),
        );
        assert_eq!(
            failed.reason.as_ref(),
            "poisoned at low HP with no food and no antipoison: stopped at 2880,3600,0 inside the danger area (the walk out did not arrive: walk has no route)"
        );
        let unconfirmed = Stopped::unconfirmed(Some(at), Some("the player still moved"));
        assert_eq!(
            unconfirmed.reason.as_ref(),
            "poisoned at low HP with no food and no antipoison: stopped at 2880,3600,0, but the player was not seen standing still (the player still moved)"
        );
        assert!(!unconfirmed.reason.ends_with(" outside the danger area"));
    }

    #[test]
    fn danger_area_is_the_declared_cross_zones_only() {
        let lease = crate::gatherer::select::tests::test_zone_lease(
            &["throwers", "other"],
            (2, 5, 0, 5),
            tile(0),
        );
        let zones = lease.zones();
        let declared: [Arc<str>; 1] = [Arc::from("throwers")];
        let step_area = DangerArea {
            stage: &[],
            step: &declared,
        };
        let stage_area = DangerArea {
            stage: &declared,
            step: &[],
        };
        for area in [&step_area, &stage_area] {
            assert_eq!(area.contains(zones, tile(3)), Some(true));
            assert_eq!(area.contains(zones, tile(1)), Some(false));
            assert_eq!(area.contains(zones, tile(6)), Some(false));
            assert_eq!(area.contains(None, tile(3)), None, "no table is unknown");
        }
        let undeclared: [Arc<str>; 1] = [Arc::from("elsewhere")];
        let other = DangerArea {
            stage: &undeclared,
            step: &[],
        };
        assert_eq!(other.contains(zones, tile(3)), Some(false));
        let empty = DangerArea {
            stage: &[],
            step: &[],
        };
        assert_eq!(
            empty.contains(None, tile(3)),
            Some(false),
            "nothing declared"
        );
    }

    /// A safe tile belongs to the area it was seen in: another stage, or
    /// another step zone list, does not see it.
    #[test]
    fn safe_tile_is_scoped_to_its_danger_area() {
        let throwers: [Arc<str>; 1] = [Arc::from("throwers")];
        let both: [Arc<str>; 2] = [Arc::from("throwers"), Arc::from("other")];
        let area = |step| DangerArea { stage: &[], step };
        let first = area(&throwers).scope(Some(0));
        assert_eq!(first, area(&throwers).scope(Some(0)), "deterministic");
        assert_ne!(first, area(&throwers).scope(Some(1)), "stage change");
        assert_ne!(first, area(&both).scope(Some(0)), "zone change");
        assert_ne!(
            first,
            DangerArea {
                stage: &throwers,
                step: &[]
            }
            .scope(Some(0)),
            "stage zones and step zones are different areas"
        );
        let mut stop = PoisonStop::default();
        stop.record_safe(tile(1), first);
        assert_eq!(stop.safe_for(first), Some(tile(1)));
        assert_eq!(stop.safe_for(area(&both).scope(Some(0))), None);
        stop.clear();
        assert_eq!(stop.safe_for(first), Some(tile(1)), "clear keeps the tile");
    }

    /// The episode reuses its one box across phases and frees it at the end.
    #[test]
    fn episode_allocates_once_and_bounds_its_stand_clicks() {
        let need = SurvivalNeed::Cure {
            or_food: 3,
            ref_heal: 12,
            why: crate::survival::math::NeedWhy::Lethal,
        };
        let mut stop = PoisonStop::default();
        let first = allocation_counter::measure(|| {
            stop.stand(need, None, None);
        });
        assert_eq!(first.count_total, 1, "the episode box");
        let reused = allocation_counter::measure(|| {
            stop.start_retreat(tile(1), need);
            stop.stand(need, Some(InDanger::NoSafeTile), None);
            assert!(stop.end_standing().is_some());
            stop.replan();
        });
        assert_eq!(reused.count_total, 0, "phases reuse the box");
        for _ in 0..MAX_STAND_CLICKS {
            assert!(stop.take_click());
        }
        assert!(!stop.take_click(), "the click budget is spent");
        stop.stop(Stopped::new(Some(tile(2)), None, None));
        assert_eq!(stop.watched_tile(), Some(tile(2)));
        assert!(stop.end_stop().is_some());
        assert_eq!(stop.clicks(), 0, "a new episode has a fresh budget");
        assert!(stop.episode.is_none());
    }
}
