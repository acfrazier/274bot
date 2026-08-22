//! Traveller: drives a route through the kernel `Driver` over ticks. The
//! caller supplies the player's current tile and the door-open state each
//! tick; the traveller targets walk legs one hop ahead and works a door
//! leg by `op_loc`, re-opening and walking through on the same tick when
//! the caller reports the door open (so a closing door cannot slam).

use api::interact::{op_loc, walk, Driver};

use crate::arrival::arrived;
use crate::router::{Leg, Route};
use crate::tile::{chebyshev, Tile};

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

/// Drives a [`Route`] toward its destination one hop per tick. The stub
/// world has no world grid: the caller passes the player's tile each tick,
/// and the walk target is picked from the armed route's own tiles.
pub struct Traveller {
    route: Option<Route>,
    dest: Option<Tile>,
    status: NavStatus,
    hop_ticks: u32,
    budget: u32,
    /// The leg currently being worked.
    leg: usize,
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
            leg: 0,
        }
    }

    /// Arm a route, replacing any previous one.
    pub fn arm(&mut self, route: Route) {
        self.dest = Some(route.dest);
        self.route = Some(route);
        self.hop_ticks = 0;
        self.leg = 0;
        self.status = NavStatus::Idle;
    }

    /// Drop the armed route and its destination.
    pub fn clear(&mut self) {
        self.route = None;
        self.dest = None;
        self.hop_ticks = 0;
        self.leg = 0;
    }

    /// The destination currently queued, if any.
    pub fn queued(&self) -> Option<Tile> {
        self.dest
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
        if arrived(here, dest, true) {
            self.status = NavStatus::Arrived;
            let status = self.status;
            self.clear();
            return status;
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
                Leg::Walk { tiles } => tiles.last().is_none_or(|last| *last == here),
                Leg::Door { to, .. } => *to == here,
            };
            if !done {
                break;
            }
            self.leg += 1;
        }

        let Some(leg) = route.legs.get(self.leg) else {
            // Every leg traversed without arriving cannot happen here: the
            // last leg ends on `dest`, and standing on `dest` returned
            // Arrived above. Report the current status defensively.
            return self.status;
        };

        match leg {
            Leg::Walk { tiles } => {
                let last = *tiles.last().expect("walk legs are non-empty");
                // Aim at the leg's far end when it is within 20 tiles;
                // otherwise hop to a tile ~15 steps ahead along the leg so
                // the client re-routes a fresh, short path each tick.
                let target = if chebyshev(here, last) <= 20 {
                    last
                } else {
                    tiles
                        .iter()
                        .copied()
                        .find(|t| chebyshev(here, *t) >= 15)
                        .unwrap_or(last)
                };
                walk(d, target.x, target.z);
                self.status = NavStatus::Walking;
            }
            Leg::Door { loc, loc_id, to, .. } => {
                op_loc(d, loc.x, loc.z, *loc_id);
                // Same-tick slam rule: when the caller reports the door
                // open, re-open and walk through on the same tick so a
                // closing door cannot slam between ticks.
                if door_open {
                    walk(d, to.x, to.z);
                }
                self.status = NavStatus::Door;
            }
        }
        self.status
    }
}

impl Default for Traveller {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use api::interact::Driver;
    use api::prot::Out;
    use client::client::MiniMenuAction;

    use crate::grid::StepGrid;
    use crate::router::find;
    use crate::tile::Tile;
    use crate::traveller::{NavStatus, Traveller};

    #[test]
    fn no_route_ticks_idle() {
        let mut t = Traveller::new();
        let mut r = Rec::default();
        assert_eq!(
            t.tick(&mut r, Tile { x: 0, z: 0, level: 0 }, false),
            NavStatus::Idle
        );
    }

    #[test]
    fn arm_queues_dest_and_clear_drops_it() {
        let mut t = Traveller::new();
        t.arm(
            find(
                &StepGrid::fixture_open_3x3(),
                Tile { x: 0, z: 0, level: 0 },
                Tile { x: 2, z: 2, level: 0 },
            )
            .unwrap(),
        );
        assert_eq!(t.queued(), Some(Tile { x: 2, z: 2, level: 0 }));
        t.clear();
        assert_eq!(t.queued(), None);
    }

