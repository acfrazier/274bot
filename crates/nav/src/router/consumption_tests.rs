use super::*;
use crate::bank_fetch::{plan_bank_fetch, BankStep};

fn two_hops(requirements: &[(i32, i32)], held: bool) -> (WorldCollision, TransportGraph) {
    let collision = bake(1, 1, &[]);
    let mut graph = TransportGraph::default();
    for level in 0..2 {
        let mut edge = door(tile(0, 0, level), tile(0, 0, level + 1), 7)
            .edges
            .remove(0);
        edge.kind = TransportKind::Boat;
        if held {
            edge.item_req = requirements.to_vec();
        } else {
            edge.consumed_req = requirements.to_vec();
        }
        graph.edges.push(edge);
    }
    graph.rebuild_index(&collision);
    (collision, graph)
}

#[test]
fn two_thirty_coin_fares_need_sixty_before_departure() {
    let (collision, graph) = two_hops(&[(995, 30)], false);
    let from = tile(0, 0, 0);
    let to = tile(0, 0, 2);
    let mut state = WorldState::empty();
    state.inv.insert(995, 30);
    assert_eq!(
        find_with(&collision, &graph, from, to, FindOptions::default(), &state),
        Err(RouteError::NoPath)
    );
    assert_eq!(
        find_missing_item_reqs(&collision, &graph, from, to, FindOptions::default(), &state),
        Some(vec![MissingReq::Carry { id: 995, count: 60 }])
    );
    state.inv.insert(995, 60);
    let route = find_with(&collision, &graph, from, to, FindOptions::default(), &state).unwrap();
    assert_eq!(route.ticks, 14.0);
    assert!(missing_item_reqs(&route, &state).is_empty());
    assert!(find_first_with(
        &collision,
        &graph,
        from,
        &[to],
        FindOptions::default(),
        &state
    )
    .route()
    .is_ok());
    let targets = [tile(0, 0, 1), to];
    let many = find_many_with(
        &collision,
        &graph,
        from,
        &targets,
        FindOptions::default(),
        &state,
    );
    assert_eq!(many.route(1).unwrap(), route);
}

#[test]
fn bank_supply_sixty_withdraws_both_fares() {
    let (collision, graph) = two_hops(&[(995, 30)], false);
    let state = WorldState::empty();
    let missing = find_missing_item_reqs(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(0, 0, 2),
        FindOptions::default(),
        &state,
    )
    .unwrap();
    let bank_collision = bake(3, 3, &[]);
    let stands = [BankStand {
        name: "Bank booth".into(),
        tile: tile(1, 1, 0),
        access: BankAccess::Booth { op: 2 },
    }];
    let plan = plan_bank_fetch(
        &missing,
        &state,
        &[(995, 60)],
        &stands,
        tile(0, 0, 0),
        &bank_collision,
    )
    .unwrap();
    assert_eq!(
        plan.steps
            .iter()
            .filter(|step| matches!(step, BankStep::Withdraw { .. }))
            .collect::<Vec<_>>(),
        vec![&BankStep::Withdraw { id: 995, count: 60 }]
    );
    assert!(find_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(0, 0, 2),
        FindOptions::default(),
        &plan.state
    )
    .is_ok());
}

#[test]
fn repeated_spell_casts_budget_two_rune_sets() {
    let runes = [(554, 1), (556, 3), (563, 1)];
    let mut edge = teleport(tile(0, 0, 1), 3, vec![], runes.to_vec())
        .teleports
        .remove(0);
    // Route aggregation must account for every cast, even equal edge facts.
    let route = crate::router::Route {
        legs: vec![
            Leg::Transport {
                edge: Box::new(edge.clone()),
            },
            Leg::Transport {
                edge: Box::new(edge.clone()),
            },
        ],
        dest: edge.to,
        ticks: 6.0,
    };
    let state = WorldState {
        inv: HashMap::from(runes),
        ..WorldState::empty()
    };
    assert_eq!(
        missing_item_reqs(&route, &state),
        vec![
            MissingReq::Carry { id: 554, count: 2 },
            MissingReq::Carry { id: 556, count: 6 },
            MissingReq::Carry { id: 563, count: 2 },
        ]
    );
    // A static pair of spell hops exercises the same running balance.
    let (collision, mut graph) = two_hops(&runes, false);
    for hop in &mut graph.edges {
        hop.kind = TransportKind::Teleport;
    }
    assert_eq!(
        find_with(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(0, 0, 2),
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath)
    );
    edge.to = tile(0, 0, 2);
    let twice = WorldState {
        inv: runes
            .into_iter()
            .map(|(id, count)| (id, count * 2))
            .collect(),
        ..state
    };
    assert!(find_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        edge.to,
        FindOptions::default(),
        &twice
    )
    .is_ok());
}

#[test]
fn a_held_key_is_reusable_and_retained_when_fares_are_missing() {
    let (collision, mut graph) = two_hops(&[(983, 1)], true);
    let state = WorldState {
        inv: HashMap::from([(983, 1)]),
        ..WorldState::empty()
    };
    let route = find_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(0, 0, 2),
        FindOptions::default(),
        &state,
    )
    .unwrap();
    assert!(missing_item_reqs(&route, &state).is_empty());
    for hop in &mut graph.edges {
        hop.consumed_req = vec![(995, 30)];
    }
    assert_eq!(
        find_missing_item_reqs(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(0, 0, 2),
            FindOptions::default(),
            &state
        ),
        Some(vec![
            MissingReq::Carry { id: 983, count: 1 },
            MissingReq::Carry { id: 995, count: 60 },
        ])
    );
}

