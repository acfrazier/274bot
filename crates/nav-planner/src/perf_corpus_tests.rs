//! Correctness of the 289 routing corpus that `examples/perf_corpus.rs`
//! times. The example only measures; these tests hold the invariants.
//!
//! Every expectation comes from the loaded pack itself (the bank table, the
//! zone table, and a second search over the same world), never from a count
//! or tick total tied to one baked data version. The pack is the revision-289
//! pack that `NAV_PACK` names (or the default build path). A missing pack
//! skips visibly in hosted CI under ADR 0019 and fails locally
//! (`api::test_support::local_asset_path`).

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use api::snapshot::WorldTile;

use crate::router::{
    find_blocking_zones, find_first_with, find_many_with, find_with, FindOptions, Leg, Route,
    RouteError, TargetError,
};
use crate::world::NavWorld;
use crate::world_state::WorldState;
use crate::zones::ZoneExempt;

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn load_world() -> Option<NavWorld> {
    let default_pack = NavWorld::default_pack_path();
    let default_pack = default_pack.to_string_lossy();
    let path = api::test_support::local_asset_path(
        "NAV_PACK",
        &default_pack,
        "revision-289 navigation pack for the routing corpus",
    )?;
    let world = NavWorld::load_pack(&path)
        .unwrap_or_else(|error| panic!("could not load nav pack at {}: {error}", path.display()));
    let data = api::game_data::for_revision(client::io::ClientRevision::R289)
        .expect("revision 289 game data");
    world
        .bind_named_bank_facts(&data)
        .expect("bind named bank facts to the pack");
    Some(world)
}

fn world() -> Option<&'static NavWorld> {
    static WORLD: LazyLock<Option<NavWorld>> = LazyLock::new(load_world);
    WORLD.as_ref()
}

fn rich_state(combat_level: i32) -> WorldState {
    WorldState {
        map_members: true,
        combat_level: Some(combat_level),
        quests: HashSet::from([
            "Prince Ali Rescue".into(),
            "Rune Mysteries".into(),
            "Lost City".into(),
            "Shilo Village".into(),
            "Tree Gnome Village".into(),
            "The Grand Tree".into(),
        ]),
        inv: HashMap::from([
            (995, 10_000),
            (554, 1_000),
            (555, 1_000),
            (556, 1_000),
            (557, 1_000),
            (558, 1_000),
            (561, 1_000),
            (563, 1_000),
        ]),
        stats: (0..21).map(|skill| (skill, 99)).collect(),
        ..WorldState::empty()
    }
}

fn zone_free() -> FindOptions {
    FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    }
}

/// A route is a chain: walk steps are single-tile moves, and the route ends
/// where it says it does.
fn assert_route_is_chain(route: &Route, from: WorldTile, to: WorldTile) {
    assert_eq!(route.dest, to);
    assert!(route.ticks.is_finite() && route.ticks >= 0.0);
    let mut previous = from;
    for leg in &route.legs {
        match leg {
            Leg::Walk { tiles } => {
                for &arrival in tiles.iter().skip(1) {
                    assert!(
                        (arrival.x - previous.x).abs() <= 1
                            && (arrival.z - previous.z).abs() <= 1
                            && arrival.level == previous.level,
                        "walk step {previous:?} -> {arrival:?} is not a single-tile move"
                    );
                    previous = arrival;
                }
            }
            Leg::Transport { edge } => previous = edge.to,
        }
    }
    assert_eq!(previous, to, "route legs do not end at the destination");
}

/// A many-target search must agree with one single-target search per goal,
/// and zone policy may only remove or lengthen routes, never add or shorten.
#[test]
fn bank_pick_agrees_with_single_target_and_zones_only_restrict() {
    let Some(world) = world() else { return };
    let targets: Vec<WorldTile> = world
        .named_bank_facts()
        .expect("named bank facts after binding")
        .banks()
        .iter()
        .filter(|bank| bank.routable)
        .map(|bank| bank.tile)
        .collect();
    assert!(!targets.is_empty(), "pack has no routable bank stands");
    let from = tile(3016, 9840, 0);
    let rich = rich_state(126);
    let low = rich_state(3);

    let free = find_many_with(
        &world.collision,
        &world.graph,
        from,
        &targets,
        zone_free(),
        &rich,
    );
    let zoned = find_many_with(
        &world.collision,
        &world.graph,
        from,
        &targets,
        FindOptions::default(),
        &low,
    );
    assert_eq!(
        zoned.completion_partitions(),
        0,
        "bank goals are settled by the first search, with no zone completion pass"
    );
    assert!(
        free.results().iter().any(Result::is_ok),
        "no routable bank stand is reachable from the start"
    );

    for (index, &target) in targets.iter().enumerate() {
        let single = find_with(
            &world.collision,
            &world.graph,
            from,
            target,
            zone_free(),
            &rich,
        );
        match (&free.results()[index], &single) {
            (Ok(cost), Ok(route)) => assert_eq!(
                cost.ticks, route.ticks,
                "many-target cost differs from the single-target route to {target:?}"
            ),
            (Err(_), Err(_)) => {}
            (many, single) => panic!(
                "many-target and single-target searches disagree for {target:?}: \
                 {many:?} vs {:?}",
                single.as_ref().map(|route| route.ticks)
            ),
        }
        // Zone policy under a weaker state can only remove or lengthen.
        if let Ok(strict) = &zoned.results()[index] {
            let relaxed = free.results()[index]
                .as_ref()
                .unwrap_or_else(|error| panic!("zones reach {target:?}, zone-free: {error:?}"));
            assert!(
                strict.ticks >= relaxed.ticks,
                "zones shortened the route to {target:?}: {} < {}",
                strict.ticks,
                relaxed.ticks
            );
        }
    }
}

