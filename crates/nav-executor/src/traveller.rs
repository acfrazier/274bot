//! Traveller follows a planner route through the client's `Driver`.
//! Each call to [`Traveller::follow`] polls one step through
//! `api::interact::Interactions` and `api::settle::Settle`.
//! The host calls it once per game tick. It returns `None` while the run is
//! active and `Some(outcome)` when the run ends.

use std::collections::VecDeque;

use std::sync::Arc;

use api::hostlog::{Category, Level};
use api::interact::{press, ActionSpec, Driver, Interactions, OpTarget, SendReason, SendResult};
use api::query::{ChatQueryExt, Query, SceneQuery};
use api::settle::{arrived, Evidence, Outcome, Settle, SettleOptions};
use api::snapshot::{GameSnapshot, LocView, NpcView, ReadContext, WidgetView, WorldTile};
use client::dash3d::CollisionFlag;

use nav_planner::contract::RoutePlanLease;
use nav_planner::essence::{
    essence_session_for_wizard, in_essence_mine, is_essence_entry_edge, EssenceSession,
    ESSENCE_MINE_EXIT_ARRIVE_RADIUS,
};
use nav_planner::executor_adapter::{
    is_dialogue_door_loc, is_slashable_web, plan_leg_count, plan_owned_leg, plan_retained_leg,
    AL_KHARID_TOLL_LEFT_LOC_ID, AL_KHARID_TOLL_RIGHT_LOC_ID, ARDOUGNE_WILDERNESS_LEVER_LOC_ID,
    CELLAR_SHIFT, SHANTAY_HENGE_LOC_ID,
};
use nav_planner::router::{Leg, Route};
use nav_planner::transport::{DoorDir, TransportEdge, TransportKind};
use thiserror::Error;

mod dialog;
mod npc_hop;
mod snapshot;
mod transport_hop;
mod walk;

use dialog::*;
use npc_hop::*;
use snapshot::*;
use transport_hop::*;
use walk::*;

/// Teleport landing scatter: `player_teleport_normal` lands at
/// `map_findsquare(to, 0, 2, lineofwalk)` — a random standable tile
/// within chebyshev 2 of the packed landing, never the tile exactly. A
/// teleport hop's arrive arm (and a scenario's dest proof) must accept
/// this radius, independent of the runner's exact `close_enough`.
const TELEPORT_ARRIVE_RADIUS: i32 = 2;
/// `movecoord(coord(), 0, 0, ±6400)` cellar hops land on the player's
/// tile. Taken from an adjacent stand that is Chebyshev 1 off the loc,
/// so the baked dest (loc ± 6400) is one tile beside the live landing.
/// Independent of host WalkNear `close_enough` 0 — not a global radius.
const CELLAR_ARRIVE_RADIUS: i32 = 1;
/// Pure vertical ladder/stair scripts change the player's level, retaining
/// the adjacent stand rather than teleporting onto the loc-baked tile.
const VERTICAL_ARRIVE_RADIUS: i32 = 1;
/// Gnome glider landing scatter: `p_teleport(map_findsquare($dest, 0, 1,
/// lineofwalk))` in `gnome_glider.rs2` — chebyshev 1, never the pad
/// exactly when a loc/NPC occupies it.
const GLIDER_ARRIVE_RADIUS: i32 = 1;
/// Sailors, customs, glider pilots wander. Content uses an 8-tile inzone
/// around the packed pad (`gnome_glider.rs2`); looking up only cheb 3 of
/// packed `at` misses them and Talk-to from the spawn is Unreachable.
const NPC_SEARCH_RADIUS: i32 = 8;
/// Frozen `CANDIDATE_SETTLE_TRIES` (`WalkExecutor.ts:100-101`).
const SCENE_SETTLE_TRIES: u32 = 3;
/// Frozen `delayTicks(2)` per settle (`WalkExecutor.ts:1182`).
const SCENE_SETTLE_TICKS: u32 = 2;

/// Why a [`Traveller::follow`] hop failed: the per-hop tick budget lapsed
/// while the player was making progress (`Expired`), the player never
/// left the tile the hop was sent from (`Dropped` — the walk or transport
/// interaction was dropped by the game), or the route's end is blocked
/// live next to the player (`EndBlocked`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopFailure {
    Expired,
    Dropped,
    /// Frozen `WalkExecutor` `'blocked'` (`WalkExecutor.ts:990–991,
    /// 1039–1040, 1092–1094`): no walk of this follow was accepted, the
    /// player stands within one tile of the route's last tile on its level,
    /// and the client refused the click onto it for the frozen stall
    /// window (`DEFAULT_PATH_STALL_TICKS`, `pathFollowPolicy.ts:6`) of
    /// distinct ticks. The player is as close as the live scene allows.
    EndBlocked,
}

/// A transport-specific failure that prevents a web cut from being sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum WebCutFailure {
    /// The packed candidate edges needed to select a current cutting action
    /// were not supplied.
    #[error("the packed transport edge list is unavailable")]
    PackedTransportEdgesUnavailable,
    /// No worn blade or carried knife qualifies against the live snapshot.
    #[error("neither a worn slash blade nor a carried knife qualifies")]
    NoQualifyingSlashTool,
}

/// A reason an NPC transport hop could not recover before its budget ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NpcHopBlockReason {
    /// The tracked NPC no longer exists in the loaded scene.
    #[error("tracked target missing")]
    TrackedTargetMissing,
    /// No operable stand remained for the target.
    #[error("no reachable operable stand")]
    NoReachableOperableStand,
    /// The driver refused the walk to the NPC's selected stand.
    #[error("approach route refused")]
    ApproachRouteRefused,
    /// The NPC interaction failed its reach check.
    #[error("reach failure")]
    ReachFailure,
    /// The active dialogue page had no recognized affirmative choice.
    #[error("unrecognized journey dialogue")]
    UnrecognizedJourneyDialogue,
}

fn blocked_target_word(kind: &TransportKind) -> &'static str {
    if is_npc_backed_kind(*kind) {
        "npc"
    } else {
        "loc"
    }
}

