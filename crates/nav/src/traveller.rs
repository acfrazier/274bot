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

/// The magic side-tab index (the 2004 icon order: combat 0, stats 1,
/// quests 2, inventory 3, equipment 4, prayer 5, magic 6).
const MAGIC_TAB: usize = 6;

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

/// One standard spellbook teleport: the packed landing tile the edge's
/// `to` identifies, the spellbook button's label word (`Cast @gre@<word>
/// teleport`, the 2004 spellbook text), and the 2004 spellbook component
/// id used when the live tree carries no matching button text.
#[derive(Debug, Clone, Copy)]
struct SpellTeleport {
    dest: &'static str,
    to: WorldTile,
    fallback_com_id: i32,
}

/// The seven spell teleports `derive_transports` packs (from
/// `magic_spells.dbrow` `tele_coord` + runes). A spell edge carries no
/// widget on the wire (`loc_id` 0), so the traveller resolves the button
/// from the landing.
const SPELL_TELEPORTS: &[SpellTeleport] = &[
    SpellTeleport {
        dest: "Varrock",
        to: WorldTile {
            x: 3213,
            z: 3424,
            level: 0,
        },
        fallback_com_id: 1164,
    },
    SpellTeleport {
        dest: "Lumbridge",
        to: WorldTile {
            x: 3221,
            z: 3218,
            level: 0,
        },
        fallback_com_id: 1167,
    },
    SpellTeleport {
        dest: "Falador",
        to: WorldTile {
            x: 2965,
            z: 3378,
            level: 0,
        },
        fallback_com_id: 1170,
    },
    SpellTeleport {
        dest: "Camelot",
        to: WorldTile {
            x: 2757,
            z: 3478,
            level: 0,
        },
        fallback_com_id: 1174,
    },
    SpellTeleport {
        dest: "Ardougne",
        to: WorldTile {
            x: 2661,
            z: 3301,
            level: 0,
        },
        fallback_com_id: 1540,
    },
    SpellTeleport {
        dest: "Watchtower",
        to: WorldTile {
            x: 2933,
            z: 4713,
            level: 2,
        },
        fallback_com_id: 1541,
    },
    SpellTeleport {
        dest: "Trollheim",
        to: WorldTile {
            x: 2890,
            z: 3679,
            level: 0,
        },
        fallback_com_id: 7455,
    },
];

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

