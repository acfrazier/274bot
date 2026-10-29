//! Frozen `Traversal.walkTo` / `walkResilient` boat-fare recovery over the
//! whole plantation job: employment, picking, a full crate, route-reported fare
//! payment and the re-walk to the boat. Driven through the registered
//! machine families and synthetic posts only.

use super::{grove_anchor, recovery_facts, BoatFacts, Recover};
use crate::machine::{self, Cx, Outcome, Started, Take};
use crate::observed::{self, CarryRow, ChatLine, EntityRow, ItemRow, SceneRow};
use crate::task_clock::InstantTaskClock;
use crate::walk::tests::{post_walk_outcome, reset, NoJs};
use api::query::ReachQueryView;
use api::snapshot::WorldTile;
use script::shim::InteractReq;
use serde_json::json;
use std::collections::VecDeque;
use std::rc::Rc;
const MUSA: WorldTile = WorldTile {
    x: 2954,
    z: 3147,
    level: 0,
};
const PORT_SARIM: WorldTile = WorldTile {
    x: 3029,
    z: 3217,
    level: 0,
};
const BRIMHAVEN_CART: WorldTile = WorldTile {
    x: 2776,
    z: 3214,
    level: 0,
};

#[test]
fn grove_anchor_uses_nearest_content_tree_to_frozen_center_deterministically() {
    use api::game_data::KaramjaSpawn;

    let spawn = |config: &str, x, z| KaramjaSpawn {
        config: config.into(),
        x,
        z,
        plane: 0,
    };
    let spawns = [
        spawn("bananatreefull", 2918, 3155),
        spawn("bananatreeone", 2926, 3161),
        spawn("bananatreeempty", 2925, 3160),
    ];
    let expected = WorldTile {
        x: 2925,
        z: 3160,
        level: 0,
    };
    assert_eq!(grove_anchor(&spawns), Some(expected));
    let reversed: Vec<_> = spawns.iter().rev().cloned().collect();
    assert_eq!(grove_anchor(&reversed), Some(expected));
}

#[test]
fn recovery_skips_fact_selection_off_island_and_logs_selected_fact_errors() {
    let mainland = WorldTile {
        x: 3100,
        z: 3200,
        level: 0,
    };
    reset();
    let mut world = World::new(mainland, 5);
    world.post();
    crate::supply_v2::configure(None);
    let mut logs = VecDeque::new();
    assert!(recovery_facts(PORT_SARIM, &mut logs).unwrap().is_none());
    assert!(
        logs.is_empty(),
        "the cheap mainland gate does not inspect selected facts"
    );

    reset();
    let mut world = World::new(MUSA, 5);
    world.post();
    crate::supply_v2::configure(None);
    assert!(recovery_facts(PORT_SARIM, &mut logs).is_err());
    assert_eq!(logs.len(), 1);
    assert!(
        logs[0].starts_with("boat fare recovery facts unavailable: "),
        "selection error should be retained as a skip diagnostic: {logs:?}"
    );
}

fn selected_facts() -> Rc<BoatFacts> {
    let data = api::game_data::for_revision(client::io::ClientRevision::R289)
        .expect("selected Karamja game data");
    crate::supply_v2::configure(Some(data));
    BoatFacts::selected().expect("selected Karamja facts")
}

/// The synthetic scene: the tile, pack, chat and the plantation rows.
struct World {
    facts: Rc<BoatFacts>,
    tick: u64,
    here: WorldTile,
    coins: i32,
    bananas: i32,
    filler: i32,
    modal: i32,
    cont: bool,
    options: Vec<String>,
    lines: Vec<ChatLine>,
    seq: i32,
    luthas_posted: bool,
    ours: bool,
}

impl World {
    fn new(here: WorldTile, coins: i32) -> Self {
        Self {
            facts: selected_facts(),
            tick: 1,
            here,
            coins,
            bananas: 0,
            filler: 0,
            modal: -1,
            cont: false,
            options: Vec::new(),
            lines: Vec::new(),
            seq: 0,
            luthas_posted: true,
            ours: false,
        }
    }

    fn post(&mut self) {
        self.tick += 1;
        let mut inv = vec![ItemRow {
            id: self.facts.coins,
            count: self.coins,
            name: Some("Coins".into()),
            slot: Some(0),
            ..ItemRow::default()
        }];
        inv.extend((0..self.bananas).map(|slot| ItemRow {
            id: self.facts.banana,
            count: 1,
            name: Some("Banana".into()),
            slot: Some(1 + slot),
            ..ItemRow::default()
        }));
        inv.extend((0..self.filler).map(|slot| ItemRow {
            id: 1511,
            count: 1,
            name: Some("Logs".into()),
            slot: Some(1 + self.bananas + slot),
            ..ItemRow::default()
        }));
        let here = self.here;
        let mut locs = vec![loc(
            self.facts.crate_loc,
            "Crate",
            self.facts.crate_tile.x,
            self.facts.crate_tile.z,
            self.facts.crate_tile.level,
            0,
            &self.facts.crate_search,
        )];
        locs.extend(self.facts.tree_actions.iter().map(|tree| {
            loc(
                tree.id,
                "Banana Tree",
                self.facts.grove_tile.x - 1,
                self.facts.grove_tile.z,
                self.facts.grove_tile.level,
                1,
                &tree.action,
            )
        }));
        observed::post(self.tick, |post| {
            post.session(true)
                .ours(self.ours)
                .here(observed::Tile {
                    x: here.x,
                    z: here.z,
                    level: here.level,
                })
                .inv_size(28)
                .inv(inv)
                .chat_modal_id(self.modal)
                .chat_continue(self.cont)
                .chat_options(self.options.clone())
                .chat_lines(self.lines.clone())
                .npcs(if self.luthas_posted {
                    vec![EntityRow {
                        index: 50,
                        name: Some(self.facts.luthas_name.clone().into()),
                        x: self.facts.luthas_anchor.x,
                        z: self.facts.luthas_anchor.z + 1,
                        level: self.facts.luthas_anchor.level,
                        distance: 1,
                        actions: vec!["Talk-to".into()].into(),
                        reachable_adj: true,
                        ..EntityRow::default()
                    }]
                } else {
                    Vec::new()
                })
                .locs(locs);
        });
    }

    fn say(&mut self, text: &str) {
        self.seq += 1;
        self.lines.push(ChatLine {
            seq: self.seq,
            text: text.into(),
        });
    }
}

fn loc(id: i32, name: &str, x: i32, z: i32, level: i32, distance: i32, action: &str) -> SceneRow {
    SceneRow {
        id,
        name: Some(name.into()),
        x,
        z,
        level,
        distance,
        actions: vec![action.into()].into(),
        shape: 10,
        angle: 0,
        in_combat: false,
    }
}

