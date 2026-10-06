use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use api::bank_memory::Origin;
use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use super::{
    bank_access_tiles, fetchable_state, nearest_bank_access, plan_bank_fetch as plan_with,
    BankRows, BankStep,
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
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: tile(1, 2, 0),
        to: tile(2, 2, 0),
        loc_id: 1530,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
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
    g.edges[0].consumed_req = vec![(995, 10)];
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
/// bank walk, no open/withdraw/close — and the post-session strict re-find
/// crosses.
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

/// The knife is in the bank snapshot while the backpack holds unrelated
/// items. The plan withdraws only the knife, closes the bank, then wears it;
/// carried inventory is preserved, and the post-session strict re-find
/// crosses.
#[test]
fn bank_trip_withdraws_wears_then_finds() {
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
            BankStep::Withdraw {
                id: KNIFE,
                count: 1
            },
            BankStep::Close,
            BankStep::Wear { id: KNIFE },
        ],
        "walk, open, withdraw the knife, close, wear"
    );
    assert_eq!(
        fetch.state.inv, state.inv,
        "unrelated backpack items stay carried"
    );
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
            BankStep::Withdraw { id: 995, count: 10 },
            BankStep::Close,
        ],
        "withdraw the toll stack; nothing is worn"
    );
    assert_eq!(fetch.state.inv.get(&995), Some(&10));
    let r = find_with(&wc, &g, from, to, FindOptions::default(), &fetch.state).unwrap();
    assert_eq!(r.dest, to);
}

/// A consumed gate needs the route's initial budget, so a bank trip tops up
/// only the deficit even when some of the resource is already carried.
#[test]
fn bank_trip_withdraws_the_full_consumable_budget() {
    let wc = walled_5x5();
    let mut g = toll_graph();
    g.edges[0].item_req.clear();
    g.edges[0].consumed_req = vec![(995, 60)];
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let mut inventory = HashMap::from([(995, 30)]);
    for id in 30_000..30_027 {
        inventory.insert(id, 1);
    }
    let state = WorldState {
        inv: inventory,
        ..WorldState::default()
    };
    let missing = find_missing_item_reqs(&wc, &g, from, to, FindOptions::default(), &state)
        .expect("the carried 30 coins do not cover the full consumed budget");
    assert_eq!(missing, vec![MissingReq::Carry { id: 995, count: 60 }]);
    let bank = [(995, 30)];
    let fetch = plan_bank_fetch(&missing, &state, &bank, &[stand(4, 0)], from)
        .expect("the bank covers the missing 30 coins");
    assert_eq!(
        fetch.steps,
        vec![
            access_walk(&[stand(4, 0)], from),
            BankStep::Open,
            BankStep::Withdraw { id: 995, count: 30 },
            BankStep::Close,
        ]
    );
    assert_eq!(fetch.state.inv.get(&995), Some(&60));
    for id in 30_000..30_027 {
        assert_eq!(fetch.state.inv.get(&id), Some(&1));
    }
    assert!(find_with(&wc, &g, from, to, FindOptions::default(), &fetch.state).is_ok());
}

#[test]
fn one_selected_wearer_satisfies_overlapping_any_of_gates() {
    let missing = [
        MissingReq::Carry { id: 1277, count: 1 },
        MissingReq::WearAny {
            ids: vec![1277, 1321],
        },
        MissingReq::WearAny {
            ids: vec![1277, 1205],
        },
    ];
    let state = WorldState {
        inv: HashMap::from([(1277, 1)]),
        ..WorldState::empty()
    };
    let fetch = plan_bank_fetch(
        &missing,
        &state,
        &[(1277, 1)],
        &[stand(4, 0)],
        tile(0, 0, 0),
    )
    .unwrap();
    assert_eq!(fetch.state.inv.get(&1277), Some(&1));
    assert!(fetch.state.worn.contains(&1277));
    assert_eq!(
        fetch
            .steps
            .iter()
            .filter(|step| matches!(step, BankStep::Wear { .. }))
            .count(),
        1
    );
    assert!(fetch
        .steps
        .contains(&BankStep::Withdraw { id: 1277, count: 1 }));
}

/// A route needing both a carried stack and a worn item plans one
/// trip that withdraws only both deficits.
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
            BankStep::Withdraw { id: 1321, count: 1 },
            BankStep::Close,
            BankStep::Wear { id: 1321 },
        ],
        "the banked alternative is withdrawn and worn"
    );
    let r = find_with(&wc, &g, from, to, FindOptions::default(), &fetch.state).unwrap();
    assert_eq!(r.dest, to);
}

