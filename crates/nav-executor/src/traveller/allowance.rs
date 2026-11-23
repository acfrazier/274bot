//! The observed ticks that a follow of one planned route may take. A native
//! walk owner reads it as its backstop: the walk ends only after the route
//! follower has used every allowance it grants itself for this route, so the
//! follower's own terminal outcome comes first.
//!
//! Each term is an allowance that the follower enforces, with its source,
//! except the ones marked as policy. All counts are observed game ticks.
//!
//! **Walk hops.** A hop's clock lapses when it has waited more than its
//! moving budget `b + ceil(b / 4)`, `b = max(budget, planned steps)`
//! (`walk.rs:228-235,554-557`). A hop that follows the route
//! plans one `pick_aim` stride of 15 steps, 20 at the end of a leg
//! (`walk.rs:84-102`). The poll that sees the lapse ends the follow, so a
//! hop takes at most `moving + 1` ticks. A leg of `n` tiles needs one hop
//! when `n` is at most 20 and one more for each 15 beyond that: a hop that
//! completes mid-leg sends the next one from the next tile
//! (`walk.rs:204`). Every send is one hop, and the follow ends with
//! `GaveUp` when `max_hops` sends are done (`walk.rs:506-512`), so the hops
//! of a whole route are `min(sum over the legs, max_hops)`. A stalled-walk
//! recovery re-sends inside the same clock (`walk.rs:360-380`).
//!
//! **Walk hops waiting unsent.** A hop that the client refuses (the scene is
//! not ready) waits unsent under the hop budget: the follow ends `Stalled`
//! when the clock that restarts at each new hop reaches `budget`
//! (`walk.rs:513-525`, commit `0b19f36ec`). That is one more clock of
//! `budget + 1` before each send, and before the first send of each walk leg.
//!
//! **Limitation: route-safety holds.** While the owner's `route_index_limit`
//! holds the walk at its limit, the unsent clock restarts on every poll
//! (`walk.rs:503-511`): the hold is intentional and lasts as long as its
//! owner keeps it. It is not bounded here, so it suspends the deadline.
//!
//! **Limitation: off-route growth.** A hop's budget is
//! `max(budget, planned steps)` (`walk.rs:575-577`) with no cap other than
//! the leg's length. A player who is off the route when a hop is planned
//! gets a longer budget, up to the leg's length, which this allowance does
//! not add: it assumes the player follows the route.
//!
//! **Transport legs.** A leg runs at most three phases in sequence, each
//! under its own enforced clock: the wait for its loc or npc to appear
//! (`loc_wait > budget`, `traveller.rs:1314,1392,1459,1641`,
//! `transport_hop.rs:992,1137,1939,1970`; reset by every send), the approach
//! walk (`approach.ticks_waited > budget`, `transport_hop.rs:1794`) and the
//! hop clock after the interaction (`hop.ticks_waited > budget`,
//! `transport_hop.rs:1563`; door recovery, occupied-obstacle retries, the
//! landing delay and gate dialogues run inside it,
//! `transport_hop.rs:1516,1661-1690`). Each lapses on the poll after `budget`
//! ticks, so a phase takes at most `budget + 1`. The clocks restart once per
//! attempt. An Npc, Boat or Glider leg has `NPC_HOP_ATTEMPTS` attempts
//! (`npc_hop.rs:3,302`). Any other leg has one.
//!
//! **Slashable web (policy from content).** Every failed cut arms one more
//! cut with a fresh hop clock and a fresh wait for the web
//! (`transport_hop.rs:985-1027`), and nothing in the follower caps their
//! count. A cut fails with probability 1/2 (`content/scripts/general_use/scripts/web.rs2:28-33`,
//! `random(2) = 1`), so [`WEB_CUT_RETRIES`] = 16 retries are all needed with
//! probability 2^-16, about 1.5e-5. The allowance grants the first attempt's
//! three clocks and two clocks (wait and hop) for each retry.
//!
//! **Once per follow.** A thieving stun defers the follow by
//! `THIEVING_STUN_WINDOW` ticks once and sends the current approach and walk
//! hop again (`traveller.rs:1117-1157`): one window, one approach clock and
//! one hop clock. The scene-settle wait after a refused first send is
//! `SCENE_SETTLE_TRIES` tries of `SCENE_SETTLE_TICKS` ticks
//! (`traveller.rs:70-72`).
//!
//! **Delivery.** The follower's outcome reaches its owner on the next
//! observation: [`RECEIPT_GRACE_TICKS`].

use super::*;

/// Observations between the follower's terminal outcome and the owner's
/// next read of it.
pub const RECEIPT_GRACE_TICKS: u64 = 2;