fn tick() -> Vec<InteractReq> {
    machine::step(&mut NoJs);
    machine::merge_ops(Vec::new())
}
fn recovery_step(
    world: &mut World,
    recover: &mut Recover,
    clock: &mut InstantTaskClock,
    logs: &mut VecDeque<String>,
) -> (Option<bool>, Vec<InteractReq>) {
    world.post();
    recovery_step_observed(recover, clock, logs)
}

fn recovery_step_observed(
    recover: &mut Recover,
    clock: &mut InstantTaskClock,
    logs: &mut VecDeque<String>,
) -> (Option<bool>, Vec<InteractReq>) {
    let mut ops = Vec::new();
    let result = {
        let mut cx = Cx::test(&mut ops, clock, None);
        recover.step(&mut cx, logs)
    };
    (result, ops)
}
fn start_recovery() -> (
    Recover,
    InstantTaskClock,
    VecDeque<String>,
    Vec<InteractReq>,
) {
    let mut ops = Vec::new();
    let mut clock = InstantTaskClock::new();
    let mut logs = VecDeque::new();
    let recover = {
        let mut cx = Cx::test(&mut ops, &mut clock, None);
        Recover::start(PORT_SARIM, &mut cx, &mut logs).expect("fare recovery starts")
    };
    (recover, clock, logs, ops)
}

/// Tick until an op matches, with no other op on the way; the match.
fn until(world: &mut World, what: &str, want: impl Fn(&InteractReq) -> bool) -> InteractReq {
    for _ in 0..40 {
        world.post();
        let ops = tick();
        if let Some(hit) = ops.iter().find(|op| want(op)) {
            assert_eq!(ops.len(), 1, "{what}: one op, got {ops:?}");
            return hit.clone();
        }
        assert!(ops.is_empty(), "{what}: unexpected ops {ops:?}");
    }
    panic!("{what}: never emitted");
}

fn walk_near(to: WorldTile, radius: i32) -> impl Fn(&InteractReq) -> bool {
    move |op| {
        matches!(
            op,
            InteractReq::WalkNear {
                x,
                z,
                level,
                radius: r,
                compat_tile_intent: true,
                ..
            } if *x == to.x && *z == to.z && *level == to.level && *r == radius
        )
    }
}
fn is_walk(op: &InteractReq) -> bool {
    matches!(op, InteractReq::Walk { .. })
}

fn token(op: &InteractReq) -> u64 {
    match op {
        InteractReq::Walk { request_id, .. } | InteractReq::WalkNear { request_id, .. } => {
            *request_id
        }
        other => panic!("not a walk: {other:?}"),
    }
}
fn walk_tile(op: &InteractReq) -> WorldTile {
    match op {
        InteractReq::Walk { x, z, level, .. } | InteractReq::WalkNear { x, z, level, .. } => {
            WorldTile {
                x: *x,
                z: *z,
                level: *level,
            }
        }
        other => panic!("not a walk: {other:?}"),
    }
}
fn walk_radius(op: &InteractReq) -> i32 {
    match op {
        InteractReq::WalkNear { radius, .. } => *radius,
        InteractReq::Walk { .. } => 0,
        other => panic!("not a walk: {other:?}"),
    }
}

fn orthogonally_adjacent(left: WorldTile, right: WorldTile) -> bool {
    left.level == right.level
        && u64::from(left.x.abs_diff(right.x)) + u64::from(left.z.abs_diff(right.z)) == 1
}

fn set_reach_bit(bits: &mut [u32], index: usize) {
    bits[index / 32] |= 1 << (index % 32);
}

fn install_luthas_reach_view(anchor: WorldTile, blocked: Option<WorldTile>) {
    install_luthas_reach_view_with(anchor, blocked, None);
}

fn install_only_walkable_unreachable_luthas_stand(anchor: WorldTile, stand: WorldTile) {
    install_luthas_reach_view_with(anchor, None, Some(stand));
}

fn install_luthas_reach_view_with(
    anchor: WorldTile,
    blocked: Option<WorldTile>,
    only_walkable_unreachable: Option<WorldTile>,
) {
    let npc = WorldTile {
        x: anchor.x,
        z: anchor.z + 1,
        level: anchor.level,
    };
    let base_x = npc.x - 2;
    let base_z = npc.z - 2;
    let index = |tile: WorldTile| ((tile.x - base_x) as usize) * 5 + (tile.z - base_z) as usize;
    let mut walkable = vec![0; 1];
    let mut reachable = vec![0; 1];
    let mut reachable_adj = vec![0; 1];
    let mut exact_rank = vec![u16::MAX; 25];
    let mut adjacent_rank = vec![u16::MAX; 25];
    for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
        let stand = WorldTile {
            x: npc.x + dx,
            z: npc.z + dz,
            level: npc.level,
        };
        if blocked == Some(stand) || only_walkable_unreachable.is_some_and(|only| only != stand) {
            continue;
        }
        let i = index(stand);
        set_reach_bit(&mut walkable, i);
        if only_walkable_unreachable == Some(stand) {
            continue;
        }
        set_reach_bit(&mut reachable, i);
        set_reach_bit(&mut reachable_adj, i);
        exact_rank[i] = 1;
        adjacent_rank[i] = 1;
    }
    let npc_index = index(npc);
    set_reach_bit(&mut reachable_adj, npc_index);
    adjacent_rank[npc_index] = 1;
    crate::reach_query::set_view_for_tests(ReachQueryView {
        available: true,
        base_x,
        base_z,
        level: npc.level,
        width: 5,
        height: 5,
        walkable,
        reachable,
        reachable_adj,
        exact_rank,
        adjacent_rank,
        step: vec![0; 25],
        canlight: Vec::new(),
    });
}

fn install_luthas_staging_reach_view(anchor: WorldTile) {
    install_luthas_staging_reach_view_with_ranks(anchor, 0, 1);
}

fn install_luthas_staging_reach_view_with_ranks(
    anchor: WorldTile,
    staged_rank: u16,
    other_rank: u16,
) {
    let npc = WorldTile {
        x: anchor.x,
        z: anchor.z + 1,
        level: anchor.level,
    };
    let staged = WorldTile {
        x: anchor.x,
        z: anchor.z - 1,
        level: anchor.level,
    };
    let base_x = anchor.x - 2;
    let base_z = anchor.z - 2;
    let width = 5;
    let height = 5;
    let index = |tile: WorldTile| {
        ((tile.x - base_x) as usize) * height as usize + (tile.z - base_z) as usize
    };
    let mut walkable = vec![0; 1];
    let mut reachable = vec![0; 1];
    let mut reachable_adj = vec![0; 1];
    let mut exact_rank = vec![u16::MAX; width as usize * height as usize];
    let mut adjacent_rank = vec![u16::MAX; width as usize * height as usize];
    for target in [anchor, npc] {
        for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            let stand = WorldTile {
                x: target.x + dx,
                z: target.z + dz,
                level: target.level,
            };
            if stand == npc {
                continue;
            }
            let i = index(stand);
            set_reach_bit(&mut walkable, i);
            set_reach_bit(&mut reachable, i);
            set_reach_bit(&mut reachable_adj, i);
            exact_rank[i] = if stand == staged {
                staged_rank
            } else {
                other_rank
            };
            adjacent_rank[i] = 1;
        }
    }
    crate::reach_query::set_view_for_tests(ReachQueryView {
        available: true,
        base_x,
        base_z,
        level: anchor.level,
        width,
        height,
        walkable,
        reachable,
        reachable_adj,
        exact_rank,
        adjacent_rank,
        step: vec![0; width as usize * height as usize],
        canlight: Vec::new(),
    });
}