/// The typed reason a route leg could not be worked.
///
/// Each variant preserves the human-readable detail emitted by the previous
/// string-based blocked outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BlockedWhy {
    /// A carried teleport item never acquired a usable inventory control.
    #[error("charged teleport item {item_id} never acquired a usable inventory control")]
    TeleportInventoryControlUnavailable { item_id: i32 },
    /// A spellbook teleport button was not accepted by the driver.
    #[error("teleport spell button {button_id} was not accepted by the driver")]
    TeleportSpellButtonRejected { button_id: i32 },
    /// The live effective level did not meet the packed skill prerequisite.
    #[error("teleport needs skill {skill} level {level}; observed {observed:?}")]
    TeleportSkillRequirementUnmet {
        skill: i32,
        level: i32,
        observed: Option<i32>,
    },
    /// The live inventory did not meet the packed item prerequisite.
    #[error(
        "teleport needs {count} of item {item_id} (runes or a charged item); carrying {carried}"
    )]
    TeleportItemRequirementUnmet {
        count: i32,
        item_id: i32,
        carried: i32,
    },
    /// A packed spell landing is not one of the standard spellbook teleports.
    #[error(
        "packed spell teleport to ({}, {}, {}) is not one of the seven standard spellbook teleports",
        .to.x,
        .to.z,
        .to.level
    )]
    UnsupportedSpellTeleport { to: WorldTile },
    /// No reachable operable NPC target appeared within the search radius.
    #[error(
        "transport npc {loc_id} has no reachable operable target within {radius} tiles of ({}, {}, {}) in the loaded scene",
        .at.x,
        .at.z,
        .at.level
    )]
    NpcTargetUnavailable {
        loc_id: i32,
        radius: i32,
        at: WorldTile,
    },
    /// No flood-reachable stand was found for the transport target.
    #[error(
        "no flood-reachable operable stand for transport {} {} at ({}, {}, {}) in the loaded scene",
        blocked_target_word(.kind),
        .loc_id,
        .at.x,
        .at.z,
        .at.level
    )]
    TransportApproachUnavailable {
        kind: TransportKind,
        loc_id: i32,
        at: WorldTile,
    },
    /// The transport target did not appear within its three-tile search area.
    #[error(
        "transport {} {} is not within 3 tiles of ({}, {}, {}) in the loaded scene",
        blocked_target_word(.kind),
        .loc_id,
        .at.x,
        .at.z,
        .at.level
    )]
    TransportTargetOutOfRange {
        kind: TransportKind,
        loc_id: i32,
        at: WorldTile,
    },
    /// A conjunctive worn-item prerequisite was not equipped.
    #[error(
        "transport {} {} requires worn_all_req item {item_id}",
        blocked_target_word(.kind),
        .loc_id
    )]
    WornAllRequirementUnmet {
        kind: TransportKind,
        loc_id: i32,
        item_id: i32,
    },
    /// A slashable web could not be cut for the typed reason.
    #[error("slashable web loc {loc_id} cannot be cut: {reason}")]
    SlashableWebCannotBeCut { loc_id: i32, reason: WebCutFailure },
    /// A web disappeared after its observed cut failure and before retry.
    #[error("slashable web loc {loc_id} disappeared before retrying its observed cut failure")]
    SlashableWebMissingOnRetry { loc_id: i32 },
    /// The live fare inventory did not meet the transport's consumption rule.
    #[error(
        "transport {loc_id} needs {count} of item {item_id} to pay the fare; carrying {carried}"
    )]
    FareUnavailable {
        loc_id: i32,
        count: i32,
        item_id: i32,
        carried: i32,
    },
    /// The active journey dialogue page had no recognized choice.
    #[error("unrecognized journey dialogue")]
    UnrecognizedJourneyDialogue,
    /// No standable tile was found adjacent to the expected troll door.
    #[error(
        "troll door loc {loc_id} has no standable tile within 1 of {tile:?} in the loaded scene"
    )]
    TrollDoorApproachUnavailable { loc_id: i32, tile: WorldTile },
    /// The expected troll-door loc was absent at its packed tile.
    #[error("troll door loc {loc_id} is not at {tile:?} in the loaded scene")]
    TrollDoorMissing { loc_id: i32, tile: WorldTile },
    /// An NPC hop's typed recovery failure outlasted the leg budget.
    #[error(
        "npc hop {loc_id}: {reason}; {attempts} attempts, leg budget {budget} ticks exhausted"
    )]
    NpcHopExpired {
        loc_id: i32,
        reason: NpcHopBlockReason,
        attempts: u32,
        budget: u32,
    },
    /// The NPC hop exhausted its bounded reach retries.
    #[error("npc hop {loc_id} gave up after {attempts} attempts: reach failure")]
    NpcReachRetriesExhausted { loc_id: i32, attempts: u32 },
}

/// The terminal outcome of a [`Traveller::follow`] run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TravelOutcome {
    /// Every leg completed; the player is at `at`.
    Arrived { at: WorldTile },
    /// A hop exhausted its budget without arriving: `Expired` while
    /// creeping, `Dropped` while stuck on the send tile.
    Stalled {
        at: WorldTile,
        aiming: WorldTile,
        why: HopFailure,
        tries: u32,
    },
    /// An `Interactions` send was refused (precondition, off-scene, or
    /// the driver rejected it).
    Refused { at: WorldTile, reason: SendReason },
    /// The leg could not be worked (e.g. a transport loc missing from the
    /// loaded scene).
    Blocked {
        at: WorldTile,
        leg: usize,
        why: BlockedWhy,
    },
    /// A gated transport leg whose quest evidence did not prove its gates
    /// before the transport interaction was sent: nothing was sent for it.
    /// `verdict` is the gates' combined truth (`False` or `Unknown`);
    /// `unresolved` names the gates the evidence leaves `Unknown` (every gate
    /// without evidence, none for evidence from another quest family).
    EvidenceUnproven {
        at: WorldTile,
        leg: usize,
        verdict: api::selected::Truth,
        unresolved: std::sync::Arc<[api::selected::QuestGate]>,
    },
    /// A live content exclusion failed before the transport send.
    EntranceRefused {
        at: WorldTile,
        leg: usize,
        requirement: nav_planner::world_state::EntranceRequirement,
    },
    /// The hop budget (`max_hops`) was exhausted before arrival.
    GaveUp { at: WorldTile, hops: u32 },
}

/// The phase of a leg reported to the `on_leg` callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegPhase {
    /// The leg is being worked.
    Start,
    /// The leg completed.
    Done,
    /// The leg failed (the run terminated on it).
    Failed,
}

