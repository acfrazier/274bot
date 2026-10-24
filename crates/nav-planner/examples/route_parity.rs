//! Deterministic full route witnesses for the retained 289 corpus.
//! This is not a timing benchmark. It records costs and complete leg sequences.

use api::snapshot::WorldTile;
use nav_planner::router::{find_blocking_zones, find_first_with, find_many_with, FindOptions};
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use nav_planner::zones::ZoneExempt;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn rich_state() -> WorldState {
    WorldState {
        map_members: true,
        combat_level: Some(126),
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

struct Case<'a> {
    name: &'a str,
    from: WorldTile,
    goals: &'a [WorldTile],
    options: FindOptions,
    state: &'a WorldState,
    account: &'a str,
}

fn base(case: &Case<'_>) -> Value {
    json!({
        "name": case.name,
        "source": format!("{:?}", case.from),
        "goals": case.goals.iter().map(|goal| format!("{goal:?}")).collect::<Vec<_>>(),
        "account": case.account,
        "policy": format!("{:?}", case.options),
    })
}

fn many(world: &NavWorld, case: Case<'_>) -> Value {
    let search = find_many_with(
        &world.collision,
        &world.graph,
        case.from,
        case.goals,
        case.options,
        case.state,
    );
    let mut row = base(&case);
    row["kind"] = json!("many");
    row["routes"] = json!((0..case.goals.len())
        .map(|index| format!("{:?}", search.route(index)))
        .collect::<Vec<_>>());
    row["settled"] = json!(search.settled());
    row
}

fn first(world: &NavWorld, case: Case<'_>) -> Value {
    let search = find_first_with(
        &world.collision,
        &world.graph,
        case.from,
        case.goals,
        case.options,
        case.state,
    );
    let mut row = base(&case);
    row["kind"] = json!("first");
    row["routes"] = json!([format!("{:?}", search.route())]);
    row["settled"] = json!(search.settled());
    row
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        return Err("usage: route_parity PACK OUTPUT_JSON".into());
    }
    let mut world =
        NavWorld::load_pack(Path::new(&args[0])).map_err(|error| format!("load: {error:?}"))?;
    let data = api::game_data::for_revision(client::io::ClientRevision::R289)?;
    world
        .bind_named_bank_facts(&data)
        .map_err(|error| format!("banks: {error:?}"))?;
    let banks: Vec<_> = world
        .named_bank_facts()
        .ok_or("missing banks")?
        .banks()
        .iter()
        .filter(|bank| bank.routable)
        .map(|bank| bank.tile)
        .collect();
    if banks.is_empty() {
        return Err("empty bank corpus".into());
    }
    let center = tile(2553, 3406, 0);
    let mut radius = Vec::new();
    for x in center.x - 10..=center.x + 10 {
        for z in center.z - 10..=center.z + 10 {
            let goal = tile(x, z, 0);
            if world.collision.standable(goal) {
                radius.push(goal);
            }
        }
    }
    let empty = WorldState::empty();
    let rich = rich_state();
    let mut low = rich_state();
    low.combat_level = Some(3);
    let mut live = WorldState::empty();
    live.map_members = true;
    live.combat_level = Some(50);
    live.inv.insert(995, 60);
    let legacy = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let zones = world.graph.zones.take();
    let mut cases = vec![
        many(
            &world,
            Case {
                name: "reachable_lumbridge_falador",
                from: tile(3222, 3218, 0),
                goals: &[tile(2965, 3379, 0)],
                options: legacy,
                state: &empty,
                account: "empty",
            },
        ),
        many(
            &world,
            Case {
                name: "bank_pick_all_stands",
                from: tile(3016, 9840, 0),
                goals: &banks,
                options: legacy,
                state: &rich,
                account: "rich126",
            },
        ),
        first(
            &world,
            Case {
                name: "radius_walk_candidates",
                from: tile(2615, 3332, 0),
                goals: &radius,
                options: legacy,
                state: &rich,
                account: "rich126",
            },
        ),
        many(
            &world,
            Case {
                name: "teleports_on_real_state",
                from: tile(3222, 3218, 0),
                goals: &[tile(2817, 3443, 0)],
                options: FindOptions {
                    allow_teleports: true,
                    ..legacy
                },
                state: &rich,
                account: "rich126",
            },
        ),
        many(
            &world,
            Case {
                name: "unreachable_fail_closed",
                from: tile(3222, 3218, 0),
                goals: &[tile(2817, 3443, 0)],
                options: legacy,
                state: &empty,
                account: "empty",
            },
        ),
    ];
    world.graph.zones = zones;
    for (name, account, state) in [
        ("reachable_lumbridge_falador_zones_l126", "rich126", &rich),
        ("reachable_lumbridge_falador_zones_l3", "rich3", &low),
    ] {
        cases.push(many(
            &world,
            Case {
                name,
                from: tile(3222, 3218, 0),
                goals: &[tile(2965, 3379, 0)],
                options: FindOptions::default(),
                state,
                account,
            },
        ));
    }
    cases.push(many(
        &world,
        Case {
            name: "bank_pick_all_stands_zones_l3",
            from: tile(3016, 9840, 0),
            goals: &banks,
            options: FindOptions::default(),
            state: &low,
            account: "rich3",
        },
    ));
    cases.push(first(
        &world,
        Case {
            name: "radius_walk_candidates_zones_l3",
            from: tile(2615, 3332, 0),
            goals: &radius,
            options: FindOptions::default(),
            state: &low,
            account: "rich3",
        },
    ));
    cases.push(first(
        &world,
        Case {
            name: "lumbridge_rellekka_refused_zones_l126",
            from: tile(3222, 3218, 0),
            goals: &[tile(2664, 3664, 0)],
            options: FindOptions::default(),
            state: &rich,
            account: "rich126",
        },
    ));
    let witness = find_blocking_zones(
        &world.collision,
        &world.graph,
        tile(3222, 3218, 0),
        tile(2664, 3664, 0),
        FindOptions::default(),
        &rich,
        &[],
    );
    cases.last_mut().expect("refusal row")["zone_witness"] = json!(format!("{witness:?}"));
    cases.push(many(
        &world,
        Case {
            name: "taverley_catherby_zones_l126",
            from: tile(2895, 3450, 0),
            goals: &[tile(2809, 3440, 0)],
            options: FindOptions::default(),
            state: &rich,
            account: "rich126",
        },
    ));
    for (name, from, to, options, state, account) in [
        (
            "danger_live_off",
            tile(2809, 3441, 0),
            tile(3103, 3163, 2),
            FindOptions::default(),
            &live,
            "live50",
        ),
        (
            "danger_live_grant",
            tile(2809, 3441, 0),
            tile(3103, 3163, 2),
            legacy,
            &live,
            "live50",
        ),
        (
            "falador_lumbridge_zones_l3",
            tile(2965, 3379, 0),
            tile(3222, 3218, 0),
            FindOptions::default(),
            &low,
            "rich3",
        ),
        (
            "deep_zone_goal_teles_off",
            tile(2895, 3450, 0),
            tile(2852, 3493, 0),
            FindOptions::default(),
            &low,
            "rich3",
        ),
        (
            "deep_zone_goal_teles_on",
            tile(2895, 3450, 0),
            tile(2852, 3493, 0),
            FindOptions {
                allow_teleports: true,
                ..FindOptions::default()
            },
            &low,
            "rich3",
        ),
        (
            "all_off_control",
            tile(3222, 3218, 0),
            tile(2965, 3379, 0),
            FindOptions::default(),
            &empty,
            "empty",
        ),
    ] {
        cases.push(first(
            &world,
            Case {
                name,
                from,
                goals: &[to],
                options,
                state,
                account,
            },
        ));
    }
    let result = json!({"schema": "274bot-route-parity-v1", "revision": 289, "cases": cases});
    std::fs::write(
        &args[1],
        serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}
