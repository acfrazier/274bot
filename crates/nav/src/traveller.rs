//! Traveller: drives a route through the kernel `Driver` over ticks. The
//! caller supplies the player's current tile and the door-open state each
//! tick; the traveller targets walk legs one hop ahead and works a door
//! leg by `op_loc` while closed. When the caller reports the door open,
//! walk through the same tick without OP_LOC1 (that would Close).
//!
//! The high-level [`Traveller::follow`] is a new pollable layer: it drives
//! a [`crate::router::Route`] leg-by-leg through `api::interact::Interactions`
//! and `api::settle::Settle` (walk legs via `walk` + an `arrived` arm,
//! transport legs via `interact` on the transport loc + a positional
//! `arrived(edge.to)` arm), advancing one step per call. The host calls it
//! every tick and gets `None` while the route is still being followed and
//! `Some(outcome)` when it terminates.

use std::collections::VecDeque;

use api::interact::{
    op_loc, press, walk, ActionSpec, Driver, Interactions, OpTarget, SendReason, SendResult,
};
use api::query::{ChatQueryExt, Query, SceneQuery};
use api::settle::{arrived, Evidence, Outcome, Settle, SettleOptions};
use api::snapshot::{GameSnapshot, LocView, NpcView, ReadContext, WidgetView, WorldTile};
use client::dash3d::CollisionFlag;

use crate::arrival::arrived as grid_arrived;
use crate::essence::{
    essence_session_for_wizard, in_essence_mine, is_essence_entry_edge, EssenceSession,
    ESSENCE_MINE_EXIT_ARRIVE_RADIUS,
};
use crate::router::{GridLeg, GridRoute, Leg, Route};
use crate::tile::{chebyshev, Tile};
use crate::transport::{DoorDir, TransportEdge, TransportKind, CELLAR_SHIFT, SHANTAY_HENGE_LOC_ID};

mod dialog;
mod legacy_grid;
mod snapshot;
mod transport_hop;
mod walk;

use dialog::*;
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

/// The traveller's state, reported by each [`Traveller::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavStatus {
    /// No route armed.
    Idle,
    /// Stepping along a walk leg.
    Walking,
    /// Working a door leg.
    Door,
    /// Standing on the destination.
    Arrived,
    /// Waiting at the closest reachable tile.
    Closest,
    /// No path to the destination.
    Blocked,
    /// Exceeded the per-hop tick budget.
    Budget,
    /// Interrupted by an external event.
    Interrupted,
}

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
        detail: String,
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
    /// Per-leg phase callback; fired during the poll that crosses the
    /// transition. May borrow the caller (like `Evidence<'a>` in settle).
    #[allow(clippy::type_complexity)]
    pub on_leg: Option<Box<dyn FnMut(&Leg, LegPhase) + 'a>>,
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
            on_leg: None,
        }
    }
}

/// Drives a [`GridRoute`] toward its destination one hop per tick. The stub
/// world has no world grid: the caller passes the player's tile each tick,
/// and the walk target is picked from the armed route's own tiles.
pub struct Traveller {
    route: Option<GridRoute>,
    dest: Option<Tile>,
    status: NavStatus,
    hop_ticks: u32,
    budget: u32,
    /// The tile observed on the previous tick; the per-hop budget resets
    /// when the player advances off it.
    last_here: Option<Tile>,
    /// Whether the most recent walk hop was accepted by the driver.
    last_walk_ok: Option<bool>,
    /// Whether the most recent door `op_loc` was accepted by the driver.
    last_op_ok: Option<bool>,
    /// The leg currently being worked.
    leg: usize,
    /// The active high-level follow run (Task 14); `None` when idle.
    follow: Option<FollowRun>,
    /// The per-slot Rune Essence mine latch: set when an essence entry
    /// hop completes, the mine exit loc may only return to the entry
    /// wizard. Survives across routes (the game's `%exit_essence_mine_coord`
    /// varp persists too); a new entry overwrites it.
    essence: Option<EssenceSession>,
}

impl Traveller {
    /// A traveller with the default budget of 60 ticks per hop.
    pub fn new() -> Self {
        Self {
            route: None,
            dest: None,
            status: NavStatus::Idle,
            hop_ticks: 0,
            budget: 60,
            last_here: None,
            last_walk_ok: None,
            last_op_ok: None,
            leg: 0,
            follow: None,
            essence: None,
        }
    }