/// Navigation observations for optional diagnostics. The receiver controls retention.
#[derive(Debug)]
pub enum TravelEvent {
    TransportState {
        tick: u32,
        at: WorldTile,
        kind: TransportKind,
        loc_id: i32,
        from: WorldTile,
        to: WorldTile,
        open: bool,
        approach: bool,
        troll: bool,
        waited: u32,
        loc_wait: u32,
        budget: u32,
        live_loc: Option<(i32, WorldTile)>,
    },
    TransportAttempt {
        tick: u32,
        kind: TransportKind,
        expected_id: i32,
        actual_id: i32,
        target: WorldTile,
        option: i32,
        refusal: Option<SendReason>,
    },
    StunDeferred {
        observed_tick: u32,
        resume_tick: u32,
    },
    StunResumed {
        tick: u32,
    },
    WalkAttempt {
        tick: u32,
        at: WorldTile,
        aim: WorldTile,
        refusal: Option<SendReason>,
    },
}

/// A callback that observes each active route leg's phase transitions.
pub type TravelLegCallback<'a> = Box<dyn FnMut(&Leg, LegPhase) + 'a>;

/// How a [`Traveller::follow`] run is parameterized. `on_leg` fires once
/// per phase transition; the same options are passed on every poll.
pub struct TravelOptions<'a> {
    /// Chebyshev radius treated as "arrived" for a hop target (default 2).
    pub close_enough: i32,
    /// Optional observations; does not select actions or retry policy.
    pub on_event: Option<Box<dyn FnMut(TravelEvent) + 'a>>,
    /// Tick budget for one hop (a walk send or a transport interact;
    /// default 60).
    pub budget_ticks_per_hop: u32,
    /// Walk hops before `GaveUp` (default 60).
    pub max_hops: u32,
    /// The packed any-tile teleport list (`TransportGraph::teleports`):
    /// a jewellery rub's destination dialog maps the answered option
    /// through the script's `switch_int($choice)`, and the choice of the
    /// edge being followed is the 1-based index of its `to` among the
    /// same-`loc_id` rub edges. `None` (the default) falls back to the
    /// modal's first choice — the dueling ring's single arena, and every
    /// Npc ride dialog.
    pub teleports: Option<&'a [TransportEdge]>,
    /// Packed loc/npc edges (`TransportGraph::edges`): spirit-tree and
    /// glider destination dialogs index `to` among same-`loc_id` siblings.
    pub edges: Option<&'a [TransportEdge]>,
    /// Pinned evidence used by route selection, rechecked before gated
    /// transport sends. Missing or undecided evidence fails closed.
    pub quest_evidence: Option<&'a nav_planner::quest_gates::QuestEvidence>,
    /// Per-leg phase callback; fired during the poll that crosses the
    /// transition. May borrow the caller (like `Evidence<'a>` in settle).
    pub on_leg: Option<TravelLegCallback<'a>>,
}

impl Default for TravelOptions<'_> {
    fn default() -> Self {
        TravelOptions {
            close_enough: 2,
            on_event: None,
            budget_ticks_per_hop: 60,
            max_hops: 60,
            teleports: None,
            edges: None,
            quest_evidence: None,
            on_leg: None,
        }
    }
}

/// Follows a planner route one leg at a time.
pub struct Traveller {
    /// The active high-level follow run (Task 14); `None` when idle.
    follow: Option<FollowRun>,
    /// Original-route leg for the most recent terminal follow outcome. Kept
    /// after the run is cleared so the host can log that exact outcome before
    /// clearing its route; reset when another follow starts.
    follow_terminal_leg: Option<usize>,
    /// The per-slot Rune Essence mine latch: set when an essence entry
    /// hop completes, the mine exit loc may only return to the entry
    /// wizard. Survives across routes (the game's `%exit_essence_mine_coord`
    /// varp persists too); a new entry overwrites it.
    essence: Option<EssenceSession>,
}

impl Traveller {
    /// An idle traveller with no active follow run.
    pub fn new() -> Self {
        Self {
            follow: None,
            follow_terminal_leg: None,
            essence: None,
        }
    }

    /// Drop the active follow run and its terminal-leg record.
    pub fn clear(&mut self) {
        self.follow = None;
        self.follow_terminal_leg = None;
    }

    /// The latched Rune Essence mine session: the wizard the player last
    /// entered the mine through, and the overworld tile the mine exit
    /// portal returns to. `None` before the first essence entry (and for
    /// bots that never visit the mine). The caller passes this back into
    /// [`nav_planner::router::FindOptions::essence`] so a route from inside the
    /// mine can use the return hop.
    pub fn essence(&self) -> Option<EssenceSession> {
        self.essence
    }

    /// Replace the per-slot essence-mine latch. The traveller latches it
    /// itself on a completed essence entry hop; the host may seed it to
    /// restore a slot's state, and the unit tests construct the latched
    /// state directly. [`Traveller::clear`] keeps it — like the game's
    /// `%exit_essence_mine_coord` varp, the exit wizard survives route
    /// teardown.
    pub fn set_essence(&mut self, essence: Option<EssenceSession>) {
        self.essence = essence;
    }

    /// The tile the traveller is currently walking toward: the active walk
    /// hop's aim, the transport hop's approach tile, or the transport
    /// arrival tile. `None` when no follow run is active (idle). The
    /// nav-debug paint strokes this as the click target.
    pub fn current_aim(&self) -> Option<WorldTile> {
        let run = self.follow.as_ref()?;
        if let Some(walk) = &run.walk {
            return Some(walk.aim);
        }
        let transport = run.transport.as_ref()?;
        transport
            .approach
            .as_ref()
            .map(|a| a.tile)
            .or(Some(transport.to))
    }

    /// Original-route leg associated with the most recent terminal
    /// [`TravelOutcome`]. `None` for an empty route arrival or before a run
    /// terminates.
    pub fn terminal_leg_index(&self) -> Option<usize> {
        self.follow_terminal_leg
    }

