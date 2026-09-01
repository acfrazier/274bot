//! BankBudget (Task 8): the fetch-and-wear session that unblocks a
//! [`crate::router::find_with`] `NoPath` whose only missing gates are
//! `item_req`/`worn_req`. `find` itself stays fail-closed — the router
//! never inserts a virtual bank leg and never relaxes an edge; this
//! session is the only thing that may fetch, and the host re-runs the
//! strict search after the steps land.
//!
//! The session is a **plan**: ordered steps the host pump executes
//! through the [`crate::traveller::Traveller`] and the `api::interact`
//! bank path (open/deposit/withdraw/close/wear), plus the [`WorldState`]
//! those steps leave behind for the post-session re-find. A `worn_req`
//! alternative already carried plans a bare [`BankStep::Wear`] — no
//! bank walk. Anything the plan cannot supply (neither carried nor
//! bankable, or the relaxed diagnosis shows a skill/quest/varp gate) is
//! `None`: the caller reports `NoPath`.

use api::snapshot::WorldTile;

use crate::pack::BankStand;
use crate::router::MissingReq;
use crate::world_state::WorldState;

/// One step of a [`BankFetch`] session, in execution order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankStep {
    /// Walk to the bank stand's tile (the host routes it).
    Walk { x: i32, z: i32, level: i32 },
    /// Open the bank (booth `Use-quickly` / teller op).
    Open,
    /// Deposit the whole backpack.
    DepositAll,
    /// Withdraw `count` of the obj from the open bank: an `item_req`
    /// count, or 1 for a `worn_req` the session then wears.
    Withdraw { id: i32, count: i32 },
    /// Wear/wield the obj from the inventory (`worn_req`).
    Wear { id: i32 },
    /// Close the bank.
    Close,
}

/// The BankBudget session, planned from a [`crate::router::find_with`]
/// `NoPath` under [`crate::router::FindOptions::allow_bank_fetch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankFetch {
    /// The ordered steps the pump executes.
    pub steps: Vec<BankStep>,
    /// The world-state the steps leave behind: the backpack deposited,
    /// every `item_req` stack withdrawn to its needed count, every
    /// `worn_req` obj worn. The post-session strict re-find must pass
    /// against this state; anything less fails closed.
    pub state: WorldState,
}