fn install_ranked_luthas_reach_view(anchor: WorldTile, ranked_stands: &[(WorldTile, u16)]) {
    let npc = WorldTile {
        x: anchor.x,
        z: anchor.z + 1,
        level: anchor.level,
    };
    let base_x = npc.x - 2;
    let base_z = npc.z - 2;
    let width = 5;
    let height = 5;
    let index = |tile: WorldTile| {
        ((tile.x - base_x) as usize) * height as usize + (tile.z - base_z) as usize
    };
    let mut walkable = vec![0; 1];
    let mut reachable = vec![0; 1];
    let mut reachable_adj = vec![0; 1];
    let mut exact_rank = vec![u16::MAX; width as usize * height as usize];
    let mut adjacent_rank = vec![u16::MAX; width as usize * height as usize];
    for (stand, rank) in ranked_stands {
        let i = index(*stand);
        set_reach_bit(&mut walkable, i);
        set_reach_bit(&mut reachable, i);
        set_reach_bit(&mut reachable_adj, i);
        exact_rank[i] = *rank;
        adjacent_rank[i] = *rank;
    }
    crate::reach_query::set_view_for_tests(ReachQueryView {
        available: true,
        base_x,
        base_z,
        level: anchor.level,
        width,
        height,
        walkable,
        reachable,
        reachable_adj,
        exact_rank,
        adjacent_rank,
        step: vec![0; width as usize * height as usize],
        canlight: Vec::new(),
    });
}

#[test]
fn a_far_start_walks_to_luthas_spawn_without_a_scene_reach_gate() {
    reset();
    let brimhaven_cart = WorldTile {
        x: 2776,
        z: 3214,
        level: 0,
    };
    let mut world = World::new(brimhaven_cart, 5);
    let anchor = world.facts.luthas_anchor;
    world.luthas_posted = false;
    install_luthas_staging_reach_view_with_ranks(anchor, 401, 401);
    world.post();
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    let mut ops = Vec::new();
    let mut clock = InstantTaskClock::new();
    let mut logs = VecDeque::new();
    let recovery = {
        let mut cx = Cx::test(&mut ops, &mut clock, None);
        Recover::start(PORT_SARIM, &mut cx, &mut logs)
    };
    assert!(
        recovery.is_ok(),
        "far-start fare recovery should start: {logs:?}"
    );
    assert!(
        matches!(ops.as_slice(), [op] if walk_near(anchor, 2)(op)),
        "a far start walks to Luthas's spawn tile with radius 2: {ops:?}"
    );
}

#[test]
fn a_short_luthas_staging_walk_without_a_row_retries() {
    reset();
    let mut world = World::new(MUSA, 5);
    let anchor = world.facts.luthas_anchor;
    world.luthas_posted = false;
    install_luthas_staging_reach_view(anchor);
    world.post();
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    let (mut recover, mut clock, mut logs, ops) = start_recovery();
    let walk = ops.first().expect("staging walk starts");
    let staging_tile = walk_tile(walk);
    world.tick += 1;
    post_walk_outcome(
        world.tick,
        token(walk),
        world.here,
        staging_tile,
        walk_radius(walk),
        false,
    );
    let (result, ops) = recovery_step_observed(&mut recover, &mut clock, &mut logs);

    assert_eq!(result, None, "a short walk is retried");
    assert!(
        matches!(ops.as_slice(), [op] if walk_near(anchor, 2)(op)),
        "retry starts a fresh walk to Luthas's spawn tile: {ops:?}"
    );
    assert!(
        logs.iter()
            .any(|line| line.contains("staging attempt 1/3 failed")),
        "short walk is recorded as a failed staging attempt: {logs:?}"
    );
}

#[test]
fn a_staging_arrival_without_luthas_waits_one_tick_before_retrying() {
    reset();
    let mut world = World::new(MUSA, 5);
    let anchor = world.facts.luthas_anchor;
    world.luthas_posted = false;
    install_luthas_staging_reach_view(anchor);
    world.post();
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    let (mut recover, mut clock, mut logs, ops) = start_recovery();
    let walk = ops.first().expect("staging walk starts");
    world.here = anchor;
    world.tick += 1;
    post_walk_outcome(
        world.tick,
        token(walk),
        world.here,
        anchor,
        walk_radius(walk),
        false,
    );
    let (result, ops) = recovery_step_observed(&mut recover, &mut clock, &mut logs);
    assert_eq!(result, None, "arrival waits for another observed tick");
    assert!(ops.is_empty());
    assert!(
        logs.iter()
            .any(|line| line.contains("staging attempt 1/3 failed")),
        "the arrival without a row spends the first attempt: {logs:?}"
    );
    assert!(
        !logs.iter().any(|line| line.contains("recovery ended")),
        "the first staging miss is not terminal"
    );

    for attempt in 2..=3 {
        world.post();
        let (result, ops) = recovery_step_observed(&mut recover, &mut clock, &mut logs);
        assert_eq!(result, None, "attempt {attempt} waits before ending");
        assert!(ops.is_empty(), "the player is already within staging radius");
        assert!(
            logs.iter()
                .any(|line| line.contains(&format!("staging attempt {attempt}/3 failed"))),
            "attempt {attempt} is recorded: {logs:?}"
        );
    }

    world.post();
    let (result, ops) = recovery_step_observed(&mut recover, &mut clock, &mut logs);
    assert_eq!(result, Some(false), "the exhausted budget ends after a tick");
    assert!(ops.is_empty());
    assert!(
        logs.back().is_some_and(|line| {
            line.contains("recovery ended after attempt 3/3")
                && line.contains("the staging walk ended without Luthas in view")
        }),
        "terminal staging failure keeps its detail: {logs:?}"
    );
}

