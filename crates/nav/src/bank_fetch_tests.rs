use std::collections::{HashMap, HashSet};

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use super::{
    bank_access_tiles, fetchable_state, nearest_bank_access, plan_bank_fetch as plan_with, BankStep,
};
use crate::collision::{pack_walk, WorldCollision};
use crate::pack::{BankAccess, BankStand};
use crate::router::{
    find_first_with, find_missing_item_reqs, find_with, FindOptions, MissingReq, RouteError,
};
use crate::transport::{TransportEdge, TransportGraph, TransportKind};
use crate::world_state::WorldState;

/// The knife obj id (a slash-weapon web hop carries it in the packed
/// graph).
const KNIFE: i32 = 946;

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

/// A `width × height` level-0 bake at (0,0) with the given per-tile
/// flags OR'd in.
fn bake(width: usize, height: usize, extras: &[(i32, i32, u32)]) -> WorldCollision {
    let mut plane = vec![0u32; width * height];
    for &(x, z, f) in extras {
        plane[z as usize * width + x as usize] |= f;
    }
    let mut flags = vec![0u32; 4 * plane.len()];
    flags[..plane.len()].copy_from_slice(&plane);
    let (walk, blocked) = pack_walk(&flags);
    WorldCollision {
        origin: tile(0, 0, 0),
        width,
        height,
        walk,
        blocked,
        flags: None,
    }
}

/// The 5×5 grid split between x=1 and x=2: `W_E` on column 1 and
/// `W_W` on column 2, so no step (or diagonal) crosses.
fn walled_5x5() -> WorldCollision {
    let mut extras = Vec::new();
    for z in 0..5 {
        extras.push((1, z, CollisionFlag::W_E as u32));
        extras.push((2, z, CollisionFlag::W_W as u32));
    }
    bake(5, 5, &extras)
}

/// One door crossing the wall, gated on a worn knife.
fn knife_graph() -> TransportGraph {
    let edge = TransportEdge {
        kind: TransportKind::Door,
        at: tile(1, 2, 0),
        to: tile(2, 2, 0),
        loc_id: 1530,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![KNIFE],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let mut graph = TransportGraph::default();
    graph.at.entry(edge.at).or_default().push(0);
    graph.edges.push(edge);
    graph
}

/// The same door gated on a 10-coin toll instead.
fn toll_graph() -> TransportGraph {
    let mut g = knife_graph();
    g.edges[0].worn_req = vec![];
    g.edges[0].item_req = vec![(995, 10)];
    g
}

/// A bank booth stand at (`x`, `z`).
fn stand(x: i32, z: i32) -> BankStand {
    BankStand {
        name: "Bank booth".into(),
        tile: tile(x, z, 0),
        access: BankAccess::Booth { op: 2 },
    }
}

fn open_grid() -> WorldCollision {
    bake(10, 10, &[])
}

fn plan_bank_fetch(
    missing: &[MissingReq],
    state: &WorldState,
    bank: &[(i32, i32)],
    stands: &[BankStand],
    from: WorldTile,
) -> Option<super::BankFetch> {
    plan_with(missing, state, bank, stands, from, &open_grid())
}

fn access_walk(stands: &[BankStand], from: WorldTile) -> BankStep {
    let tile = nearest_bank_access(&open_grid(), stands, from).expect("access tile");
    BankStep::Walk {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}

/// `worn_req` with the knife already carried plans a bare Wear — no
/// bank walk, no open/deposit/withdraw/close — and the post-session
/// strict re-find crosses.
#[test]
fn worn_req_with_knife_in_inventory_wears_in_place() {
    let wc = walled_5x5();
    let g = knife_graph();
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let state = WorldState {
        inv: HashMap::from([(KNIFE, 1)]),
        ..WorldState::default()
    };
    // The strict find still fails closed with the knife merely
    // carried — a bare `find_with` never wears it.
    assert!(matches!(
        find_with(&wc, &g, from, to, FindOptions::default(), &state),
        Err(RouteError::NoPath)
    ));
    let missing = find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &state)
        .expect("only the worn knife is missing");
    assert_eq!(missing, vec![MissingReq::WearAny { ids: vec![KNIFE] }]);
    let fetch = plan_bank_fetch(&missing, &state, &[], &[stand(4, 0)], from)
        .expect("a carried knife plans a bare wear");
    assert_eq!(
        fetch.steps,
        vec![BankStep::Wear { id: KNIFE }],
        "no bank trip: wear the carried knife in place"
    );
    assert!(
        fetch.state.worn.contains(&KNIFE),
        "the post-session state wears the knife"
    );
    assert!(
        !fetch.state.inv.contains_key(&KNIFE),
        "the worn knife leaves the inventory"
    );
    let r = find_with(&wc, &g, from, to, FindOptions::default(), &fetch.state).unwrap();
    assert_eq!(r.dest, to);
}

/// Session unit: the inventory is full of junk and the knife is in
/// the bank snapshot — the plan deposits the backpack, withdraws the
/// knife, wears it, closes, and the post-session strict re-find
/// crosses.
#[test]
fn bank_trip_deposits_withdraws_wears_then_finds() {
    let wc = walled_5x5();
    let g = knife_graph();
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let state = WorldState {
        inv: HashMap::from([(1, 3), (2, 2)]), // the junk backpack, no knife
        ..WorldState::default()
    };
    assert!(matches!(
        find_with(&wc, &g, from, to, FindOptions::default(), &state),
        Err(RouteError::NoPath)
    ));
    let missing = find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &state)
        .expect("only the worn knife is missing");
    assert_eq!(missing, vec![MissingReq::WearAny { ids: vec![KNIFE] }]);
    // The bank snapshot holds the knife (1).
    let bank = [(KNIFE, 1)];
    let fetch = plan_bank_fetch(&missing, &state, &bank, &[stand(4, 0)], from)
        .expect("the banked knife plans a full trip");
    assert_eq!(
        fetch.steps,
        vec![
            access_walk(&[stand(4, 0)], tile(0, 0, 0)),
            BankStep::Open,
            BankStep::DepositAll,
            BankStep::Withdraw {
                id: KNIFE,
                count: 1
            },
            BankStep::Wear { id: KNIFE },
            BankStep::Close,
        ],
        "walk, open, deposit the junk, withdraw the knife, wear, close"
    );
    assert!(fetch.state.inv.is_empty(), "the junk stays deposited");
    assert!(
        fetch.state.worn.contains(&KNIFE),
        "the knife is worn after the trip"
    );
    let r = find_with(&wc, &g, from, to, FindOptions::default(), &fetch.state).unwrap();
    assert_eq!(r.dest, to);
}

