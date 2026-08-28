//! Traveller: drives a route through the kernel `Driver` over ticks. The
//! caller supplies the player's current tile and the door-open state each
//! tick; the traveller targets walk legs one hop ahead and works a door
//! leg by `op_loc` while closed. When the caller reports the door open,
//! walk through the same tick without OP_LOC1 (that would Close).
//!
//! The high-level [`Traveller::follow`] is a new pollable layer: it drives
//! a [`crate::router::Route`] leg-by-leg through `api::interact::Interactions`
//! + `api::settle::Settle` (walk legs via `walk` + an `arrived` arm,
//! transport legs via `interact` on the transport loc + a positional
//! `arrived(edge.to)` arm), advancing one step per call. The host calls it
//! every tick and gets `None` while the route is still being followed and
//! `Some(outcome)` when it terminates.

use std::collections::VecDeque;

use api::interact::{op_loc, walk, ActionSpec, Driver, Interactions, OpTarget, SendReason, SendResult};
use api::query::{ChatQueryExt, Query, SceneQuery};
use api::settle::{arrived, Evidence, Outcome, Settle, SettleOptions};
use api::snapshot::{GameSnapshot, LocView, ReadContext, WorldTile};
use client::dash3d::CollisionFlag;

use crate::arrival::arrived as grid_arrived;
use crate::router::{GridLeg, GridRoute, Leg, Route};
use crate::tile::{chebyshev, Tile};
use crate::transport::{DoorDir, TransportEdge, TransportKind};

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
/// while the player was making progress (`Expired`), or the player never
/// left the tile the hop was sent from (`Dropped` — the walk or transport
/// interaction was dropped by the game).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopFailure {
    Expired,
    Dropped,
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

/// How a [`Traveller::follow`] run is parameterized. `on_leg` fires once
/// per phase transition; the same `TravelOptions` is passed on every poll,
/// so the callback stays with the caller across ticks.
pub struct TravelOptions<'a> {
    /// Chebyshev radius treated as "arrived" for a hop target (default 2).
    pub close_enough: i32,
    /// Tick budget for one hop (a walk send or a transport interact;
    /// default 60).
    pub budget_ticks_per_hop: u32,
    /// Walk hops before `GaveUp` (default 60).
    pub max_hops: u32,
    /// Per-leg phase callback; fired during the poll that crosses the
    /// transition. May borrow the caller (like `Evidence<'a>` in settle).
    pub on_leg: Option<Box<dyn FnMut(&Leg, LegPhase) + 'a>>,
}