#[test]
fn staging_walk_failures_share_three_attempts_and_keep_the_last_detail() {
    reset();
    let mut world = World::new(MUSA, 5);
    let anchor = world.facts.luthas_anchor;
    world.luthas_posted = false;
    install_luthas_staging_reach_view(anchor);
    world.post();
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    let (mut recover, mut clock, mut logs, ops) = start_recovery();
    let mut walk = ops[0].clone();
    for attempt in 1..=3 {
        let staging_tile = walk_tile(&walk);
        world.tick += 1;
        post_walk_outcome(
            world.tick,
            token(&walk),
            world.here,
            staging_tile,
            walk_radius(&walk),
            true,
        );
        let (result, ops) = recovery_step_observed(&mut recover, &mut clock, &mut logs);
        if attempt < 3 {
            assert_eq!(result, None, "attempt {attempt} retries");
            assert_eq!(ops.len(), 1, "attempt {attempt} starts a fresh walk");
            walk = ops[0].clone();
        } else {
            assert_eq!(result, Some(false), "the third failure is terminal");
            assert!(ops.is_empty());
        }
    }

    assert_eq!(
        logs.iter()
            .filter(|line| line.contains("staging attempt") && line.contains("failed"))
            .count(),
        3,
        "staging failures use the three-attempt budget: {logs:?}"
    );
    assert!(
        logs.back().is_some_and(|line| {
            line.contains("recovery ended after attempt 3/3")
                && line.contains("the host walk did not reach Luthas's spawn tile")
        }),
        "terminal outcome keeps the last failure detail: {logs:?}"
    );
    assert!(
        !logs.iter().any(|line| line.contains("Luthas approach")),
        "staging failures do not log talk approaches"
    );
}

#[test]
fn being_at_luthas_staging_tile_checks_the_row_before_another_walk() {
    reset();
    let mut world = World::new(MUSA, 5);
    let anchor = world.facts.luthas_anchor;
    world.here = WorldTile {
        x: anchor.x,
        z: anchor.z - 1,
        level: anchor.level,
    };
    world.luthas_posted = false;
    install_luthas_staging_reach_view(anchor);
    world.post();
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    let (mut ops, mut clock, mut logs) = (Vec::new(), InstantTaskClock::new(), VecDeque::new());
    let mut recover = {
        let mut cx = Cx::test(&mut ops, &mut clock, None);
        Recover::start(PORT_SARIM, &mut cx, &mut logs).expect("staging wait starts")
    };
    assert!(ops.is_empty(), "the current tile needs no new walk");
    assert!(
        logs.iter()
            .any(|line| line.contains("staging attempt 1/3 failed")),
        "arrival without Luthas spends the first attempt: {logs:?}"
    );

    world.post();
    let (result, ops) = recovery_step_observed(&mut recover, &mut clock, &mut logs);
    assert_eq!(result, None, "one missing row does not end recovery");
    assert!(ops.is_empty());
    assert!(
        logs.iter()
            .any(|line| line.contains("staging attempt 2/3 failed")),
        "the next attempt also waits for a posted row: {logs:?}"
    );
}

#[test]
fn luthas_stand_selection_uses_search_rank_before_manhattan_distance() {
    reset();
    let mut world = World::new(MUSA, 5);
    let anchor = world.facts.luthas_anchor;
    let npc = WorldTile {
        x: anchor.x,
        z: anchor.z + 1,
        level: anchor.level,
    };
    let lower_rank = anchor;
    let closer = WorldTile {
        x: npc.x + 1,
        z: npc.z,
        level: npc.level,
    };
    world.here = WorldTile {
        x: closer.x + 1,
        z: closer.z,
        level: closer.level,
    };
    install_ranked_luthas_reach_view(anchor, &[(lower_rank, 1), (closer, 2)]);
    world.post();
    let options = api::query::SceneReachOptions {
        max_steps: Some(400),
        adjacent_ok: false,
    };
    crate::reach_query::with_view(|view| {
        let rank = |tile: WorldTile| {
            let lx = (tile.x - view.base_x) as usize;
            let lz = (tile.z - view.base_z) as usize;
            view.exact_rank[lx * view.height as usize + lz]
        };
        assert!(view.walkable(lower_rank));
        assert!(view.walkable(closer));
        assert!(view.can_reach(lower_rank, &options));
        assert!(view.can_reach(closer, &options));
        assert_eq!(rank(lower_rank), 1);
        assert_eq!(rank(closer), 2);
    });
    assert_eq!(
        super::next_orthogonal_stand(npc, &super::TriedStands::default(), "no stand"),
        Ok(lower_rank)
    );
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    let (_recover, _clock, _logs, ops) = start_recovery();
    assert_eq!(ops.len(), 1);
    assert_eq!(
        walk_tile(&ops[0]),
        lower_rank,
        "search rank wins when the lower-rank tile is farther away"
    );
}

fn is_talk(name: String) -> impl Fn(&InteractReq) -> bool {
    move |op| {
        matches!(op, InteractReq::Npc { name: got, action, index: Some(50) }
            if got == &name && action == "Talk-to")
    }
}

fn is_loc(id: i32, action: String) -> impl Fn(&InteractReq) -> bool {
    move |op| {
        matches!(op, InteractReq::Loc { action: got_action, id: Some(got_id), .. }
            if *got_id == id && got_action == &action)
    }
}

fn is_continue(op: &InteractReq) -> bool {
    matches!(op, InteractReq::ContinueDialog { component_id: None })
}

/// A failed walk whose navigator named `(id, count)` as the one short.
fn fail_short(world: &mut World, token: u64, dest: WorldTile, radius: i32, short: (i32, i32)) {
    world.tick += 1;
    post_walk_outcome(world.tick, token, world.here, dest, radius, true);
    observed::post(world.tick, |post| {
        post.walk_missing_carry(vec![CarryRow {
            id: short.0,
            count: short.1,
            name: Some("Coins".into()),
        }]);
    });
}

/// Talk-to Luthas, answer his option list with the preferred line, turn one
/// Continue page, then let the chat close.
fn talk(world: &mut World, options: &[String], want: i32, after_answer: impl FnOnce(&mut World)) {
    let luthas_name = world.facts.luthas_name.clone();
    until(world, "talk to Luthas", is_talk(luthas_name));
    world.modal = 4882;
    world.options = options.to_vec();
    let answer = until(world, "answer Luthas", |op| {
        matches!(op, InteractReq::Answer { .. })
    });
    assert_eq!(answer, InteractReq::Answer { option: want });
    world.modal = 4883;
    world.options.clear();
    world.cont = true;
    after_answer(world);
    until(world, "continue Luthas", is_continue);
    world.modal = -1;
    world.cont = false;
}