    /// Advance the high-level route follower one step: pollable, never
    /// blocking. `None` means the route is still being followed — call
    /// again next tick with the same `route` (a clone) and `options`.
    /// `Some(outcome)` is terminal; the run is cleared before returning.
    ///
    /// The route is consumed when a run starts; while a run is active the
    /// passed `route` is ignored, so the caller re-passes it (or a clone)
    /// on every poll. The snapshot is the host's per-tick `GameSnapshot`;
    /// `Interactions` and `Settle` are built fresh from it each call, so
    /// each call performs at most one driver send (walk or transport op)
    /// plus one settle poll. Door-kind edges with either a packed open leaf
    /// or a cardinal direction (except dialogue doors and slashable webs)
    /// use state-aware recovery inside the original hop budget. Retry only
    /// against the matching closed loc when it is present; a self-closing
    /// script's displaced inactive leaf is never treated as open.
    pub fn follow<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        route: Route,
        options: &mut TravelOptions<'_>,
    ) -> Option<TravelOutcome> {
        self.follow_with_route(d, snapshot, || route, options)
    }

    /// Follow a route retained by shared immutable owners without deep-cloning
    /// it on every poll. The active run takes its own leg queue only when it
    /// starts; later polls discard this cheap Arc clone.
    pub fn follow_shared<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        route: Arc<Route>,
        options: &mut TravelOptions<'_>,
    ) -> Option<TravelOutcome> {
        self.follow_with_route(d, snapshot, || route.as_ref().clone(), options)
    }

    /// Follow a retained compact route plan without expanding its leg vector.
    /// The active run retains a cheap plan handle and resolves each borrowed
    /// leg only when it becomes current.
    pub fn follow_plan<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        plan: RoutePlanLease,
        options: &mut TravelOptions<'_>,
    ) -> Option<TravelOutcome> {
        self.follow_with_start(d, snapshot, options, |options| {
            FollowRun::start_plan(plan, options)
        })
    }

    fn follow_with_start<'a, D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions<'a>,
        start_run: impl FnOnce(&TravelOptions<'a>) -> FollowRun,
    ) -> Option<TravelOutcome> {
        if self.follow.is_none() {
            self.follow_terminal_leg = None;
            self.follow = Some(start_run(options));
        }
        let outcome = self
            .follow
            .as_mut()
            .and_then(|run| run.step(d, snapshot, options, &mut self.essence));
        if api::hostlog::debug_enabled() {
            let run = self.follow.as_ref();
            api::host_log!(
                Category::NavTrace,
                Level::Debug,
                "follow here={:?} walk={} transport={} leg={} hops={} outcome={:?}",
                here(snapshot),
                run.is_some_and(|r| r.walk.is_some()),
                run.is_some_and(|r| r.transport.is_some()),
                run.map(|r| r.leg_index).unwrap_or(0),
                run.map(|r| r.hops).unwrap_or(0),
                outcome,
            );
        }
        if outcome.is_some() {
            self.follow_terminal_leg = self.follow.as_ref().and_then(|run| {
                if matches!(&outcome, Some(TravelOutcome::Arrived { .. })) {
                    run.leg_index.checked_sub(1)
                } else {
                    Some(run.leg_index)
                }
            });
            self.follow = None;
        }
        outcome
    }

    fn follow_with_route<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        start_route: impl FnOnce() -> Route,
        options: &mut TravelOptions<'_>,
    ) -> Option<TravelOutcome> {
        self.follow_with_start(d, snapshot, options, |options| {
            FollowRun::start(start_route(), options)
        })
    }
}

impl Default for Traveller {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// The follow run (Task 14): the legs still to work plus the per-hop state.
// ---------------------------------------------------------------------------

// Server thieving stuns last 8 or 10 ticks; their visual starts one tick
// before the lock. One window/recovery per route bounds this inference.
const THIEVING_STUN_WINDOW: u32 = 11;

enum RouteLegSource {
    Legacy(VecDeque<Leg>),
    Plan {
        plan: RoutePlanLease,
        next_leg: usize,
        leg_count: usize,
    },
}

impl RouteLegSource {
    fn remaining_is_empty(&self) -> bool {
        match self {
            Self::Legacy(legs) => legs.is_empty(),
            Self::Plan {
                next_leg,
                leg_count,
                ..
            } => next_leg >= leg_count,
        }
    }
}

enum FollowLeg {
    Legacy(Leg),
    Plan { plan: RoutePlanLease, index: usize },
}

impl FollowLeg {
    fn walk_tiles(&self) -> Option<&[WorldTile]> {
        match self {
            Self::Legacy(Leg::Walk { tiles }) => Some(tiles),
            Self::Legacy(Leg::Transport { .. }) => None,
            Self::Plan { plan, index } => match plan_retained_leg(plan, *index)? {
                nav_planner::router::CompactRouteLeg::Transport(_) => None,
                nav_planner::router::CompactRouteLeg::Walk(tiles) => Some(tiles),
            },
        }
    }

    fn into_transport(self) -> Leg {
        match self {
            Self::Legacy(leg @ Leg::Transport { .. }) => leg,
            Self::Legacy(Leg::Walk { .. }) => unreachable!("walk leg routed as transport"),
            Self::Plan { plan, index } => {
                let leg = plan_owned_leg(&plan, index)
                    .expect("retained route plan leg remains addressable");
                if !matches!(leg, Leg::Transport { .. }) {
                    unreachable!("compact route walk leg routed as transport");
                }
                leg
            }
        }
    }
}

/// The legs still to work and current hop state. Budgets are captured at
/// start; callbacks stay with the caller's options.
struct FollowRun {
    stun_wait: Option<u32>,
    stun_recovery_used: bool,
    legs: RouteLegSource,
    pending_leg: Option<FollowLeg>,
    /// The index (in the original route) of the leg being worked, reported
    /// in `Blocked`; grows as legs complete.
    leg_index: usize,
    /// Walk hops performed so far (the m8aq `hops` counter).
    hops: u32,
    max_hops: u32,
    close_enough: i32,
    budget: u32,
    /// Ticks the current transport leg has waited for its loc to appear.
    loc_wait: u32,
    /// Refused clicks this follow may still sit out while nothing was
    /// clicked yet (frozen `CANDIDATE_SETTLE_TRIES`, one budget per walk).
    settle_left: u32,
    /// The tick a sat-out refusal is retried at (frozen `delayTicks(2)`).
    settle_until: Option<u32>,
    walk: Option<WalkHop>,
    transport: Option<TransportHop>,
}

impl FollowRun {
    fn start(route: Route, options: &TravelOptions<'_>) -> FollowRun {
        Self::with_legs(RouteLegSource::Legacy(route.legs.into()), options)
    }

    fn start_plan(plan: RoutePlanLease, options: &TravelOptions<'_>) -> FollowRun {
        let leg_count = plan_leg_count(&plan);
        Self::with_legs(
            RouteLegSource::Plan {
                plan,
                next_leg: 0,
                leg_count,
            },
            options,
        )
    }