impl Default for TravelOptions<'_> {
    fn default() -> Self {
        TravelOptions {
            close_enough: 2,
            budget_ticks_per_hop: 60,
            max_hops: 60,
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
        }
    }

    /// Arm a route, replacing any previous one.
    pub fn arm(&mut self, route: GridRoute) {
        self.dest = Some(route.dest);
        self.route = Some(route);
        self.hop_ticks = 0;
        self.last_here = None;
        self.last_walk_ok = None;
        self.last_op_ok = None;
        self.leg = 0;
        self.status = NavStatus::Idle;
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

    /// The destination currently queued, if any.
    pub fn queued(&self) -> Option<Tile> {
        self.dest
    }

    /// Whether the most recent walk hop was accepted by the driver (for
    /// live diagnostics; `None` before the first hop).
    pub fn last_walk_ok(&self) -> Option<bool> {
        self.last_walk_ok
    }

    /// Whether the most recent door `op_loc` was accepted (live diagnostics).
    pub fn last_op_ok(&self) -> Option<bool> {
        self.last_op_ok
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

    /// The tiles still ahead on the armed route, front to back. Walk legs
    /// contribute all their tiles; a door leg contributes its `from` and
    /// `to` so the polyline stays connected across the crossing. When
    /// `here` is given (the player's observed tile), legs already traversed
    /// are skipped exactly as [`Traveller::tick`] skips them, and the
    /// current walk leg is trimmed to the tiles from `here` onward so the
    /// line shrinks as the player walks, not only at leg end. Empty when
    /// nothing is armed or every leg is done.
    pub fn remaining_walk_tiles(&self, here: Option<Tile>) -> Vec<Tile> {
        let Some(route) = self.route.as_ref() else {
            return Vec::new();
        };
        let mut leg = self.leg.min(route.legs.len());
        if let Some(here) = here {
            while leg < route.legs.len() {
                let done = match &route.legs[leg] {
                    GridLeg::Walk { tiles } => tiles.last().is_none_or(|last| *last == here),
                    GridLeg::Door { to, .. } => *to == here,
                };
                if !done {
                    break;
                }
                leg += 1;
            }
        }
        let mut out = Vec::new();
        for (i, l) in route.legs.iter().enumerate().skip(leg) {
            match l {
                GridLeg::Walk { tiles } => {
                    if i == leg {
                        if let Some(here) = here {
                            if let Some(pos) = tiles.iter().position(|t| *t == here) {
                                out.extend(tiles[pos..].iter().copied());
                                continue;
                            }
                        }
                    }
                    out.extend(tiles.iter().copied());
                }
                GridLeg::Door { from, to, .. } => {
                    out.push(*from);
                    out.push(*to);
                }
            }
        }
        // A door's `to` is the next walk leg's first tile: drop the
        // duplicate crossing tile so the line does not double back.
        out.dedup();
        out
    }

    /// The current Door leg's loc tile and closed loc id, given `here`.
    /// Skips already-traversed legs the same way [`Traveller::tick`] does.
    /// `None` when nothing is armed or the current leg is a walk.
    pub fn current_door(&self, here: Tile) -> Option<(Tile, i32)> {
        let route = self.route.as_ref()?;
        let mut leg = self.leg.min(route.legs.len());
        while leg < route.legs.len() {
            let done = match &route.legs[leg] {
                GridLeg::Walk { tiles } => tiles.last().is_none_or(|last| *last == here),
                GridLeg::Door { to, .. } => *to == here,
            };
            if !done {
                break;
            }
            leg += 1;
        }
        match route.legs.get(leg) {
            Some(GridLeg::Door { loc, loc_id, .. }) => Some((*loc, *loc_id)),
            _ => None,
        }
    }

    /// Advance the route one tick: send the driver the next hop toward
    /// `dest`, or work the current door leg. `here` is the player's tile;
    /// `door_open` is the door's current state (the caller reads it live).
    pub fn tick<D: Driver>(&mut self, d: &mut D, here: Tile, door_open: bool) -> NavStatus {
        let Some(route) = self.route.as_ref() else {
            self.status = NavStatus::Idle;
            return self.status;
        };
        let Some(dest) = self.dest else {
            self.status = NavStatus::Idle;
            return self.status;
        };

        // Stub world: every route dest is a walkable tile, so arrival is
        // exactly standing on it. Solid-adjacent arrival comes later.
        if grid_arrived(here, dest, true) {
            self.status = NavStatus::Arrived;
            let status = self.status;
            self.clear();
            return status;
        }

        // The budget is per hop, not per route: any advance off the
        // previous tile restarts the clock for the next hop.
        if self.last_here != Some(here) {
            self.hop_ticks = 0;
            self.last_here = Some(here);
        }
        self.hop_ticks += 1;
        if self.hop_ticks > self.budget {
            self.status = NavStatus::Budget;
            let status = self.status;
            self.clear();
            return status;
        }

        // Skip legs already traversed: standing on a walk leg's last tile
        // (a door's `from`) moves on to the door; standing on a door's
        // `to` moves on to the next walk leg.
        while self.leg < route.legs.len() {
            let done = match &route.legs[self.leg] {
                GridLeg::Walk { tiles } => tiles.last().is_none_or(|last| *last == here),
                GridLeg::Door { to, .. } => *to == here,
            };
            if !done {
                break;
            }
            self.leg += 1;
        }

        let Some(leg) = route.legs.get(self.leg) else {
            // Remaining empty without arriving: do not silent-spin.
            self.status = NavStatus::Budget;
            let status = self.status;
            self.clear();
            return status;
        };

        match leg {
            GridLeg::Walk { tiles } => {
                let last = *tiles.last().expect("walk legs are non-empty");
                // Aim at the leg's far end when it is within 20 tiles;
                // otherwise hop to a tile ~15 steps ahead of `here` along
                // the leg so the client re-routes a fresh, short path each
                // tick and never aims back toward the leg start.
                let target = if chebyshev(here, last) <= 20 {
                    last
                } else {
                    tiles
                        .iter()
                        .copied()
                        .skip_while(|t| *t != here)
                        .nth(15)
                        .unwrap_or(last)
                };
                let mut accepted = walk(d, target.x, target.z);
                if !accepted {
                    // The client rejected the leg shot (collision has no
                    // route that far). Retry the tile right after `here` on
                    // this leg once so the next rebuild step still sends.
                    let next = tiles
                        .iter()
                        .position(|t| *t == here)
                        .and_then(|i| tiles.get(i + 1))
                        .copied()
                        .unwrap_or(last);
                    if next != target {
                        accepted = walk(d, next.x, next.z);
                    }
                }
                self.last_walk_ok = Some(accepted);
                self.status = NavStatus::Walking;
            }
            GridLeg::Door {
                loc, loc_id, to, ..
            } => {
                if !door_open {
                    // Closed: OP_LOC1 the packed typecode (opens 1530).
                    self.last_op_ok = Some(op_loc(d, loc.x, loc.z, *loc_id));
                } else {
                    // Already open: do not OP_LOC1 the live loc (that
                    // Closes). Walk through this tick.
                    self.last_walk_ok = Some(walk(d, to.x, to.z));
                }
                self.status = NavStatus::Door;
            }
        }
        self.status
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
    /// `op_loc` and the same-tick walk together on an open door.
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
            .and_then(|run| run.step(d, snapshot, options));
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

/// The legs still to work and the current hop's state. `close_enough`,
/// `budget_ticks_per_hop` and `max_hops` are captured when the run starts;
/// the `on_leg` callback stays with the caller's `TravelOptions`.
struct FollowRun {
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
    walk: Option<WalkHop>,
    transport: Option<TransportHop>,
}

impl FollowRun {
    fn start(route: Route, options: &TravelOptions<'_>) -> FollowRun {
        FollowRun {
            legs: route.legs.into(),
            leg_index: 0,
            hops: 0,
            max_hops: options.max_hops,
            close_enough: options.close_enough,
            budget: options.budget_ticks_per_hop,
            loc_wait: 0,
            walk: None,
            transport: None,
        }
    }

    /// One poll step: settle an active hop, or start the next leg (sending
    /// its first hop). At most one driver send per call. A settle poll that
    /// is still watching ends the call (`None`); only a matched settle may
    /// advance the run — to the next leg or to a terminal outcome.
    fn step<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions<'_>,
    ) -> Option<TravelOutcome> {
        loop {
            if self.walk.is_some() {
                match self.poll_walk(d, snapshot, options) {
                    Poll::Terminal(outcome) => return Some(outcome),
                    Poll::Watching => return None,
                    Poll::LegDone => continue,
                }
            }
            if self.transport.is_some() {
                match self.poll_transport(d, snapshot, options) {
                    Poll::Terminal(outcome) => return Some(outcome),
                    Poll::Watching => return None,
                    Poll::LegDone => continue,
                }
            }
            // No active hop: work the next leg (or finish).
            let Some(leg) = self.legs.pop_front() else {
                return Some(TravelOutcome::Arrived {
                    at: here(snapshot),
                });
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
                    if self.hops >= self.max_hops {
                        fire_leg(options, &leg, LegPhase::Failed);
                        return Some(TravelOutcome::GaveUp {
                            at: here(snapshot),
                            hops: self.hops,
                        });
                    }
                    self.hops += 1;
                    let here = here(snapshot);
                    let (aim, aim_index) = pick_aim(tiles, here, 0);
                    let mut ix = Interactions::new(snapshot, d);
                    match ix.walk(aim) {
                        SendResult::Sent { .. } => {
                            self.walk = Some(WalkHop {
                                leg,
                                cursor: 0,
                                aim,
                                aim_index,
                                sent: true,
                                ticks_waited: 0,
                                sent_tile: Some(here),
                                tries: 0,
                            });
                            return None;
                        }
                        SendResult::Refused { reason, .. } => {
                            fire_leg(options, &leg, LegPhase::Failed);
                            return Some(TravelOutcome::Refused {
                                at: here,
                                reason,
                            });
                        }
                    }
                }
                Leg::Transport { .. } => {
                    let edge = match &leg {
                        Leg::Transport { edge } => edge,
                        Leg::Walk { .. } => unreachable!("transport arm holds a transport leg"),
                    };
                    fire_leg(options, &leg, LegPhase::Start);
                    let here = here(snapshot);
                    // Multiloc open-state: when the live loc at `at` already
                    // reads open, interacting is wrong (an OP on the open
                    // leaf would Close it) — walk straight through to `to`
                    // and settle `arrived(to)` with the normal budget.
                    if edge_loc_open(snapshot, edge) {
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
                                    chat_seq: chat_seq(snapshot),
                                    approach: None,
                                });
                                return None;
                            }
                            SendResult::Refused { reason, .. } => {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Refused {
                                    at: here,
                                    reason,
                                });
                            }
                        }
                    }
                    // The game only accepts an `op_loc` from adjacent: when
                    // the player is not yet within chebyshev 1 of `at`, walk
                    // to the nearest standable tile there first and wait to
                    // arrive (the approach, settled by the transport hop),
                    // before the loc is found and interacted with.
                    if cheb(here, edge.at) > 1 {
                        let Some(approach) = approach_tile(snapshot, edge, here) else {
                            // No standable tile adjacent to the loc in the
                            // loaded scene: keep waiting, bounded by the hop
                            // budget. The leg stays on the front while
                            // waiting.
                            self.loc_wait += 1;
                            if self.loc_wait > self.budget {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Blocked {
                                    at: here,
                                    leg: self.leg_index,
                                    detail: format!(
                                        "no standable tile within 1 of transport loc {} at ({}, {}, {}) in the loaded scene",
                                        edge.loc_id, edge.at.x, edge.at.z, edge.at.level
                                    ),
                                });
                            }
                            self.legs.push_front(leg);
                            return None;
                        };
                        let to = edge.to;
                        let at = edge.at;
                        let mut ix = Interactions::new(snapshot, d);
                        match ix.walk(approach) {
                            SendResult::Sent { .. } => {
                                self.loc_wait = 0;
                                self.transport = Some(TransportHop {
                                    leg,
                                    to,
                                    ticks_waited: 0,
                                    sent_tile: Some(here),
                                    tries: 0,
                                    troll: false,
                                    chat_seq: chat_seq(snapshot),
                                    approach: Some(ApproachHop {
                                        tile: approach,
                                        at,
                                        ticks_waited: 0,
                                    }),
                                });
                                return None;
                            }
                            SendResult::Refused { reason, .. } => {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Refused {
                                    at: here,
                                    reason,
                                });
                            }
                        }
                    }
                    match find_transport_loc(snapshot, edge) {
                        Some(loc) => {
                            let to = edge.to;
                            let mut ix = Interactions::new(snapshot, d);
                            match ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(edge.option)) {
                                SendResult::Sent { .. } => {
                                    self.loc_wait = 0;
                                    self.transport = Some(TransportHop {
                                        leg,
                                        to,
                                        ticks_waited: 0,
                                        sent_tile: Some(here),
                                        tries: 0,
                                        troll: false,
                                        chat_seq: chat_seq(snapshot),
                                        approach: None,
                                    });
                                    return None;
                                }
                                SendResult::Refused { reason, .. } => {
                                    fire_leg(options, &leg, LegPhase::Failed);
                                    return Some(TravelOutcome::Refused {
                                        at: here,
                                        reason,
                                    });
                                }
                            }
                        }
                        None => {
                            // The loc has not appeared in the loaded scene
                            // yet: keep waiting, bounded by the hop budget.
                            // The leg stays on the front while waiting.
                            self.loc_wait += 1;
                            if self.loc_wait > self.budget {
                                fire_leg(options, &leg, LegPhase::Failed);
                                return Some(TravelOutcome::Blocked {
                                    at: here,
                                    leg: self.leg_index,
                                    detail: format!(
                                        "transport loc {} is not within 3 tiles of ({}, {}, {}) in the loaded scene",
                                        edge.loc_id, edge.at.x, edge.at.z, edge.at.level
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

    /// One walk-hop step: send an armed (unsent) hop, or settle the sent
    /// one against the `arrived` arm and the hop budget. A still-watching
    /// settle ends the call; a matched hop arms the next hop (sent on the
    /// next call) or completes the leg.
    fn poll_walk<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
    ) -> Poll {
        let mut hop = self.walk.take().expect("walk hop present");
        let here = here(snapshot);
        if !hop.sent {
            // The previous poll matched mid-leg and armed this hop: send
            // it now (one walk per call).
            if self.hops >= self.max_hops {
                fire_leg(options, &hop.leg(), LegPhase::Failed);
                return Poll::Terminal(TravelOutcome::GaveUp {
                    at: here,
                    hops: self.hops,
                });
            }
            self.hops += 1;
            let (aim, aim_index) = pick_aim(hop.tiles(), here, hop.cursor);
            let mut ix = Interactions::new(snapshot, d);
            return match ix.walk(aim) {
                SendResult::Sent { .. } => {
                    hop.aim = aim;
                    hop.aim_index = aim_index;
                    hop.sent = true;
                    hop.ticks_waited = 0;
                    hop.sent_tile = Some(here);
                    self.walk = Some(hop);
                    Poll::Watching
                }
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg(), LegPhase::Failed);
                    Poll::Terminal(TravelOutcome::Refused { at: here, reason })
                }
            };
        }
        // Last tile of the walk is the dest: exact stand. A loose
        // close_enough reports Arrived short; the runner then re-arms the
        // original route and the walker yo-yos back through the door.
        let radius = if hop.aim_index + 1 >= hop.tiles().len() {
            0
        } else {
            self.close_enough
        };
        let arms = [("arrived", arrived(hop.aim, radius))];
        if crate::debug_enabled() {
            eprintln!(
                "[nav-walk] here={here:?} aim={:?} radius={radius} ticks_waited={} sent_tile={:?}",
                hop.aim, hop.ticks_waited, hop.sent_tile
            );
        }
        let mut settle = Settle::new(
            SettleOptions {
                arms: &arms,
                // The run enforces the per-hop budget; the settle only
                // reports a disconnect (an arm read alone never lapses).
                budget_ticks: u32::MAX,
                budget_ms: None,
            },
            ReadContext::new(snapshot),
        );
        match settle.poll(ReadContext::new(snapshot)) {
            Some(Outcome::Matched { .. }) => {
                // The hop completed: the leg is done exactly when the aim
                // was its last tile (the hop's arrival already proved the
                // player is within `close_enough` of the aim).
                if hop.aim_index + 1 >= hop.tiles().len() {
                    fire_leg(options, &hop.leg(), LegPhase::Done);
                    self.leg_index += 1;
                    return Poll::LegDone;
                }
                // Mid-leg: arm the next hop; the next call sends it.
                hop.cursor = hop.aim_index + 1;
                hop.sent = false;
                self.walk = Some(hop);
                Poll::Watching
            }
            // A disconnect ends the watch; the hop was effectively dropped.
            Some(Outcome::Expired { .. }) => {
                fire_leg(options, &hop.leg(), LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Stalled {
                    at: here,
                    aiming: hop.aim,
                    why: HopFailure::Dropped,
                    tries: hop.tries.max(1),
                })
            }
            // `poll` never produces `Refused` (only `Interactions` does);
            // keep watching defensively.
            Some(Outcome::Refused { .. }) => {
                self.walk = Some(hop);
                Poll::Watching
            }
            None => {
                hop.ticks_waited += 1;
                if hop.ticks_waited > self.budget {
                    let why = if hop.sent_tile == Some(here) {
                        HopFailure::Dropped
                    } else {
                        HopFailure::Expired
                    };
                    fire_leg(options, &hop.leg(), LegPhase::Failed);
                    Poll::Terminal(TravelOutcome::Stalled {
                        at: here,
                        aiming: hop.aim,
                        why,
                        tries: hop.tries.max(1),
                    })
                } else {
                    self.walk = Some(hop);
                    Poll::Watching
                }
            }
        }
    }

    /// One transport-hop settle step: match the positional `arrived(edge.to)`
    /// arm (level + proximity, so a level-changing transport completes only
    /// within `close_enough` of `to` on the destination level), fail fast on
    /// the game's "I can't reach that!" chat (a line new since the hop
    /// started — the `settle::said` sequence delta), or lapse the budget.
    /// A door hop that lapses its cheap budget escalates to the automatic
    /// troll (see [`FollowRun::troll_door`]); only a troll hop (or a
    /// non-door transport) lapses to the real `Stalled`.
    fn poll_transport<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
    ) -> Poll {
        let mut hop = self.transport.take().expect("transport hop present");
        let here = here(snapshot);
        if hop.troll {
            if let Some(outcome) = self.troll_door(d, snapshot, &mut hop, options) {
                return Poll::Terminal(outcome);
            }
        } else if hop.approach.is_none() {
            if let Leg::Transport { edge } = &hop.leg {
            // Cheap hop: Open was sent; the live open leaf can sit a tile
            // off `at`. Walk through as soon as it reads open — do not sit
            // until the troll budget while the door is already open.
            if edge.kind == TransportKind::Door
                && edge_loc_open(snapshot, edge)
                && !door_crossed(edge, here)
            {
                let mut ix = Interactions::new(snapshot, d);
                match ix.walk(hop.to) {
                    SendResult::Sent { .. } => {
                        if crate::debug_enabled() {
                            eprintln!("[nav-transport] cheap hop walk-through to {:?}", hop.to);
                        }
                    }
                    SendResult::Refused { reason, .. } => {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        return Poll::Terminal(TravelOutcome::Refused { at: here, reason });
                    }
                }
                self.transport = Some(hop);
                return Poll::Watching;
            }
            }
        }
        // The pre-interact approach: the player must stand within chebyshev
        // 1 of the loc before the game accepts an `op_loc`. While the
        // approach is armed, watch it instead of the arrive arm; only once
        // adjacent does the hop find the loc, interact, and settle
        // `arrived(to)`.
        if hop.approach.is_some() {
            match self.poll_approach(snapshot, &mut hop, options) {
                Poll::Watching => {
                    self.transport = Some(hop);
                    return Poll::Watching;
                }
                Poll::Terminal(outcome) => return Poll::Terminal(outcome),
                // The player is adjacent now: fall through, send the loc
                // interact, then watch `arrived(to)`.
                Poll::LegDone => {}
            }
            let edge = match &hop.leg {
                Leg::Transport { edge } => edge.clone(),
                Leg::Walk { .. } => unreachable!("transport hop holds a transport leg"),
            };
            return match find_transport_loc(snapshot, &edge) {
                Some(loc) => {
                    let mut ix = Interactions::new(snapshot, d);
                    match ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(edge.option)) {
                        SendResult::Sent { .. } => {
                            self.loc_wait = 0;
                            hop.ticks_waited = 0;
                            hop.sent_tile = Some(here);
                            self.transport = Some(hop);
                            Poll::Watching
                        }
                        SendResult::Refused { reason, .. } => {
                            fire_leg(options, &hop.leg, LegPhase::Failed);
                            Poll::Terminal(TravelOutcome::Refused { at: here, reason })
                        }
                    }
                }
                None => {
                    // The loc has not appeared in the loaded scene yet:
                    // keep waiting, bounded by the hop budget.
                    self.loc_wait += 1;
                    if self.loc_wait > self.budget {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        Poll::Terminal(TravelOutcome::Blocked {
                            at: here,
                            leg: self.leg_index,
                            detail: format!(
                                "transport loc {} is not within 3 tiles of ({}, {}, {}) in the loaded scene",
                                edge.loc_id, edge.at.x, edge.at.z, edge.at.level
                            ),
                        })
                    } else {
                        self.transport = Some(hop);
                        Poll::Watching
                    }
                }
            };
        }
        // The "I can't reach that!" watch: the client pathfind failed
        // right after the send, so the hop is refused immediately instead
        // of sitting out the settle budget. `chat_seq` is the hop-start
        // watermark, so only genuinely new lines count.
        let hop_seq = hop.chat_seq;
        let edge = match &hop.leg {
            Leg::Transport { edge } => edge.clone(),
            Leg::Walk { .. } => unreachable!("transport hop holds a transport leg"),
        };
        if crate::debug_enabled() {
            eprintln!(
                "[nav-transport] here={here:?} to={:?} troll={} ticks_waited={} loc_id={} open={}",
                hop.to,
                hop.troll,
                hop.ticks_waited,
                edge.loc_id,
                edge_loc_open(snapshot, &edge),
            );
        }
        let close_enough = self.close_enough;
        let arrived_arm: Evidence<'static> = if edge.kind == TransportKind::Door && edge.dir.is_some()
        {
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                let Some(here) = now.world_tile() else {
                    return false;
                };
                if here.level != edge.to.level {
                    return false;
                }
                door_crossed(&edge, here)
                    && (here.x - edge.to.x).abs().max((here.z - edge.to.z).abs()) <= close_enough
            })
        } else {
            arrived(edge.to, close_enough)
        };
        let arms: [(&str, Evidence<'static>); 2] = [
            ("arrived", arrived_arm),
            (
                "unreachable",
                Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                    Query::new(now.chat())
                        .since(hop_seq)
                        .text_contains(&["i can't reach that"])
                        .exists()
                }),
            ),
        ];
        let mut settle = Settle::new(
            SettleOptions {
                arms: &arms,
                budget_ticks: u32::MAX,
                budget_ms: None,
            },
            ReadContext::new(snapshot),
        );
        match settle.poll(ReadContext::new(snapshot)) {
            Some(Outcome::Matched { arm: "unreachable", .. }) => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Refused {
                    at: here,
                    reason: SendReason::Unreachable,
                })
            }
            Some(Outcome::Matched { .. }) => {
                fire_leg(options, &hop.leg, LegPhase::Done);
                self.leg_index += 1;
                Poll::LegDone
            }
            Some(Outcome::Expired { .. }) => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Stalled {
                    at: here,
                    aiming: hop.to,
                    why: HopFailure::Dropped,
                    tries: hop.tries.max(1),
                })
            }
            // `poll` never produces `Refused` (only `Interactions` does);
            // keep watching defensively.
            Some(Outcome::Refused { .. }) => {
                self.transport = Some(hop);
                Poll::Watching
            }
            None => {
                hop.ticks_waited += 1;
                if hop.ticks_waited > self.budget {
                    // The cheap one-interact door hop lapsed: a door the
                    // closer keeps slamming can never cross that way, so
                    // escalate this same leg to the automatic troll
                    // instead of stalling. Re-send every tick (op_loc
                    // always, plus the same-tick walk when the door reads
                    // open); only a troll hop that lapses again — or a
                    // non-door transport — returns the real `Stalled`.
                    let door_leg = matches!(
                        &hop.leg,
                        Leg::Transport { edge } if edge.kind == TransportKind::Door
                    );
                    if door_leg && !hop.troll {
                        hop.troll = true;
                        hop.ticks_waited = 0;
                        self.transport = Some(hop);
                        Poll::Watching
                    } else {
                        let why = if hop.sent_tile == Some(here) {
                            HopFailure::Dropped
                        } else {
                            HopFailure::Expired
                        };
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        Poll::Terminal(TravelOutcome::Stalled {
                            at: here,
                            aiming: hop.to,
                            why,
                            tries: hop.tries.max(1),
                        })
                    }
                } else {
                    self.transport = Some(hop);
                    Poll::Watching
                }
            }
        }
    }

    /// One approach-hop settle step: match the adjacency arm
    /// (`arrived(at, 1)` — only once the player stands within chebyshev 1
    /// of the loc may the hop send the interact), or lapse the budget. A
    /// door hop whose approach lapses escalates to the automatic troll
    /// (the cheap hop lapsed before its first interact — the troll walks
    /// the player to the door too); only a non-door transport lapses to
    /// the real `Stalled`. The approach walk itself was sent when the hop
    /// was armed, so this method only settles.
    fn poll_approach(
        &mut self,
        snapshot: &GameSnapshot,
        hop: &mut TransportHop,
        options: &mut TravelOptions<'_>,
    ) -> Poll {
        let mut approach = hop.approach.take().expect("approach hop present");
        let here = here(snapshot);
        let arms = [("arrived", arrived(approach.at, 1))];
        let mut settle = Settle::new(
            SettleOptions {
                arms: &arms,
                // The run enforces the per-hop budget; the settle only
                // reports a disconnect (an arm read alone never lapses).
                budget_ticks: u32::MAX,
                budget_ms: None,
            },
            ReadContext::new(snapshot),
        );
        match settle.poll(ReadContext::new(snapshot)) {
            Some(Outcome::Matched { .. }) => {
                // The player is adjacent: the caller sends the loc interact.
                Poll::LegDone
            }
            Some(Outcome::Expired { .. }) => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Stalled {
                    at: here,
                    aiming: approach.tile,
                    why: HopFailure::Dropped,
                    tries: hop.tries.max(1),
                })
            }
            // `poll` never produces `Refused` (only `Interactions` does);
            // keep watching defensively.
            Some(Outcome::Refused { .. }) => {
                hop.approach = Some(approach);
                Poll::Watching
            }
            None => {
                approach.ticks_waited += 1;
                if approach.ticks_waited > self.budget {
                    // The approach walk never landed: for a door, escalate
                    // this same leg to the automatic troll instead of
                    // stalling; only a troll hop that lapses again — or a
                    // non-door transport — returns the real `Stalled`.
                    let door_leg = matches!(
                        &hop.leg,
                        Leg::Transport { edge } if edge.kind == TransportKind::Door
                    );
                    if door_leg && !hop.troll {
                        hop.troll = true;
                        hop.approach = None;
                        hop.ticks_waited = 0;
                        Poll::Watching
                    } else {
                        let why = if hop.sent_tile == Some(here) {
                            HopFailure::Dropped
                        } else {
                            HopFailure::Expired
                        };
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        Poll::Terminal(TravelOutcome::Stalled {
                            at: here,
                            aiming: approach.tile,
                            why,
                            tries: hop.tries.max(1),
                        })
                    }
                } else {
                    hop.approach = Some(approach);
                    Poll::Watching
                }
            }
        }
    }

    /// One door-troll poll: read the door's open/closed state from the
    /// snapshot's locs and re-send — `op_loc` always, plus the same-tick
    /// walk when the door reads open — so a tick-perfect closer cannot
    /// slam the door between the open and the walk. Returns a terminal
    /// outcome (a refused send, or a missing-loc block after the loc-wait
    /// budget) or `None` to keep polling.
    fn troll_door<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        hop: &mut TransportHop,
        options: &mut TravelOptions<'_>,
    ) -> Option<TravelOutcome> {
        let edge = match &hop.leg {
            Leg::Transport { edge } => edge.clone(),
            Leg::Walk { .. } => unreachable!("troll hop holds a transport leg"),
        };
        let here = here(snapshot);
        let tile = door_tile(&edge);
        if crate::debug_enabled() {
            eprintln!(
                "[nav-troll] here={here:?} at={tile:?} cheb={} ticks_waited={} loc_wait={}",
                cheb(here, edge.at),
                hop.ticks_waited,
                self.loc_wait
            );
        }
        // The game only accepts an `op_loc` from adjacent: while the
        // player is outside chebyshev 1 of the door (the cheap hop may
        // have lapsed while still approaching), re-send the approach walk
        // instead of the interact. Only once adjacent does the troll
        // re-send `op_loc` + the same-tick walk.
        if cheb(here, edge.at) > 1 {
            let Some(approach) = approach_tile(snapshot, &edge, here) else {
                // No standable tile adjacent to the door in the loaded
                // scene: keep waiting, bounded by the hop budget.
                self.loc_wait += 1;
                if self.loc_wait > self.budget {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Some(TravelOutcome::Blocked {
                        at: here,
                        leg: self.leg_index,
                        detail: format!(
                            "troll door loc {} has no standable tile within 1 of {tile:?} in the loaded scene",
                            edge.loc_id
                        ),
                    });
                }
                return None;
            };
            let mut ix = Interactions::new(snapshot, d);
            match ix.walk(approach) {
                SendResult::Sent { .. } => {}
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Some(TravelOutcome::Refused { at: here, reason });
                }
            }
            return None;
        }
        // The live loc's tile can sit a tile or two off the derived `at`
        // (the cheap hop's `find_transport_loc` already tolerates that),
        // so an exact-tile lookup misses it and the troll blocks while
        // the walker stands still. Search by id within chebyshev 3 of
        // `at` instead — the edge's closed `loc_id`, or the open leaf's
        // `open_loc_id` when the door reads open — nearest first, same
        // shape as `find_transport_loc`.
        let Some(loc) = snapshot
            .locs()
            .iter()
            .filter(|loc| {
                loc.tile.level == tile.level
                    && (loc.id == edge.loc_id
                        || edge
                            .open_loc_id
                            .is_some_and(|open_id| loc.id == open_id))
            })
            .map(|loc| (loc, cheb(loc.tile, tile)))
            .filter(|(_, gap)| *gap <= 3)
            .min_by_key(|(_, gap)| *gap)
            .map(|(loc, _)| loc)
        else {
            // The door's loc is not in the loaded scene yet (the loc
            // family is stale, or the door is out of view): keep waiting,
            // bounded by the hop budget.
            self.loc_wait += 1;
            if self.loc_wait > self.budget {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                return Some(TravelOutcome::Blocked {
                    at: here,
                    leg: self.leg_index,
                    detail: format!(
                        "troll door loc {} is not at {tile:?} in the loaded scene",
                        edge.loc_id
                    ),
                });
            }
            return None;
        };
        self.loc_wait = 0;
        let open = loc.id != edge.loc_id;
        let mut ix = Interactions::new(snapshot, d);
        if crate::debug_enabled() {
            eprintln!(
                "[nav-troll] door loc={} at={:?} open={} closed_id={}",
                loc.id, loc.tile, open, edge.loc_id
            );
        }
        // OP_LOC1 on an open door is Close. Closed: Open. Open: walk
        // through this tick (do not click — that slams it in the walker's
        // face and they turn back to the door).
        if !open {
            match ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(edge.option)) {
                SendResult::Sent { .. } => {
                    if crate::debug_enabled() {
                        eprintln!("[nav-troll] Open SENT");
                    }
                }
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Some(TravelOutcome::Refused { at: here, reason });
                }
            }
            return None;
        }
        match ix.walk(hop.to) {
            SendResult::Sent { .. } => {
                if crate::debug_enabled() {
                    eprintln!("[nav-troll] walk-through SENT to {:?}", hop.to);
                }
            }
            SendResult::Refused { reason, .. } => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                return Some(TravelOutcome::Refused { at: here, reason });
            }
        }
        None
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
    /// re-armed with `sent: false` and sent on the next call.
    sent: bool,
    ticks_waited: u32,
    /// The player's tile when the hop's walk was sent (stall detection).
    sent_tile: Option<WorldTile>,
    tries: u32,
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
/// every poll (op_loc always, plus the same-tick walk when open) after
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
    chat_seq: i32,
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
    tile: WorldTile,
    at: WorldTile,
    ticks_waited: u32,
}