fn start_walk_to(world: &mut World) -> machine::Handle {
    world.post();
    let args = json!({
        "tile": { "x": PORT_SARIM.x, "z": PORT_SARIM.z, "level": 0 },
    });
    match machine::start("walk-to", args, Vec::new(), 0) {
        Started::Running(handle) => handle,
        other => panic!("walk-to runs, got {other:?}"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RetryPoint {
    InitialTalk,
    PostFillTalk,
}

fn fail_luthas_reach(world: &mut World, what: &str) -> WorldTile {
    let luthas = world.facts.luthas_anchor;
    let name = world.facts.luthas_name.clone();
    until(world, what, is_talk(name));
    world.say("I can't reach that!");
    world.post();
    let ops = tick();
    let stand = match ops.as_slice() {
        [InteractReq::Walk { x, z, level, .. }] => WorldTile {
            x: *x,
            z: *z,
            level: *level,
        },
        other => panic!("failed Luthas reach starts a fresh stand walk: {other:?}"),
    };
    let npc = WorldTile {
        x: luthas.x,
        z: luthas.z + 1,
        level: luthas.level,
    };
    assert_ne!(stand, luthas, "retry uses another Luthas stand");
    assert!(
        orthogonally_adjacent(stand, npc),
        "retry stand {stand:?} is orthogonally adjacent to posted Luthas {npc:?}"
    );
    world.here = stand;
    stand
}

/// The whole plantation job up to the re-walk to the boat; returns the
/// row, the world, the first walk and the re-walk.
fn earn_the_fare(
    retry_point: Option<RetryPoint>,
) -> (machine::Handle, World, InteractReq, InteractReq) {
    reset();
    let mut world = World::new(MUSA, 5);
    install_luthas_reach_view(world.facts.luthas_anchor, None);
    let luthas = world.facts.luthas_anchor;
    let crate_tile = world.facts.crate_tile;
    let grove_tile = world.facts.grove_tile;
    let crate_id = world.facts.crate_loc;
    let crate_action = world.facts.crate_search.clone();
    let tree_id = world.facts.tree_actions[0].id;
    let tree_action = world.facts.tree_actions[0].action.clone();
    let capacity = world.facts.crate_capacity;
    let employment = world.facts.preferences[0].clone();
    let paid = world.facts.preferences[1].clone();
    let coins_id = world.facts.coins;
    let fare = world.facts.coin_payout;
    let h = start_walk_to(&mut world);
    let first = machine::merge_ops(Vec::new());
    assert!(
        matches!(first.as_slice(), [op] if walk_near(PORT_SARIM, 2)(op)),
        "the first walk goes out in the caller's tick: {first:?}"
    );

    // The navigator reports the route's required coin count.
    fail_short(
        &mut world,
        token(&first[0]),
        PORT_SARIM,
        2,
        (coins_id, fare),
    );
    let walk_to_luthas = until(&mut world, "walk to Luthas", is_walk);
    assert_ne!(token(&walk_to_luthas), token(&first[0]));
    let first_stand = walk_tile(&walk_to_luthas);
    let npc = WorldTile {
        x: luthas.x,
        z: luthas.z + 1,
        level: luthas.level,
    };
    assert!(orthogonally_adjacent(first_stand, npc));
    world.here = first_stand;
    if retry_point == Some(RetryPoint::InitialTalk) {
        fail_luthas_reach(&mut world, "first talk to Luthas");
    }

    // Employment (luthas.rs2:8–10, the multi2's first line).
    let employment_options = [
        employment,
        "That customs officer is annoying isn't she?".into(),
    ];
    talk(&mut world, &employment_options, 1, |_| {});

    // fillCrate: the empty crate, then the grove.
    until(&mut world, "walk to the crate", walk_near(crate_tile, 2));
    world.here = crate_tile;
    until(
        &mut world,
        "search the crate",
        is_loc(crate_id, crate_action.clone()),
    );
    world.say("The crate is completely empty.");
    until(&mut world, "walk to the grove", walk_near(grove_tile, 4));
    world.here = grove_tile;
    for picked in 0..capacity {
        until(
            &mut world,
            "search a banana tree",
            is_loc(tree_id, tree_action.clone()),
        );
        world.bananas = picked + 1;
        world.say("You pick a banana.");
    }

    // Pack each banana (banana_crate.rs2 `oplocu`): one use per banana.
    until(
        &mut world,
        "walk back to the crate",
        walk_near(crate_tile, 2),
    );
    world.here = crate_tile;
    for packed in 0..capacity {
        let use_on = until(&mut world, "use a banana on the crate", |op| {
            matches!(op, InteractReq::UseOn { .. })
        });
        match use_on {
            InteractReq::UseOn {
                kind,
                x,
                z,
                source_item_id,
                ..
            } => {
                assert_eq!(kind, "loc");
                assert_eq!((x, z), (crate_tile.x, crate_tile.z));
                assert_eq!(source_item_id, Some(world.facts.banana));
            }
            _ => unreachable!(),
        }
        world.bananas = capacity - 1 - packed;
    }
    assert_eq!(world.bananas, 0);

    // The closing read says full; back to Luthas for the route-reported fare.
    until(
        &mut world,
        "re-read the crate",
        is_loc(crate_id, crate_action),
    );
    world.say("The crate is full of bananas.");
    let walk_back_to_luthas = until(&mut world, "walk back to Luthas", is_walk);
    let second_stand = walk_tile(&walk_back_to_luthas);
    let npc = WorldTile {
        x: luthas.x,
        z: luthas.z + 1,
        level: luthas.level,
    };
    assert!(orthogonally_adjacent(second_stand, npc));
    world.here = second_stand;
    let paid_options = [
        "Will you pay me for another crate full?".into(),
        paid,
        "So where are these bananas going to be delivered to?".into(),
        "That customs officer is annoying isn't she?".into(),
    ];
    if retry_point == Some(RetryPoint::PostFillTalk) {
        fail_luthas_reach(&mut world, "first post-fill talk to Luthas");
    }

    talk(&mut world, &paid_options, 2, |world| {
        world.coins = fare + 5;
        world.say(&format!("Luthas hands you {fare} coins."));
    });

    let again = until(
        &mut world,
        "walk to the boat again",
        walk_near(PORT_SARIM, 2),
    );
    (h, world, first[0].clone(), again)
}

#[test]
fn walk_to_earns_the_fare_at_the_plantation_then_takes_the_boat() {
    let (h, mut world, first, again) = earn_the_fare(None);
    assert_eq!(machine::take(h), Take::Pending);
    world.here = PORT_SARIM;
    world.post();
    assert!(tick().is_empty());
    assert_ne!(token(&again), token(&first));
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(true))));
}

#[test]
fn walk_to_retries_a_failed_luthas_approach_and_completes() {
    let (h, mut world, first, again) = earn_the_fare(Some(RetryPoint::InitialTalk));
    assert_eq!(machine::take(h), Take::Pending);
    world.here = PORT_SARIM;
    world.post();
    assert!(tick().is_empty());
    assert_ne!(token(&again), token(&first));
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(true))));
}

#[test]
fn walk_to_retries_a_failed_post_fill_luthas_approach_and_completes() {
    let (h, mut world, first, again) = earn_the_fare(Some(RetryPoint::PostFillTalk));
    assert_eq!(machine::take(h), Take::Pending);
    world.here = PORT_SARIM;
    world.post();
    assert!(tick().is_empty());
    assert_ne!(token(&again), token(&first));
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(true))));
}