/// A missing `item_req` count (the 10-coin toll) withdraws the
/// needed stack from the bank; no Wear step is planned — the item is
/// carried, not worn.
#[test]
fn bank_trip_withdraws_an_item_req_stack_without_wearing() {
    let wc = walled_5x5();
    let g = toll_graph();
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let state = WorldState {
        inv: HashMap::from([(1, 3)]),
        ..WorldState::default()
    };
    let missing = find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &state)
        .expect("only the toll count is missing");
    assert_eq!(missing, vec![MissingReq::Carry { id: 995, count: 10 }]);
    let fetch = plan_bank_fetch(&missing, &state, &[(995, 50)], &[stand(4, 0)], from)
        .expect("the bank covers the toll");
    assert_eq!(
        fetch.steps,
        vec![
            access_walk(&[stand(4, 0)], tile(0, 0, 0)),
            BankStep::Open,
            BankStep::DepositAll,
            BankStep::Withdraw { id: 995, count: 10 },
            BankStep::Close,
        ],
        "withdraw the toll stack; nothing is worn"
    );
    assert_eq!(fetch.state.inv.get(&995), Some(&10));
    let r = find_with(&wc, &g, from, to, FindOptions::default(), &fetch.state).unwrap();
    assert_eq!(r.dest, to);
}

/// A route needing both a carried stack and a worn item plans one
/// trip that withdraws both after the deposit.
#[test]
fn one_trip_withdraws_multiple_missing_reqs() {
    let wc = walled_5x5();
    let mut g = toll_graph();
    g.edges[0].worn_req = vec![KNIFE];
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let state = WorldState::default();
    let missing = find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &state)
        .expect("only carry/wear facts are missing");
    assert_eq!(
        missing,
        vec![
            MissingReq::WearAny { ids: vec![KNIFE] },
            MissingReq::Carry { id: 995, count: 10 },
        ]
    );
    let fetch = plan_bank_fetch(
        &missing,
        &state,
        &[(995, 50), (KNIFE, 1)],
        &[stand(4, 0)],
        from,
    )
    .expect("the bank covers both");
    assert!(fetch
        .steps
        .contains(&BankStep::Withdraw { id: 995, count: 10 }));
    assert!(fetch.steps.contains(&BankStep::Withdraw {
        id: KNIFE,
        count: 1
    }));
    assert!(fetch.steps.contains(&BankStep::Wear { id: KNIFE }));
    assert_eq!(fetch.state.inv.get(&995), Some(&10));
    assert!(fetch.state.worn.contains(&KNIFE));
    let r = find_with(&wc, &g, from, to, FindOptions::default(), &fetch.state).unwrap();
    assert_eq!(r.dest, to);
}