    fn with_legs(legs: RouteLegSource, options: &TravelOptions<'_>) -> FollowRun {
        FollowRun {
            stun_wait: None,
            stun_recovery_used: false,
            legs,
            pending_leg: None,
            leg_index: 0,
            hops: 0,
            max_hops: options.max_hops,
            close_enough: options.close_enough,
            budget: options.budget_ticks_per_hop,
            loc_wait: 0,
            settle_left: SCENE_SETTLE_TRIES,
            settle_until: None,
            walk: None,
            transport: None,
        }
    }

    fn next_leg(&mut self) -> Option<FollowLeg> {
        if let Some(leg) = self.pending_leg.take() {
            return Some(leg);
        }
        match &mut self.legs {
            RouteLegSource::Legacy(legs) => legs.pop_front().map(FollowLeg::Legacy),
            RouteLegSource::Plan {
                plan,
                next_leg,
                leg_count,
            } => {
                if *next_leg >= *leg_count {
                    return None;
                }
                let index = *next_leg;
                *next_leg += 1;
                Some(FollowLeg::Plan {
                    plan: plan.clone(),
                    index,
                })
            }
        }
    }

    fn requeue_transport(&mut self, leg: Leg) {
        self.pending_leg = Some(FollowLeg::Legacy(leg));
    }