/// The carried knife remains in place while the bank trip fetches the
/// missing coin stack; after the bank closes, the knife is worn.
/// No carried item is cleared to make room for the fetch.
#[test]
fn bank_trip_keeps_a_carried_wearable() {
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
        .expect("the carried knife is the WearAny alternative");
    assert_eq!(
        fetch.steps,
        vec![
            access_walk(&[stand(4, 0)], tile(0, 0, 0)),
            BankStep::Open,
            BankStep::Withdraw { id: 995, count: 10 },
            BankStep::Close,
            BankStep::Wear { id: KNIFE },
        ],
        "withdraw only the coin deficit and keep the knife until it is worn"
    );
    assert_eq!(fetch.state.inv.get(&995), Some(&10));
    assert!(
        !fetch.state.inv.contains_key(&KNIFE),
        "the carried knife leaves the backpack only when it is worn"
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

/// The post-session state keeps the player's other facts (worn
/// items, skills, and unrelated carried objects survive).
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
    assert_eq!(fetch.state.inv.get(&1), Some(&3), "the carried obj remains");
}

/// The fetchable facts open exactly the gates a session can meet, and the
/// planner budgets each missing fact: a carried obj is worn in place without
/// a stand; banked held or consumed resources count only with a stand to walk
/// to, then add to the carried stack; an obj held nowhere stays refused.
/// Combined counts saturate rather than wrapping for both carried and banked
/// stacks.
#[test]
fn fetchable_state_opens_exactly_the_gates_a_session_can_meet() {
    let door = |item_req: Vec<(i32, i32)>, consumed_req: Vec<(i32, i32)>, worn_req: Vec<i32>| {
        TransportEdge {
            takeoff: None,
            item_req,
            consumed_req,
            item_returns: vec![],
            worn_req,
            ..knife_graph().edges[0].clone()
        }
    };
    let state = WorldState {
        inv: HashMap::from([(KNIFE, 1), (995, 4)]),
        ..WorldState::default()
    };
    let bank = [(995, 6), (1277, 1)];
    let cases = [
        (
            door(vec![], vec![], vec![KNIFE]),
            MissingReq::WearAny { ids: vec![KNIFE] },
            true,
            true,
        ),
        (
            door(vec![], vec![], vec![1277]),
            MissingReq::WearAny { ids: vec![1277] },
            false,
            true,
        ),
        (
            door(vec![(995, 10)], vec![], vec![]),
            MissingReq::Carry { id: 995, count: 10 },
            false,
            true,
        ),
        (
            door(vec![], vec![(995, 10)], vec![]),
            MissingReq::Carry { id: 995, count: 10 },
            false,
            true,
        ),
        (
            door(vec![(995, 11)], vec![], vec![]),
            MissingReq::Carry { id: 995, count: 11 },
            false,
            false,
        ),
        (
            door(vec![], vec![], vec![2]),
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
            door(vec![(995, 1)], vec![], vec![KNIFE]),
            MissingReq::WearAny { ids: vec![KNIFE] },
            true,
            true,
        ),
        (
            door(vec![(995, i32::MAX)], vec![], vec![]),
            MissingReq::Carry {
                id: 995,
                count: i32::MAX,
            },
            false,
            true,
        ),
        (
            door(vec![], vec![(995, i32::MAX)], vec![]),
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

#[test]
fn worn_all_req_is_not_fetchable_from_inventory_or_bank() {
    let wc = walled_5x5();
    let mut graph = knife_graph();
    graph.edges[0].worn_req.clear();
    graph.edges[0].worn_all_req = vec![KNIFE];
    let edge = graph.edges[0].clone();
    let from = tile(0, 0, 0);
    let to = tile(4, 4, 0);
    let stands = [stand(4, 0)];
    let inventory = WorldState {
        inv: HashMap::from([(KNIFE, 1)]),
        ..WorldState::empty()
    };
    let banked = WorldState::empty();
    let bank = [(KNIFE, 1)];

    for (state, bank_items) in [(&inventory, &[][..]), (&banked, &bank[..])] {
        assert!(matches!(
            find_with(&wc, &graph, from, to, FindOptions::default(), state),
            Err(RouteError::NoPath)
        ));
        assert!(
            find_missing_item_reqs(&wc, &graph, from, to, FindOptions::default(), state).is_none(),
            "the carry/wear diagnosis cannot cross worn_all_req"
        );
        assert!(
            !fetchable_state(state, bank_items, &stands).allows(&edge),
            "a carried or banked candidate never becomes observed equipment"
        );
    }
}

/// BankBudget's access tiles are the standable tiles in line with the stand
/// (east/west/north/south): never the stand tile, even when it itself is
/// standable (a teller's spawn), never a blocked neighbour, and never a
/// diagonal — a teller across a booth does not answer from the corner.
#[test]
fn bank_access_tiles_are_standable_neighbours_not_the_stand() {
    let wc = bake(
        5,
        5,
        &[
            (2, 2, CollisionFlag::SQ_BLOCKED as u32),
            (2, 3, CollisionFlag::SQ_BLOCKED as u32),
        ],
    );
    let booth = stand(2, 2);
    let mut tiles: Vec<_> = bank_access_tiles(&wc, &booth).collect();
    tiles.sort_by_key(|t| (t.x, t.z));
    assert_eq!(
        tiles,
        vec![tile(1, 2, 0), tile(2, 1, 0), tile(3, 2, 0)],
        "the in-line standable neighbours only"
    );
    assert!(super::is_bank_access(&wc, &booth, tile(1, 2, 0)));
    assert!(
        !super::is_bank_access(&wc, &booth, tile(1, 1, 0)),
        "a diagonal"
    );
    assert!(
        !super::is_bank_access(&wc, &booth, tile(2, 3, 0)),
        "blocked"
    );
    let open = open_grid();
    let teller = stand(4, 4);
    assert!(open.standable(teller.tile));
    assert!(!bank_access_tiles(&open, &teller).any(|t| t == teller.tile));
    assert!(!super::is_bank_access(&open, &teller, teller.tile));
}

/// A tile across a wall from the stand is no access, whichever of the two
/// tiles carries the wall face (the client stamps both): a teller against
/// the bank's outer wall is not used from the street behind it.
#[test]
fn bank_access_tiles_skip_a_tile_walled_off_from_the_stand() {
    let teller = stand(2, 2);
    for faces in [
        vec![
            (2, 1, CollisionFlag::W_N as u32),
            (2, 2, CollisionFlag::W_S as u32),
        ],
        vec![(2, 1, CollisionFlag::W_N as u32)],
        vec![(2, 2, CollisionFlag::W_S as u32)],
    ] {
        let wc = bake(5, 5, &faces);
        assert!(
            wc.standable(tile(2, 1, 0)),
            "the street tile stays standable"
        );
        let mut tiles: Vec<_> = bank_access_tiles(&wc, &teller).collect();
        tiles.sort_by_key(|t| (t.x, t.z));
        assert_eq!(
            tiles,
            vec![tile(1, 2, 0), tile(2, 3, 0), tile(3, 2, 0)],
            "the walled-off south tile is dropped ({faces:?})"
        );
        assert!(!super::is_bank_access(&wc, &teller, tile(2, 1, 0)));
        assert!(super::is_bank_access(&wc, &teller, tile(2, 3, 0)));
    }
}

/// On the real 289 pack, BankBudget walks to a standable access tile from
/// the customer street — not onto a booth loc or a teller spawn.
#[test]
fn packed_bank_fetch_walks_to_access_from_customer_streets() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    // Draynor's street is the road east of the bank: (3088,3240) is the
    // bankers' aisle, which reaches no bank customer tile.
    let samples = [
        ("Varrock West", tile(3180, 3430, 0)),
        ("Draynor", tile(3105, 3250, 0)),
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

fn rows(origin: Origin, rows: &[(i32, i32)]) -> BankRows {
    BankRows {
        origin,
        rows: rows.to_vec(),
    }
}

/// The toll route's diagnosis from an empty pack: 10 coins.
fn toll_missing() -> Vec<MissingReq> {
    let missing = find_missing_item_reqs(
        &walled_5x5(),
        &toll_graph(),
        tile(0, 0, 0),
        tile(4, 4, 0),
        FindOptions::default(),
        &WorldState::default(),
    )
    .expect("only the toll count is missing");
    assert_eq!(missing, vec![MissingReq::Carry { id: 995, count: 10 }]);
    missing
}

/// design-bank-snapshot §2.4/§4 F6: `Session` and `Unknown` rows are the
/// planner input as they are (borrowed, no copy), so a toll they lack is
/// `NoPath` in place.
#[test]
fn planning_rows_borrow_session_and_unknown_rows() {
    let missing = toll_missing();
    let session = rows(Origin::Session, &[(1, 3), (KNIFE, 1)]);
    let planned = session.planning_rows(&missing);
    assert!(
        matches!(planned, Cow::Borrowed(_)),
        "Session rows are borrowed"
    );
    assert_eq!(&*planned, &session.rows[..]);
    assert_eq!(
        plan_bank_fetch(
            &missing,
            &WorldState::default(),
            &planned,
            &[stand(4, 0)],
            tile(0, 0, 0)
        ),
        None,
        "a Session bank without the toll is NoPath"
    );

    let unknown = BankRows::default();
    assert_eq!(unknown.origin, Origin::Unknown);
    let planned = unknown.planning_rows(&missing);
    assert!(
        matches!(planned, Cow::Borrowed(_)),
        "Unknown rows are borrowed"
    );
    assert!(planned.is_empty());
    assert_eq!(
        plan_bank_fetch(
            &missing,
            &WorldState::default(),
            &planned,
            &[stand(4, 0)],
            tile(0, 0, 0)
        ),
        None,
        "an Unknown bank is NoPath"
    );
}

/// A `Hint` shortage is advisory (D4): each diagnosed `Carry` is raised to
/// `max(hinted, count)`, never lowered, the rows stay sorted, and a toll
/// the hint lacks plans the one verifying trip — Walk, Open, Withdraw,
/// Close, never a deposit.
#[test]
fn planning_rows_raise_hinted_carry_to_the_diagnosis() {
    let missing = toll_missing();
    let lacking = rows(Origin::Hint, &[(1, 3), (KNIFE, 1)]);
    let planned = lacking.planning_rows(&missing);
    assert_eq!(&*planned, &[(1, 3), (KNIFE, 1), (995, 10)][..]);
    let fetch = plan_bank_fetch(
        &missing,
        &WorldState::default(),
        &planned,
        &[stand(4, 0)],
        tile(0, 0, 0),
    )
    .expect("a Hint without the toll still plans the verifying trip");
    assert_eq!(
        fetch.steps,
        vec![
            access_walk(&[stand(4, 0)], tile(0, 0, 0)),
            BankStep::Open,
            BankStep::Withdraw { id: 995, count: 10 },
            BankStep::Close,
        ]
    );
    assert_no_deposit(&fetch.steps);

    let short = rows(Origin::Hint, &[(995, 3)]);
    assert_eq!(&*short.planning_rows(&missing), &[(995, 10)][..]);
    let rich = rows(Origin::Hint, &[(995, 50)]);
    assert_eq!(
        &*rich.planning_rows(&missing),
        &[(995, 50)][..],
        "a hinted surplus is kept"
    );
}

/// A `WearAny` gains one unit of its first alternative only when the hint
/// holds none of them; a unit the same id's `Carry` reserves is not one.
#[test]
fn planning_rows_add_one_wear_alternative_only_when_none_is_hinted() {
    let wear = [MissingReq::WearAny {
        ids: vec![1321, 1323],
    }];
    let none = rows(Origin::Hint, &[(995, 5)]);
    assert_eq!(&*none.planning_rows(&wear), &[(995, 5), (1321, 1)][..]);
    let second = rows(Origin::Hint, &[(1323, 1)]);
    let planned = second.planning_rows(&wear);
    assert_eq!(
        &*planned,
        &[(1323, 1)][..],
        "a hinted alternative adds nothing"
    );

    let both = [
        MissingReq::Carry {
            id: KNIFE,
            count: 1,
        },
        MissingReq::WearAny { ids: vec![KNIFE] },
    ];
    let empty = rows(Origin::Hint, &[]);
    let planned = empty.planning_rows(&both);
    assert_eq!(&*planned, &[(KNIFE, 2)][..], "carry one and wear one");
    let fetch = plan_bank_fetch(
        &both,
        &WorldState::default(),
        &planned,
        &[stand(4, 0)],
        tile(0, 0, 0),
    )
    .expect("the overlay covers the carried and the worn knife");
    assert!(fetch.steps.contains(&BankStep::Withdraw {
        id: KNIFE,
        count: 2
    }));
    assert!(fetch.steps.contains(&BankStep::Wear { id: KNIFE }));
    assert_no_deposit(&fetch.steps);
}

/// D7 pin: a BankBudget session never deposits. Every step kind is named
/// here, so a deposit variant cannot be added without revisiting this.
fn assert_no_deposit(steps: &[BankStep]) {
    for step in steps {
        match step {
            BankStep::Walk { .. }
            | BankStep::Open
            | BankStep::Withdraw { .. }
            | BankStep::WithdrawX { .. }
            | BankStep::WithdrawXAmount { .. }
            | BankStep::Wear { .. }
            | BankStep::Close => {}
        }
    }
}
