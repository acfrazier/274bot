use super::*;

impl Traveller {
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
}