    /// Drop the armed route, its destination, and any active follow run.
    pub fn clear(&mut self) {
        self.route = None;
        self.dest = None;
        self.hop_ticks = 0;
        self.last_here = None;
        self.last_walk_ok = None;
        self.last_op_ok = None;
        self.leg = 0;
        self.follow = None;
    }

    /// The latched Rune Essence mine session: the wizard the player last
    /// entered the mine through, and the overworld tile the mine exit
    /// portal returns to. `None` before the first essence entry (and for
    /// bots that never visit the mine). The caller passes this back into
    /// [`crate::router::FindOptions::essence`] so a route from inside the
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
    /// plus one settle poll — except the door-troll fallback, which sends
    /// Open followed by a bounded adjacent crossing probe, or a walk when open.
    pub fn follow<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        route: Route,
        options: &mut TravelOptions<'_>,
    ) -> Option<TravelOutcome> {
        if self.follow.is_none() {
            self.follow = Some(FollowRun::start(route, options));
        }
        let outcome = self
            .follow
            .as_mut()
            .and_then(|run| run.step(d, snapshot, options, &mut self.essence));
        if crate::debug_enabled() {
            let run = self.follow.as_ref();
            eprintln!(
                "[nav-follow] here={:?} walk={} transport={} leg={} hops={} outcome={:?}",
                here(snapshot),
                run.is_some_and(|r| r.walk.is_some()),
                run.is_some_and(|r| r.transport.is_some()),
                run.map(|r| r.leg_index).unwrap_or(0),
                run.map(|r| r.hops).unwrap_or(0),
                outcome,
            );
        }
        if outcome.is_some() {
            self.follow = None;
        }
        outcome
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

/// The legs still to work and current hop state. Budgets are captured at
/// start; callbacks stay with the caller's options.
struct FollowRun {
    stun_wait: Option<u32>,
    stun_recovery_used: bool,
    legs: VecDeque<Leg>,
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
        FollowRun {
            stun_wait: None,
            stun_recovery_used: false,
            legs: route.legs.into(),
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
            let Some(leg) = self.legs.pop_front() else {
                return Some(TravelOutcome::Arrived { at: here(snapshot) });
            };
            match leg {
                Leg::Walk { .. } => {
                    let tiles = match &leg {
                        Leg::Walk { tiles } => tiles,
                        Leg::Transport { .. } => unreachable!("walk arm holds a walk leg"),
                    };
                    fire_leg(options, &leg, LegPhase::Start);
                    // Single-tile walk legs (the `find(from == to)` shape)
                    // carry no distance: no-op, never send.
                    if tiles.len() <= 1 {
                        fire_leg(options, &leg, LegPhase::Done);
                        self.leg_index += 1;
                        continue;
                    }
                    let here = here(snapshot);
                    let hop = WalkHop {
                        leg,
                        cursor: 0,
                        aim: here,
                        aim_index: 0,
                        sent: false,
                        ticks_waited: 0,
                        sent_tile: None,
                        sent_tick: snapshot.tick(),
                        tries: 0,
                        stall_idle_ticks: 0,
                        stall_idle_last_tick: None,
                        stall_idle_at: None,
                        stall_recovered: false,
                        end_refused_ticks: 0,
                        end_refused_last_tick: None,
                    };
                    match self.send_walk_hop(d, snapshot, options, hop, here) {
                        Poll::Watching => return None,
                        Poll::Terminal(outcome) => return Some(outcome),
                        Poll::LegDone => continue,
                    }
                }
                Leg::Transport { .. } => {
                    let edge = match &leg {
                        Leg::Transport { edge } => edge,
                        Leg::Walk { .. } => unreachable!("transport arm holds a transport leg"),
                    };
                    fire_leg(options, &leg, LegPhase::Start);
                    let here = here(snapshot);
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
                                    tries: 0,
                                    troll: false,
                                    open_sent_tick: None,
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
                            TeleportSend::Wait => {
                                self.loc_wait += 1;
                                if self.loc_wait > self.budget {
                                    fire_leg(options, &leg, LegPhase::Failed);
                                    return Some(TravelOutcome::Blocked {
                                        at: here,
                                        leg: self.leg_index,
                                        detail: format!(
                                            "packed teleport to ({}, {}, {}) never became workable in the loaded scene",
                                            edge.to.x, edge.to.z, edge.to.level
                                        ),
                                    });
                                }
                                self.legs.push_front(leg);
                                return None;
                            }
                            TeleportSend::Blocked(detail) => {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Blocked {
                                    at: here,
                                    leg: self.leg_index,
                                    detail,
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
                                    tries: 0,
                                    troll: false,
                                    open_sent_tick: None,
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
                    // The game only accepts an `op_loc`/`op_npc` from
                    // adjacent. Packed `at` is the spawn, not a leash:
                    // sailors/officers/pilots wander, so NPC hops approach
                    // the live tile (Musa customs Talk-to from the packed
                    // pier while the officer is four tiles away is
                    // Unreachable). Loc hops still use packed `at`.
                    let approach_at = if npc_backed(edge) {
                        match find_transport_target(snapshot, edge) {
                            Some(TransportTarget::Npc(npc)) => npc.tile,
                            Some(TransportTarget::Loc(loc)) => loc.tile,
                            None => {
                                self.loc_wait += 1;
                                if self.loc_wait > self.budget {
                                    fire_leg(options, &leg, LegPhase::Failed);
                                    return Some(TravelOutcome::Blocked {
                                        at: here,
                                        leg: self.leg_index,
                                        detail: format!(
                                            "transport {} {} is not within {} tiles of ({}, {}, {}) in the loaded scene",
                                            target_word(edge),
                                            edge.loc_id,
                                            NPC_SEARCH_RADIUS,
                                            edge.at.x,
                                            edge.at.z,
                                            edge.at.level
                                        ),
                                    });
                                }
                                self.legs.push_front(leg);
                                return None;
                            }
                        }
                    } else {
                        edge.at
                    };
                    if cheb(here, approach_at) > 1 {
                        if self
                            .settle_until
                            .is_some_and(|until| snapshot.tick() < until)
                        {
                            self.legs.push_front(leg);
                            return None;
                        }
                        self.settle_until = None;
                        let Some(approach) = approach_tile(snapshot, approach_at, here) else {
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
                                    detail: format!(
                                        "no standable tile within 1 of transport {} {} at ({}, {}, {}) in the loaded scene",
                                        target_word(edge),
                                        edge.loc_id,
                                        edge.at.x,
                                        edge.at.z,
                                        edge.at.level
                                    ),
                                });
                            }
                            self.legs.push_front(leg);
                            return None;
                        };
                        let to = edge.to;
                        let mut ix = Interactions::new(snapshot, d);
                        let result = ix.walk(approach);
                        report_walk(options, snapshot, here, approach, &result);
                        match result {
                            SendResult::Sent { .. } => {
                                self.loc_wait = 0;
                                self.transport = Some(TransportHop {
                                    leg,
                                    to,
                                    ticks_waited: 0,
                                    sent_tile: Some(here),
                                    tries: 0,
                                    troll: false,
                                    open_sent_tick: None,
                                    chat_seq: chat_seq(snapshot),
                                    dialog_page: None,
                                    approach: Some(ApproachHop {
                                        tile: approach,
                                        at: approach_at,
                                        ticks_waited: 0,
                                        sent_tick: snapshot.tick(),
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
                                self.legs.push_front(leg);
                                return None;
                            }
                            SendResult::Refused { reason, .. } => {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Refused { at: here, reason });
                            }
                        }
                    }
                    match find_transport_target(snapshot, edge) {
                        Some(target) => {
                            let to = edge.to;
                            let mut ix = Interactions::new(snapshot, d);
                            match interact_transport(snapshot, &mut ix, target, edge, options) {
                                SendResult::Sent { .. } => {
                                    // Already-open trapdoor: this interact is
                                    // Climb-down. Mark tries so poll_transport
                                    // does not send it a second time.
                                    let tries = if edge.open_loc_id.is_some()
                                        && edge.kind != TransportKind::Door
                                        && edge_loc_open(snapshot, edge)
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
                                    self.loc_wait = 0;
                                    self.transport = Some(TransportHop {
                                        leg,
                                        to,
                                        ticks_waited: 0,
                                        sent_tile: Some(here),
                                        tries,
                                        troll: false,
                                        open_sent_tick,
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
                                    detail: format!(
                                        "transport {} {} is not within 3 tiles of ({}, {}, {}) in the loaded scene",
                                        target_word(edge),
                                        edge.loc_id,
                                        edge.at.x,
                                        edge.at.z,
                                        edge.at.level
                                    ),
                                });
                            }
                            self.legs.push_front(leg);
                            return None;
                        }
                    }
                }
            }
        }
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
    leg: Leg,
    /// Index into the leg tiles of the last cleared tile.
    cursor: usize,
    aim: WorldTile,
    aim_index: usize,
    /// Whether the walk for `aim` has been sent; a matched mid-leg hop is
    /// re-armed with `sent: false` and sent on the same poll (click-ahead).
    sent: bool,
    sent_tick: u32,
    ticks_waited: u32,
    /// The player's tile when the hop's walk was sent. Used for
    /// Dropped vs Expired stall classification (`here == sent_tile` →
    /// Dropped). Cancelled-walk recovery idle is tracked separately at
    /// the latest observed position (`stall_idle_at`).
    sent_tile: Option<WorldTile>,
    tries: u32,
    /// Distinct game ticks observed idle at `stall_idle_at` with no map
    /// flag and no movement — the cancelled-walk recovery window.
    /// Separate from `ticks_waited` (poll budget); duplicate snapshot
    /// polls do not count. Tile movement resets this counter.
    stall_idle_ticks: u32,
    /// Snapshot tick last credited to `stall_idle_ticks`.
    stall_idle_last_tick: Option<u32>,
    /// Latest observed tile while counting cancelled-walk idle. A change
    /// of tile (partial hop progress) clears the idle window and starts
    /// a new one at the new position.
    stall_idle_at: Option<WorldTile>,
    /// A cancelled-walk recovery already reissued this hop's aim once.
    stall_recovered: bool,
    /// Distinct game ticks the route's last tile, one step away, refused
    /// the click while no walk of this follow was accepted
    /// ([`HopFailure::EndBlocked`]).
    end_refused_ticks: u32,
    /// Snapshot tick last credited to `end_refused_ticks`.
    end_refused_last_tick: Option<u32>,
}

impl WalkHop {
    fn tiles(&self) -> &[WorldTile] {
        match &self.leg {
            Leg::Walk { tiles } => tiles,
            Leg::Transport { .. } => unreachable!("walk hop holds a walk leg"),
        }
    }

    /// Reconstruct the leg for the phase callback (called only on leg
    /// transitions).
    fn leg(&self) -> Leg {
        Leg::Walk {
            tiles: self.tiles().to_vec(),
        }
    }
}

/// One transport-leg hop: the edge (for the phase callback) plus the
/// arrival target and the stall clock. `troll` marks the automatic
/// door-troll fallback: the hop re-reads the door's state and re-sends
/// while closed, probes after Open, and walks when open after
/// the cheap one-interact hop lapsed its budget. `chat_seq` is the chat
/// ring's latest sequence when the hop started: the watermark for the
/// "I can't reach that!" fast-fail watch (`settle::said`'s sequence
/// delta, never a stale-head check).
struct TransportHop {
    leg: Leg,
    to: WorldTile,
    ticks_waited: u32,
    sent_tile: Option<WorldTile>,
    tries: u32,
    troll: bool,
    /// The tick a door Open was sent: one crossing probe
    /// ([`door_step_pending`]) on a later delivered tick. The troll spends
    /// it on the next tick; the cheap hop keeps it until the player stands
    /// on `at` with a wall-clear step to `to`.
    open_sent_tick: Option<u32>,
    chat_seq: i32,
    /// The chat option-page last answered (joined option texts). A new
    /// page (spirit-tree dest list after "Where can I go?") is answered;
    /// the same page is never re-pressed.
    dialog_page: Option<String>,
    /// The pre-interact approach walk: the standable take-off tile within
    /// chebyshev 1 of the edge's `at` the player must reach before the loc
    /// interact can be sent. `None` once the player stands adjacent (or on
    /// a hop with no interact, like the open-leaf walk-through).
    approach: Option<ApproachHop>,
}

/// The transport approach: the standable take-off tile within chebyshev 1
/// of the edge's `at` (the walk target), the `at` the adjacency settle is
/// measured against, and the stall clock.
struct ApproachHop {
    sent_tick: u32,
    retry_pending: bool,
    tile: WorldTile,
    at: WorldTile,
    ticks_waited: u32,
}

/// Fire the `on_leg` callback for a phase transition, using the `options`
/// of the poll in which the transition happened.
fn fire_leg(options: &mut TravelOptions<'_>, leg: &Leg, phase: LegPhase) {
    if let Some(cb) = options.on_leg.as_mut() {
        cb(leg, phase);
    }
}

#[cfg(test)]
#[path = "traveller_tests.rs"]
mod tests;