/// Route steps one hop plans at most while it follows the route: the
/// `pick_aim` stride is 15 and its end cutoff 20 (`walk.rs:84-102`).
const HOP_MAX_STEPS: u32 = 20;
/// Route steps `pick_aim` advances per hop (`walk.rs:98-101`).
const HOP_STRIDE: u32 = 15;
/// Clocks of one transport attempt: loc or npc wait, approach, hop.
const ATTEMPT_CLOCKS: u32 = 3;
/// Clocks of one web-cut retry: the wait for the web and the hop.
const RETRY_CLOCKS: u32 = 2;
/// Failed web cuts the allowance covers. A cut fails with probability 1/2
/// (`content/scripts/general_use/scripts/web.rs2:28-33`), so this many failures in a row have probability
/// 2^-16.
pub(super) const WEB_CUT_RETRIES: u32 = 16;

/// What the allowance needs of one route leg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LegShape {
    /// A walk leg of this many tiles, the starting tile included.
    Walk { tiles: usize },
    /// A transport leg that runs this many phase clocks in all.
    Transport { clocks: u32 },
}

/// The phase clocks that one transport leg runs before it ends.
pub(super) fn transport_clocks(edge: &TransportEdge) -> u32 {
    if is_npc_backed_kind(edge.kind) {
        NPC_HOP_ATTEMPTS * ATTEMPT_CLOCKS
    } else if is_slashable_web(edge) {
        ATTEMPT_CLOCKS + WEB_CUT_RETRIES * RETRY_CLOCKS
    } else {
        ATTEMPT_CLOCKS
    }
}

/// The hops a walk leg of `tiles` tiles needs when it follows the route.
/// `pick_aim` counts the starting tile when the player is off the route
/// (`walk.rs:92-96`), so count tiles, not steps.
fn leg_hops(tiles: usize) -> u64 {
    let tiles = u32::try_from(tiles).unwrap_or(u32::MAX);
    if tiles <= HOP_MAX_STEPS {
        1
    } else {
        1 + u64::from((tiles - HOP_MAX_STEPS).div_ceil(HOP_STRIDE))
    }
}

/// The most ticks one walk hop takes: its moving budget, plus the poll that
/// finds it lapsed.
pub(super) fn hop_ticks(budget: u32) -> u64 {
    let budget = budget.max(HOP_MAX_STEPS);
    u64::from(budget) + u64::from(budget.div_ceil(WALK_MOVEMENT_MARGIN_DIVISOR)) + 1
}

/// The observed ticks that following `legs` may take under `budget` ticks
/// per hop and `max_hops` hops.
pub(super) fn route_allowance_ticks(
    legs: impl IntoIterator<Item = LegShape>,
    budget: u32,
    max_hops: u32,
) -> u64 {
    let clock = u64::from(budget) + 1;
    let mut walk_hops = 0_u64;
    let mut walk_legs = 0_u64;
    let mut transport = 0_u64;
    for leg in legs {
        match leg {
            // A one-tile walk leg sends nothing (`traveller.rs:1213`).
            LegShape::Walk { tiles } if tiles <= 1 => {}
            LegShape::Walk { tiles } => {
                walk_hops = walk_hops.saturating_add(leg_hops(tiles));
                walk_legs += 1;
            }
            LegShape::Transport { clocks } => {
                transport = transport.saturating_add(u64::from(clocks) * clock);
            }
        }
    }
    let hop = hop_ticks(budget);
    // Each send waits unsent first, under one more clock, and so does the
    // first send of each walk leg.
    let sends = walk_hops.min(u64::from(max_hops));
    let walk = sends
        .saturating_mul(hop)
        .saturating_add(sends.saturating_add(walk_legs).saturating_mul(clock));
    let once = u64::from(SCENE_SETTLE_TRIES * SCENE_SETTLE_TICKS)
        + u64::from(THIEVING_STUN_WINDOW)
        + clock
        + hop;
    walk.saturating_add(transport)
        .saturating_add(once)
        .saturating_add(RECEIPT_GRACE_TICKS)
}

/// The shape of every leg of a run that starts now.
pub(super) fn leg_shapes(legs: &RouteLegSource) -> Vec<LegShape> {
    let shape = |leg: &Leg| match leg {
        Leg::Walk { tiles } => LegShape::Walk { tiles: tiles.len() },
        Leg::Transport { edge } => LegShape::Transport {
            clocks: transport_clocks(edge),
        },
    };
    match legs {
        RouteLegSource::Legacy(legs) => legs.iter().map(shape).collect(),
        RouteLegSource::Plan {
            plan, leg_count, ..
        } => (0..*leg_count)
            .filter_map(|index| match plan_retained_leg(plan, index)? {
                nav_planner::router::CompactRouteLeg::Walk(tiles) => {
                    Some(LegShape::Walk { tiles: tiles.len() })
                }
                nav_planner::router::CompactRouteLeg::Transport(_) => {
                    plan_owned_leg(plan, index).as_ref().map(shape)
                }
            })
            .collect(),
    }
}
