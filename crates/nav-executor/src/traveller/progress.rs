//! Progress of a follow, for the owner's watchdog.
//!
//! The follower ends its own follow when any of its clocks passes its bound.
//! Every clock counts observed ticks from its start, so a follow that starts
//! no new clock ends within the longest clock in force plus one observed
//! tick. A native walk owner reads each start of a clock as progress
//! ([`super::Traveller::take_clock_progress`]) and ends the walk when no
//! progress comes within that time: its deadline is the tick of the last
//! progress plus the longest clock in force plus one.
//!
//! Clocks, with their bounds (`budget` is `TravelOptions::budget_ticks_per_hop`,
//! 60 by default, `traveller.rs:548`):
//! - a sent walk hop: its moving budget [`moving_budget_ticks`]
//!   (`walk.rs:17-26,242-243`), at least `budget`, longer for a hop that plans
//!   more steps (`walk.rs:575-577`);
//! - an unsent walk hop: `budget` (`walk.rs:513-525`);
//! - a transport leg's wait for its loc or npc: `budget` polls
//!   (`traveller.rs:1313,1391,1458,1640`, `transport_hop.rs:991,1136,1938,1969`);
//! - an approach walk: `budget` (`transport_hop.rs:1794`);
//! - the hop clock after an interaction, which holds door recovery, occupied
//!   retries, the landing delay and gate dialogues: `budget`
//!   (`transport_hop.rs:1563`);
//! - an npc leg's one leg clock: `budget` (`transport_hop.rs:1559-1561`);
//! - the scene-settle wait: `SCENE_SETTLE_TICKS` (`traveller.rs:72`);
//! - the thieving stun window: `THIEVING_STUN_WINDOW` (`traveller.rs:857`).
//!
//! A start of any of them calls [`clock_started`]: every [`TickCount::sent_at`]
//! (the hop, approach, unsent and npc clocks), a loc wait restart, the stun
//! window and the settle wait. A new follow, leg, hop or transport is progress
//! too ([`FollowRun::progress_marks`]).
//!
//! Limitation: a follower that keeps starting clocks without moving on (a
//! route that oscillates, a route-safety hold that restarts its clock on each
//! poll, `walk.rs:503-511`) posts progress for as long as it does, so it
//! suspends the owner's deadline. Its own liveness is the follower's.

use super::*;
use std::cell::Cell;

thread_local! {
    /// Clock starts on this thread. A follow runs on one thread within one
    /// call, so the change across a call is the starts of that follow.
    static CLOCK_STARTS: Cell<u32> = const { Cell::new(0) };
}

/// A clock of the follower starts now.
pub(super) fn clock_started() {
    CLOCK_STARTS.with(|starts| starts.set(starts.get().wrapping_add(1)));
}

/// The clock starts on this thread so far.
pub(super) fn clock_starts() -> u32 {
    CLOCK_STARTS.with(Cell::get)
}

impl FollowRun {
    /// What a move to a new leg, hop or transport changes.
    pub(super) fn progress_marks(&self) -> (usize, u32, bool, bool) {
        (
            self.leg_index,
            self.hops,
            self.walk.is_some(),
            self.transport.is_some(),
        )
    }

    /// The longest bound among the clocks that may be running: `budget`
    /// (every transport phase, the unsent hop), the moving budget of the
    /// current hop, and the stun window.
    pub(super) fn longest_clock_ticks(&self) -> u32 {
        let moving = self
            .walk
            .as_ref()
            .filter(|hop| hop.sent)
            .map_or(0, |hop| moving_budget_ticks(hop.budget_ticks));
        self.budget.max(moving).max(THIEVING_STUN_WINDOW)
    }

    /// A loc or npc wait starts over.
    pub(super) fn restart_loc_wait(&mut self) {
        self.loc_wait = 0;
        clock_started();
    }
}
