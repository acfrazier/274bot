//! The planner half of the compat `probeDest` / walk-verify budget test. The script-js unit test
//! `walk_verify_and_probe_dest_ignore_caller_budgets_below_node_budget_for_long_route` proves that
//! the request carries no caller `maxExpansions` and these options (`allow_teleports` false,
//! `allow_wilderness` true, `allow_bank_fetch` false); this test proves what the planner then does
//! on a map whose route needs more than the upstream 300,000 expansions: a caller budget below the
//! planner's own would exhaust, the default budget reaches the destination.

use api::snapshot::WorldTile;
use nav_planner::collision::{pack_walk, WorldCollision};
use nav_planner::router::{self, FindOptions, NODE_BUDGET};
use nav_planner::transport::TransportGraph;
use nav_planner::WorldState;

#[test]
fn a_long_route_exhausts_the_upstream_budget_but_not_the_default_one() {
    const WIDTH: usize = 768;
    const HEIGHT: usize = 768;
    const WALL_X: usize = WIDTH / 2;
    const { assert!(300_000 < NODE_BUDGET) };
    let from = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let to = WorldTile {
        x: WIDTH as i32 - 1,
        z: 0,
        level: 0,
    };
    let mut flags = vec![0u32; WIDTH * HEIGHT * 4];
    for z in 0..HEIGHT - 1 {
        flags[z * WIDTH + WALL_X] = client::dash3d::CollisionFlag::SQ_BLOCKED as u32;
    }
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin: from,
        width: WIDTH,
        height: HEIGHT,
        walk,
        blocked,
        flags: None,
    };
    let graph = TransportGraph::default();
    let options = FindOptions {
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: false,
        ..FindOptions::default()
    };
    let state = WorldState::empty();
    assert_eq!(
        router::find_with_avoid_bounded(
            &collision,
            &graph,
            from,
            to,
            options,
            &state,
            &[],
            300_000
        ),
        Err(router::RouteError::BudgetExhausted),
        "the map requires more than the upstream 300,000 expansion count"
    );
    let routed = router::find_with_avoid(&collision, &graph, from, to, options, &state, &[])
        .expect("the default router budget reaches the long-route destination");
    assert_eq!(routed.dest, to);
}