#[test]
fn failed_luthas_approaches_spend_the_budget_and_keep_the_last_detail() {
    reset();
    let mut world = World::new(MUSA, 5);
    install_luthas_reach_view(world.facts.luthas_anchor, None);
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    let luthas = world.facts.luthas_anchor;
    world.post();
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    let mut ops = Vec::new();
    let mut clock = InstantTaskClock::new();
    let mut logs = VecDeque::new();
    let mut recover = {
        let mut cx = Cx::test(&mut ops, &mut clock, None);
        Recover::start(PORT_SARIM, &mut cx, &mut logs).expect("fare recovery starts")
    };
    assert!(matches!(ops.as_slice(), [InteractReq::Walk { .. }]));
    let initial_stand = walk_tile(&ops[0]);
    let npc = WorldTile {
        x: luthas.x,
        z: luthas.z + 1,
        level: luthas.level,
    };
    assert!(orthogonally_adjacent(initial_stand, npc));

    world.here = initial_stand;
    assert!(
        recovery_step(&mut world, &mut recover, &mut clock, &mut logs)
            .1
            .is_empty()
    );
    let mut retry_stand = None;
    let mut last_stand = initial_stand;
    for attempt in 0..3 {
        if let Some(stand) = retry_stand.take() {
            world.here = stand;
            assert!(
                recovery_step(&mut world, &mut recover, &mut clock, &mut logs)
                    .1
                    .is_empty()
            );
        }
        let ops = recovery_step(&mut world, &mut recover, &mut clock, &mut logs).1;
        assert!(
            matches!(ops.as_slice(), [InteractReq::Npc { action, .. }] if action == "Talk-to"),
            "approach {attempt} talks to Luthas: {ops:?}"
        );
        world.say("I can't reach that!");
        let (result, ops) = recovery_step(&mut world, &mut recover, &mut clock, &mut logs);
        if attempt < 2 {
            assert_eq!(result, None, "a failed approach is not terminal");
            let stand = match ops.as_slice() {
                [InteractReq::Walk { x, z, level, .. }] => WorldTile {
                    x: *x,
                    z: *z,
                    level: *level,
                },
                other => panic!("failure starts a fresh walk to another stand: {other:?}"),
            };
            assert_ne!(stand, last_stand, "retry changes stand after each failure");
            let npc = WorldTile {
                x: luthas.x,
                z: luthas.z + 1,
                level: luthas.level,
            };
            assert_ne!(stand, luthas, "retry does not reuse the initial stand");
            assert!(
                orthogonally_adjacent(stand, npc),
                "retry stand {stand:?} is orthogonally adjacent to posted Luthas {npc:?}"
            );
            last_stand = stand;
            retry_stand = Some(stand);
        } else {
            assert_eq!(result, Some(false), "the final failed approach is terminal");
            assert!(ops.is_empty());
        }
    }
    assert!(
        logs.back()
            .is_some_and(|line| line.contains("server can't reach it")),
        "terminal log keeps the final reach detail: {logs:?}"
    );
    assert_eq!(
        logs.iter()
            .filter(|line| line.starts_with("reach: 'Luthas' at"))
            .count(),
        3,
        "each failed approach keeps its reach detail"
    );
    assert_eq!(
        logs.iter()
            .filter(|line| line.contains("failed cause=reach"))
            .count(),
        3,
        "each failed approach has a diagnostic line"
    );
}

#[test]
fn a_walkable_but_unreachable_luthas_stand_is_not_selected() {
    reset();
    let mut world = World::new(MUSA, 5);
    let luthas = world.facts.luthas_anchor;
    let unreachable = luthas;
    install_only_walkable_unreachable_luthas_stand(luthas, unreachable);
    world.post();
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    crate::reach_query::with_view(|view| {
        assert!(view.walkable(unreachable));
        assert!(!view.can_reach(
            unreachable,
            &api::query::SceneReachOptions {
                max_steps: Some(400),
                adjacent_ok: false,
            },
        ));
    });

    let mut ops = Vec::new();
    let mut clock = InstantTaskClock::new();
    let mut logs = VecDeque::new();
    let recovery = {
        let mut cx = Cx::test(&mut ops, &mut clock, None);
        Recover::start(PORT_SARIM, &mut cx, &mut logs)
    };
    assert!(
        recovery.is_err(),
        "a walkable but unreachable stand must not start an approach"
    );
    assert!(
        ops.is_empty(),
        "no unreachable stand walk is emitted: {ops:?}"
    );
}

#[test]
fn first_luthas_approach_uses_an_orthogonal_stand_from_a_diagonal_tile() {
    reset();
    let mut world = World::new(MUSA, 5);
    let luthas = world.facts.luthas_anchor;
    let npc = WorldTile {
        x: luthas.x,
        z: luthas.z + 1,
        level: luthas.level,
    };
    world.here = WorldTile {
        x: npc.x + 1,
        z: npc.z + 1,
        level: npc.level,
    };
    assert_eq!(world.here.x.abs_diff(npc.x), 1);
    assert_eq!(world.here.z.abs_diff(npc.z), 1);
    install_luthas_reach_view(luthas, None);
    world.post();
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    let (_, _, _, ops) = start_recovery();
    assert!(matches!(ops.as_slice(), [InteractReq::Walk { .. }]));
    let stand = walk_tile(&ops[0]);
    assert!(
        orthogonally_adjacent(stand, npc),
        "first approach stand {stand:?} must be orthogonal to posted Luthas {npc:?}"
    );
}

#[test]
fn initial_luthas_move_stages_without_a_posted_row_then_replans_orthogonally() {
    reset();
    let mut world = World::new(MUSA, 5);
    let anchor = world.facts.luthas_anchor;
    let npc = WorldTile {
        x: anchor.x,
        z: anchor.z + 1,
        level: anchor.level,
    };
    world.here = WorldTile {
        x: anchor.x + 16,
        z: anchor.z - 6,
        level: anchor.level,
    };
    world.luthas_posted = false;
    install_luthas_staging_reach_view(anchor);
    world.post();
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));

    let mut ops = Vec::new();
    let mut clock = InstantTaskClock::new();
    let mut logs = VecDeque::new();
    let mut recover = {
        let mut cx = Cx::test(&mut ops, &mut clock, None);
        Recover::start(PORT_SARIM, &mut cx, &mut logs)
            .expect("a no-row recovery starts with a staging walk")
    };
    assert!(
        matches!(ops.as_slice(), [op] if walk_near(anchor, 2)(op)),
        "a no-row recovery walks to Luthas's spawn tile: {ops:?}"
    );
    let staging_tile = walk_tile(&ops[0]);
    assert_eq!(staging_tile, anchor);
    assert!(
        !ops.iter().any(|op| matches!(op, InteractReq::Npc { .. })),
        "staging does not talk to Luthas"
    );

    world.here = staging_tile;
    world.luthas_posted = true;
    world.post();
    world.tick += 1;
    post_walk_outcome(
        world.tick,
        token(&ops[0]),
        world.here,
        staging_tile,
        2,
        false,
    );
    let (result, ops) = recovery_step_observed(&mut recover, &mut clock, &mut logs);
    assert_eq!(result, None);
    assert!(
        ops.is_empty(),
        "the staging tile is already an orthogonal talk stand: {ops:?}"
    );
    assert!(
        orthogonally_adjacent(staging_tile, npc),
        "the staging tile is orthogonal to the posted Luthas tile"
    );
    let (result, ops) = recovery_step(&mut world, &mut recover, &mut clock, &mut logs);
    assert_eq!(result, None);
    assert!(
        matches!(ops.as_slice(), [InteractReq::Npc { action, .. }] if action == "Talk-to"),
        "only the posted-row orthogonal stand dispatches Talk-to: {ops:?}"
    );
}