/// The player's world tile from the snapshot: the canonical route-based
/// tile (`base + route_x[0]`, the server-confirmed position), the same
/// source the settle `arrived` arm and the runner's `arrived` proof read.
/// `(0, 0, 0)` before the first `PLAYER_INFO` (the m8aq `here()`
/// fallback).
fn here(snapshot: &GameSnapshot) -> WorldTile {
    snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level })
        .unwrap_or(WorldTile {
            x: 0,
            z: 0,
            level: 0,
        })
}

/// Chebyshev distance between world tiles (level ignored, like `nav::tile`).
fn cheb(a: WorldTile, b: WorldTile) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

/// The next walk-hop target: the leg's last tile when it is within ~20
/// tiles, otherwise a tile ~15 steps ahead of the player (or the last
/// uncleared tile when the leg is shorter), so each hop re-routes a fresh
/// short path and never aims back toward the leg start.
fn pick_aim(tiles: &[WorldTile], here: WorldTile, cursor: usize) -> (WorldTile, usize) {
    let last = *tiles.last().expect("walk legs are non-empty");
    if cheb(here, last) <= 20 {
        (last, tiles.len() - 1)
    } else {
        let base = tiles
            .iter()
            .position(|t| *t == here)
            .filter(|&i| i >= cursor)
            .unwrap_or(cursor);
        let idx = (base + 15).min(tiles.len() - 1).max(base + 1);
        (tiles[idx], idx)
    }
}