#[test]
fn a_stack_needed_as_a_held_gate_after_spend_must_remain() {
    let (collision, mut graph) = two_hops(&[(995, 30)], false);
    graph.edges[1].consumed_req.clear();
    graph.edges[1].item_req = vec![(995, 1)];
    let mut state = WorldState {
        inv: HashMap::from([(995, 30)]),
        ..WorldState::empty()
    };
    assert_eq!(
        find_with(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(0, 0, 2),
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath)
    );
    state.inv.insert(995, 31);
    assert!(find_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(0, 0, 2),
        FindOptions::default(),
        &state
    )
    .is_ok());
}

#[test]
fn jewellery_downgrade_supplies_the_next_charge_not_the_original_variant() {
    let (collision, mut graph) = two_hops(&[(1706, 1)], false);
    graph.edges[0].item_returns = vec![(1704, 1)];
    graph.edges[1].consumed_req = vec![(1704, 1)];
    let state = WorldState {
        inv: HashMap::from([(1706, 1)]),
        ..WorldState::empty()
    };
    let route = find_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(0, 0, 2),
        FindOptions::default(),
        &state,
    )
    .unwrap();
    assert!(missing_item_reqs(&route, &state).is_empty());
    assert!(find_first_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        &[tile(0, 0, 2)],
        FindOptions::default(),
        &state
    )
    .route()
    .is_ok());
    graph.edges[1].consumed_req = vec![(1706, 1)];
    assert_eq!(
        find_with(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(0, 0, 2),
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath)
    );
}

#[test]
fn returned_variant_opens_a_held_gate_even_with_generous_input_supply() {
    let (collision, mut graph) = two_hops(&[(1706, 1)], false);
    graph.edges[0].item_returns = vec![(1704, 1)];
    graph.edges[1].consumed_req.clear();
    graph.edges[1].item_req = vec![(1704, 1)];
    let state = WorldState {
        inv: HashMap::from([(1706, 100)]),
        ..WorldState::empty()
    };
    let route = find_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(0, 0, 2),
        FindOptions::default(),
        &state,
    )
    .unwrap();
    assert_eq!(route.ticks, 14.0);
    assert!(missing_item_reqs(&route, &state).is_empty());
    assert!(find_first_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        &[tile(0, 0, 2)],
        FindOptions::default(),
        &state,
    )
    .route()
    .is_ok());
}

#[test]
fn a_slower_arrival_with_unspent_coins_is_not_discarded() {
    let (collision, mut graph) = two_hops(&[(995, 30)], false);
    let mut free = graph.edges[0].clone();
    free.consumed_req.clear();
    free.ticks = 10;
    graph.edges.push(free);
    graph.rebuild_index(&collision);
    let state = WorldState {
        inv: HashMap::from([(995, 30)]),
        ..WorldState::empty()
    };
    let route = find_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(0, 0, 2),
        FindOptions::default(),
        &state,
    )
    .unwrap();
    assert_eq!(route.ticks, 17.0);
    assert!(missing_item_reqs(&route, &state).is_empty());
}

#[test]
fn oversized_resource_catalog_fails_closed_instead_of_dropping_debits() {
    for (count, error) in [(64, RouteError::NoPath), (65, RouteError::BudgetExhausted)] {
        let requirements: Vec<_> = (0..count).map(|id| (30_000 + id, 1)).collect();
        let (collision, graph) = two_hops(&requirements, false);
        let state = WorldState {
            inv: requirements.into_iter().collect(),
            ..WorldState::empty()
        };
        assert_eq!(
            find_with(
                &collision,
                &graph,
                tile(0, 0, 0),
                tile(0, 0, 2),
                FindOptions::default(),
                &state,
            ),
            Err(error),
        );
    }
}

#[test]
fn large_consumption_totals_do_not_overflow_the_unmetered_proof() {
    let (collision, graph) = two_hops(&[(995, i32::MAX)], false);
    let state = WorldState {
        inv: HashMap::from([(995, i32::MAX)]),
        ..WorldState::empty()
    };
    assert_eq!(
        find_with(
            &collision,
            &graph,
            tile(0, 0, 0),
            tile(0, 0, 2),
            FindOptions::default(),
            &state
        ),
        Err(RouteError::NoPath)
    );
}

#[test]
fn missing_worn_gate_exposes_the_satisfied_carry_reservation() {
    let (collision, mut graph) = two_hops(&[(983, 1)], true);
    graph.edges[1].worn_req = vec![983];
    let state = WorldState {
        inv: HashMap::from([(983, 1)]),
        ..WorldState::empty()
    };
    let missing = find_missing_item_reqs(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(0, 0, 2),
        FindOptions::default(),
        &state,
    )
    .unwrap();
    assert_eq!(
        missing,
        vec![
            MissingReq::Carry { id: 983, count: 1 },
            MissingReq::WearAny { ids: vec![983] }
        ]
    );
    let stands = [BankStand {
        name: "Bank booth".into(),
        tile: tile(1, 1, 0),
        access: BankAccess::Booth { op: 2 },
    }];
    let plan = plan_bank_fetch(
        &missing,
        &state,
        &[(983, 1)],
        &stands,
        tile(0, 0, 0),
        &bake(3, 3, &[]),
    )
    .unwrap();
    assert_eq!(plan.state.inv.get(&983), Some(&1));
    assert!(plan.state.worn.contains(&983));
    assert!(find_with(
        &collision,
        &graph,
        tile(0, 0, 0),
        tile(0, 0, 2),
        FindOptions::default(),
        &plan.state
    )
    .is_ok());
}