    /// One poll step: settle an active hop, or start the next leg (sending
    /// its first hop). At most one driver send per call. A settle poll that
    /// is still watching ends the call (`None`); only a matched settle may
    /// advance the run — to the next leg or to a terminal outcome.
    /// `essence` is the traveller's per-slot mine latch: a completed
    /// essence entry hop records the wizard here.
    fn step<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions<'_>,
        essence: &mut Option<EssenceSession>,
    ) -> Option<TravelOutcome> {
        if snapshot.ingame() && snapshot.attached() {
            if !self.stun_recovery_used {
                if let Some(observed) = snapshot.thieving_stun_tick() {
                    let after_send = |sent: u32| observed.wrapping_sub(sent) < (1u32 << 31);
                    let pending_was_affected = self
                        .walk
                        .as_ref()
                        .is_some_and(|h| h.sent && after_send(h.sent_tick))
                        || self
                            .transport
                            .as_ref()
                            .and_then(|h| h.approach.as_ref())
                            .is_some_and(|a| after_send(a.sent_tick));
                    if snapshot.tick().wrapping_sub(observed) < THIEVING_STUN_WINDOW
                        || pending_was_affected
                    {
                        self.stun_recovery_used = true;
                        self.stun_wait = Some(observed);
                        if let Some(cb) = options.on_event.as_mut() {
                            cb(TravelEvent::StunDeferred {
                                observed_tick: observed,
                                resume_tick: observed.wrapping_add(THIEVING_STUN_WINDOW),
                            });
                        }
                    }
                }
            }
            if let Some(observed) = self.stun_wait {
                if snapshot.tick().wrapping_sub(observed) < THIEVING_STUN_WINDOW {
                    return None;
                }
                self.stun_wait = None;
                let at = here(snapshot);
                if let Some(hop) = self.walk.as_mut() {
                    let radius = if hop.aim_index + 1 >= hop.tiles().len() {
                        0
                    } else {
                        self.close_enough.max(2)
                    };
                    if hop.sent && (at.level != hop.aim.level || cheb(at, hop.aim) > radius) {
                        hop.sent = false;
                    }
                }
                if let Some(approach) = self.transport.as_mut().and_then(|h| h.approach.as_mut()) {
                    approach.retry_pending = true;
                }
                if let Some(cb) = options.on_event.as_mut() {
                    cb(TravelEvent::StunResumed {
                        tick: snapshot.tick(),
                    });
                }
            }
        } else {
            // Keep existing disconnect/refusal handling reachable even when
            // no further PLAYER_INFO ticks arrive to advance the stun window.
            self.stun_wait = None;
        }
        loop {
            if self.walk.is_some() {
                match self.poll_walk(d, snapshot, options) {
                    Poll::Terminal(outcome) => return Some(outcome),
                    Poll::Watching => return None,
                    Poll::LegDone => continue,
                }
            }
            if self.transport.is_some() {
                match self.poll_transport(d, snapshot, options, essence) {
                    Poll::Terminal(outcome) => return Some(outcome),
                    Poll::Watching => return None,
                    Poll::LegDone => continue,
                }
            }
            // No active hop: work the next leg (or finish).
            let Some(route_leg) = self.next_leg() else {
                return Some(TravelOutcome::Arrived { at: here(snapshot) });
            };
            if let Some(tiles) = route_leg.walk_tiles() {
                fire_follow_leg(options, &route_leg, LegPhase::Start);
                // Single-tile walk legs (the `find(from == to)` shape)
                // carry no distance: no-op, never send.
                if tiles.len() <= 1 {
                    fire_follow_leg(options, &route_leg, LegPhase::Done);
                    self.leg_index += 1;
                    continue;
                }
                let here = here(snapshot);
                let hop = WalkHop {
                    leg: route_leg,
                    cursor: 0,
                    aim: here,
                    aim_index: 0,
                    sent: false,
                    ticks_waited: 0,
                    sent_tile: None,
                    sent_tick: snapshot.tick(),
                    start_tile: here,
                    first_sent_tick: snapshot.tick(),
                    tries: 0,
                    stall_idle_ticks: 0,
                    stall_idle_last_tick: None,
                    stall_idle_at: None,
                    end_refused_ticks: 0,
                    end_refused_last_tick: None,
                };
                match self.send_walk_hop(d, snapshot, options, hop, here) {
                    Poll::Watching => return None,
                    Poll::Terminal(outcome) => return Some(outcome),
                    Poll::LegDone => continue,
                }
            }
            let leg = route_leg.into_transport();
            match leg {
                Leg::Walk { .. } => unreachable!("route transport materialized as a walk leg"),
                Leg::Transport { .. } => {
                    let edge = match &leg {
                        Leg::Transport { edge } => edge,
                        Leg::Walk { .. } => unreachable!("transport arm holds a transport leg"),
                    };
                    fire_leg(options, &leg, LegPhase::Start);
                    let here = here(snapshot);
                    if let Some(outcome) =
                        self.check_transport_gate(edge, here, snapshot, options.quest_evidence)
                    {
                        fire_leg(options, &leg, LegPhase::Failed);
                        return Some(outcome);
                    }
                    // A packed Teleport hop is any-tile: no approach and no
                    // loc/npc target — the edge names the op itself (the
                    // held-item Rub on the charged jewellery obj, or the
                    // spellbook button of the standard spell the landing
                    // identifies), never the WalkTo `::tele` cheat.
                    if edge.kind == TransportKind::Teleport {
                        match teleport_send(snapshot, d, edge) {
                            TeleportSend::Sent => {
                                let to = edge.to;
                                self.loc_wait = 0;
                                self.transport = Some(TransportHop {
                                    leg,
                                    to,
                                    ticks_waited: 0,
                                    sent_tile: Some(here),
                                    arrival_footprint: None,
                                    tries: 0,
                                    troll: false,
                                    npc_index: None,
                                    npc_recovery: NpcRecovery::default(),
                                    open_sent_tick: None,
                                    door_retry: DoorRetry::default(),
                                    chat_seq: chat_seq(snapshot),
                                    dialog_page: None,
                                    approach: None,
                                });
                                return None;
                            }
                            TeleportSend::Refused(reason) => {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Refused { at: here, reason });
                            }
                            TeleportSend::Wait(reason) => {
                                self.loc_wait += 1;
                                if self.loc_wait > self.budget {
                                    fire_leg(options, &leg, LegPhase::Failed);
                                    return Some(TravelOutcome::Blocked {
                                        at: here,
                                        leg: self.leg_index,
                                        why: reason.into(),
                                    });
                                }
                                self.requeue_transport(leg);
                                return None;
                            }
                            TeleportSend::Blocked(why) => {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Blocked {
                                    at: here,
                                    leg: self.leg_index,
                                    why,
                                });
                            }
                        }
                    }
                    // Multiloc open-state: when the live door loc at `at`
                    // already reads open, interacting is wrong (an OP on
                    // the open leaf would Close it) — walk straight
                    // through to `to` and settle `arrived(to)` with the
                    // normal budget. Doors only: an Npc edge's `at` is the
                    // driver's tile, and a nearby loc with a Close op
                    // would misread as an open leaf.
                    if edge.kind == TransportKind::Door && edge_loc_open(snapshot, edge) {
                        let to = edge.to;
                        let mut ix = Interactions::new(snapshot, d);
                        match ix.walk(to) {
                            SendResult::Sent { .. } => {
                                self.loc_wait = 0;
                                self.transport = Some(TransportHop {
                                    leg,
                                    to,
                                    ticks_waited: 0,
                                    sent_tile: Some(here),
                                    arrival_footprint: None,
                                    tries: 0,
                                    troll: false,
                                    npc_index: None,
                                    npc_recovery: NpcRecovery::default(),
                                    open_sent_tick: None,
                                    door_retry: DoorRetry::default(),
                                    chat_seq: chat_seq(snapshot),
                                    dialog_page: None,
                                    approach: None,
                                });
                                return None;
                            }
                            SendResult::Refused { reason, .. } => {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Refused { at: here, reason });
                            }
                        }
                    }
                    // The game only accepts an `op_loc`/`op_npc` from an
                    // operable adjacent stand. Packed `at` is the spawn, not
                    // a leash: sailors/officers/pilots wander, and OP_NPC
                    // paths to the live network route head rather than the
                    // rendered/interpolated tile.
                    let mut selected_npc_index = None;
                    let mut selected_stand = None;
                    let approach_at = if npc_backed(edge) {
                        match select_npc_approach(snapshot, edge, here, None, &[]) {
                            Some((npc, stand)) => {
                                selected_npc_index = Some(npc.index);
                                selected_stand = Some(stand);
                                npc.network
                            }
                            None => {
                                self.loc_wait += 1;
                                if self.loc_wait > self.budget {
                                    fire_leg(options, &leg, LegPhase::Failed);
                                    return Some(TravelOutcome::Blocked {
                                        at: here,
                                        leg: self.leg_index,
                                        why: BlockedWhy::NpcTargetUnavailable {
                                            loc_id: edge.loc_id,
                                            radius: NPC_SEARCH_RADIUS,
                                            at: edge.at,
                                        },
                                    });
                                }
                                self.requeue_transport(leg);
                                return None;
                            }
                        }
                    } else {
                        edge.takeoff.unwrap_or(edge.at)
                    };
                    let needs_approach = if let Some(takeoff) = edge.takeoff {
                        here != takeoff
                            || loc_transport_ready(snapshot, edge, here).is_some_and(|ready| !ready)
                    } else if selected_npc_index.is_some() {
                        !find_transport_target_instance(snapshot, edge, selected_npc_index)
                            .is_some_and(|target| match target {
                                TransportTarget::Npc(npc) => {
                                    npc_interaction_ready(snapshot, here, npc)
                                }
                                TransportTarget::Loc(_) => false,
                            })
                    } else {
                        loc_transport_ready(snapshot, edge, here)
                            .map_or(cheb(here, approach_at) > 1, |ready| !ready)
                    };
                    if needs_approach {
                        if self
                            .settle_until
                            .is_some_and(|until| snapshot.tick() < until)
                        {
                            self.requeue_transport(leg);
                            return None;
                        }
                        self.settle_until = None;
                        let Some(approach) = edge.takeoff.or(selected_stand).or_else(|| {
                            if npc_backed(edge) {
                                return None;
                            }
                            if loc_transport_ready(snapshot, edge, here).is_some() {
                                if let Some(loc) = find_transport_loc(snapshot, edge) {
                                    let flood = SceneQuery::new(snapshot.scene(), Some(here))
                                        .flood_reach()?;
                                    return api::query::loc_approach::booth_approach(
                                        loc,
                                        snapshot.scene(),
                                        here,
                                        &flood,
                                    )?
                                    .dest;
                                }
                            }
                            approach_tile(snapshot, approach_at, here)
                        }) else {
                            // No standable tile adjacent to the target in
                            // the loaded scene: keep waiting, bounded by
                            // the hop budget. The leg stays on the front
                            // while waiting.
                            self.loc_wait += 1;
                            if self.loc_wait > self.budget {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Blocked {
                                    at: here,
                                    leg: self.leg_index,
                                    why: BlockedWhy::TransportApproachUnavailable {
                                        kind: edge.kind,
                                        loc_id: edge.loc_id,
                                        at: edge.at,
                                    },
                                });
                            }
                            self.requeue_transport(leg);
                            return None;
                        };
                        let to = edge.to;
                        let mut ix = Interactions::new(snapshot, d);
                        let result = ix.walk(approach);
                        report_walk(options, snapshot, here, approach, &result);
                        match result {
                            SendResult::Sent { .. } => {
                                let npc_recovery = NpcRecovery {
                                    waited: self.loc_wait,
                                    last_tick: Some(snapshot.tick()),
                                    ..Default::default()
                                };
                                self.loc_wait = 0;
                                self.transport = Some(TransportHop {
                                    leg,
                                    to,
                                    ticks_waited: 0,
                                    sent_tile: Some(here),
                                    arrival_footprint: None,
                                    tries: 0,
                                    troll: false,
                                    open_sent_tick: None,
                                    door_retry: DoorRetry::default(),
                                    chat_seq: chat_seq(snapshot),
                                    npc_index: selected_npc_index,
                                    npc_recovery,
                                    dialog_page: None,
                                    approach: Some(ApproachHop {
                                        tile: approach,
                                        at: approach_at,
                                        sent_tick: snapshot.tick(),
                                        ticks_waited: 0,
                                        retry_pending: false,
                                    }),
                                });
                                return None;
                            }
                            // Frozen `WalkExecutor.ts:1178-1184`: with nothing
                            // clicked yet the scene may still be loading
                            // (a region rebuild empties the client's local
                            // route), so a refused click is sat out two ticks
                            // at a time, `CANDIDATE_SETTLE_TRIES` per walk,
                            // before it ends the follow.
                            SendResult::Refused {
                                reason:
                                    SendReason::Unreachable
                                    | SendReason::OffScene
                                    | SendReason::SceneUnavailable,
                                ..
                            } if self.settle_left > 0 => {
                                self.settle_left -= 1;
                                self.settle_until =
                                    Some(snapshot.tick().saturating_add(SCENE_SETTLE_TICKS));
                                self.requeue_transport(leg);
                                return None;
                            }
                            SendResult::Refused { reason, .. } => {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Refused { at: here, reason });
                            }
                        }
                    }
                    match find_transport_target_instance(snapshot, edge, selected_npc_index) {
                        Some(target) => {
                            let to = edge.to;
                            let action = if is_slashable_web(edge) {
                                match current_web_action(snapshot, edge, options.edges) {
                                    Ok(action) => action,
                                    Err(reason) => {
                                        fire_leg(options, &leg, LegPhase::Failed);
                                        return Some(web_action_failure(
                                            here,
                                            self.leg_index,
                                            edge,
                                            reason,
                                        ));
                                    }
                                }
                            } else {
                                edge
                            };
                            let chat_seq_at_send = chat_seq(snapshot);
                            let arrival_footprint = target.footprint();
                            let mut ix = Interactions::new(snapshot, d);
                            match interact_transport(
                                snapshot,
                                &mut ix,
                                target,
                                action,
                                &mut options.on_event,
                            ) {
                                SendResult::Sent { .. } => {
                                    // Already-open trapdoor: this interact is
                                    // Climb-down. Mark tries so poll_transport
                                    // does not send it a second time.
                                    let tries = if npc_backed(edge)
                                        || (edge.kind == TransportKind::Door
                                            && !is_slashable_web(edge))
                                        || (edge.open_loc_id.is_some()
                                            && edge.kind != TransportKind::Door
                                            && edge_loc_open(snapshot, edge))
                                    {
                                        1
                                    } else {
                                        0
                                    };
                                    // A door Open arms the one crossing
                                    // probe: a scripted door can place the
                                    // player on `at` and never read open
                                    // (see `door_step_pending`).
                                    let open_sent_tick =
                                        (edge.kind == TransportKind::Door).then(|| snapshot.tick());
                                    let npc_recovery = NpcRecovery {
                                        waited: self.loc_wait,
                                        last_tick: Some(snapshot.tick()),
                                        ..Default::default()
                                    };
                                    self.loc_wait = 0;
                                    self.transport = Some(TransportHop {
                                        leg,
                                        to,
                                        ticks_waited: 0,
                                        sent_tile: Some(here),
                                        arrival_footprint,
                                        tries,
                                        troll: false,
                                        npc_index: selected_npc_index,
                                        npc_recovery,
                                        open_sent_tick,
                                        door_retry: DoorRetry::opened_at(open_sent_tick),
                                        chat_seq: chat_seq_at_send,
                                        dialog_page: None,
                                        approach: None,
                                    });
                                    return None;
                                }
                                SendResult::Refused { reason, .. } => {
                                    fire_leg(options, &leg, LegPhase::Failed);
                                    return Some(TravelOutcome::Refused { at: here, reason });
                                }
                            }
                        }
                        None => {
                            // The target has not appeared in the loaded
                            // scene yet: keep waiting, bounded by the hop
                            // budget. The leg stays on the front while
                            // waiting.
                            self.loc_wait += 1;
                            if self.loc_wait > self.budget {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Blocked {
                                    at: here,
                                    leg: self.leg_index,
                                    why: BlockedWhy::TransportTargetOutOfRange {
                                        kind: edge.kind,
                                        loc_id: edge.loc_id,
                                        at: edge.at,
                                    },
                                });
                            }
                            self.requeue_transport(leg);
                            return None;
                        }
                    }
                }
            }
        }
    }

    /// Recheck worn-all equipment, content exclusions and quest evidence
    /// before a transport send. This never changes the player's equipment
    /// or deposits an excluded item.
    fn check_transport_gate(
        &self,
        edge: &TransportEdge,
        at: WorldTile,
        snapshot: &GameSnapshot,
        evidence: Option<&nav_planner::quest_gates::QuestEvidence>,
    ) -> Option<TravelOutcome> {
        if let Some(requirement) =
            nav_planner::world_state::WorldState::snapshot_exclusion_failure(edge, snapshot)
        {
            return Some(TravelOutcome::EntranceRefused {
                at,
                leg: self.leg_index,
                requirement,
            });
        }
        if let Some(missing) = edge.worn_all_req.iter().find(|&&id| {
            !snapshot
                .equipment()
                .iter()
                .any(|item| item.count > 0 && item.def.id == id)
        }) {
            return Some(TravelOutcome::Blocked {
                at,
                leg: self.leg_index,
                why: BlockedWhy::WornAllRequirementUnmet {
                    kind: edge.kind,
                    loc_id: edge.loc_id,
                    item_id: *missing,
                },
            });
        }
        use api::selected::Truth;
        let gates = edge.quest_gates.as_ref()?;
        let verdict = gates.test(evidence);
        if verdict == Truth::True {
            return None;
        }
        let unresolved = match evidence {
            None => gates.gates().into(),
            Some(evidence) if evidence.family() != gates.family() => Vec::new().into(),
            Some(evidence) => gates
                .gates()
                .iter()
                .filter(|gate| evidence.test(gate) == Truth::Unknown)
                .cloned()
                .collect(),
        };
        Some(TravelOutcome::EvidenceUnproven {
            at,
            leg: self.leg_index,
            verdict,
            unresolved,
        })
    }
}