#[test]
fn retries_skip_a_blocked_luthas_stand() {
    reset();
    let mut world = World::new(MUSA, 5);
    let luthas = world.facts.luthas_anchor;
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    let npc = WorldTile {
        x: luthas.x,
        z: luthas.z + 1,
        level: luthas.level,
    };
    let blocked = WorldTile {
        x: npc.x + 1,
        z: npc.z,
        level: npc.level,
    };
    install_luthas_reach_view(luthas, Some(blocked));
    world.post();
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));
    let (mut recover, mut clock, mut logs, initial_ops) = start_recovery();
    assert!(matches!(initial_ops.as_slice(), [InteractReq::Walk { .. }]));
    let initial_stand = walk_tile(&initial_ops[0]);
    assert_ne!(initial_stand, blocked);
    assert!(orthogonally_adjacent(initial_stand, npc));

    world.here = initial_stand;
    assert!(
        recovery_step(&mut world, &mut recover, &mut clock, &mut logs)
            .1
            .is_empty()
    );
    let talk_ops = recovery_step(&mut world, &mut recover, &mut clock, &mut logs).1;
    assert!(matches!(
        talk_ops.as_slice(),
        [InteractReq::Npc { action, .. }] if action == "Talk-to"
    ));
    world.say("I can't reach that!");
    let (result, ops) = recovery_step(&mut world, &mut recover, &mut clock, &mut logs);
    assert_eq!(result, None);
    assert!(matches!(ops.as_slice(), [InteractReq::Walk { .. }]));
    let stand = walk_tile(&ops[0]);
    assert_ne!(
        stand, blocked,
        "the blocked orthogonal stand is never selected"
    );
    assert!(orthogonally_adjacent(stand, npc));
}

#[test]
fn dialogue_timeout_retries_within_the_talk_budget() {
    reset();
    let mut world = World::new(MUSA, 5);
    let luthas = world.facts.luthas_anchor;
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    install_luthas_reach_view(luthas, None);
    world.post();
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));
    let (mut recover, mut clock, mut logs, initial_ops) = start_recovery();
    let mut walk = initial_ops.into_iter().next().expect("initial talk walk");
    assert!(is_walk(&walk));
    let initial_stand = walk_tile(&walk);
    let npc = WorldTile {
        x: luthas.x,
        z: luthas.z + 1,
        level: luthas.level,
    };
    assert!(orthogonally_adjacent(initial_stand, npc));

    for attempt in 0..3 {
        world.here = if attempt == 0 {
            initial_stand
        } else {
            walk_tile(&walk)
        };
        assert!(
            recovery_step(&mut world, &mut recover, &mut clock, &mut logs)
                .1
                .is_empty()
        );
        let talk_ops = recovery_step(&mut world, &mut recover, &mut clock, &mut logs).1;
        assert!(matches!(
            talk_ops.as_slice(),
            [InteractReq::Npc { action, .. }] if action == "Talk-to"
        ));

        clock.deadline = Some(clock.now());
        let (result, ops) = recovery_step(&mut world, &mut recover, &mut clock, &mut logs);
        if attempt < 2 {
            assert_eq!(result, None, "dialogue timeout {attempt} retries");
            assert!(matches!(ops.as_slice(), [InteractReq::Walk { .. }]));
            walk = ops[0].clone();
        } else {
            assert_eq!(result, Some(false), "the third timeout ends recovery");
            assert!(ops.is_empty());
        }
    }

    assert_eq!(
        logs.iter()
            .filter(|line| line.contains("failed cause=dialogue-timeout"))
            .count(),
        3
    );
    assert!(logs.back().is_some_and(|line| {
        line.contains("recovery ended after attempt 3/3")
            && line.contains("never opened a dialogue")
    }));
}

#[test]
fn a_failed_talk_walk_retries_and_logs_the_terminal_failure() {
    reset();
    let mut world = World::new(MUSA, 5);
    let luthas = world.facts.luthas_anchor;
    let coins = world.facts.coins;
    let fare = world.facts.coin_payout;
    install_luthas_reach_view(luthas, None);
    fail_short(&mut world, 1, PORT_SARIM, 2, (coins, fare));
    world.post();
    let (mut recover, mut clock, mut logs, initial_ops) = start_recovery();
    let mut walk = initial_ops.into_iter().next().expect("initial talk walk");
    assert!(is_walk(&walk));
    let first_stand = walk_tile(&walk);
    let npc = WorldTile {
        x: luthas.x,
        z: luthas.z + 1,
        level: luthas.level,
    };
    assert!(orthogonally_adjacent(first_stand, npc));

    for attempt in 0..3 {
        let dest = walk_tile(&walk);
        let radius = 0;
        world.tick += 1;
        post_walk_outcome(world.tick, token(&walk), world.here, dest, radius, true);
        let (result, ops) = recovery_step_observed(&mut recover, &mut clock, &mut logs);
        if attempt < 2 {
            assert_eq!(result, None, "failed walk {attempt} retries");
            assert!(matches!(ops.as_slice(), [InteractReq::Walk { .. }]));
            let retry = ops[0].clone();
            assert_ne!(walk_tile(&retry), dest, "retry chooses another stand");
            walk = retry;
        } else {
            assert_eq!(result, Some(false), "the third failed walk ends recovery");
            assert!(ops.is_empty());
        }
    }

    assert_eq!(
        logs.iter()
            .filter(|line| line.contains("failed cause=walk"))
            .count(),
        3
    );
    assert!(logs.back().is_some_and(|line| {
        line.contains("recovery ended after attempt 3/3")
            && line.contains("did not reach the selected stand")
    }));
}