/// `worn_req` is any-of: the session fetches whichever alternative
/// the bank actually holds — a bank with only the scimitar plans a
/// scimitar trip, not a NoPath for the sword.
#[test]
fn bank_trip_fetches_any_one_worn_alternative() {
    let wc = walled_5x5();
    let mut g = knife_graph();
    g.edges[0].worn_req = vec![1277, 1321]; // bronze sword, bronze scimitar
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let state = WorldState {
        inv: HashMap::from([(1, 3)]),
        ..WorldState::default()
    };
    let missing = find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &state)
        .expect("only the worn blade is missing");
    assert_eq!(
        missing,
        vec![MissingReq::WearAny {
            ids: vec![1277, 1321],
        }]
    );
    // The bank holds only the scimitar: that one is fetched.
    let bank = [(1321, 1)];
    let fetch = plan_bank_fetch(&missing, &state, &bank, &[stand(4, 0)], from)
        .expect("one banked alternative is enough");
    assert_eq!(
        fetch.steps,
        vec![
            access_walk(&[stand(4, 0)], tile(0, 0, 0)),
            BankStep::Open,
            BankStep::DepositAll,
            BankStep::Withdraw { id: 1321, count: 1 },
            BankStep::Wear { id: 1321 },
            BankStep::Close,
        ],
        "the banked alternative is withdrawn and worn"
    );
    let r = find_with(&wc, &g, from, to, FindOptions::default(), &fetch.state).unwrap();
    assert_eq!(r.dest, to);
}

/// The deposit supplies the bank with the carried stack: a carried
/// knife (no bank row) plus banked coins plans a trip that withdraws
/// and wears the knife after the deposit — it must not fail closed.
#[test]
fn bank_trip_supply_includes_the_deposited_backpack() {
    let wc = walled_5x5();
    let mut g = toll_graph();
    g.edges[0].worn_req = vec![KNIFE];
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let state = WorldState {
        inv: HashMap::from([(KNIFE, 1)]),
        ..WorldState::default()
    };
    let missing = find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &state)
        .expect("the worn knife and the toll count are missing");
    assert_eq!(
        missing,
        vec![
            MissingReq::WearAny { ids: vec![KNIFE] },
            MissingReq::Carry { id: 995, count: 10 },
        ]
    );
    // The bank holds coins only — the knife is carried, not banked.
    let bank = [(995, 50)];
    let fetch = plan_bank_fetch(&missing, &state, &bank, &[stand(4, 0)], from)
        .expect("the deposit supplies the knife");
    assert_eq!(
        fetch.steps,
        vec![
            access_walk(&[stand(4, 0)], tile(0, 0, 0)),
            BankStep::Open,
            BankStep::DepositAll,
            BankStep::Withdraw {
                id: KNIFE,
                count: 1
            },
            BankStep::Wear { id: KNIFE },
            BankStep::Withdraw { id: 995, count: 10 },
            BankStep::Close,
        ],
        "the deposited knife is withdrawn and worn, then the toll stack"
    );
    let r = find_with(&wc, &g, from, to, FindOptions::default(), &fetch.state).unwrap();
    assert_eq!(r.dest, to);
}

/// A missing req whose item is neither carried nor in the bank, or an
/// item_req the bank cannot cover in full, fails closed to `None` —
/// the caller reports `NoPath`.
#[test]
fn plan_fails_closed_when_the_item_is_nowhere() {
    let state = WorldState {
        inv: HashMap::from([(1, 3)]),
        ..WorldState::default()
    };
    assert_eq!(
        plan_bank_fetch(
            &[MissingReq::WearAny { ids: vec![KNIFE] }],
            &state,
            &[],
            &[stand(4, 0)],
            tile(0, 0, 0),
        ),
        None,
        "a knife in neither inventory nor bank plans no session"
    );
    let bank_short = [(995, 5)];
    assert_eq!(
        plan_bank_fetch(
            &[MissingReq::Carry { id: 995, count: 10 }],
            &state,
            &bank_short,
            &[stand(4, 0)],
            tile(0, 0, 0),
        ),
        None,
        "a 5-coin bank cannot cover a 10-coin toll"
    );
    assert_eq!(
        plan_bank_fetch(
            &[MissingReq::WearAny { ids: vec![KNIFE] }],
            &state,
            &[(KNIFE, 1)],
            &[],
            tile(0, 0, 0),
        ),
        None,
        "no stand, no trip"
    );
}