/// Plan a BankBudget session for a strict [`find_with`] `NoPath` whose
/// diagnosis is `missing` ([`crate::router::find_missing_item_reqs`]).
/// `state` is the search's gating facts; `bank` is the open bank's rows
/// (obj id, count) from the live snapshot; `stands` is the packed bank
/// stand table ([`crate::world::NavWorld::banks`]); `from` is the
/// player's tile, which picks the nearest stand.
///
/// A `worn_req` alternative already carried plans only [`BankStep::Wear`]
/// — no bank walk. Otherwise the plan walks to the nearest stand, opens
/// the bank, deposits the backpack, withdraws every missing item (a
/// `worn_req` one is then worn), and closes. The deposit supplies the
/// bank with the carried stack, so a `worn_req` alternative that is
/// merely carried is fetchable after it; every needed amount must be
/// covered by bank + carried stacks combined. `None` when the plan
/// cannot be built — the caller reports
/// [`crate::router::RouteError::NoPath`].
pub fn plan_bank_fetch(
    missing: &[MissingReq],
    state: &WorldState,
    bank: &[(i32, i32)],
    stands: &[BankStand],
    from: WorldTile,
) -> Option<BankFetch> {
    if missing.is_empty() {
        return None;
    }
    // Wear-from-inventory only: every missing req is a worn_req with at
    // least one alternative already carried. No bank trip at all.
    let all_worn_carried = missing.iter().all(|r| match r {
        MissingReq::WearAny { ids } => ids
            .iter()
            .any(|id| state.inv.get(id).is_some_and(|&c| c >= 1)),
        MissingReq::Carry { .. } => false,
    });
    if all_worn_carried {
        let mut post = state.clone();
        let mut steps = Vec::new();
        for r in missing {
            match r {
                MissingReq::WearAny { ids } => {
                    let id = ids
                        .iter()
                        .find(|id| state.inv.get(id).is_some_and(|&c| c >= 1))
                        .copied()
                        .expect("all-worn-carry checked the alternatives");
                    post.worn.insert(id);
                    if let Some(c) = post.inv.get_mut(&id) {
                        *c -= 1;
                        if *c <= 0 {
                            post.inv.remove(&id);
                        }
                    }
                    steps.push(BankStep::Wear { id });
                }
                MissingReq::Carry { .. } => unreachable!("all-worn-carry arm"),
            }
        }
        return Some(BankFetch { steps, state: post });
    }

    // Bank trip. The deposit moves the carried stacks into the bank, so
    // every needed amount may come from the bank's rows plus the
    // backpack: `supply` is the combined count.
    let bank_count = |id: i32| {
        bank.iter()
            .find(|&&(i, _)| i == id)
            .map(|&(_, c)| c)
            .unwrap_or(0)
    };
    let supply = |id: i32| bank_count(id) + state.inv.get(&id).copied().unwrap_or(0);
    for r in missing {
        match r {
            MissingReq::Carry { id, count } => {
                if supply(*id) < *count {
                    return None;
                }
            }
            MissingReq::WearAny { ids } => {
                if !ids.iter().any(|id| supply(*id) >= 1) {
                    return None;
                }
            }
        }
    }
    // The nearest stand (same level preferred) is the walk target.
    let stand = stands.iter().min_by_key(|s| {
        (
            s.tile.level != from.level,
            (s.tile.x - from.x).abs().max((s.tile.z - from.z).abs()),
        )
    })?;

    let mut steps = vec![
        BankStep::Walk {
            x: stand.tile.x,
            z: stand.tile.z,
            level: stand.tile.level,
        },
        BankStep::Open,
        BankStep::DepositAll,
    ];
    // The deposit clears the backpack; the withdrawals rebuild it to
    // exactly what the strict gate needs.
    let mut post = state.clone();
    post.inv.clear();
    for r in missing {
        match r {
            MissingReq::Carry { id, count } => {
                steps.push(BankStep::Withdraw {
                    id: *id,
                    count: *count,
                });
                post.inv.insert(*id, *count);
            }
            MissingReq::WearAny { ids } => {
                let id = ids
                    .iter()
                    .find(|id| supply(**id) >= 1)
                    .copied()
                    .expect("the supply check passed an alternative");
                steps.push(BankStep::Withdraw { id, count: 1 });
                steps.push(BankStep::Wear { id });
                post.worn.insert(id);
            }
        }
    }
    steps.push(BankStep::Close);
    Some(BankFetch { steps, state: post })
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;

    use super::{plan_bank_fetch, BankStep};
    use crate::collision::{pack_walk, WorldCollision};
    use crate::pack::{BankAccess, BankStand};
    use crate::router::{find_missing_item_reqs, find_with, FindOptions, MissingReq, RouteError};
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
        assert_eq!(
            missing,
            vec![MissingReq::WearAny {
                ids: vec![KNIFE],
            }]
        );
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
        assert_eq!(
            missing,
            vec![MissingReq::WearAny {
                ids: vec![KNIFE],
            }]
        );
        // The bank snapshot holds the knife (1).
        let bank = [(KNIFE, 1)];
        let fetch = plan_bank_fetch(&missing, &state, &bank, &[stand(4, 0)], from)
            .expect("the banked knife plans a full trip");
        assert_eq!(
            fetch.steps,
            vec![
                BankStep::Walk {
                    x: 4,
                    z: 0,
                    level: 0
                },
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
                BankStep::Walk {
                    x: 4,
                    z: 0,
                    level: 0
                },
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
                MissingReq::WearAny {
                    ids: vec![KNIFE],
                },
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
                BankStep::Walk {
                    x: 4,
                    z: 0,
                    level: 0
                },
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
                MissingReq::WearAny {
                    ids: vec![KNIFE],
                },
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
                BankStep::Walk {
                    x: 4,
                    z: 0,
                    level: 0
                },
                BankStep::Open,
                BankStep::DepositAll,
                BankStep::Withdraw { id: KNIFE, count: 1 },
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

    /// The session walks to the nearest bank stand.
    #[test]
    fn plans_walk_to_the_nearest_stand() {
        let fetch = plan_bank_fetch(
            &[MissingReq::WearAny { ids: vec![KNIFE] }],
            &WorldState::default(),
            &[(KNIFE, 1)],
            &[stand(9, 9), stand(4, 0)],
            tile(0, 0, 0),
        )
        .expect("a banked knife plans a trip");
        assert_eq!(
            fetch.steps[0],
            BankStep::Walk {
                x: 4,
                z: 0,
                level: 0
            },
            "the nearest stand wins"
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
}