/// The eight tiles within chebyshev 1 of a tile, swept in a fixed order so
/// the nearest-approach choice is deterministic.
const APPROACH_RING: [(i32, i32); 8] = [
    (0, 1),
    (0, -1),
    (1, 0),
    (-1, 0),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

/// The nearest standable tile within chebyshev 1 of `edge.at` — the
/// take-off the transport interact must be sent from (the game only
/// accepts an `op_loc` from adjacent). `at` itself is the interact target
/// — a blocked loc tile — and is never a candidate.
/// [`scene_standable`] mirrors `WorldCollision::standable` against the
/// loaded scene's collision flags. `None` when no adjacent tile is
/// standable.
fn approach_tile(snapshot: &GameSnapshot, edge: &TransportEdge, here: WorldTile) -> Option<WorldTile> {
    APPROACH_RING
        .iter()
        .map(|(dx, dz)| WorldTile {
            x: edge.at.x + dx,
            z: edge.at.z + dz,
            level: edge.at.level,
        })
        .filter(|t| scene_standable(snapshot, *t))
        .min_by_key(|t| cheb(*t, here))
}

/// Whether a tile is standable in the loaded scene: no footprint block —
/// no `WALK_SCENERY` footprint, no `WR_GRND` ground block, and no
/// `SQ_BLOCKED` base — the same test as `WorldCollision::standable`
/// against the client's raw collision flags. Tiles the scene has no flags
/// for (outside it, or on another level) are not standable.
fn scene_standable(snapshot: &GameSnapshot, tile: WorldTile) -> bool {
    SceneQuery::new(snapshot.scene(), None)
        .collision_at(tile)
        .is_some_and(|flags| {
            flags & (CollisionFlag::WALK_SCENERY | CollisionFlag::WR_GRND | CollisionFlag::SQ_BLOCKED)
                == 0
        })
}

/// The snapshot loc for a transport edge: the edge's `loc_id` on the
/// edge's level within 3 tiles of `edge.at` (the m8aq `gap <= 3`),
/// nearest first.
fn find_transport_loc<'s>(snapshot: &'s GameSnapshot, edge: &TransportEdge) -> Option<&'s LocView> {
    snapshot
        .locs()
        .iter()
        .filter(|loc| loc.id == edge.loc_id && loc.tile.level == edge.at.level)
        .map(|loc| (loc, cheb(loc.tile, edge.at)))
        .filter(|(_, gap)| *gap <= 3)
        .min_by_key(|(_, gap)| *gap)
        .map(|(loc, _)| loc)
}

/// Closed or open leaf within chebyshev 3 of `edge.at` (live Catherby
/// open 1531 sits a tile off the derived `at`).
fn find_door_loc<'s>(snapshot: &'s GameSnapshot, edge: &TransportEdge) -> Option<&'s LocView> {
    snapshot
        .locs()
        .iter()
        .filter(|loc| {
            loc.tile.level == edge.at.level
                && (loc.id == edge.loc_id || edge.open_loc_id == Some(loc.id))
        })
        .map(|loc| (loc, cheb(loc.tile, edge.at)))
        .filter(|(_, gap)| *gap <= 3)
        .min_by_key(|(_, gap)| *gap)
        .map(|(loc, _)| loc)
}

/// Whether the live loc family already reads **open**. Searches closed
/// and open ids within 3 of `at` — an exact-tile check misses the
/// Catherby open leaf at (2816,3439) while `at` is (2816,3438).
fn edge_loc_open(snapshot: &GameSnapshot, edge: &TransportEdge) -> bool {
    if let Some(loc) = find_door_loc(snapshot, edge) {
        return loc.id != edge.loc_id;
    }
    snapshot.locs().iter().any(|loc| {
        cheb(loc.tile, edge.at) <= 3
            && loc.tile.level == edge.at.level
            && loc
                .actions
                .iter()
                .flatten()
                .any(|action| action.trim().eq_ignore_ascii_case("close"))
    })
}

/// The chat ring's latest sequence — the hop-start watermark for the
/// "I can't reach that!" watch (the `settle::said` sequence delta; never
/// a stale-head check on the single most recent line).
fn chat_seq(snapshot: &GameSnapshot) -> i32 {
    Query::new(snapshot.chat_lines()).latest_sequence()
}

/// The door's own tile: the edge's `at` — in the new edge model `at` IS
/// the loc tile (the interact target), so no midpoint derivation. The
/// door-troll read compares the loc's live id at this tile against the
/// edge's closed id.
fn door_tile(edge: &TransportEdge) -> WorldTile {
    edge.at
}

/// Whether `here` has crossed a door edge to its far side. `dir` is the
/// wall's crossing direction; proximity alone can match a near-side
/// approach tile to `to` when `close_enough` is 2.
fn door_crossed(edge: &TransportEdge, here: WorldTile) -> bool {
    match edge.dir {
        Some(DoorDir::N) => here.z > edge.at.z,
        Some(DoorDir::S) => here.z < edge.at.z,
        Some(DoorDir::E) => here.x > edge.at.x,
        Some(DoorDir::W) => here.x < edge.at.x,
        None => true,
    }
}

/// Fire the `on_leg` callback for a phase transition, using the `options`
/// of the poll in which the transition happened.
fn fire_leg(options: &mut TravelOptions<'_>, leg: &Leg, phase: LegPhase) {
    if let Some(cb) = options.on_leg.as_mut() {
        cb(leg, phase);
    }
}

#[cfg(test)]
mod tests {
    use api::interact::{Driver, SendReason};
    use api::prot::Out;
    use api::snapshot::{GameSnapshot, WorldTile};
    use client::client::{Client, ClientConfig, ClientPlayer, MiniMenuAction};
    use client::config::LocType;
    use client::io::{ClientStream, ServerProt};
    use std::sync::Arc;

    use crate::grid::StepGrid;
    use crate::router::{find_on_grid, Leg, Route};
    use crate::tile::Tile;
    use crate::transport::{DoorDir, TransportEdge, TransportKind};
    use crate::traveller::{
        door_tile, FollowRun, HopFailure, LegPhase, NavStatus, Poll, TransportHop, TravelOptions,
        TravelOutcome, Traveller,
    };

    #[test]
    fn no_route_ticks_idle() {
        let mut t = Traveller::new();
        let mut r = Rec::default();
        assert_eq!(
            t.tick(
                &mut r,
                Tile {
                    x: 0,
                    z: 0,
                    level: 0
                },
                false
            ),
            NavStatus::Idle
        );
    }

    #[test]
    fn remaining_is_empty_without_route() {
        let t = Traveller::new();
        assert!(t.remaining_walk_tiles(None).is_empty());
    }

    #[test]
    fn remaining_covers_armed_route_tiles() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_3x3(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 2,
                    z: 2,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let tiles = t.remaining_walk_tiles(None);
        assert_eq!(
            tiles.first(),
            Some(&Tile {
                x: 0,
                z: 0,
                level: 0
            })
        );
        assert_eq!(
            tiles.last(),
            Some(&Tile {
                x: 2,
                z: 2,
                level: 0
            })
        );
    }