/// The session walks to an access tile of the nearest bank stand, not
/// the stand's own tile.
#[test]
fn plans_walk_to_the_nearest_stand() {
    let stands = [stand(9, 9), stand(4, 0)];
    let from = tile(0, 0, 0);
    let fetch = plan_bank_fetch(
        &[MissingReq::WearAny { ids: vec![KNIFE] }],
        &WorldState::default(),
        &[(KNIFE, 1)],
        &stands,
        from,
    )
    .expect("a banked knife plans a trip");
    assert_eq!(
        fetch.steps[0],
        access_walk(&[stand(4, 0)], from),
        "the nearest stand's access tile wins"
    );
    let BankStep::Walk { x, z, level } = fetch.steps[0] else {
        panic!("expected a Walk");
    };
    assert_ne!(
        (x, z, level),
        (4, 0, 0),
        "Walk dest must not be the stand tile"
    );
    assert!(
        open_grid().standable(tile(x, z, level)),
        "Walk dest must be standable"
    );
}

/// An empty diagnosis means nothing to fetch: no session.
#[test]
fn plan_with_no_missing_reqs_is_none() {
    assert_eq!(
        plan_bank_fetch(
            &[],
            &WorldState::default(),
            &[],
            &[stand(4, 0)],
            tile(0, 0, 0),
        ),
        None
    );
}

/// The post-session state keeps the player's other facts (a worn
/// item and a skill level survive the deposit).
#[test]
fn bank_trip_keeps_worn_and_skill_facts() {
    let state = WorldState {
        inv: HashMap::from([(1, 3)]),
        worn: HashSet::from([1712]), // a charged glory stays worn
        stats: HashMap::from([(6, 25)]),
        ..WorldState::default()
    };
    let fetch = plan_bank_fetch(
        &[MissingReq::Carry { id: 995, count: 10 }],
        &state,
        &[(995, 50)],
        &[stand(4, 0)],
        tile(0, 0, 0),
    )
    .expect("the bank covers the toll");
    assert!(fetch.state.worn.contains(&1712), "worn facts survive");
    assert_eq!(fetch.state.stats.get(&6), Some(&25), "skills survive");
    assert_eq!(fetch.state.inv.get(&995), Some(&10));
    assert!(!fetch.state.inv.contains_key(&1), "the junk is deposited");
}

/// The fetchable facts open exactly the gates a session can meet, and the
/// planner plans each one's missing fact: a carried obj is worn in place
/// without a stand; banked rows count only with a stand to walk to, and
/// then add to the carried stack; an obj held nowhere stays refused. Beside
/// a full bank stack, a carried coin stays a fact (the combined count
/// saturates rather than wrapping) for a wear-only session, and the
/// combined stack still meets a full-stack requirement.
#[test]
fn fetchable_state_opens_exactly_the_gates_a_session_can_meet() {
    let door = |item_req: Vec<(i32, i32)>, worn_req: Vec<i32>| TransportEdge {
        item_req,
        worn_req,
        ..knife_graph().edges[0].clone()
    };
    let state = WorldState {
        inv: HashMap::from([(KNIFE, 1), (995, 4)]),
        ..WorldState::default()
    };
    let bank = [(995, 6), (1277, 1)];
    let cases = [
        (
            door(vec![], vec![KNIFE]),
            MissingReq::WearAny { ids: vec![KNIFE] },
            true,
            true,
        ),
        (
            door(vec![], vec![1277]),
            MissingReq::WearAny { ids: vec![1277] },
            false,
            true,
        ),
        (
            door(vec![(995, 10)], vec![]),
            MissingReq::Carry { id: 995, count: 10 },
            false,
            true,
        ),
        (
            door(vec![(995, 11)], vec![]),
            MissingReq::Carry { id: 995, count: 11 },
            false,
            false,
        ),
        (
            door(vec![], vec![2]),
            MissingReq::WearAny { ids: vec![2] },
            false,
            false,
        ),
    ];
    let coins = WorldState {
        inv: HashMap::from([(KNIFE, 1), (995, 1)]),
        ..WorldState::default()
    };
    let full_stack = [(995, i32::MAX)];
    let full_stack_cases = [
        (
            door(vec![(995, 1)], vec![KNIFE]),
            MissingReq::WearAny { ids: vec![KNIFE] },
            true,
            true,
        ),
        (
            door(vec![(995, i32::MAX)], vec![]),
            MissingReq::Carry {
                id: 995,
                count: i32::MAX,
            },
            false,
            true,
        ),
    ];
    for (state, bank, cases) in [
        (&state, &bank[..], &cases[..]),
        (&coins, &full_stack[..], &full_stack_cases[..]),
    ] {
        for stands in [Vec::new(), vec![stand(4, 0)]] {
            let fetchable = fetchable_state(state, bank, &stands);
            for (edge, missing, without_stand, with_stand) in cases {
                let expected = if stands.is_empty() {
                    *without_stand
                } else {
                    *with_stand
                };
                assert!(!state.allows(edge));
                assert_eq!(
                    fetchable.allows(edge),
                    expected,
                    "{missing:?} stands={}",
                    stands.len()
                );
                assert_eq!(
                    plan_bank_fetch(
                        std::slice::from_ref(missing),
                        state,
                        bank,
                        &stands,
                        tile(0, 0, 0)
                    )
                    .is_some(),
                    expected,
                    "the planner agrees on {missing:?} stands={}",
                    stands.len()
                );
            }
        }
    }
}