/// The outcome of one settle poll inside a follow step.
enum Poll {
    /// The hop is still being watched: the call ends (`None` to the host).
    Watching,
    /// The hop's leg completed: the run may start the next leg (or finish)
    /// in the same call.
    LegDone,
    /// The run terminated on this poll.
    Terminal(TravelOutcome),
}

/// One walk-leg hop: the leg's tiles, the aim tile, and the stall clock.
struct WalkHop {
    leg: FollowLeg,
    /// Index into the leg tiles of the last cleared tile.
    cursor: usize,
    aim: WorldTile,
    aim_index: usize,
    /// Whether the walk for `aim` has been sent; a matched mid-leg hop is
    /// re-armed with `sent: false` and sent on the same poll (click-ahead).
    sent: bool,
    sent_tick: u32,
    /// Original send for this aim, retained across stall-recovery resends.
    start_tile: WorldTile,
    first_sent_tick: u32,
    ticks_waited: u32,
    /// The player's tile when the hop's walk was sent. Used for
    /// Dropped vs Expired stall classification (`here == sent_tile` →
    /// Dropped). Stalled-walk recovery idle is tracked separately at the
    /// latest observed position (`stall_idle_at`).
    sent_tile: Option<WorldTile>,
    tries: u32,
    /// Distinct game ticks observed idle at `stall_idle_at` without actor
    /// movement — a map flag alone does not prove the server is walking.
    /// Separate from `ticks_waited` (poll budget); duplicate snapshot
    /// polls do not count. Tile movement resets this counter.
    stall_idle_ticks: u32,
    /// Snapshot tick last credited to `stall_idle_ticks`.
    stall_idle_last_tick: Option<u32>,
    /// Latest observed tile while counting stalled-walk idle. A change
    /// of tile (partial hop progress) clears the idle window and starts
    /// a new one at the new position.
    stall_idle_at: Option<WorldTile>,
    /// Distinct game ticks the route's last tile, one step away, refused
    /// the click while no walk of this follow was accepted
    /// ([`HopFailure::EndBlocked`]).
    end_refused_ticks: u32,
    /// Snapshot tick last credited to `end_refused_ticks`.
    end_refused_last_tick: Option<u32>,
}