/// When zone policy refuses a route that exists without zones, the witness
/// names the zones in the way. Exempting exactly those makes the route legal.
/// Candidate refusals come from the performance corpus; at least one must
/// refuse so this test cannot pass vacuously.
#[test]
fn zone_refusal_witness_is_what_lifts_the_refusal() {
    let Some(world) = world() else { return };
    let table = world.graph.zones.as_ref().expect("pack carries zones");
    let rich = rich_state(126);
    let low = rich_state(3);
    let candidates = [
        (tile(3222, 3218, 0), tile(2664, 3664, 0), &rich),
        (tile(2965, 3379, 0), tile(3222, 3218, 0), &low),
        (tile(2895, 3450, 0), tile(2852, 3493, 0), &low),
    ];
    let mut refused = 0;
    for (from, to, state) in candidates {
        let strict = find_first_with(
            &world.collision,
            &world.graph,
            from,
            &[to],
            FindOptions::default(),
            state,
        );
        let witness = find_blocking_zones(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions::default(),
            state,
            &[],
        );
        let Err(RouteError::NoPath) = strict.route() else {
            assert!(
                witness.is_none(),
                "a route exists from {from:?} to {to:?} yet zones are blamed"
            );
            continue;
        };
        let relaxed = find_with(&world.collision, &world.graph, from, to, zone_free(), state);
        if relaxed.is_err() {
            // Unreachable even without zones: zones are not the cause.
            assert!(witness.is_none(), "{from:?} -> {to:?} is blamed on zones");
            continue;
        }
        refused += 1;
        let blockers = witness.expect("a zone refusal has a witness");
        assert!(
            !blockers.is_empty(),
            "empty zone witness for {from:?} -> {to:?}"
        );
        for &key in &blockers {
            assert!(!table.name(key).is_empty());
        }
        let lifted = find_with(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions {
                zones: ZoneExempt::named(&blockers).expect("witness fits a bounded exemption"),
                ..FindOptions::default()
            },
            state,
        )
        .unwrap_or_else(|error| {
            panic!("exempting the witness zones does not lift the refusal: {error:?}")
        });
        assert_route_is_chain(&lifted, from, to);
        let free = relaxed.expect("checked above");
        assert!(lifted.ticks >= free.ticks);
    }
    assert!(
        refused > 0,
        "none of the corpus refusal candidates is refused any more; update the candidates"
    );
}

/// A stronger character can only open routes: the same trip is never longer
/// at combat 126 than at combat 3, and never shorter than with zones off.
#[test]
fn taverley_catherby_route_is_monotone_in_permissions() {
    let Some(world) = world() else { return };
    let from = tile(2895, 3450, 0);
    let to = tile(2809, 3440, 0);
    let route_for = |options: FindOptions, state: &WorldState| {
        find_with(&world.collision, &world.graph, from, to, options, state)
    };
    let strong = route_for(FindOptions::default(), &rich_state(126))
        .expect("a rich level-126 character routes Taverley to Catherby");
    assert_route_is_chain(&strong, from, to);
    let free = route_for(zone_free(), &rich_state(126)).expect("zone-free route");
    assert!(
        free.ticks <= strong.ticks,
        "zone policy shortened the route: {} < {}",
        strong.ticks,
        free.ticks
    );
    match route_for(FindOptions::default(), &rich_state(3)) {
        Ok(weak) => {
            assert_route_is_chain(&weak, from, to);
            assert!(
                strong.ticks <= weak.ticks,
                "combat 126 is slower than combat 3: {} > {}",
                strong.ticks,
                weak.ticks
            );
        }
        Err(error) => assert_eq!(error, RouteError::NoPath, "unexpected failure {error:?}"),
    }
}

/// A goal the pack proves unreachable (a blocked tile that no transport edge
/// lands on) must be refused with a stated reason. A router that returns a
/// route to it, or drops the goal, fails here.
#[test]
fn pack_proven_unreachable_target_is_refused_with_no_path() {
    let Some(world) = world() else { return };
    let from = tile(3222, 3218, 0);
    let is_landing = |t: WorldTile| {
        world
            .graph
            .edges
            .iter()
            .chain(&world.graph.teleports)
            .any(|edge| edge.to == t)
    };
    let target = (0..100)
        .flat_map(|dx| (0..100).map(move |dz| tile(from.x + dx, from.z + dz, 0)))
        .find(|&t| !world.collision.standable(t) && !is_landing(t))
        .expect("the pack has a blocked tile near the start that no transport lands on");
    let targets = [target];
    let search = find_many_with(
        &world.collision,
        &world.graph,
        from,
        &targets,
        zone_free(),
        &WorldState::empty(),
    );
    assert_eq!(search.results().len(), 1);
    assert_eq!(
        search.results()[0].as_ref().map(|cost| cost.ticks),
        Err(&TargetError::NoPath),
        "blocked, landing-free {target:?} must be refused"
    );
    assert_eq!(
        find_with(
            &world.collision,
            &world.graph,
            from,
            target,
            zone_free(),
            &WorldState::empty(),
        )
        .err(),
        Some(RouteError::NoPath)
    );
}