#[test]
fn an_interrupt_during_the_re_walk_stops_it_and_ends_false() {
    // Frozen follow pass: EventSignal.pending ends the walk (WalkExecutor.ts:844-853).
    let (h, mut world, _, again) = earn_the_fare(None);
    world.ours = true;
    world.post();
    assert_eq!(
        tick(),
        vec![InteractReq::AbortWalk {
            request_id: token(&again)
        }],
        "the interrupted re-walk stops its host follow"
    );
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
}

#[test]
fn walk_to_does_not_recover_a_short_that_is_not_the_boat_fare() {
    for (from, count) in [
        // Karamja, but the missing gate is a different coin count.
        (MUSA, 10),
        // The fare row, but walking from the mainland.
        (
            WorldTile {
                x: 3100,
                z: 3200,
                level: 0,
            },
            selected_facts().coin_payout,
        ),
    ] {
        reset();
        let mut world = World::new(from, 5);
        let coins_id = world.facts.coins;
        let h = start_walk_to(&mut world);
        let first = machine::merge_ops(Vec::new());
        fail_short(
            &mut world,
            token(&first[0]),
            PORT_SARIM,
            2,
            (coins_id, count),
        );
        let retry = tick();
        assert!(
            matches!(retry.as_slice(), [op] if walk_near(PORT_SARIM, 2)(op)),
            "{count} coins from {from:?}: no fare recovery, but a fresh walk: {retry:?}"
        );

        world.tick += 1;
        post_walk_outcome(
            world.tick,
            token(&retry[0]),
            world.here,
            PORT_SARIM,
            2,
            true,
        );
        assert!(tick().is_empty());
        assert_eq!(
            machine::take(h),
            Take::Settled(Outcome::Done(json!(false))),
            "the bounded retry returns the second failed walk"
        );
    }
}

#[test]
fn walk_to_retries_a_failed_initial_walk_without_missing_fare_evidence() {
    reset();
    let mut world = World::new(BRIMHAVEN_CART, 5);
    let h = start_walk_to(&mut world);
    let first = machine::merge_ops(Vec::new());
    let first_token = token(&first[0]);
    world.tick += 1;
    post_walk_outcome(
        world.tick,
        first_token,
        world.here,
        PORT_SARIM,
        2,
        true,
    );

    let retry = tick();
    assert!(
        matches!(retry.as_slice(), [op] if walk_near(PORT_SARIM, 2)(op)),
        "a failed walk without the exact fare signal gets a fresh route: {retry:?}"
    );
    let retry_token = token(&retry[0]);
    assert_ne!(retry_token, first_token, "the retry uses a fresh request");
    assert_eq!(machine::take(h), Take::Pending);

    world.tick += 1;
    post_walk_outcome(
        world.tick,
        retry_token,
        world.here,
        PORT_SARIM,
        2,
        true,
    );
    assert!(tick().is_empty());
    assert_eq!(
        machine::take(h),
        Take::Settled(Outcome::Done(json!(false))),
        "the second failed route spends the retry budget"
    );
}

#[test]
fn walk_to_with_a_full_pack_and_no_banana_does_not_recover() {
    reset();
    let mut world = World::new(MUSA, 5);
    world.filler = 27;
    let coins_id = world.facts.coins;
    let fare = world.facts.coin_payout;
    let h = start_walk_to(&mut world);
    let first = machine::merge_ops(Vec::new());
    fail_short(
        &mut world,
        token(&first[0]),
        PORT_SARIM,
        2,
        (coins_id, fare),
    );
    let retry = tick();
    assert!(
        matches!(retry.as_slice(), [op] if walk_near(PORT_SARIM, 2)(op)),
        "no free slot prevents fare recovery but not the fresh route: {retry:?}"
    );

    world.tick += 1;
    post_walk_outcome(
        world.tick,
        token(&retry[0]),
        world.here,
        PORT_SARIM,
        2,
        true,
    );
    assert!(tick().is_empty());
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
}

#[test]
fn walk_resilient_baked_failure_short_of_the_fare_goes_to_luthas() {
    reset();
    let mut world = World::new(MUSA, 5);
    let coins_id = world.facts.coins;
    let luthas = world.facts.luthas_anchor;
    let fare = world.facts.coin_payout;
    install_luthas_reach_view(luthas, None);
    world.post();
    let args = json!({
        "tile": { "x": PORT_SARIM.x, "z": PORT_SARIM.z, "level": 0 },
        "opts": { "radius": 2 },
    });
    let Started::Running(h) = machine::start("walk-resilient", args, Vec::new(), 0) else {
        panic!("walk-resilient runs");
    };
    machine::step(&mut NoJs);
    let baked = machine::merge_ops(Vec::new());
    assert!(
        matches!(baked.as_slice(), [op] if walk_near(PORT_SARIM, 2)(op)),
        "{baked:?}"
    );
    fail_short(
        &mut world,
        token(&baked[0]),
        PORT_SARIM,
        2,
        (coins_id, fare),
    );
    let ops = tick();
    assert!(matches!(ops.as_slice(), [InteractReq::Walk { .. }]));
    let stand = walk_tile(&ops[0]);
    let npc = WorldTile {
        x: luthas.x,
        z: luthas.z + 1,
        level: luthas.level,
    };
    assert!(
        orthogonally_adjacent(stand, npc),
        "the baked walk recovery chooses a stand next to Luthas: {ops:?}"
    );
    assert_eq!(machine::take(h), Take::Pending);
}

#[test]
fn walk_to_waits_out_the_frozen_300_second_default() {
    // Frozen `opts?.timeoutMs ?? 300_000` (WalkExecutor.ts:228).
    reset();
    let mut world = World::new(
        WorldTile {
            x: 3100,
            z: 3200,
            level: 0,
        },
        5,
    );
    let h = start_walk_to(&mut world);
    let first = machine::merge_ops(Vec::new());
    assert!(
        matches!(first.as_slice(), [op] if walk_near(PORT_SARIM, 2)(op)),
        "{first:?}"
    );
    machine::age(h, 60_001);
    world.post();
    assert!(tick().is_empty(), "no stop at a minute");
    assert_eq!(machine::take(h), Take::Pending);
    machine::age(h, 240_000);
    world.post();
    let retry = tick();
    assert!(
        matches!(
            retry.as_slice(),
            [InteractReq::AbortWalk { request_id }, op]
                if *request_id == token(&first[0]) && walk_near(PORT_SARIM, 2)(op)
        ),
        "the failed first walk aborts and starts one fresh host walk: {retry:?}"
    );
    let retry_token = token(&retry[1]);
    assert_eq!(
        machine::take(h),
        Take::Pending,
        "the first failed walk does not end walkTo"
    );

    machine::age(h, 300_000);
    world.post();
    assert_eq!(
        tick(),
        vec![InteractReq::AbortWalk {
            request_id: retry_token
        }],
        "the one fresh host walk keeps the frozen 300-second bound"
    );
    assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
}