    #[test]
    fn remaining_is_empty_when_standing_on_dest() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_3x3(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 2,
                    z: 2,
                    level: 0,
                },
            )
            .unwrap(),
        );
        assert!(t
            .remaining_walk_tiles(Some(Tile {
                x: 2,
                z: 2,
                level: 0
            }))
            .is_empty());
    }

    #[test]
    fn remaining_skips_completed_legs_and_connects_doors() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_door_corridor(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 4,
                    z: 0,
                    level: 0,
                },
            )
            .unwrap(),
        );
        // Standing on the door's from-tile: the first walk leg is done and
        // the door connects straight to the far walk leg (no duplicate
        // crossing tile).
        let tiles = t.remaining_walk_tiles(Some(Tile {
            x: 1,
            z: 0,
            level: 0,
        }));
        let expected = vec![
            Tile {
                x: 1,
                z: 0,
                level: 0,
            },
            Tile {
                x: 3,
                z: 0,
                level: 0,
            },
            Tile {
                x: 4,
                z: 0,
                level: 0,
            },
        ];
        assert_eq!(tiles, expected);
    }

    #[test]
    fn remaining_trims_current_leg_to_here() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_1x40(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 39,
                    z: 0,
                    level: 0,
                },
            )
            .unwrap(),
        );
        // Mid-leg: the line starts at the player's tile, not the leg start.
        let tiles = t.remaining_walk_tiles(Some(Tile {
            x: 15,
            z: 0,
            level: 0,
        }));
        assert_eq!(
            tiles.first(),
            Some(&Tile {
                x: 15,
                z: 0,
                level: 0
            })
        );
        assert_eq!(tiles.len(), 25);
        assert_eq!(
            tiles.last(),
            Some(&Tile {
                x: 39,
                z: 0,
                level: 0
            })
        );
    }

    #[test]
    fn arm_queues_dest_and_clear_drops_it() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_3x3(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 2,
                    z: 2,
                    level: 0,
                },
            )
            .unwrap(),
        );
        assert_eq!(
            t.queued(),
            Some(Tile {
                x: 2,
                z: 2,
                level: 0
            })
        );
        t.clear();
        assert_eq!(t.queued(), None);
    }

    #[test]
    fn walk_leg_sends_walk_toward_dest() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_3x3(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 2,
                    z: 2,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((0, 0)),
            ..Rec::default()
        };
        assert_eq!(
            t.tick(
                &mut r,
                Tile {
                    x: 0,
                    z: 0,
                    level: 0
                },
                false
            ),
            NavStatus::Walking
        );
        assert!(r.walked.is_some());
    }

    #[test]
    fn long_walk_leg_hop_targets_fifteen_ahead() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_1x40(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 39,
                    z: 0,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((0, 0)),
            ..Rec::default()
        };
        t.tick(
            &mut r,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            false,
        );
        // Far end is 39 away (> 20): hop to a tile ~15 steps ahead.
        let (x, z) = r.walked.expect("walk sent");
        assert!((10..=20).contains(&x), "hop target x was {x}");
        assert_eq!(z, 0);
    }

    #[test]
    fn long_walk_leg_second_hop_stays_ahead() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_1x40(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 39,
                    z: 0,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((15, 0)),
            ..Rec::default()
        };
        // Second hop from 15 tiles in: the target must stay ahead of
        // `here`, not point back toward the leg start.
        t.tick(
            &mut r,
            Tile {
                x: 15,
                z: 0,
                level: 0,
            },
            false,
        );
        let (x, z) = r.walked.expect("walk sent");
        assert!((25..=35).contains(&x), "second-hop target x was {x}");
        assert_eq!(z, 0);
    }

    #[test]
    fn arrived_on_dest_clears_and_reports_arrived() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_3x3(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 2,
                    z: 2,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let mut r = Rec::default();
        assert_eq!(
            t.tick(
                &mut r,
                Tile {
                    x: 2,
                    z: 2,
                    level: 0
                },
                false
            ),
            NavStatus::Arrived
        );
        assert_eq!(t.queued(), None);
    }

    #[test]
    fn door_open_walks_without_requiring_op_loc() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_door_corridor(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 4,
                    z: 0,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((1, 0)),
            ..Rec::default()
        };
        // skip to door by standing on from-tile; door already open
        assert_eq!(
            t.tick(
                &mut r,
                Tile {
                    x: 1,
                    z: 0,
                    level: 0
                },
                true
            ),
            NavStatus::Door
        );
        assert!(r.walked.is_some(), "open door walks through");
        // locs may be 0: OP_LOC1 on an open loc Closes it
    }

    #[test]
    fn current_door_is_the_armed_door_leg() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_door_corridor(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 4,
                    z: 0,
                    level: 0,
                },
            )
            .unwrap(),
        );
        assert_eq!(
            t.current_door(Tile {
                x: 1,
                z: 0,
                level: 0
            }),
            Some((
                Tile {
                    x: 2,
                    z: 0,
                    level: 0
                },
                1530
            ))
        );
        assert_eq!(
            t.current_door(Tile {
                x: 0,
                z: 0,
                level: 0
            }),
            None
        );
    }

    #[test]
    fn door_closed_only_op_loc() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_door_corridor(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 4,
                    z: 0,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((1, 0)),
            ..Rec::default()
        };
        assert_eq!(
            t.tick(
                &mut r,
                Tile {
                    x: 1,
                    z: 0,
                    level: 0
                },
                false
            ),
            NavStatus::Door
        );
        assert!(r.locs >= 1);
        assert!(r.walked.is_none());
    }

    #[test]
    fn budget_exceeded_reports_budget_and_clears() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_3x3(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 2,
                    z: 2,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((0, 0)),
            ..Rec::default()
        };
        let mut status = NavStatus::Walking;
        for _ in 0..61 {
            status = t.tick(
                &mut r,
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                false,
            );
        }
        assert_eq!(status, NavStatus::Budget);
        assert_eq!(t.queued(), None);
    }

    #[test]
    fn budget_resets_when_here_advances() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_3x3(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 2,
                    z: 2,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((0, 0)),
            ..Rec::default()
        };
        let mut status;
        for _ in 0..59 {
            status = t.tick(
                &mut r,
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                false,
            );
            assert_eq!(status, NavStatus::Walking);
        }
        // The 60th tick moves off the stuck tile: the clock restarts, so
        // the traveller keeps walking instead of tripping the budget.
        status = t.tick(
            &mut r,
            Tile {
                x: 1,
                z: 0,
                level: 0,
            },
            false,
        );
        assert_eq!(status, NavStatus::Walking);
    }

    #[test]
    fn walk_leg_falls_back_to_next_tile_when_dest_rejected() {
        let mut t = Traveller::new();
        t.arm(
            find_on_grid(
                &StepGrid::fixture_open_3x3(),
                Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                Tile {
                    x: 2,
                    z: 2,
                    level: 0,
                },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((0, 0)),
            reject_far: true,
            ..Rec::default()
        };
        // The leg far end (2,2) is 2 away and the driver rejects it; the
        // traveller retries the adjacent tile so the hop still goes out.
        assert_eq!(
            t.tick(
                &mut r,
                Tile {
                    x: 0,
                    z: 0,
                    level: 0
                },
                false
            ),
            NavStatus::Walking
        );
        assert!(r.walked.is_some(), "adjacent fallback hop was sent");
    }

    // --- Task 14: pollable high-level follow (drive legs via interact+settle) ---

    /// An attached, ingame client with the scene built at base (3200, 3200).
    fn scene_client() -> Client {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let stream = ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
        // Keep the listener alive so the connect stays established.
        std::mem::forget(listener);
        let mut c = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        });
        c.stream = Some(stream);
        c.ingame = true;
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c
    }

    /// Plant the local player at scene (x, z); the actor's world tile lands
    /// on (3200 + x, 3200 + z).
    fn plant_player(c: &mut Client, x: i32, z: i32) {
        let mut lp = ClientPlayer::at(x, z);
        lp.entity.x = x * 128 + 64;
        lp.entity.z = z * 128 + 64;
        c.local_player = Some(lp);
    }

    /// Bump every gen and rebuild every family into a fresh snapshot (tick 1).
    fn snap_at(c: &mut Client, x: i32, z: i32) -> GameSnapshot {
        plant_player(c, x, z);
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        let mut snap = GameSnapshot::new();
        snap.rebuild(c);
        snap
    }

    /// Bump every gen and rebuild into the existing snapshot (tick + 1).
    fn bump_rebuild(c: &mut Client, snap: &mut GameSnapshot) {
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        snap.rebuild(c);
    }

    /// A wall loc (id 1, "Ladder") at scene (3, 4) → world tile (3203, 3204).
    fn plant_ladder(c: &mut Client, op: Option<&str>) {
        let typecode = 0x4000_0000 + (1 << 14) + 3 + (4 << 7);
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.locs.len() <= 1 {
                cache.locs.push(LocType::default());
            }
            cache.locs[1] = LocType {
                id: 1,
                name: "Ladder".into(),
                op: vec![op.map(str::to_string), None, None, None, None],
                ..Default::default()
            };
        }
        c.world.set_wall(0, 3, 4, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
    }

    /// A wall loc at scene (`scene_x`, 0) — the `at` tile of [`door_edge`]
    /// or an offset of it — as the closed door (1530, "Open") or the open
    /// state (1531, "Close"), mirroring the Catherby range-house door
    /// configs.
    fn plant_door(c: &mut Client, open: bool, scene_x: i32) {
        plant_door_at(c, open, scene_x, 0);
    }

    fn plant_door_at(c: &mut Client, open: bool, scene_x: i32, scene_z: i32) {
        let id = if open { 1531 } else { 1530 };
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.locs.len() <= 1531 {
                cache.locs.push(LocType::default());
            }
            cache.locs[1530] = LocType {
                id: 1530,
                name: "Door".into(),
                op: vec![Some("Open".into()), None, None, None, None],
                ..Default::default()
            };
            cache.locs[1531] = LocType {
                id: 1531,
                name: "Door".into(),
                op: vec![Some("Close".into()), None, None, None, None],
                ..Default::default()
            };
        }
        let typecode = 0x4000_0000 + (id << 14) + scene_x + (scene_z << 7);
        c.world
            .set_wall(0, scene_x, scene_z, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
    }

    /// A level-0 walk leg over the given (x, z) world tiles.
    fn walk_leg(tiles: &[(i32, i32)]) -> Leg {
        Leg::Walk {
            tiles: tiles
                .iter()
                .map(|(x, z)| WorldTile {
                    x: *x,
                    z: *z,
                    level: 0,
                })
                .collect(),
        }
    }

    /// A Catherby-style door edge on the fixture scene: closed loc 1530 at
    /// the door tile (3201, 3200) — the edge's `at` — crossing east to
    /// (3203, 3200) (`to` 2 tiles away; the far-side tile between is
    /// blocked, so the far-side walk-out lands 2 out).
    fn door_edge() -> TransportEdge {
        TransportEdge {
            kind: TransportKind::Door,
            at: WorldTile {
                x: 3201,
                z: 3200,
                level: 0,
            },
            to: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            loc_id: 1530,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
        }
    }

    /// A ladder edge standing at (3202, 3204) → (3202, 3205).
    fn ladder_edge() -> TransportEdge {
        TransportEdge {
            kind: TransportKind::Ladder,
            at: WorldTile {
                x: 3202,
                z: 3204,
                level: 0,
            },
            to: WorldTile {
                x: 3202,
                z: 3205,
                level: 0,
            },
            loc_id: 1,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
        }
    }

    /// Drive `follow` to a terminal outcome, running `on_tick` (the host's
    /// per-tick cadence) and rebuilding the snapshot between polls.
    fn drive<D: Driver>(
        t: &mut Traveller,
        d: &mut D,
        c: &mut Client,
        snap: &mut GameSnapshot,
        route: &Route,
        options: &mut TravelOptions<'_>,
        mut on_tick: impl FnMut(&mut Client),
    ) -> TravelOutcome {
        loop {
            if let Some(outcome) = t.follow(d, snap, route.clone(), options) {
                return outcome;
            }
            on_tick(c);
            bump_rebuild(c, snap);
        }
    }

    #[test]
    fn follow_walks_a_single_leg_to_arrival() {
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[(3200, 3200), (3200, 3201)])],
            dest: WorldTile {
                x: 3200,
                z: 3201,
                level: 0,
            },
            ticks: 0.5, // 1 run step
        };
        let mut options = TravelOptions::default();
        // One tile to the leg end: the run sends one walk and the arrived
        // arm completes the hop once the player steps onto the tile.
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |c| {
            plant_player(c, 0, 1);
        });
        assert!(matches!(
            outcome,
            TravelOutcome::Arrived { at } if at == WorldTile { x: 3200, z: 3201, level: 0 }
        ));
        assert_eq!(rec.walked.len(), 1, "one walk send");
        assert_eq!(rec.walked, vec![(0, 1)]);
    }

    #[test]
    fn follow_does_not_arrive_short_of_the_last_walk_tile() {
        // The leg's last tile is the destination: the settle must require
        // the player to stand on it exactly. A loose `close_enough` would
        // report Arrived one tile short, and the scenario runner would
        // re-arm the original route (through a door, that walks back).
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[(3200, 3200), (3200, 3201), (3200, 3202)])],
            dest: WorldTile {
                x: 3200,
                z: 3202,
                level: 0,
            },
            ticks: 1.0,
        };
        let mut options = TravelOptions::default(); // close_enough = 2

        assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        assert_eq!(rec.walked, vec![(0, 2)], "walk aims at the last tile");

        plant_player(&mut c, 0, 1);
        bump_rebuild(&mut c, &mut snap);
        assert!(
            t.follow(&mut rec, &snap, route.clone(), &mut options).is_none(),
            "a walk leg must not finish one tile short of its last tile"
        );

        plant_player(&mut c, 0, 2);
        bump_rebuild(&mut c, &mut snap);
        match t.follow(&mut rec, &snap, route.clone(), &mut options) {
            Some(TravelOutcome::Arrived { at }) => {
                assert_eq!(at, WorldTile { x: 3200, z: 3202, level: 0 });
            }
            other => panic!("expected Arrived on the last tile, got {other:?}"),
        }
    }

    #[test]
    fn current_aim_is_none_idle_and_the_walk_hop_aim_in_follow() {
        let mut t = Traveller::new();
        assert_eq!(t.current_aim(), None, "idle traveller has no aim");
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let route = Route {
            legs: vec![walk_leg(&[(3200, 3200), (3200, 3201), (3200, 3202)])],
            dest: WorldTile {
                x: 3200,
                z: 3202,
                level: 0,
            },
            ticks: 1.0,
        };
        let mut options = TravelOptions::default();
        assert!(
            t.follow(&mut rec, &snap, route.clone(), &mut options).is_none(),
            "the run must be active after the first poll"
        );
        // A short leg aims at its last tile (the same `pick_aim` the walk
        // send used).
        assert_eq!(
            t.current_aim(),
            Some(WorldTile {
                x: 3200,
                z: 3202,
                level: 0
            }),
            "the walk hop's aim is the leg's last tile"
        );
    }

    #[test]
    fn follow_skips_single_tile_walk_legs() {
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[(3200, 3200)])],
            dest: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            ticks: 0.0, // no steps
        };
        let mut options = TravelOptions::default();
        // A single-tile leg (the find(from == to) shape) is a no-op: no
        // walk is sent, the run arrives immediately.
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |_| {});
        assert!(matches!(outcome, TravelOutcome::Arrived { .. }));
        assert_eq!(rec.walked.len(), 0, "no walk for a single-tile leg");
    }

    #[test]
    fn follow_transport_leg_interacts_and_arrives() {
        let mut c = scene_client();
        plant_ladder(&mut c, Some("Climb"));
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![Leg::Transport {
                edge: ladder_edge(),
            }],
            dest: WorldTile {
                x: 3202,
                z: 3205,
                level: 0,
            },
            ticks: 2.0, // the ladder edge's ticks
        };
        let mut options = TravelOptions::default();
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |c| {
            plant_player(c, 2, 5);
        });
        assert!(matches!(
            outcome,
            TravelOutcome::Arrived { at } if at == WorldTile { x: 3202, z: 3205, level: 0 }
        ));
        assert_eq!(rec.loc_ops, 1, "one OP_LOC1 interact sent");
    }

    #[test]
    fn follow_approaches_a_transport_loc_before_interacting() {
        // The router may arm a transport leg from a take-off tile up to
        // `INTERACT_RADIUS` (3) from `at`, but the game only accepts an
        // `op_loc` from adjacent. The player starts 3 tiles south of the
        // ladder loc: `follow` must first walk to the nearest standable
        // tile within chebyshev 1 of `at` (here (3202, 3203)) and only
        // then send `op_loc`; once adjacent it interacts and settles
        // `arrived(edge.to)`.
        let mut c = scene_client();
        plant_ladder(&mut c, Some("Climb"));
        let mut snap = snap_at(&mut c, 2, 1); // cheb 3 from the ladder's `at`
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![Leg::Transport {
                edge: ladder_edge(),
            }],
            dest: WorldTile {
                x: 3202,
                z: 3205,
                level: 0,
            },
            ticks: 2.0, // the ladder edge's ticks
        };
        let mut options = TravelOptions::default();
        // Poll 1: the hop walks to the adjacent standable tile, never the
        // interact (the click would be dropped from 3 tiles away).
        assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        assert_eq!(rec.walked, vec![(2, 3)], "the approach walk goes out first");
        assert_eq!(rec.loc_ops, 0, "no OP_LOC1 before the player is adjacent");
        // The player steps onto the approach tile: the hop sends `op_loc`.
        plant_player(&mut c, 2, 3);
        bump_rebuild(&mut c, &mut snap);
        assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        assert_eq!(rec.loc_ops, 1, "one OP_LOC1 once adjacent");
        // The ladder carries the player to `edge.to`: the run arrives.
        plant_player(&mut c, 2, 5);
        bump_rebuild(&mut c, &mut snap);
        match t.follow(&mut rec, &snap, route.clone(), &mut options) {
            Some(TravelOutcome::Arrived { at }) => {
                assert_eq!(at, WorldTile { x: 3202, z: 3205, level: 0 });
            }
            other => panic!("expected Arrived, got {other:?}"),
        }
    }

    #[test]
    fn follow_auto_trolls_a_door_when_the_cheap_hop_lapses() {
        // A tick-perfect closer: the mock alternates the door loc's id
        // (closed 1530 / open 1531) every tick, and the player only
        // crosses when a walk is actually sent. The cheap one-interact
        // hop can never cross within its budget, so `follow` must escalate
        // on its own — no option flag — and troll the door: re-open it
        // every tick and walk through in the same tick the door reads
        // open, so the closer cannot slam it shut between the open and the
        // walk. The edge carries the open leaf's id (`open_loc_id`), the
        // id the troll's radius lookup matches when the door reads open.
        let mut c = scene_client();
        plant_door(&mut c, false, 1);
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![Leg::Transport {
                edge: TransportEdge {
                    open_loc_id: Some(1531),
                    ..door_edge()
                },
            }],
            dest: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            ticks: 1.0,
        };
        // Default options: the fallback must engage automatically.
        let mut options = TravelOptions {
            budget_ticks_per_hop: 3,
            close_enough: 1,
            ..TravelOptions::default()
        };

        let mut tick = 0u32;
        let mut crossed = false;
        loop {
            match t.follow(&mut rec, &snap, route.clone(), &mut options) {
                Some(TravelOutcome::Arrived { at }) => {
                    assert_eq!(
                        at,
                        WorldTile {
                            x: 3203,
                            z: 3200,
                            level: 0
                        }
                    );
                    break;
                }
                Some(other) => panic!("expected Arrived, got {other:?}"),
                None => {}
            }
            tick += 1;
            assert!(tick < 200, "the automatic troll fallback never arrived");
            // The closer slams the door shut each tick: alternate the
            // door's open/closed state, and only move the player once a
            // walk was actually sent (the troll's same-tick walk).
            let open = tick % 2 == 1;
            plant_door(&mut c, open, 1);
            if open && !crossed && !rec.walked.is_empty() {
                crossed = true;
                plant_player(&mut c, 3, 0);
            }
            bump_rebuild(&mut c, &mut snap);
        }
        assert!(crossed, "the troll's same-tick walk never crossed the door");
        assert!(
            rec.loc_ops >= 2,
            "the troll must re-open the door after the cheap hop lapses, got {} loc ops",
            rec.loc_ops
        );
        assert!(
            rec.walked.contains(&(3, 0)),
            "the troll walks through the open door in the same tick"
        );
    }

    #[test]
    fn follow_troll_walks_an_open_door_without_closing_it() {
        // OP_LOC1 on an open door is Close. After the cheap hop lapses the
        // troll must walk through an already-open door and not click it.
        let mut c = scene_client();
        plant_door(&mut c, false, 1);
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![Leg::Transport {
                edge: TransportEdge {
                    open_loc_id: Some(1531),
                    ..door_edge()
                },
            }],
            dest: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            ticks: 1.0,
        };
        let mut options = TravelOptions {
            budget_ticks_per_hop: 2,
            close_enough: 1,
            ..TravelOptions::default()
        };

        // Cheap hop: interact, then sit the budget out.
        assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        let ops_after_cheap = rec.loc_ops;
        assert!(ops_after_cheap >= 1, "cheap hop sent OP_LOC1");
        for _ in 0..3 {
            bump_rebuild(&mut c, &mut snap);
            assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        }

        plant_door(&mut c, true, 1);
        bump_rebuild(&mut c, &mut snap);
        let ops_before_open = rec.loc_ops;
        assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        assert_eq!(
            rec.loc_ops, ops_before_open,
            "troll must not OP_LOC1 an open door (that Closes it)"
        );
        assert!(
            rec.walked.contains(&(3, 0)),
            "troll walks through the open door"
        );
    }

    #[test]
    fn follow_troll_finds_an_offset_door_loc_within_radius() {
        // The door loc's live tile is offset from the edge's derived `at`
        // by +1 in x, so the exact-tile lookup (`l.tile == at`) never
        // finds it and the troll blocks while the walker stands still.
        // The troll must search by id within radius 3 of `at` — the same
        // shape as `find_transport_loc` — to find the offset loc, re-open
        // it, and walk through on the open tick instead of `Blocked`.
        let mut c = scene_client();
        plant_door(&mut c, false, 2); // offset from door_edge()'s at (3201, 3200)
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![Leg::Transport {
                edge: TransportEdge {
                    open_loc_id: Some(1531),
                    ..door_edge()
                },
            }],
            dest: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            ticks: 1.0,
        };
        let mut options = TravelOptions {
            budget_ticks_per_hop: 3,
            close_enough: 1,
            ..TravelOptions::default()
        };

        let mut tick = 0u32;
        let mut crossed = false;
        loop {
            match t.follow(&mut rec, &snap, route.clone(), &mut options) {
                Some(TravelOutcome::Arrived { at }) => {
                    assert_eq!(
                        at,
                        WorldTile {
                            x: 3203,
                            z: 3200,
                            level: 0
                        }
                    );
                    break;
                }
                Some(other) => panic!("expected Arrived, got {other:?}"),
                None => {}
            }
            tick += 1;
            assert!(tick < 200, "the troll never crossed the offset door");
            // A tick-perfect closer slams the door each tick; the player
            // only crosses once a walk was actually sent (the troll's
            // same-tick walk), exactly like the on-at troll test.
            let open = tick % 2 == 1;
            plant_door(&mut c, open, 2);
            if open && !crossed && !rec.walked.is_empty() {
                crossed = true;
                plant_player(&mut c, 3, 0);
            }
            bump_rebuild(&mut c, &mut snap);
        }
        assert!(crossed, "the troll's same-tick walk never crossed the door");
        assert!(
            rec.loc_ops >= 2,
            "the troll must re-open the offset door after the cheap hop lapses, got {} loc ops",
            rec.loc_ops
        );
        assert!(
            rec.walked.contains(&(3, 0)),
            "the troll walks through the open door in the same tick"
        );
    }

    #[test]
    fn follow_walks_through_an_open_leaf_without_op_loc() {
        // The open leaf (1531) is planted at `edge.at` and the edge
        // carries its id: `follow` must not interact (OP_LOC1 on an open
        // leaf Closes it) — it walks straight through and arrives on
        // `edge.to` with no op_loc at all.
        let mut c = scene_client();
        plant_door(&mut c, true, 1);
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![Leg::Transport {
                edge: TransportEdge {
                    open_loc_id: Some(1531),
                    ..door_edge()
                },
            }],
            dest: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            ticks: 1.0,
        };
        let mut options = TravelOptions::default();
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |c| {
            plant_player(c, 3, 0);
        });
        assert!(matches!(
            outcome,
            TravelOutcome::Arrived { at } if at == WorldTile { x: 3203, z: 3200, level: 0 }
        ));
        assert_eq!(rec.loc_ops, 0, "no OP_LOC1 through an open leaf");
        assert!(rec.walked.contains(&(3, 0)), "walk straight through the open door");
    }

    #[test]
    fn follow_walks_through_a_close_leaf_without_op_loc() {
        // The door's config carries no `open_loc_id`: the open leaf is
        // still recognized by the closed id being absent from `edge.at`
        // while a same-tile loc offers the "Close" op.
        let mut c = scene_client();
        plant_door(&mut c, true, 1);
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![Leg::Transport {
                edge: door_edge(), // open_loc_id: None
            }],
            dest: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            ticks: 1.0,
        };
        let mut options = TravelOptions::default();
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |c| {
            plant_player(c, 3, 0);
        });
        assert!(matches!(
            outcome,
            TravelOutcome::Arrived { at } if at == WorldTile { x: 3203, z: 3200, level: 0 }
        ));
        assert_eq!(rec.loc_ops, 0, "no OP_LOC1 through a close leaf");
        assert!(rec.walked.contains(&(3, 0)), "walk straight through the open door");
    }

    #[test]
    fn follow_cheap_hop_walks_when_the_open_leaf_is_offset() {
        // Live Catherby: closed 1530 is derived at (2816,3438) but the
        // open 1531 sits a tile north. `edge_loc_open` used exact `at`, so
        // after Open the hop kept waiting (open=false) until the troll
        // budget. Cheap hop must see the offset open leaf and walk.
        let mut c = scene_client();
        plant_door_at(&mut c, false, 1, 0);
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let edge = TransportEdge {
            open_loc_id: Some(1531),
            dir: Some(DoorDir::E),
            ..door_edge()
        };
        let route = Route {
            legs: vec![Leg::Transport { edge }],
            dest: WorldTile {
                x: 3203,
                z: 3200,
                level: 0,
            },
            ticks: 1.0,
        };
        let mut options = TravelOptions::default();

        assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        assert_eq!(rec.loc_ops, 1, "cheap hop Opens the closed door");

        c.world.del_wall(0, 1, 0);
        plant_door_at(&mut c, true, 1, 1); // open leaf at (3201, 3201)
        bump_rebuild(&mut c, &mut snap);
        let walks = rec.walked.len();
        assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        assert!(
            rec.walked.len() > walks,
            "cheap hop must walk through once the offset open leaf appears"
        );
    }

    #[test]
    fn follow_fails_fast_when_the_game_reports_cant_reach() {
        // The client pathfind failed right after the interact: the game
        // says "I can't reach that!" (a chat line new since the hop
        // started) and the hop must fail fast with the unreachable
        // signal — never sit out the settle budget. The player starts
        // adjacent to the ladder, so the interact is the hop's first send
        // (a further start would walk the approach first).
        let mut c = scene_client();
        plant_ladder(&mut c, Some("Climb"));
        let mut snap = snap_at(&mut c, 2, 3);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![Leg::Transport {
                edge: ladder_edge(),
            }],
            dest: WorldTile {
                x: 3202,
                z: 3205,
                level: 0,
            },
            ticks: 2.0, // the ladder edge's ticks
        };
        // `close_enough: 1` keeps the player outside the arrive arm's
        // radius of `edge.to` while adjacent to `at` (the far side is
        // 2 tiles past the ladder), so the can't-reach line is the only
        // arm that can fire.
        let mut options = TravelOptions {
            close_enough: 1,
            ..TravelOptions::default()
        };
        // Poll 1: the interact is sent and the hop starts watching.
        assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        assert_eq!(rec.loc_ops, 1, "one OP_LOC1 interact sent");
        // One tick later the game reports the pathfind failed.
        c.add_chat(0, "I can't reach that!", "");
        bump_rebuild(&mut c, &mut snap);
        match t.follow(&mut rec, &snap, route.clone(), &mut options) {
            Some(TravelOutcome::Refused {
                at,
                reason: SendReason::Unreachable,
            }) => {
                assert_eq!(at, WorldTile { x: 3202, z: 3203, level: 0 });
            }
            other => panic!("expected Refused(Unreachable), got {other:?}"),
        }
    }

    #[test]
    fn door_tile_is_the_edge_at() {
        // A door with `to` 2 tiles away: the door's own tile is the
        // edge's `at` (the loc tile), never the midpoint of `at`/`to`.
        let edge = door_edge();
        assert_eq!(door_tile(&edge), edge.at);
        assert_ne!(door_tile(&edge).x, (edge.at.x + edge.to.x) / 2);
    }

    #[test]
    fn follow_stalls_dropped_when_the_player_never_moves() {
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[
                (3200, 3200),
                (3200, 3201),
                (3200, 3202),
                (3200, 3203),
                (3200, 3204),
            ])],
            dest: WorldTile {
                x: 3200,
                z: 3204,
                level: 0,
            },
            ticks: 2.0, // 4 run steps at 0.5 ticks each
        };
        let mut options = TravelOptions {
            budget_ticks_per_hop: 3,
            ..TravelOptions::default()
        };
        // The player never leaves the send tile: the hop lapses as Dropped.
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |_| {});
        assert!(matches!(
            outcome,
            TravelOutcome::Stalled { at, aiming, why: HopFailure::Dropped, .. }
                if at == WorldTile { x: 3200, z: 3200, level: 0 }
                    && aiming == WorldTile { x: 3200, z: 3204, level: 0 }
        ));
    }

    #[test]
    fn follow_stalls_expired_when_progress_stalls() {
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[
                (3200, 3200),
                (3200, 3201),
                (3200, 3202),
                (3200, 3203),
                (3200, 3204),
                (3200, 3205),
                (3200, 3206),
                (3200, 3207),
            ])],
            dest: WorldTile {
                x: 3200,
                z: 3207,
                level: 0,
            },
            ticks: 3.5, // 7 run steps at 0.5 ticks each
        };
        let mut options = TravelOptions {
            budget_ticks_per_hop: 3,
            ..TravelOptions::default()
        };
        // The player creeps one tile per tick toward the far leg end but
        // never arrives within the tiny hop budget: the hop lapses as
        // Expired (progress was made).
        let mut z = 0;
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |c| {
            z += 1;
            plant_player(c, 0, z);
        });
        assert!(matches!(
            outcome,
            TravelOutcome::Stalled { why: HopFailure::Expired, .. }
        ));
    }

    #[test]
    fn follow_refuses_when_the_walk_send_is_rejected() {
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            reject_far: true,
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[
                (3200, 3200),
                (3200, 3201),
                (3200, 3202),
                (3200, 3203),
                (3200, 3204),
            ])],
            dest: WorldTile {
                x: 3200,
                z: 3204,
                level: 0,
            },
            ticks: 2.0, // 4 run steps at 0.5 ticks each
        };
        let mut options = TravelOptions::default();
        // The driver rejects the multi-tile hop: `Interactions::walk`
        // refuses with `Unreachable`, and follow reports it verbatim.
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |_| {});
        assert!(matches!(
            outcome,
            TravelOutcome::Refused { reason: SendReason::Unreachable, .. }
        ));
    }

    #[test]
    fn follow_gives_up_after_max_hops() {
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let tiles: Vec<(i32, i32)> = (0..40).map(|z| (3200, 3200 + z)).collect();
        let route = Route {
            legs: vec![walk_leg(&tiles)],
            dest: WorldTile {
                x: 3200,
                z: 3239,
                level: 0,
            },
            ticks: 19.5, // 39 run steps at 0.5 ticks each
        };
        let mut options = TravelOptions {
            close_enough: 200,
            max_hops: 2,
            ..TravelOptions::default()
        };
        // A loose close-enough matches every poll, so each call starts a
        // fresh hop until the hop cap trips `GaveUp`.
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |_| {});
        assert!(matches!(outcome, TravelOutcome::GaveUp { hops: 2, .. }));
    }

    #[test]
    fn follow_reports_leg_phases() {
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[(3200, 3200), (3200, 3201), (3200, 3202)])],
            dest: WorldTile {
                x: 3200,
                z: 3202,
                level: 0,
            },
            ticks: 1.0, // 2 run steps at 0.5 ticks each
        };
        let mut phases = Vec::new();
        let mut options = TravelOptions {
            on_leg: Some(Box::new(|leg: &Leg, phase: LegPhase| {
                phases.push((leg.clone(), phase));
            })),
            ..TravelOptions::default()
        };
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |c| {
            plant_player(c, 0, 2);
        });
        assert!(matches!(outcome, TravelOutcome::Arrived { .. }));
        // `options` borrows `phases` through the on_leg callback; dropping
        // it ends the borrow so the recording can be asserted.
        drop(options);
        assert_eq!(phases.len(), 2, "start + done for one leg");
        assert!(matches!(&phases[0], (Leg::Walk { .. }, LegPhase::Start)));
        assert!(matches!(&phases[1], (Leg::Walk { .. }, LegPhase::Done)));
    }

    #[test]
    fn follow_blocks_when_the_transport_loc_is_missing() {
        // The player starts adjacent to the edge's `at`, so the run reaches
        // the loc lookup without an approach walk; loc id 99 is never
        // planted in the scene.
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 2, 3);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        // Loc id 99 is never planted in the scene.
        let edge = TransportEdge {
            loc_id: 99,
            ..ladder_edge()
        };
        let route = Route {
            legs: vec![Leg::Transport { edge }],
            dest: WorldTile {
                x: 3202,
                z: 3205,
                level: 0,
            },
            ticks: 2.0, // the ladder edge's ticks
        };
        let mut options = TravelOptions {
            budget_ticks_per_hop: 3,
            ..TravelOptions::default()
        };
        // The loc never appears: after the loc-wait budget, the leg blocks.
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |_| {});
        assert!(matches!(
            outcome,
            TravelOutcome::Blocked { leg: 0, detail, .. } if detail.contains("99")
        ));
    }

    #[test]
    fn follow_drives_walk_then_transport_legs_in_order() {
        let mut c = scene_client();
        plant_ladder(&mut c, Some("Climb"));
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![
                // The walk leg ends adjacent to the ladder's `at`, so the
                // transport leg interacts on its first poll (no approach
                // walk) and the send counts stay per-leg.
                walk_leg(&[
                    (3200, 3200),
                    (3200, 3201),
                    (3200, 3202),
                    (3200, 3203),
                    (3201, 3203),
                ]),
                Leg::Transport {
                    edge: ladder_edge(),
                },
            ],
            dest: WorldTile {
                x: 3202,
                z: 3205,
                level: 0,
            },
            ticks: 3.0, // 2 run steps (1.0) + the ladder edge's 2 ticks
        };
        let mut phases = Vec::new();
        let mut options = TravelOptions {
            on_leg: Some(Box::new(|leg: &Leg, phase: LegPhase| {
                phases.push((leg.clone(), phase));
            })),
            ..TravelOptions::default()
        };
        let mut step = 0;
        let outcome = drive(&mut t, &mut rec, &mut c, &mut snap, &route, &mut options, |c| {
            step += 1;
            match step {
                1 => plant_player(c, 1, 3), // reach the walk leg's end (adjacent to the ladder)
                _ => plant_player(c, 2, 5), // cross the transport to edge.to
            }
        });
        assert!(matches!(
            outcome,
            TravelOutcome::Arrived { at } if at == WorldTile { x: 3202, z: 3205, level: 0 }
        ));
        assert_eq!(rec.walked.len(), 1, "one walk for the walk leg");
        assert_eq!(rec.loc_ops, 1, "one OP_LOC1 for the transport leg");
        // `options` borrows `phases` through the on_leg callback; dropping
        // it ends the borrow so the recording can be asserted.
        drop(options);
        assert_eq!(phases.len(), 4, "start/done per leg");
        assert!(matches!(&phases[0], (Leg::Walk { .. }, LegPhase::Start)));
        assert!(matches!(&phases[1], (Leg::Walk { .. }, LegPhase::Done)));
        assert!(matches!(&phases[2], (Leg::Transport { .. }, LegPhase::Start)));
        assert!(matches!(&phases[3], (Leg::Transport { .. }, LegPhase::Done)));
    }

    /// Recording driver for the follow tests: the build base matches the
    /// fixture scene (3200, 3200), so loc scene coords translate like the
    /// live client and `Interactions`' in-scene check passes. Walks are
    /// recorded scene-relative (`dx, dz` from the route origin).
    #[derive(Default)]
    struct FollowRec {
        walked: Vec<(i32, i32)>,
        loc_ops: usize,
        reject_far: bool,
        route: Option<(i32, i32)>,
        sink: Sink,
    }

    impl Driver for FollowRec {
        fn set_menu(&mut self, _slot: i32, action: i32, _a: i32, _b: i32, _c: i32) {
            if action == MiniMenuAction::OP_LOC1 {
                self.loc_ops += 1;
            }
        }

        fn do_action(&mut self, _slot: i32) -> bool {
            true
        }

        fn try_move(
            &mut self,
            src_x: i32,
            src_z: i32,
            dx: i32,
            dz: i32,
            _try_nearest: bool,
            _loc_width: i32,
            _loc_length: i32,
            _loc_angle: i32,
            _loc_shape: i32,
            _forceapproach: i32,
            _ty: i32,
        ) -> bool {
            if self.reject_far && (src_x - dx).abs().max((src_z - dz).abs()) > 1 {
                return false;
            }
            self.walked.push((dx, dz));
            true
        }

        fn local_route(&self) -> Option<(i32, i32)> {
            self.route
        }

        fn build_base(&self) -> (i32, i32) {
            (3200, 3200)
        }

        fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
            None
        }

        fn out(&mut self) -> &mut dyn Out {
            &mut self.sink
        }

        fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
            true
        }
    }

    #[test]
    fn follow_waits_across_polls_for_the_hop_to_arrive() {
        // Regression: a hop spanning several ticks must return `None` from
        // each still-waiting poll (the host re-polls next tick), never a
        // stall caused by re-polling the same snapshot within one call.
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[
                (3200, 3200),
                (3200, 3201),
                (3200, 3202),
                (3200, 3203),
                (3200, 3204),
                (3200, 3205),
            ])],
            dest: WorldTile {
                x: 3200,
                z: 3205,
                level: 0,
            },
            ticks: 2.5, // 5 run steps at 0.5 ticks each
        };
        let mut options = TravelOptions {
            budget_ticks_per_hop: 10,
            ..TravelOptions::default()
        };
        // Poll 1: the run sends the hop's walk.
        assert!(t.follow(&mut rec, &snap, route.clone(), &mut options).is_none());
        // Poll 2: the player is mid-leg — the settle must still be watching
        // (the call returns `None`), never a stall from an intra-call spin.
        plant_player(&mut c, 0, 2);
        bump_rebuild(&mut c, &mut snap);
        assert!(
            t.follow(&mut rec, &snap, route.clone(), &mut options).is_none(),
            "a mid-walk poll must not stall"
        );
        // Poll 3: the player reaches the leg end — the run arrives.
        plant_player(&mut c, 0, 5);
        bump_rebuild(&mut c, &mut snap);
        match t.follow(&mut rec, &snap, route.clone(), &mut options) {
            Some(TravelOutcome::Arrived { at }) => {
                assert_eq!(
                    at,
                    WorldTile {
                        x: 3200,
                        z: 3205,
                        level: 0
                    }
                )
            }
            other => panic!("expected Arrived, got {other:?}"),
        }
    }

    #[test]
    fn follow_steps_a_long_walk_leg_hop_by_hop() {
        // A leg longer than close_enough needs several hops: the player
        // advances toward the end between polls and each matched hop arms
        // the next one until the last tile is reached.
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let tiles: Vec<(i32, i32)> = (0..40).map(|z| (3200, 3200 + z)).collect();
        let route = Route {
            legs: vec![walk_leg(&tiles)],
            dest: WorldTile {
                x: 3200,
                z: 3239,
                level: 0,
            },
            ticks: 19.5, // 39 run steps at 0.5 ticks each
        };
        let mut options = TravelOptions {
            close_enough: 3,
            ..TravelOptions::default()
        };
        let mut z = 0;
        loop {
            if let Some(outcome) = t.follow(&mut rec, &snap, route.clone(), &mut options) {
                assert!(matches!(outcome, TravelOutcome::Arrived { .. }));
                break;
            }
            z = (z + 15).min(39);
            plant_player(&mut c, 0, z);
            bump_rebuild(&mut c, &mut snap);
        }
        assert!(
            rec.walked.len() >= 2,
            "a long leg needs several hops, got {}",
            rec.walked.len()
        );
    }

    #[test]
    fn level_change_transport_requires_proximity_to_to() {
        // Regression: a level-changing transport completes only when the
        // player is within `close_enough` of `to` on the destination level
        // — never merely for standing anywhere on that level.
        let mut c = scene_client();
        c.minusedlevel = 1;
        let snap = snap_at(&mut c, 100, 100);
        let edge = TransportEdge {
            kind: TransportKind::Ladder,
            at: WorldTile {
                x: 3202,
                z: 3204,
                level: 0,
            },
            to: WorldTile {
                x: 3202,
                z: 3205,
                level: 1,
            },
            loc_id: 1,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
        };
        let route = Route {
            legs: vec![Leg::Transport { edge }],
            dest: WorldTile {
                x: 3202,
                z: 3205,
                level: 1,
            },
            ticks: 2.0, // the ladder edge's ticks
        };
        let mut options = TravelOptions::default();
        let mut run = FollowRun::start(route, &options);
        let leg = run.legs.pop_front().expect("the transport leg");
        let mut rec = FollowRec::default();
        run.transport = Some(TransportHop {
            leg,
            to: WorldTile {
                x: 3202,
                z: 3205,
                level: 1,
            },
            ticks_waited: 0,
            sent_tile: Some(WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            }),
            tries: 0,
            troll: false,
            chat_seq: 0,
            approach: None,
        });
        // The player is on the destination level (1) but far from `to`:
        // the hop must still be watching.
        assert!(matches!(
            run.poll_transport(&mut rec, &snap, &mut options),
            Poll::Watching
        ));
        // Within close_enough of `to` on the destination level: the hop
        // completes the leg.
        plant_player(&mut c, 2, 5);
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        let mut snap = GameSnapshot::new();
        snap.rebuild(&mut c);
        assert!(matches!(
            run.poll_transport(&mut rec, &snap, &mut options),
            Poll::LegDone
        ));
    }

    /// Recording driver: captures the last walk target and counts OP_LOC1
    /// interactions. `route` stands in for the local player tile so
    /// `api::walk` finds a route origin. `reject_far` mirrors the live
    /// client rejecting a tryMove shot of more than one tile.
    #[derive(Default)]
    struct Rec {
        walked: Option<(i32, i32)>,
        locs: usize,
        route: Option<(i32, i32)>,
        reject_far: bool,
        sink: Sink,
    }

    impl Driver for Rec {
        fn set_menu(&mut self, _slot: i32, action: i32, _a: i32, _b: i32, _c: i32) {
            if action == MiniMenuAction::OP_LOC1 {
                self.locs += 1;
            }
        }

        fn do_action(&mut self, _slot: i32) -> bool {
            true
        }

        fn try_move(
            &mut self,
            src_x: i32,
            src_z: i32,
            dx: i32,
            dz: i32,
            _try_nearest: bool,
            _loc_width: i32,
            _loc_length: i32,
            _loc_angle: i32,
            _loc_shape: i32,
            _forceapproach: i32,
            _ty: i32,
        ) -> bool {
            if self.reject_far && (src_x - dx).abs().max((src_z - dz).abs()) > 1 {
                return false;
            }
            self.walked = Some((dx, dz));
            true
        }

        fn local_route(&self) -> Option<(i32, i32)> {
            self.route
        }

        fn build_base(&self) -> (i32, i32) {
            (0, 0)
        }

        fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
            None
        }

        fn out(&mut self) -> &mut dyn Out {
            &mut self.sink
        }

        fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
            true
        }
    }

    /// Minimal outbound sink: the recording driver never writes packets.
    #[derive(Default)]
    struct Sink;

    impl Out for Sink {
        fn p1_enc(&mut self, _opcode: i32) {}
        fn p1(&mut self, _value: i32) {}
        fn p2(&mut self, _value: i32) {}
        fn p4(&mut self, _value: i32) {}
        fn pjstr(&mut self, _s: &str) {}
    }
}