/// Distinct game ticks a sent walk may sit without tile progress, with
/// no map flag and no movement, before one same-aim recovery reissue.
/// Counts at the latest observed position (partial-hop progress still
/// recovers once the player stops). Covers the short engine freeze
/// window (Bind ~3 ticks); matches the canonical WalkExecutor stallTicks
/// default of 5. Not a second hop budget.
const WALK_STALL_RECOVER_IDLE_TICKS: u32 = 5;

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

    /// One walk-hop step: send an armed (unsent) hop, or settle the sent
    /// one against the `arrived` arm and the hop budget. A still-watching
    /// settle ends the call; a matched mid-leg hop click-aheads the next
    /// walk on this same poll so the player does not stop on the hop tile.
    fn poll_walk<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
    ) -> Poll {
        let mut hop = self.walk.take().expect("walk hop present");
        let here = here(snapshot);
        if !hop.sent {
            return self.send_walk_hop(d, snapshot, options, hop, here);
        }
        // Last tile of the walk is the dest: exact stand. Mid-hops
        // click-ahead at Chebyshev 2 (at least `close_enough`) so the
        // next walk goes out while the player is still moving.
        const CLICK_AHEAD: i32 = 2;
        let last = hop.aim_index + 1 >= hop.tiles().len();
        let radius = if last {
            0
        } else {
            self.close_enough.max(CLICK_AHEAD)
        };
        let arms = [("arrived", arrived(hop.aim, radius))];
        if crate::debug_enabled() {
            eprintln!(
                "[nav-walk] tick={} here={here:?} aim={:?} radius={radius} ticks_waited={} sent_tile={:?} actor={:?} map_flag={:?}",
                snapshot.tick(), hop.aim, hop.ticks_waited, hop.sent_tile,
                snapshot.local_player().map(|p| (p.player.actor.moving, p.player.actor.running, p.player.actor.in_combat, &p.player.actor.target)),
                snapshot.map_flag()
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
                // Mid-leg click-ahead: send the next hop on this poll.
                hop.cursor = hop.aim_index + 1;
                hop.sent = false;
                self.send_walk_hop(d, snapshot, options, hop, here)
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
                } else if self.should_recover_cancelled_walk(&hop, snapshot, here) {
                    self.recover_cancelled_walk(d, snapshot, options, hop, here)
                } else {
                    self.note_walk_stall_idle(&mut hop, snapshot, here);
                    self.walk = Some(hop);
                    Poll::Watching
                }
            }
        }
    }

    /// True when a sent walk has been cancelled long enough to warrant
    /// one same-aim recovery: no map flag, not moving, recovery not yet
    /// spent, and enough *distinct* game ticks with no tile progress at
    /// the latest observed position (duplicate snapshot polls do not
    /// count). Progress away from the original `sent_tile` still qualifies
    /// once the player stops; `sent_tile` remains only for Dropped/Expired.
    fn should_recover_cancelled_walk(
        &self,
        hop: &WalkHop,
        snapshot: &GameSnapshot,
        here: WorldTile,
    ) -> bool {
        if hop.stall_recovered || hop.sent_tile.is_none() {
            return false;
        }
        if snapshot.map_flag().is_some() {
            return false;
        }
        if snapshot
            .local_player()
            .is_some_and(|p| p.player.actor.moving)
        {
            return false;
        }
        let idle = if hop.stall_idle_at != Some(here) {
            // First observation at this tile after movement (or start).
            1
        } else {
            match hop.stall_idle_last_tick {
                Some(t) if t == snapshot.tick() => hop.stall_idle_ticks,
                _ => hop.stall_idle_ticks.saturating_add(1),
            }
        };
        idle >= WALK_STALL_RECOVER_IDLE_TICKS
    }

    /// Advance or clear the cancelled-walk idle counter. Only distinct
    /// snapshot ticks count; active map flag, actor movement, or an
    /// actual tile change resets the window so a live hop is never
    /// spuriously reissued. Idle is tracked at the latest observed tile
    /// so partial-hop progress can still recover once movement stops.
    fn note_walk_stall_idle(&self, hop: &mut WalkHop, snapshot: &GameSnapshot, here: WorldTile) {
        let active = snapshot.map_flag().is_some()
            || snapshot
                .local_player()
                .is_some_and(|p| p.player.actor.moving);
        if hop.sent_tile.is_none() || active || hop.stall_recovered {
            hop.stall_idle_ticks = 0;
            hop.stall_idle_last_tick = None;
            hop.stall_idle_at = None;
            return;
        }
        if hop.stall_idle_at != Some(here) {
            hop.stall_idle_at = Some(here);
            hop.stall_idle_ticks = 0;
            hop.stall_idle_last_tick = None;
        }
        let tick = snapshot.tick();
        if hop.stall_idle_last_tick == Some(tick) {
            return;
        }
        hop.stall_idle_last_tick = Some(tick);
        hop.stall_idle_ticks = hop.stall_idle_ticks.saturating_add(1);
    }

    /// One same-aim walk reissue after a cancelled hop. Does not reset
    /// `ticks_waited` (hop budget stays finite), does not consume another
    /// `max_hops` slot, and does not re-pick aim. At most one recovery per
    /// stalled hop; further cancellation exhausts the original bound.
    fn recover_cancelled_walk<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions<'_>,
        mut hop: WalkHop,
        here: WorldTile,
    ) -> Poll {
        let aim = hop.aim;
        let mut ix = Interactions::new(snapshot, d);
        let result = ix.walk(aim);
        report_walk(options, snapshot, here, aim, &result);
        match result {
            SendResult::Sent { .. } => {
                hop.sent = true;
                hop.sent_tick = snapshot.tick();
                hop.sent_tile = Some(here);
                hop.tries = hop.tries.max(1) + 1;
                hop.stall_recovered = true;
                hop.stall_idle_ticks = 0;
                hop.stall_idle_last_tick = None;
                hop.stall_idle_at = None;
                // Keep ticks_waited: recovery is not a fresh hop budget.
                self.walk = Some(hop);
                Poll::Watching
            }
            SendResult::Refused {
                reason:
                    SendReason::OffScene | SendReason::Unreachable | SendReason::SceneUnavailable,
                ..
            } => {
                // Spend the recovery slot so we do not re-send every poll
                // while the scene is unavailable; the original tick budget
                // still bounds the hop.
                hop.stall_recovered = true;
                hop.stall_idle_ticks = 0;
                hop.stall_idle_last_tick = None;
                hop.stall_idle_at = None;
                self.walk = Some(hop);
                Poll::Watching
            }
            SendResult::Refused { reason, .. } => {
                fire_leg(options, &hop.leg(), LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Refused { at: here, reason })
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
        essence: &mut Option<EssenceSession>,
    ) -> Poll {
        let mut hop = self.transport.take().expect("transport hop present");
        let here = here(snapshot);
        if let (Some(cb), Leg::Transport { edge }) = (options.on_event.as_mut(), &hop.leg) {
            cb(TravelEvent::TransportState {
                tick: snapshot.tick(),
                at: here,
                kind: edge.kind,
                loc_id: edge.loc_id,
                from: edge.at,
                to: edge.to,
                open: edge_loc_open(snapshot, edge),
                approach: hop.approach.is_some(),
                troll: hop.troll,
                waited: hop.ticks_waited,
                loc_wait: self.loc_wait,
                budget: self.budget,
                live_loc: find_transport_loc(snapshot, edge).map(|l| (l.id, l.tile)),
            });
        }
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
                    let result = ix.walk(hop.to);
                    report_walk(options, snapshot, here, hop.to, &result);
                    match result {
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
                } else if edge.kind == TransportKind::Door
                    && hop
                        .open_sent_tick
                        .is_some_and(|sent| snapshot.tick() != sent)
                    && door_step_pending(edge, here)
                    && SceneQuery::new(snapshot.scene(), None).can_step(here, edge.to)
                {
                    // The Open already carried the player through the
                    // door's wall onto `at` (Tenzing's 3745
                    // `open_and_close_door2` teleports the entering player
                    // onto the loc tile, whose wall is on the far edge, and
                    // swaps in an inviswall for 3 ticks, so the door never
                    // reads open): take the clear step to `to` now instead
                    // of sitting out the cheap budget. A step still behind
                    // the wall is left to the open-door walk above — a walk
                    // packet there would cancel the queued Open.
                    hop.open_sent_tick = None;
                    let mut ix = Interactions::new(snapshot, d);
                    let result = ix.pending_door_step(edge.to);
                    report_walk(options, snapshot, here, edge.to, &result);
                    match result {
                        SendResult::Sent { .. } => {
                            if crate::debug_enabled() {
                                eprintln!("[nav-transport] cheap hop door step to {:?}", edge.to);
                            }
                        }
                        SendResult::Refused { reason, .. } => {
                            fire_leg(options, &hop.leg, LegPhase::Failed);
                            return Poll::Terminal(TravelOutcome::Refused { at: here, reason });
                        }
                    }
                    self.transport = Some(hop);
                    return Poll::Watching;
                } else if edge.open_loc_id.is_some()
                    && edge.kind != TransportKind::Door
                    && edge_loc_open(snapshot, edge)
                    && hop.tries == 0
                {
                    return match find_transport_target(snapshot, edge) {
                        Some(target) => {
                            let mut ix = Interactions::new(snapshot, d);
                            match interact_transport(snapshot, &mut ix, target, edge, options) {
                                SendResult::Sent { .. } => {
                                    hop.tries = 1;
                                    hop.ticks_waited = 0;
                                    hop.sent_tile = Some(here);
                                    self.loc_wait = 0;
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
                            self.transport = Some(hop);
                            Poll::Watching
                        }
                    };
                }
            }
        }
        // The pre-interact approach: the player must stand within chebyshev
        // 1 of the loc (or the driver NPC) before the game accepts an
        // interact. While the approach is armed, watch it instead of the
        // arrive arm; only once adjacent does the hop find the target,
        // interact, and settle `arrived(to)`.
        if hop.approach.is_some() {
            match self.poll_approach(d, snapshot, &mut hop, options) {
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
            return match find_transport_target(snapshot, &edge) {
                Some(target) => {
                    let mut ix = Interactions::new(snapshot, d);
                    match interact_transport(snapshot, &mut ix, target, &edge, options) {
                        SendResult::Sent { .. } => {
                            self.loc_wait = 0;
                            hop.ticks_waited = 0;
                            hop.sent_tile = Some(here);
                            if edge.kind == TransportKind::Door {
                                hop.open_sent_tick = Some(snapshot.tick());
                            }
                            if edge.open_loc_id.is_some()
                                && edge.kind != TransportKind::Door
                                && edge_loc_open(snapshot, &edge)
                            {
                                hop.tries = 1;
                            }
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
                    // The target has not appeared in the loaded scene yet:
                    // keep waiting, bounded by the hop budget.
                    self.loc_wait += 1;
                    if self.loc_wait > self.budget {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        Poll::Terminal(TravelOutcome::Blocked {
                            at: here,
                            leg: self.leg_index,
                            detail: format!(
                                "transport {} {} is not within 3 tiles of ({}, {}, {}) in the loaded scene",
                                target_word(&edge),
                                edge.loc_id,
                                edge.at.x,
                                edge.at.z,
                                edge.at.level
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
        // The latch target, read before the arrive-arm builder below moves
        // `edge` into the door closure: a completed essence entry hop
        // records the wizard the player entered through.
        let entry_wizard = is_essence_entry_edge(&edge).then_some(edge.loc_id);
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
        // An Npc hop's first op can open a chat dialog instead of riding
        // immediately (the live cart drivers' `opnpc1` says "Hello!", then
        // asks "Is that Ok?" with a "Yes please…" choice), a jewellery
        // rub opens the destination choice (the glory's "Where would you
        // like to teleport to?" with each location named — the dueling
        // ring's single arena first and "Nowhere." last), and the Shantay
        // henge's gated branch (loc 4031 `oploc1`) shows the pass
        // handover (`~chatnpc`/`~objbox`/`~chatplayer`, each a
        // `p_pausebutton` chat modal) before consuming the pass and
        // teleporting. Drive the dialog the same way: press the modal's
        // continue button while it is up (each press advances a page —
        // including the post-choice "Great!" pages and mesboxes before
        // the ride), and press the ride choice exactly once when the
        // choice page is up, then keep watching `arrived(to)` for the
        // ride. A plain door hop (and the toll gates, whose branch
        // choices differ) never drives chat here.
        if drives_hop_dialogs(&edge) {
            let mut ix = Interactions::new(snapshot, d);
            if snapshot.chat_continue_component_id() != -1 {
                match ix.continue_dialog() {
                    SendResult::Sent { .. } => {
                        if crate::debug_enabled() {
                            eprintln!("[nav-transport] continued the npc {} dialog", edge.loc_id);
                        }
                    }
                    SendResult::Refused { .. } => {}
                }
            } else if !snapshot.chat_options().is_empty() {
                // A jewellery rub's destination choice is the edge's case
                // index: the script maps the answered option through
                // `switch_int($choice)`, so the choice of the edge being
                // followed is the 1-based index of its `to` among the
                // packed same-`loc_id` rub edges (the dueling ring — the
                // only sibling — answers 1). Npc ride dialogs (cart fare,
                // Elkoy escort) always answer the modal's FIRST choice,
                // independent of the NPC op index (`edge.option`: Talk-to
                // is op 1, the essence wizard's teleport op 3/4).
                // Spirit trees are two pages: adult trees ask "Where can I
                // go?" before the dest list; answering the dest index on
                // the first page is "No thanks, old tree." and the hop
                // returns. Answer each distinct option-page once.
                let page = chat_page_key(snapshot);
                if hop.dialog_page.as_deref() != Some(page.as_str()) {
                    let choice = hop_dialog_choice(
                        &hop.leg,
                        options.teleports,
                        options.edges,
                        snapshot.chat_options().len(),
                    );
                    match ix.answer_choice(choice) {
                        SendResult::Sent { .. } => {
                            hop.dialog_page = Some(page);
                            if crate::debug_enabled() {
                                eprintln!(
                                    "[nav-transport] answered choice {} for {} {}",
                                    choice,
                                    match edge.kind {
                                        TransportKind::SpiritTree => "spirit tree",
                                        TransportKind::Teleport => "jewellery",
                                        _ => "npc",
                                    },
                                    edge.loc_id
                                );
                            }
                        }
                        SendResult::Refused { .. } => {}
                    }
                }
            } else if snapshot.modals().main == GLIDER_MAP_ROOT {
                // Gnome glider: after "Can you take me on the glider?" the
                // script `if_openmain(glidermap)` and dests are IF_BUTTON
                // on com_21..=25, not chat. Boat `ship_journey` has no
                // dest buttons — the hop just waits for the telejump.
                if let Some(com) = dest_map_component(&edge) {
                    let page = format!("if:{com}");
                    if hop.dialog_page.as_deref() != Some(page.as_str()) {
                        if let Some(widget) =
                            snapshot.widgets().iter().find(|w| w.component_id == com)
                        {
                            match ix.press(widget) {
                                SendResult::Sent { .. } => {
                                    hop.dialog_page = Some(page);
                                    if crate::debug_enabled() {
                                        eprintln!(
                                            "[nav-transport] pressed glidermap {} for dest ({}, {}, {})",
                                            com, edge.to.x, edge.to.z, edge.to.level
                                        );
                                    }
                                }
                                SendResult::Refused { .. } => {}
                            }
                        }
                    }
                }
            }
        }
        let close_enough = self.close_enough;
        let arrived_arm: Evidence<'static> = if edge.kind == TransportKind::Door
            && edge.dir.is_some()
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
        } else if edge.kind == TransportKind::Door && edge.dir.is_none() {
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                now.world_tile()
                    .is_some_and(|here| door_dir_none_arrived(&edge, here, close_enough))
            })
        } else if is_essence_entry_edge(&edge) {
            // The entry teleport lands at a random `essence_mine_teleports`
            // coord — never the pad exactly — so any tile inside the
            // enclosed mine completes the hop (and latches the session).
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                now.world_tile().is_some_and(in_essence_mine)
            })
        } else if edge.kind == TransportKind::EssenceExit {
            // The exit portal teleports to `map_findsquare(anchor, 0, 2,
            // lineofwalk)`: a random standable tile within chebyshev 2 of
            // the wizard's anchor, never the anchor exactly.
            let to = edge.to;
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                now.world_tile().is_some_and(|t| {
                    t.level == to.level
                        && (t.x - to.x).abs().max((t.z - to.z).abs())
                            <= ESSENCE_MINE_EXIT_ARRIVE_RADIUS
                })
            })
        } else if edge.kind == TransportKind::Teleport {
            // `player_teleport_normal` lands at `map_findsquare(to, 0, 2,
            // lineofwalk)`: a random standable tile within chebyshev 2 of
            // the packed landing, never the tile exactly — so the hop
            // accepts that radius regardless of the runner's exact
            // `close_enough`.
            arrived(edge.to, TELEPORT_ARRIVE_RADIUS)
        } else if edge.kind == TransportKind::Glider {
            arrived(edge.to, GLIDER_ARRIVE_RADIUS)
        } else if (edge.to.z - edge.at.z).abs() == CELLAR_SHIFT && edge.to.level == edge.at.level {
            // `movecoord(coord(), 0, 0, ±6400)` lands on the player's tile,
            // one Chebyshev off the loc-baked dest when the hop is taken
            // from an adjacent stand. Host WalkNear uses close_enough 0.
            arrived(edge.to, CELLAR_ARRIVE_RADIUS.max(close_enough))
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
            Some(Outcome::Matched {
                arm: "unreachable", ..
            }) => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Refused {
                    at: here,
                    reason: SendReason::Unreachable,
                })
            }
            Some(Outcome::Matched { .. }) => {
                // Agility forcemove holds the player after they land on
                // `to`. Completing on the first arrived poll sends the
                // next walk into a locked player (live Yanille ledge
                // Dropped at 2580,9512). Packed `edge.ticks` is the anim
                // delay — stay on this hop until it elapses.
                if let Leg::Transport { edge } = &hop.leg {
                    let delay = edge.ticks.max(0) as u32;
                    if edge.kind == TransportKind::AgilityShortcut && delay > 0 {
                        // Count packed ticks from the first landed poll, not
                        // from the interact — the forcemove can land on `to`
                        // while the player is still locked (live ledge
                        // Dropped the next walk).
                        if hop.sent_tile.is_some() {
                            hop.sent_tile = None;
                            hop.ticks_waited = 0;
                        }
                        hop.ticks_waited += 1;
                        if hop.ticks_waited < delay {
                            self.transport = Some(hop);
                            return Poll::Watching;
                        }
                    }
                }
                // A completed essence entry hop latches the mine session:
                // the exit portal may only return to this wizard.
                if let Some(wizard) = entry_wizard {
                    if let Some(session) = essence_session_for_wizard(wizard) {
                        *essence = Some(session);
                        if crate::debug_enabled() {
                            eprintln!(
                                "[nav-transport] essence entry latched wizard {} -> {:?}",
                                session.wizard_npc, session.return_tile
                            );
                        }
                    }
                }
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
                    // instead of stalling. Re-open while closed, probe the
                    // adjacent crossing after Open, and walk when open; only
                    // a troll hop that lapses again — or a
                    // non-door transport — returns the real `Stalled`.
                    let door_leg = matches!(
                        &hop.leg,
                        Leg::Transport { edge } if edge.kind == TransportKind::Door
                    );
                    if door_leg && !hop.troll {
                        hop.troll = true;
                        hop.ticks_waited = 0;
                        // The troll arms its own probe when it sends Open.
                        hop.open_sent_tick = None;
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

    fn send_walk_hop<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
        mut hop: WalkHop,
        here: WorldTile,
    ) -> Poll {
        // A preceding transport may land directly on this walk's final
        // tile (cellar shifts can land one tile off their packed `to`).
        // Complete only on full endpoint equality, including level: an
        // intermediate/clipped self-aim still has route left to follow.
        if hop.tiles().last().copied() == Some(here) {
            fire_leg(options, &hop.leg(), LegPhase::Done);
            self.leg_index += 1;
            return Poll::LegDone;
        }
        if self.hops >= self.max_hops {
            fire_leg(options, &hop.leg(), LegPhase::Failed);
            return Poll::Terminal(TravelOutcome::GaveUp {
                at: here,
                hops: self.hops,
            });
        }
        let tiles = hop.tiles();
        let here_i = tiles
            .iter()
            .position(|t| *t == here)
            .filter(|&i| i >= hop.cursor)
            .unwrap_or(hop.cursor);
        let (mut aim, mut idx) = pick_aim_in_scene(tiles, here, hop.cursor, snapshot.scene());
        loop {
            if aim == here {
                self.walk = Some(hop);
                return Poll::Watching;
            }
            let mut ix = Interactions::new(snapshot, d);
            let result = ix.walk(aim);
            report_walk(options, snapshot, here, aim, &result);
            match result {
                SendResult::Sent { .. } => {
                    self.hops += 1;
                    hop.aim = aim;
                    hop.aim_index = idx;
                    hop.sent = true;
                    hop.sent_tick = snapshot.tick();
                    hop.ticks_waited = 0;
                    hop.sent_tile = Some(here);
                    hop.stall_idle_ticks = 0;
                    hop.stall_idle_last_tick = None;
                    hop.stall_idle_at = None;
                    hop.stall_recovered = false;
                    self.walk = Some(hop);
                    return Poll::Watching;
                }
                SendResult::Refused {
                    reason: SendReason::OffScene | SendReason::Unreachable,
                    ..
                } if idx > here_i + 1 => {
                    // Far hop left the scene or the local route died:
                    // walk the index back (the v1 traveller retried the
                    // tile after `here`). Click-ahead can outrun rebuild.
                    idx -= 1;
                    aim = tiles[idx];
                }
                SendResult::Refused {
                    reason:
                        SendReason::OffScene | SendReason::Unreachable | SendReason::SceneUnavailable,
                    ..
                } => {
                    // Rebuild (`scene_state != 2`) refuses the walk before
                    // a packet goes out — wait, do not fail the follow.
                    self.walk = Some(hop);
                    return Poll::Watching;
                }
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg(), LegPhase::Failed);
                    return Poll::Terminal(TravelOutcome::Refused { at: here, reason });
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
    /// was armed; a known stun may rearm that walk once before settling.
    fn poll_approach<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        hop: &mut TransportHop,
        options: &mut TravelOptions<'_>,
    ) -> Poll {
        let mut approach = hop.approach.take().expect("approach hop present");
        let here = here(snapshot);
        if approach.retry_pending
            && (here.level != approach.at.level || cheb(here, approach.at) > 1)
        {
            let mut ix = Interactions::new(snapshot, d);
            let result = ix.walk(approach.tile);
            report_walk(options, snapshot, here, approach.tile, &result);
            match result {
                SendResult::Sent { .. } => {
                    approach.retry_pending = false;
                    approach.ticks_waited = 0;
                    approach.sent_tick = snapshot.tick();
                    hop.sent_tile = Some(here);
                    hop.approach = Some(approach);
                    return Poll::Watching;
                }
                SendResult::Refused {
                    reason:
                        SendReason::OffScene | SendReason::Unreachable | SendReason::SceneUnavailable,
                    ..
                } => {}
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Poll::Terminal(TravelOutcome::Refused { at: here, reason });
                }
            }
        }
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
    /// snapshot's locs: Open a closed door, walk through an open door,
    /// and continue to the exit once crossed without reopening behind us. Returns a terminal
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
        // Once on the destination side, a closer behind us must not pull
        // us back. Use the same directional/level evidence as arrival;
        // directionless edges cannot establish crossing from position.
        let crossed =
            edge.dir.is_some() && here.level == edge.to.level && door_crossed(&edge, here);
        if crossed {
            self.loc_wait = 0;
            if cheb(here, hop.to) <= self.close_enough {
                return None; // Let the normal settle arm finish the leg.
            }
            let mut ix = Interactions::new(snapshot, d);
            let result = ix.walk(hop.to);
            report_walk(options, snapshot, here, hop.to, &result);
            return match result {
                SendResult::Sent { .. } => None,
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    Some(TravelOutcome::Refused { at: here, reason })
                }
            };
        }
        if let Some(sent_tick) = hop.open_sent_tick.take() {
            if snapshot.tick() == sent_tick {
                hop.open_sent_tick = Some(sent_tick);
                return None;
            }
            if door_step_pending(&edge, here) {
                let mut ix = Interactions::new(snapshot, d);
                let result = ix.pending_door_step(edge.to);
                report_walk(options, snapshot, here, edge.to, &result);
                return match result {
                    SendResult::Sent { .. } => None,
                    SendResult::Refused { reason, .. } => {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        Some(TravelOutcome::Refused { at: here, reason })
                    }
                };
            }
        }
        // Adjacency is needed to Open a closed door, not to walk through
        // an open one. Re-approaching an open door countermanded the exit
        // walk whenever its destination was several tiles beyond the door.
        if cheb(here, edge.at) > 1 && !edge_loc_open(snapshot, &edge) {
            let Some(approach) = approach_tile(snapshot, edge.at, here) else {
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
            let result = ix.walk(approach);
            report_walk(options, snapshot, here, approach, &result);
            match result {
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
                        || edge.open_loc_id.is_some_and(|open_id| loc.id == open_id))
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
            match interact_transport(snapshot, &mut ix, TransportTarget::Loc(loc), &edge, options) {
                SendResult::Sent { .. } => {
                    hop.open_sent_tick = Some(snapshot.tick());
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
        let result = ix.walk(hop.to);
        report_walk(options, snapshot, here, hop.to, &result);
        match result {
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

fn report_walk(
    options: &mut TravelOptions<'_>,
    snapshot: &GameSnapshot,
    at: WorldTile,
    aim: WorldTile,
    result: &SendResult<'_>,
) {
    if crate::debug_enabled() {
        let refusal = match result {
            SendResult::Sent { .. } => None,
            SendResult::Refused { reason, .. } => Some(*reason),
        };
        eprintln!(
            "[nav-walk-send] tick={} at={at:?} aim={aim:?} sent={} refusal={refusal:?} actor={:?} map_flag={:?}",
            snapshot.tick(), refusal.is_none(),
            snapshot.local_player().map(|p| (p.player.actor.moving, p.player.actor.running, p.player.actor.in_combat, &p.player.actor.target)),
            snapshot.map_flag()
        );
    }
    if let Some(cb) = options.on_event.as_mut() {
        let refusal = match result {
            SendResult::Sent { .. } => None,
            SendResult::Refused { reason, .. } => Some(*reason),
        };
        cb(TravelEvent::WalkAttempt {
            tick: snapshot.tick(),
            at,
            aim,
            refusal,
        });
    }
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

fn tile_in_scene(scene: &api::snapshot::SceneView, t: WorldTile) -> bool {
    if scene.width <= 0 || scene.height <= 0 {
        return true;
    }
    let lx = t.x - scene.base_x;
    let lz = t.z - scene.base_z;
    lx >= 0 && lz >= 0 && lx < scene.width && lz < scene.height
}

/// [`pick_aim`] then walk the index back until the tile is inside the
/// loaded scene, so click-ahead cannot `OffScene` at the 104-edge.
fn pick_aim_in_scene(
    tiles: &[WorldTile],
    here: WorldTile,
    cursor: usize,
    scene: &api::snapshot::SceneView,
) -> (WorldTile, usize) {
    let (aim, mut idx) = pick_aim(tiles, here, cursor);
    if tile_in_scene(scene, aim) {
        return (aim, idx);
    }
    let here_i = tiles
        .iter()
        .position(|t| *t == here)
        .filter(|&i| i >= cursor)
        .unwrap_or(cursor);
    while idx > here_i && !tile_in_scene(scene, tiles[idx]) {
        idx -= 1;
    }
    (tiles[idx], idx)
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
fn approach_tile(snapshot: &GameSnapshot, at: WorldTile, here: WorldTile) -> Option<WorldTile> {
    APPROACH_RING
        .iter()
        .map(|(dx, dz)| WorldTile {
            x: at.x + dx,
            z: at.z + dz,
            level: at.level,
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
            flags
                & (CollisionFlag::WALK_SCENERY | CollisionFlag::WR_GRND | CollisionFlag::SQ_BLOCKED)
                == 0
        })
}

/// Chebyshev distance from `at` to the closest tile of `loc`'s rotated
/// footprint. A 1×1 loc equals origin distance; a length-6 ropeswing
/// whose origin is 4 tiles from `at` still matches when `at` sits on the
/// footprint. Does not widen the 3-tile search radius.
fn loc_chebyshev_to_footprint(loc: &LocView, at: WorldTile) -> i32 {
    let fw = loc.footprint_width.max(1);
    let fl = loc.footprint_length.max(1);
    let min_x = loc.tile.x;
    let max_x = loc.tile.x + fw - 1;
    let min_z = loc.tile.z;
    let max_z = loc.tile.z + fl - 1;
    let dx = if at.x < min_x {
        min_x - at.x
    } else if at.x > max_x {
        at.x - max_x
    } else {
        0
    };
    let dz = if at.z < min_z {
        min_z - at.z
    } else if at.z > max_z {
        at.z - max_z
    } else {
        0
    };
    dx.max(dz)
}

/// The snapshot loc for a transport edge: the edge's closed `loc_id` or
/// `open_loc_id` on the edge's level within 3 tiles of `edge.at` measured
/// to the rotated footprint (the m8aq `gap <= 3`), nearest first.
/// Trapdoors `loc_change` closed→open (1568→1570); matching only the
/// closed id leaves Climb-down unarmed.
fn find_transport_loc<'s>(snapshot: &'s GameSnapshot, edge: &TransportEdge) -> Option<&'s LocView> {
    snapshot
        .locs()
        .iter()
        .filter(|loc| {
            loc.tile.level == edge.at.level
                && (loc.id == edge.loc_id || edge.open_loc_id == Some(loc.id))
        })
        .map(|loc| (loc, loc_chebyshev_to_footprint(loc, edge.at)))
        .filter(|(_, gap)| *gap <= 3)
        .min_by_key(|(_, gap)| *gap)
        .map(|(loc, _)| loc)
}

/// The chat-modal choice an Npc hop answers to ride: the ride is always
/// the modal's FIRST choice (the cart drivers' "Yes please…" fare and
/// Elkoy's escort both present it first). This is the hop's dialog rule,
/// independent of the NPC op index — [`TransportEdge::option`] is the op
/// (Talk-to is op 1, the essence wizard's teleport op 3/4) and stays the
/// interact's operation. Also the fallback choice when a jewellery hop's
/// teleport list is unavailable ([`TravelOptions::teleports`]).
const NPC_RIDE_CHOICE: i32 = 1;

/// The dialog choice a jewellery rub hop answers: the 1-based index of
/// the edge's `to` among the packed same-`loc_id` rub edges — the
/// `switch_int($choice)` case order the bake emitted (the dueling ring's
/// only sibling answers 1). Npc hops (and jewellery hops without a
/// teleport list) fall back to the modal's FIRST choice. Spirit-tree dest
/// pages use the same rule among same-`loc_id`/`at` packed siblings.
fn dest_dialog_choice(
    leg: &Leg,
    teleports: Option<&[TransportEdge]>,
    packed: Option<&[TransportEdge]>,
) -> i32 {
    let Leg::Transport { edge } = leg else {
        return NPC_RIDE_CHOICE;
    };
    let siblings = match edge.kind {
        TransportKind::Teleport if edge.loc_id > 0 => teleports,
        TransportKind::SpiritTree => packed,
        _ => return NPC_RIDE_CHOICE,
    };
    let Some(list) = siblings else {
        return NPC_RIDE_CHOICE;
    };
    list.iter()
        .filter(|e| e.kind == edge.kind && e.loc_id == edge.loc_id)
        .filter(|e| edge.kind != TransportKind::SpiritTree || e.at == edge.at)
        .position(|e| e.to == edge.to)
        .map(|i| i as i32 + 1)
        .unwrap_or(NPC_RIDE_CHOICE)
}

/// Joined chat option texts: a new dest-list page is a different key
/// than the spirit-tree "Where can I go?" gate that opened it.
fn chat_page_key(snapshot: &GameSnapshot) -> String {
    snapshot
        .chat_options()
        .iter()
        .map(|o| o.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The chat choice this hop presses on the current option page.
/// Jewellery dests and Npc fare use [`dest_dialog_choice`]. Adult spirit
/// trees (`spirit_tree.rs2` ent / stronghold_ent) put "No thanks, old
/// tree." first on a 2-option page; dest-index 1 there silently drops
/// the hop. The 2-option page answers 2 ("Where can I go?"); the 3-option
/// dest list then uses packed sibling order. The young tree (loc 1317)
/// is a single "Yes please." / "No thank you." — choice 1 rides.
fn hop_dialog_choice(
    leg: &Leg,
    teleports: Option<&[TransportEdge]>,
    packed: Option<&[TransportEdge]>,
    n_options: usize,
) -> i32 {
    let Leg::Transport { edge } = leg else {
        return NPC_RIDE_CHOICE;
    };
    if edge.kind == TransportKind::SpiritTree {
        return spirit_tree_choice(leg, edge, packed, n_options);
    }
    dest_dialog_choice(leg, teleports, packed)
}

fn spirit_tree_choice(
    leg: &Leg,
    edge: &TransportEdge,
    packed: Option<&[TransportEdge]>,
    n_options: usize,
) -> i32 {
    let n_dests = spirit_tree_dest_count(edge, packed);
    if n_dests == 1 {
        return NPC_RIDE_CHOICE;
    }
    if n_options >= 3 {
        return dest_dialog_choice(leg, None, packed);
    }
    2
}

fn spirit_tree_dest_count(edge: &TransportEdge, packed: Option<&[TransportEdge]>) -> usize {
    let Some(list) = packed else {
        return 0;
    };
    let mut dests: Vec<WorldTile> = Vec::new();
    for e in list {
        if e.kind == TransportKind::SpiritTree && e.loc_id == edge.loc_id && !dests.contains(&e.to)
        {
            dests.push(e.to);
        }
    }
    dests.len()
}

/// `glidermap` (interface.pack 802): dest IF_BUTTON children of
/// `if_openmain(glidermap)` in `gnome_glider.rs2`.
const GLIDER_MAP_ROOT: i32 = 802;

/// Packed glider `to` → `glidermap:com_21`..=`com_25` (interface.pack
/// 824..=828). Pad tiles match `GLIDER_PADS` / `GLIDER_HUB` in transport.rs.
const GLIDER_DEST_BUTTONS: &[(WorldTile, i32)] = &[
    (
        WorldTile {
            x: 2971,
            z: 2969,
            level: 0,
        },
        824,
    ), // gandius — com_21
    (
        WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        },
        825,
    ), // ta_quir_priw — com_22
    (
        WorldTile {
            x: 2850,
            z: 3497,
            level: 0,
        },
        826,
    ), // sindarpos — com_23
    (
        WorldTile {
            x: 3320,
            z: 3430,
            level: 0,
        },
        827,
    ), // lemanto_andra — com_24
    (
        WorldTile {
            x: 3284,
            z: 3211,
            level: 0,
        },
        828,
    ), // kar_hewo — com_25
];

/// The glidermap dest button for a Glider hop's `to`, if that landing is
/// one of the five Gnome Air pads. Other kinds (boat `ship_journey`) have
/// no dest IF_BUTTON.
fn dest_map_component(edge: &TransportEdge) -> Option<i32> {
    if edge.kind != TransportKind::Glider {
        return None;
    }
    GLIDER_DEST_BUTTONS
        .iter()
        .find(|(to, _)| *to == edge.to)
        .map(|(_, id)| *id)
}

/// The outcome of sending a packed teleport hop's op.
enum TeleportSend {
    /// The op was accepted; the hop now settles `arrived(to)`.
    Sent,
    /// The interact/press was refused by the driver.
    Refused(SendReason),
    /// The hop cannot be worked yet (the charged item is not in the
    /// loaded inventory, or the driver dropped the press): keep waiting,
    /// bounded by the hop budget.
    Wait,
    /// The edge can never be executed (a spell landing outside the seven
    /// standard spellbook teleports).
    Blocked(String),
}

/// The magic-tab button of a spell teleport edge: the standard spell the
/// edge's landing names, looked up live by the spellbook button text
/// (`Cast @gre@<dest> teleport`, the 2004 label), else the 2004
/// spellbook component id. `None` when the landing is not one of the
/// seven standard spellbook teleports (a pack row this model cannot
/// execute).
fn spell_button(snapshot: &GameSnapshot, edge: &TransportEdge) -> Option<i32> {
    let spell = SPELL_TELEPORTS.iter().find(|s| s.to == edge.to)?;
    let root = snapshot
        .side_tabs()
        .get(MAGIC_TAB)
        .map(|t| t.root_component_id)
        .unwrap_or(-1);
    let label = format!("Cast @gre@{} teleport", spell.dest);
    if root != -1 {
        let com_id = api::query::widget_search::button_by_text(snapshot, root, &label);
        if com_id != -1 {
            return Some(com_id);
        }
    }
    Some(spell.fallback_com_id)
}

/// The widget view for `com_id` in the snapshot's open roots or side
/// tabs, `None` when no live tree carries it.
fn find_component(snapshot: &GameSnapshot, com_id: i32) -> Option<&WidgetView> {
    snapshot
        .widgets()
        .iter()
        .chain(snapshot.side_tabs().iter().flat_map(|t| t.widgets.iter()))
        .find(|w| w.component_id == com_id)
}

/// Send a packed `TransportKind::Teleport` hop's op: a held-item Rub
/// (`OP_HELD<option>` on the charged jewellery obj the edge names) or the
/// spellbook button of the standard spell the edge's landing names (a
/// gated IF_BUTTON press on the live button, else the unconditional 2004
/// fallback id). Never the WalkTo `::tele` cheat.
fn teleport_send<D: Driver>(
    snapshot: &GameSnapshot,
    d: &mut D,
    edge: &TransportEdge,
) -> TeleportSend {
    if edge.loc_id > 0 {
        let Some(item) = snapshot
            .inventory()
            .iter()
            .find(|it| it.def.id == edge.loc_id)
        else {
            return TeleportSend::Wait;
        };
        let mut ix = Interactions::new(snapshot, d);
        return match ix.interact(OpTarget::Item(item), ActionSpec::Operation(edge.option)) {
            SendResult::Sent { .. } => TeleportSend::Sent,
            SendResult::Refused { reason, .. } => TeleportSend::Refused(reason),
        };
    }
    let Some(com_id) = spell_button(snapshot, edge) else {
        return TeleportSend::Blocked(format!(
            "packed spell teleport to ({}, {}, {}) is not one of the seven standard spellbook teleports",
            edge.to.x, edge.to.z, edge.to.level
        ));
    };
    match find_component(snapshot, com_id) {
        Some(widget) => {
            let mut ix = Interactions::new(snapshot, d);
            match ix.press(widget) {
                SendResult::Sent { .. } => TeleportSend::Sent,
                SendResult::Refused { reason, .. } => TeleportSend::Refused(reason),
            }
        }
        None => {
            if press(d, com_id) {
                TeleportSend::Sent
            } else {
                TeleportSend::Wait
            }
        }
    }
}

/// The live target of a transport hop: the edge's loc view (doors,
/// ladders, stairs, boats, agility, gliders, spirit trees) or — for an
/// NPC-triggered hop (`TransportKind::Npc`: cart, essence-mine wizard,
/// Elkoy) — the edge's NPC view.
enum TransportTarget<'s> {
    Loc(&'s LocView),
    Npc(&'s NpcView),
}

/// The snapshot target for a transport edge: the edge's `loc_id` within
/// 3 tiles of `edge.at` for loc edges ([`find_transport_loc`]), or the
/// edge's npc type id within [`NPC_SEARCH_RADIUS`] of `edge.at` for
/// [`TransportKind::Npc`] edges ([`NPC_SEARCH_RADIUS`], nearest first).
/// An Npc edge's `loc_id` is the npc.pack type id, matched against the
/// live NPC's `r#type`.
fn find_transport_target<'s>(
    snapshot: &'s GameSnapshot,
    edge: &TransportEdge,
) -> Option<TransportTarget<'s>> {
    if npc_backed(edge) {
        snapshot
            .npcs()
            .iter()
            .filter(|npc| {
                npc.r#type == Some(edge.loc_id as usize) && npc.tile.level == edge.at.level
            })
            .map(|npc| (npc, cheb(npc.tile, edge.at)))
            .filter(|(_, gap)| *gap <= NPC_SEARCH_RADIUS)
            .min_by_key(|(_, gap)| *gap)
            .map(|(npc, _)| TransportTarget::Npc(npc))
    } else {
        find_transport_loc(snapshot, edge).map(TransportTarget::Loc)
    }
}

/// Send the transport hop's interact for the edge's target kind: an
/// `op_loc` for loc edges, an `op_npc` for `TransportKind::Npc` edges,
/// both with the edge's `option`. `option` 0 on a loc hop uses the first
/// `item_req` obj on the loc (`oplocu` — unequippable knife on a web).
fn interact_transport<'t>(
    snapshot: &'t GameSnapshot,
    ix: &mut Interactions<'t>,
    target: TransportTarget<'t>,
    edge: &TransportEdge,
    options: &mut TravelOptions<'_>,
) -> SendResult<'t> {
    let (actual_id, tile) = match &target {
        TransportTarget::Loc(l) => (l.id, l.tile),
        TransportTarget::Npc(n) => (n.r#type.map(|id| id as i32).unwrap_or(-1), n.tile),
    };
    let result = match target {
        TransportTarget::Loc(loc) if uses_held_on_loc(edge) => {
            let id = edge.item_req[0].0;
            match snapshot.inventory().iter().find(|it| it.def.id == id) {
                Some(item) => ix.use_item_on(item, OpTarget::Loc(loc)),
                None => SendResult::Refused {
                    tick: snapshot.tick() as u64,
                    reason: SendReason::StaleTarget,
                },
            }
        }
        TransportTarget::Loc(loc) => {
            ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(edge.option))
        }
        TransportTarget::Npc(npc) => {
            ix.interact(OpTarget::Npc(npc), ActionSpec::Operation(edge.option))
        }
    };
    if let Some(cb) = options.on_event.as_mut() {
        let refusal = match &result {
            SendResult::Sent { .. } => None,
            SendResult::Refused { reason, .. } => Some(*reason),
        };
        cb(TravelEvent::TransportAttempt {
            tick: snapshot.tick(),
            kind: edge.kind,
            expected_id: edge.loc_id,
            actual_id,
            target: tile,
            option: edge.option,
            refusal,
        });
    }
    result
}

/// Knife-on-web: `option` 0 + an `item_req` means `oplocu`, not `oploc1`.
fn uses_held_on_loc(edge: &TransportEdge) -> bool {
    edge.option == 0 && !edge.item_req.is_empty()
}

/// The block message's target word for an edge: "npc" for
/// [`TransportKind::Npc`] edges, "loc" for every loc-targeted kind.
fn target_word(edge: &TransportEdge) -> &'static str {
    if npc_backed(edge) {
        "npc"
    } else {
        "loc"
    }
}

/// Dock sailors / cart drivers: `loc_id` is the npc.pack type. Boat was
/// packed as `TransportKind::Boat` with that id; looking it up as a loc
/// (Port Sarim seaman 378) is why live Follow died on the pier.
fn npc_backed(edge: &TransportEdge) -> bool {
    matches!(
        edge.kind,
        TransportKind::Npc | TransportKind::Boat | TransportKind::Glider
    )
}

/// Closed or open leaf within chebyshev 3 of `edge.at` (live Catherby
/// open 1531 sits a tile off the derived `at`).
fn find_door_loc<'s>(snapshot: &'s GameSnapshot, edge: &TransportEdge) -> Option<&'s LocView> {
    find_transport_loc(snapshot, edge)
}

/// Whether a transport hop drives the script's chat dialogs itself: an
/// Npc hop's `opnpc1` opens the ride's chat (the cart fare, Elkoy's
/// escort), a jewellery rub opens its destination choice, a spirit tree's
/// `oploc1` opens the dest dialog (`spirit_tree.rs2`: "Where can I go?"
/// then the sibling list, or the young tree's "Yes please."), and the
/// Shantay henge's gated branch (loc 4031 `oploc1` in `shantay_pass.rs2`)
/// shows the pass handover (`~chatnpc`/`~objbox`/`~chatplayer`, each a
/// `p_pausebutton` chat modal) before consuming the pass and teleporting.
/// A plain door hop never opens chat, and the toll gates' branch choices
/// differ (their follow is not driven here).
fn drives_hop_dialogs(edge: &TransportEdge) -> bool {
    npc_backed(edge)
        || edge.kind == TransportKind::SpiritTree
        || (edge.kind == TransportKind::Teleport && edge.loc_id > 0)
        || (edge.kind == TransportKind::Door && edge.loc_id == SHANTAY_HENGE_LOC_ID)
}

/// Whether the live loc family already reads **open**. Packed closed/open
/// ids are resolved by [`find_door_loc`] within chebyshev 3 of `at` (the
/// Catherby open leaf at (2816,3439) while `at` is (2816,3438)). When
/// that misses, an unpacked swing door with no `open_loc_id` may still
/// read open if the closed id is gone and a loc **on `edge.at`** offers
/// Close — not a nearby unrelated Close loc (sealed `dir=None` stand
/// hops such as ranging 2514 sit a tile off the door loc).
fn edge_loc_open(snapshot: &GameSnapshot, edge: &TransportEdge) -> bool {
    if let Some(loc) = find_door_loc(snapshot, edge) {
        return loc.id != edge.loc_id;
    }
    snapshot.locs().iter().any(|loc| {
        loc.tile == edge.at
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

/// Whether an Open left the player on the door's own tile `at` with `to`
/// one cardinal step away on the crossing side: the post-Open step that
/// finishes the crossing. `open_and_close_door2` doors (Tenzing's 3745)
/// teleport the entering player onto `at` and swap in an inviswall for
/// three ticks, so the loc never reads open; ordinary doors reach the same
/// state when the player opens from `at`.
fn door_step_pending(edge: &TransportEdge, here: WorldTile) -> bool {
    here == edge.at
        && edge.to.level == here.level
        && here.x.abs_diff(edge.to.x) + here.z.abs_diff(edge.to.z) == 1
        && edge.dir.is_some()
        && door_crossed(edge, edge.to)
}

/// Door hops without a cardinal `dir` normally settle with
/// `arrived(to, close_enough)`. When the origin stand sits inside that
/// radius (`Cheb(at, to) <= close_enough` on the same level), the
/// tolerance is geometrically invalid: standing on `at` — including after
/// a script `~forcemove` and before `p_teleport` — looks like arrival.
/// Those short hops require the exact landing. Far dir=None doors
/// (Zanaris, levers, Shantay south) keep the runner's radius. Cardinal
/// `dir=Some` doors stay on [`door_crossed`].
fn door_dir_none_arrived(edge: &TransportEdge, here: WorldTile, close_enough: i32) -> bool {
    if here.level != edge.to.level {
        return false;
    }
    let to_gap = (here.x - edge.to.x).abs().max((here.z - edge.to.z).abs());
    let hop_span = if edge.at.level == edge.to.level {
        (edge.at.x - edge.to.x)
            .abs()
            .max((edge.at.z - edge.to.z).abs())
    } else {
        i32::MAX
    };
    if hop_span <= close_enough {
        here == edge.to
    } else {
        to_gap <= close_enough
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
#[path = "traveller_tests.rs"]
mod tests;