impl WalkHop {
    fn tiles(&self) -> &[WorldTile] {
        self.leg.walk_tiles().expect("walk hop retains a walk leg")
    }

    /// Reconstruct the legacy callback value when a phase transition fires.
    fn leg(&self) -> Leg {
        match &self.leg {
            FollowLeg::Legacy(leg) => leg.clone(),
            FollowLeg::Plan { plan, index } => plan_owned_leg(plan, *index)
                .expect("retained route plan walk leg remains addressable"),
        }
    }
}

/// One transport-leg hop: the edge (for the phase callback) plus the
/// arrival target and the stall clock. `troll` marks state-aware door
/// recovery within the original hop budget for Door-kind edges with a packed
/// open leaf or cardinal crossing direction (except dialogue doors and webs):
/// re-read the matching loc, Open while closed (paced by `door_retry`), probe
/// and walk when open. `chat_seq` is the watermark
/// for fresh "I can't reach that!" and web cut-failure evidence; NPC-backed
/// hops refresh it at every interaction and recover only before fare dialogue.
struct TransportHop {
    leg: Leg,
    to: WorldTile,
    ticks_waited: u32,
    sent_tile: Option<WorldTile>,
    /// Live loc footprint captured at click time; survives the plane rebuild.
    arrival_footprint: Option<(WorldTile, i32, i32)>,
    tries: u32,
    troll: bool,
    /// Current NPC slot, re-picked by reachable stand cost before interactions.
    /// Once fare dialogue starts, the hop stays with it until arrival/expiry.
    npc_index: Option<usize>,
    npc_recovery: NpcRecovery,
    /// The tick a door Open was sent: one crossing probe
    /// ([`door_step_pending`]) on a later delivered tick. Only the troll
    /// reads it (a door hop can send its Open before the troll rule marks
    /// it, at creation or during the approach): it
    /// spends it on the next tick, except that it keeps it while the player
    /// stands on `at` behind a closed leaf and a wall (a step there would
    /// cancel the queued Open).
    open_sent_tick: Option<u32>,
    /// Door recovery's re-Open pacing (see [`DoorRetry`]).
    door_retry: DoorRetry,
    chat_seq: i32,
    /// The chat option-page last answered (joined option texts). A new
    /// page (spirit-tree dest list after "Where can I go?") is answered;
    /// the same page is never re-pressed.
    dialog_page: Option<String>,
    /// The pre-interact approach walk: a standable tile adjacent to the live
    /// interaction target. NPC-backed hops re-arm this against the tracked
    /// instance's current network tile before sending; loc-backed hops use
    /// the edge's `at`. `None` after the interaction is sent (or on a hop
    /// with no interact, like the open-leaf walk-through).
    approach: Option<ApproachHop>,
}

/// The transport approach: a standable take-off tile within Chebyshev 1 of
/// the target position, the position the adjacency settle is measured
/// against, its per-arm loc/door budget, and the send state.
struct ApproachHop {
    sent_tick: u32,
    ticks_waited: u32,
    retry_pending: bool,
    tile: WorldTile,
    at: WorldTile,
}

/// Fire the `on_leg` callback for a phase transition, using the `options`
/// of the poll in which the transition happened.
fn fire_leg(options: &mut TravelOptions<'_>, leg: &Leg, phase: LegPhase) {
    if let Some(cb) = options.on_leg.as_mut() {
        cb(leg, phase);
    }
}

fn fire_follow_leg(options: &mut TravelOptions<'_>, leg: &FollowLeg, phase: LegPhase) {
    if let Some(cb) = options.on_leg.as_mut() {
        match leg {
            FollowLeg::Legacy(leg) => cb(leg, phase),
            FollowLeg::Plan { plan, index } => {
                let leg = plan_owned_leg(plan, *index)
                    .expect("retained route plan leg remains addressable");
                cb(&leg, phase);
            }
        }
    }
}

#[cfg(test)]
#[path = "traveller_tests.rs"]
mod tests;
