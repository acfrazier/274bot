//! Traveller: drives a route through the kernel `Driver` over ticks. The
//! caller supplies the player's current tile and the door-open state each
//! tick; the traveller targets walk legs one hop ahead and works a door
//! leg by `op_loc` while closed. When the caller reports the door open,
//! walk through the same tick without OP_LOC1 (that would Close).
//!
//! The high-level [`Traveller::follow`] is a new pollable layer: it drives
//! a [`crate::router::Route`] leg-by-leg through `api::interact::Interactions`
//! + `api::settle::Settle` (walk legs via `walk` + an `arrived` arm,
//! transport legs via `interact` on the transport loc + `arrived`/level
//! arms), advancing one step per call. The host calls it every tick and
//! gets `None` while the route is still being followed and `Some(outcome)`
//! when it terminates.

use std::collections::VecDeque;

use api::interact::{op_loc, walk, ActionSpec, Driver, Interactions, OpTarget, SendReason, SendResult};
use api::settle::{arrived, Evidence, Outcome, Settle, SettleOptions};
use api::snapshot::{GameSnapshot, LocView, ReadContext, WorldTile};

use crate::arrival::arrived as grid_arrived;
use crate::router::{GridLeg, GridRoute, Leg, Route};
use crate::tile::{chebyshev, Tile};
use crate::transport::TransportEdge;

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
    /// plus one settle poll.
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
    /// its first hop). At most one driver send per call.
    fn step<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions<'_>,
    ) -> Option<TravelOutcome> {
        loop {
            if self.walk.is_some() {
                if let Some(outcome) = self.poll_walk(d, snapshot, options) {
                    return Some(outcome);
                }
                continue;
            }
            if self.transport.is_some() {
                if let Some(outcome) = self.poll_transport(snapshot, options) {
                    return Some(outcome);
                }
                continue;
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
                    match find_transport_loc(snapshot, edge) {
                        Some(loc) => {
                            let to = edge.to;
                            let to_level = edge.to.level;
                            let level_changing = edge.from.level != edge.to.level;
                            let mut ix = Interactions::new(snapshot, d);
                            match ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(edge.option)) {
                                SendResult::Sent { .. } => {
                                    self.loc_wait = 0;
                                    self.transport = Some(TransportHop {
                                        leg,
                                        to,
                                        to_level,
                                        level_changing,
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
                                        edge.loc_id, edge.from.x, edge.from.z, edge.from.level
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

    /// One walk-hop settle step: match the `arrived` arm, or lapse the
    /// hop budget. On arrival the next hop starts in the same call (still
    /// only one walk send).
    fn poll_walk<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
    ) -> Option<TravelOutcome> {
        let mut hop = self.walk.take().expect("walk hop present");
        let here = here(snapshot);
        let arms = [("arrived", arrived(hop.aim, self.close_enough))];
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
                    return None;
                }
                if self.hops >= self.max_hops {
                    fire_leg(options, &hop.leg(), LegPhase::Failed);
                    return Some(TravelOutcome::GaveUp {
                        at: here,
                        hops: self.hops,
                    });
                }
                self.hops += 1;
                hop.cursor = hop.aim_index + 1;
                let (aim, aim_index) = pick_aim(hop.tiles(), here, hop.cursor);
                let mut ix = Interactions::new(snapshot, d);
                match ix.walk(aim) {
                    SendResult::Sent { .. } => {
                        hop.aim = aim;
                        hop.aim_index = aim_index;
                        hop.ticks_waited = 0;
                        hop.sent_tile = Some(here);
                        self.walk = Some(hop);
                        None
                    }
                    SendResult::Refused { reason, .. } => {
                        fire_leg(options, &hop.leg(), LegPhase::Failed);
                        Some(TravelOutcome::Refused { at: here, reason })
                    }
                }
            }
            // A disconnect ends the watch; the hop was effectively dropped.
            Some(Outcome::Expired { .. }) => {
                fire_leg(options, &hop.leg(), LegPhase::Failed);
                Some(TravelOutcome::Stalled {
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
                None
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
                    Some(TravelOutcome::Stalled {
                        at: here,
                        aiming: hop.aim,
                        why,
                        tries: hop.tries.max(1),
                    })
                } else {
                    self.walk = Some(hop);
                    None
                }
            }
        }
    }

    /// One transport-hop settle step: match `arrived(edge.to)` (plus the
    /// level-change arm for a level-changing edge), or lapse the budget.
    fn poll_transport(
        &mut self,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
    ) -> Option<TravelOutcome> {
        let mut hop = self.transport.take().expect("transport hop present");
        let here = here(snapshot);
        let arms = [
            ("arrived", arrived(hop.to, self.close_enough)),
            ("level", crossed_to(hop.to_level, hop.level_changing)),
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
            Some(Outcome::Matched { .. }) => {
                fire_leg(options, &hop.leg, LegPhase::Done);
                self.leg_index += 1;
                None
            }
            Some(Outcome::Expired { .. }) => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Some(TravelOutcome::Stalled {
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
                None
            }
            None => {
                hop.ticks_waited += 1;
                if hop.ticks_waited > self.budget {
                    let why = if hop.sent_tile == Some(here) {
                        HopFailure::Dropped
                    } else {
                        HopFailure::Expired
                    };
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    Some(TravelOutcome::Stalled {
                        at: here,
                        aiming: hop.to,
                        why,
                        tries: hop.tries.max(1),
                    })
                } else {
                    self.transport = Some(hop);
                    None
                }
            }
        }
    }
}

/// One walk-leg hop: the leg's tiles, the aim tile, and the stall clock.
struct WalkHop {
    leg: Leg,
    /// Index into the leg tiles of the last cleared tile.
    cursor: usize,
    aim: WorldTile,
    aim_index: usize,
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

/// One transport-leg hop: the edge (for the phase callback and the arrival
/// arms) plus the stall clock.
struct TransportHop {
    leg: Leg,
    to: WorldTile,
    to_level: i32,
    /// The edge changes level: the `level` arm is live.
    level_changing: bool,
    ticks_waited: u32,
    sent_tile: Option<WorldTile>,
    tries: u32,
}

/// The player's world tile from the snapshot; `(0, 0, 0)` before the first
/// `PLAYER_INFO` (the m8aq `here()` fallback).
fn here(snapshot: &GameSnapshot) -> WorldTile {
    snapshot
        .local_player()
        .map(|lp| lp.player.actor.tile)
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

/// The snapshot loc for a transport edge: the edge's `loc_id` on the
/// edge's level within 3 tiles of `edge.from` (the m8aq `gap <= 3`),
/// nearest first.
fn find_transport_loc<'s>(snapshot: &'s GameSnapshot, edge: &TransportEdge) -> Option<&'s LocView> {
    snapshot
        .locs()
        .iter()
        .filter(|loc| loc.id == edge.loc_id && loc.tile.level == edge.from.level)
        .map(|loc| (loc, cheb(loc.tile, edge.from)))
        .filter(|(_, gap)| *gap <= 3)
        .min_by_key(|(_, gap)| *gap)
        .map(|(loc, _)| loc)
}

/// The level-change settle arm: fires once the local player stands on
/// `level` — only meaningful for a level-changing edge (a same-level
/// transport relies on the `arrived` arm alone).
fn crossed_to(level: i32, changing: bool) -> Evidence<'static> {
    Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
        if !changing {
            return false;
        }
        now.local_player()
            .is_some_and(|lp| lp.player.actor.tile.level == level)
    })
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
    use api::snapshot::{GameSnapshot, ReadContext, WorldTile};
    use client::client::{Client, ClientConfig, ClientPlayer, MiniMenuAction};
    use client::config::LocType;
    use client::io::{ClientStream, ServerProt};
    use std::sync::Arc;

    use crate::grid::StepGrid;
    use crate::router::{find_on_grid, Leg, Route};
    use crate::tile::Tile;
    use crate::transport::{TransportEdge, TransportKind};
    use crate::traveller::{
        crossed_to, HopFailure, LegPhase, NavStatus, TravelOptions, TravelOutcome, Traveller,
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

    /// A ladder edge standing at (3202, 3204) → (3202, 3205).
    fn ladder_edge() -> TransportEdge {
        TransportEdge {
            kind: TransportKind::Ladder,
            from: WorldTile {
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
            ticks: 0,
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
            ticks: 0,
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
            ticks: 2,
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
            ticks: 0,
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
            ticks: 0,
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
            ticks: 0,
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
            ticks: 0,
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
            ticks: 0,
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
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
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
            ticks: 2,
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
                walk_leg(&[(3200, 3200), (3200, 3201), (3200, 3202)]),
                Leg::Transport {
                    edge: ladder_edge(),
                },
            ],
            dest: WorldTile {
                x: 3202,
                z: 3205,
                level: 0,
            },
            ticks: 2,
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
                1 => plant_player(c, 0, 2), // reach the walk leg's end
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

    #[test]
    fn level_change_arm_only_fires_for_a_changing_edge() {
        let mut c = scene_client();
        let snap = snap_at(&mut c, 0, 0);
        let ctx = ReadContext::new(&snap);
        // The fixture player is on level 0: the arm fires only when the
        // edge actually changes level and the target level matches.
        assert!(crossed_to(0, true)(&ctx, &ctx));
        assert!(!crossed_to(1, true)(&ctx, &ctx));
        assert!(!crossed_to(0, false)(&ctx, &ctx));
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