/// Access tiles are orthogonal standable neighbours; the stand tile is
/// never one of them, even when it itself is standable.
#[test]
fn bank_access_tiles_are_standable_neighbours_not_the_stand() {
    let wc = bake(5, 5, &[(2, 2, CollisionFlag::SQ_BLOCKED as u32)]);
    let booth = stand(2, 2);
    let tiles = bank_access_tiles(&wc, &booth);
    assert!(
        !tiles.contains(&booth.tile),
        "the booth loc tile is not an access tile"
    );
    assert!(tiles.contains(&tile(3, 2, 0)));
    assert!(tiles.contains(&tile(1, 2, 0)));
    assert!(tiles.contains(&tile(2, 3, 0)));
    assert!(tiles.contains(&tile(2, 1, 0)));
    assert_eq!(tiles.len(), 4);
}

/// On the real 289 pack, BankBudget walks to a standable access tile from
/// the customer street — not onto a booth loc or a teller spawn.
#[test]
fn packed_bank_fetch_walks_to_access_from_customer_streets() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let samples = [
        ("Varrock West", tile(3180, 3430, 0)),
        ("Draynor", tile(3088, 3240, 0)),
        ("Falador East", tile(3008, 3352, 0)),
        ("Al Kharid", tile(3264, 3163, 0)),
    ];
    let opts = FindOptions::default();
    let state = WorldState::default();
    for (name, from) in samples {
        assert!(
            world.collision.standable(from),
            "{name} street {from:?} must be standable"
        );
        let nearby: Vec<_> = world
            .banks()
            .iter()
            .filter(|stand| {
                stand.tile.level == from.level
                    && (stand.tile.x - from.x)
                        .abs()
                        .max((stand.tile.z - from.z).abs())
                        <= 16
            })
            .cloned()
            .collect();
        assert!(
            !nearby.is_empty(),
            "{name}: packed stands within 16 of {from:?}"
        );
        for stand in &nearby {
            if matches!(stand.access, BankAccess::Booth { .. }) {
                assert!(
                    find_with(
                        &world.collision,
                        &world.graph,
                        from,
                        stand.tile,
                        opts,
                        &state,
                    )
                    .is_err(),
                    "{name}: booth {} at {:?} must not be a walk dest",
                    stand.name,
                    stand.tile
                );
            }
        }
        let access = nearest_bank_access(&world.collision, world.banks(), from)
            .unwrap_or_else(|| panic!("{name}: a packed access tile from {from:?}"));
        assert!(
            world.collision.standable(access),
            "{name}: access {access:?} must be standable"
        );
        assert!(
            world.banks().iter().all(|stand| stand.tile != access),
            "{name}: access {access:?} must not be a packed stand tile"
        );
        let fetch = plan_with(
            &[MissingReq::Carry { id: 995, count: 1 }],
            &state,
            &[(995, 1)],
            world.banks(),
            from,
            &world.collision,
        )
        .unwrap_or_else(|| panic!("{name}: plan from {from:?}"));
        let BankStep::Walk { x, z, level } = fetch.steps[0] else {
            panic!("{name}: expected a Walk");
        };
        assert!(
            world
                .banks()
                .iter()
                .all(|stand| stand.tile != WorldTile { x, z, level }),
            "{name}: planned Walk ({x},{z},{level}) must not be a stand tile"
        );
        let access_tiles: Vec<_> = nearby
            .iter()
            .flat_map(|stand| bank_access_tiles(&world.collision, stand))
            .collect();
        find_first_with(
            &world.collision,
            &world.graph,
            from,
            &access_tiles,
            opts,
            &state,
        )
        .into_route()
        .unwrap_or_else(|err| {
            panic!("{name}: some nearby access tile must be routable from {from:?}: {err:?}")
        });
    }
}