    #[test]
    fn walk_leg_sends_walk_toward_dest() {
        let mut t = Traveller::new();
        t.arm(
            find(
                &StepGrid::fixture_open_3x3(),
                Tile { x: 0, z: 0, level: 0 },
                Tile { x: 2, z: 2, level: 0 },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((0, 0)),
            ..Rec::default()
        };
        assert_eq!(
            t.tick(&mut r, Tile { x: 0, z: 0, level: 0 }, false),
            NavStatus::Walking
        );
        assert!(r.walked.is_some());
    }

    #[test]
    fn long_walk_leg_hop_targets_fifteen_ahead() {
        let mut t = Traveller::new();
        t.arm(
            find(
                &StepGrid::fixture_open_1x40(),
                Tile { x: 0, z: 0, level: 0 },
                Tile { x: 39, z: 0, level: 0 },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((0, 0)),
            ..Rec::default()
        };
        t.tick(&mut r, Tile { x: 0, z: 0, level: 0 }, false);
        // Far end is 39 away (> 20): hop to a tile ~15 steps ahead.
        let (x, z) = r.walked.expect("walk sent");
        assert!((10..=20).contains(&x), "hop target x was {x}");
        assert_eq!(z, 0);
    }

    #[test]
    fn arrived_on_dest_clears_and_reports_arrived() {
        let mut t = Traveller::new();
        t.arm(
            find(
                &StepGrid::fixture_open_3x3(),
                Tile { x: 0, z: 0, level: 0 },
                Tile { x: 2, z: 2, level: 0 },
            )
            .unwrap(),
        );
        let mut r = Rec::default();
        assert_eq!(
            t.tick(&mut r, Tile { x: 2, z: 2, level: 0 }, false),
            NavStatus::Arrived
        );
        assert_eq!(t.queued(), None);
    }

    #[test]
    fn door_open_same_tick_op_loc_then_walk() {
        let mut t = Traveller::new();
        t.arm(
            find(
                &StepGrid::fixture_door_corridor(),
                Tile { x: 0, z: 0, level: 0 },
                Tile { x: 4, z: 0, level: 0 },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((1, 0)),
            ..Rec::default()
        };
        // skip to door by standing on from-tile
        assert_eq!(
            t.tick(&mut r, Tile { x: 1, z: 0, level: 0 }, true),
            NavStatus::Door
        );
        assert!(r.locs >= 1);
        assert!(r.walked.is_some());
    }

    #[test]
    fn door_closed_only_op_loc() {
        let mut t = Traveller::new();
        t.arm(
            find(
                &StepGrid::fixture_door_corridor(),
                Tile { x: 0, z: 0, level: 0 },
                Tile { x: 4, z: 0, level: 0 },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((1, 0)),
            ..Rec::default()
        };
        assert_eq!(
            t.tick(&mut r, Tile { x: 1, z: 0, level: 0 }, false),
            NavStatus::Door
        );
        assert!(r.locs >= 1);
        assert!(r.walked.is_none());
    }

    #[test]
    fn budget_exceeded_reports_budget_and_clears() {
        let mut t = Traveller::new();
        t.arm(
            find(
                &StepGrid::fixture_open_3x3(),
                Tile { x: 0, z: 0, level: 0 },
                Tile { x: 2, z: 2, level: 0 },
            )
            .unwrap(),
        );
        let mut r = Rec {
            route: Some((0, 0)),
            ..Rec::default()
        };
        let mut status = NavStatus::Walking;
        for _ in 0..61 {
            status = t.tick(&mut r, Tile { x: 0, z: 0, level: 0 }, false);
        }
        assert_eq!(status, NavStatus::Budget);
        assert_eq!(t.queued(), None);
    }

    /// Recording driver: captures the last walk target and counts OP_LOC1
    /// interactions. `route` stands in for the local player tile so
    /// `api::walk` finds a route origin.
    #[derive(Default)]
    struct Rec {
        walked: Option<(i32, i32)>,
        locs: usize,
        route: Option<(i32, i32)>,
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
            _src_x: i32,
            _src_z: i32,
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
            self.walked = Some((dx, dz));
            true
        }

        fn local_route(&self) -> Option<(i32, i32)> {
            self.route
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
