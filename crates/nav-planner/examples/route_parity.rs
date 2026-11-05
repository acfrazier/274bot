//! Deterministic full route witnesses for the retained 289 corpus.
//! This is not a timing benchmark. It records costs and complete leg sequences.

use api::prereq_knowledge::Observation;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceProvider, EvidenceStamp, QuestListEvidenceProvider};
use api::selected::{InclusiveRange, QuestGate, RunKey, Truth};
use api::snapshot::{GameSnapshot, QuestStatusView, SnapshotView, WorldTile};
use nav_planner::quest_gates::QuestEvidence;
use nav_planner::router::{find_blocking_zones, find_first_with, find_many_with, FindOptions};
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use nav_planner::zones::ZoneExempt;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

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

/// Corpus stage witnesses evaluate only explicitly supplied quest/signal pairs.
/// Other windows stay unknown; completion still uses the selected quest resolver.
struct CorpusEvidence {
    completion: QuestListEvidenceProvider,
    signals: Vec<(&'static str, &'static str, i32)>,
    stamp: EvidenceStamp,
}

impl EvidenceProvider for CorpusEvidence {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation> {
        let QuestGate::Window(window) = gate else {
            return self.completion.observe_gate(gate, required_after);
        };
        let truth = if !self.stamp.meets(required_after) {
            Truth::Unknown
        } else {
            self.signals
                .iter()
                .find(|(quest, signal, _)| {
                    window.quest.0.as_ref() == *quest && window.signal.0.as_ref() == *signal
                })
                .map_or(Truth::Unknown, |(_, _, value)| {
                    window.values.test(&InclusiveRange {
                        min: Some(*value),
                        max: Some(*value),
                    })
                })
        };
        Some(Observation {
            truth,
            observed: Some(self.stamp),
        })
    }
}

fn quest_state(
    data: &api::game_data::SelectedGameData,
    signals: &[(&'static str, &'static str, i32)],
) -> Result<WorldState, String> {
    let facts = data
        .quest_identity()
        .ok_or_else(|| "quest facts unavailable".to_string())?;
    let catalog =
        QuestCatalog::from_identity(Some(facts)).map_err(|error| format!("catalog: {error:?}"))?;
    let stamp = EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 1,
        sequence: 1,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        catalog
            .iter()
            // A stage witness asserts completion only at the selected terminal value.
            .filter(|quest| {
                signals
                    .iter()
                    .find(|(id, _, _)| quest.id.0.as_ref() == *id)
                    .is_none_or(|(_, _, value)| {
                        facts
                            .rows
                            .iter()
                            .any(|row| row.id == quest.id.0.as_ref() && row.complete == *value)
                    })
            })
            .enumerate()
            .map(|(index, quest)| QuestStatusView {
                component_id: index as i32,
                name: quest.display.to_string(),
                colour: nav_planner::world_state::QUEST_COMPLETE_COLOUR,
            })
            .collect(),
        true,
    );
    let provider = CorpusEvidence {
        completion: QuestListEvidenceProvider::from_snapshot(
            &catalog,
            SnapshotView::new(Some(&snapshot), stamp),
        ),
        signals: signals.to_vec(),
        stamp,
    };
    let mut state = rich_state();
    for (quest, _, _) in signals {
        let facts = catalog
            .quest(quest)
            .map_err(|error| format!("quest {quest}: {error:?}"))?;
        state.quests.remove(facts.display.as_ref());
    }
    state.inventory_ready = true;
    state.equipment_ready = true;
    state.quest_evidence = Some(QuestEvidence::new(Arc::new(provider), stamp));
    Ok(state)
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
    row["costs"] = json!((0..case.goals.len())
        .map(|index| search.route(index).ok().map(|route| route.ticks))
        .collect::<Vec<_>>());
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
    row["costs"] = json!([search.route().ok().map(|route| route.ticks)]);
    row["proof"] = json!(format!("{:?}", search.proof()));
    row
}

fn observed_item_state() -> WorldState {
    let mut state = WorldState::empty();
    state.map_members = true;
    state.inventory_ready = true;
    state.equipment_ready = true;
    state
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
    let members = WorldState {
        map_members: true,
        ..WorldState::empty()
    };
    let rich = rich_state();
    let mut low = rich_state();
    low.combat_level = Some(3);
    let mut live = WorldState::empty();
    live.map_members = true;
    live.combat_level = Some(50);
    live.inv.insert(995, 60);
    let bronze_axe_id = data
        .item_by_alias("bronze_axe")
        .ok_or("missing bronze axe game-data item")?
        .id;
    let mut unknown_items = WorldState::empty();
    unknown_items.map_members = true;
    let observed_empty = observed_item_state();
    let mut carried_axe = observed_item_state();
    carried_axe.inv.insert(bronze_axe_id, 1);
    let mut worn_axe = observed_item_state();
    worn_axe.worn.insert(bronze_axe_id);
    let entrana_approach = [tile(2818, 3352, 0)];
    let port_sarim = [tile(3092, 3242, 0)];
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
        first(
            &world,
            Case {
                name: "stronghold_middle_booth_abandoned_no_path",
                from: tile(2452, 3481, 1),
                goals: &[tile(2449, 3480, 1), tile(2449, 3482, 1)],
                options: FindOptions {
                    allow_wilderness: true,
                    allow_bank_fetch: true,
                    ..legacy
                },
                state: &members,
                account: "members_empty",
            },
        ),
    ];
    cases.extend([
        many(
            &world,
            Case {
                name: "entrana_outbound_unknown_item_observation",
                from: tile(3092, 3242, 0),
                goals: &entrana_approach,
                options: legacy,
                state: &unknown_items,
                account: "items-unobserved",
            },
        ),
        many(
            &world,
            Case {
                name: "entrana_outbound_observed_empty",
                from: tile(3092, 3242, 0),
                goals: &entrana_approach,
                options: legacy,
                state: &observed_empty,
                account: "items-observed-empty",
            },
        ),
        many(
            &world,
            Case {
                name: "entrana_outbound_carried_bronze_axe",
                from: tile(3092, 3242, 0),
                goals: &entrana_approach,
                options: legacy,
                state: &carried_axe,
                account: "bronze-axe-carried",
            },
        ),
        many(
            &world,
            Case {
                name: "entrana_outbound_worn_bronze_axe",
                from: tile(3092, 3242, 0),
                goals: &entrana_approach,
                options: legacy,
                state: &worn_axe,
                account: "bronze-axe-worn",
            },
        ),
        many(
            &world,
            Case {
                name: "entrana_return_with_carried_bronze_axe",
                from: tile(2818, 3352, 0),
                goals: &port_sarim,
                options: legacy,
                state: &carried_axe,
                account: "bronze-axe-carried",
            },
        ),
    ]);
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
    // The Waterfall account needs no glory for the wilderness-allowed route.
    // Keep both goal shapes to detect first-goal-only completeness gaps.
    let waterfall = WorldState {
        inv: HashMap::from([(954, 1), (995, 60)]),
        worn: HashSet::from([1163, 1333, 1113, 1201, 1079]),
        inventory_ready: true,
        equipment_ready: true,
        stats: (0..21)
            .map(|id| (id, if id < 7 && id != 5 { 40 } else { 1 }))
            .collect(),
        combat_level: Some(46),
        map_members: true,
        ..WorldState::empty()
    };
    let mut almera = Vec::new();
    for x in 2514..=2530 {
        for z in 3490..=3506 {
            let goal = tile(x, z, 0);
            if world.collision.standable(goal) {
                almera.push(goal);
            }
        }
    }
    for (area_name, exact_name, allow_teleports) in [
        (
            "waterfall_area_teles_off",
            "waterfall_exact_teles_off",
            false,
        ),
        ("waterfall_area_teles_on", "waterfall_exact_teles_on", true),
    ] {
        let options = FindOptions {
            allow_teleports,
            allow_wilderness: true,
            ..FindOptions::default()
        };
        cases.push(first(
            &world,
            Case {
                name: area_name,
                from: tile(3092, 3243, 0),
                goals: &almera,
                options,
                state: &waterfall,
                account: "waterfall",
            },
        ));
        cases.push(many(
            &world,
            Case {
                name: exact_name,
                from: tile(3092, 3243, 0),
                goals: &[tile(2530, 3490, 0)],
                options,
                state: &waterfall,
                account: "waterfall",
            },
        ));
    }
    let complete = quest_state(&data, &[])?;
    let direct = quest_state(
        &data,
        &[
            ("grandtree", "grandtree", 80),
            ("mm", "mm_main", 2),
            ("biohazard", "biohazard", 14),
        ],
    )?;
    let password = quest_state(
        &data,
        &[("grandtree", "grandtree", 80), ("mm", "mm_main", 1)],
    )?;
    let unreleased = quest_state(
        &data,
        &[("grandtree", "grandtree", 79), ("mm", "mm_main", 2)],
    )?;
    let searching = quest_state(&data, &[("biohazard", "biohazard", 10)])?;
    let observed = WorldState {
        map_members: true,
        inventory_ready: true,
        equipment_ready: true,
        combat_level: Some(126),
        ..WorldState::empty()
    };
    let mut legends_started = observed.clone();
    legends_started.quest_evidence =
        quest_state(&data, &[("legends", "legendsquest", 1)])?.quest_evidence;
    legends_started.combat_level = Some(40);
    legends_started.quests.insert("The Grand Tree".into());
    legends_started.varps.insert(139, 1);
    let mut legends_ready = complete.clone();
    legends_ready.inv.insert(714, 1);
    legends_ready.inv.insert(975, 1);
    legends_ready.inv.insert(1349, 1);
    for (gate, west, east) in [
        ("fortress", tile(3025, 3511, 1), tile(3026, 3511, 1)),
        ("shipyard_left", tile(2944, 3041, 0), tile(2945, 3041, 0)),
        ("shipyard_right", tile(2944, 3042, 0), tile(2945, 3042, 0)),
        ("guidor_left", tile(3263, 3405, 0), tile(3264, 3405, 0)),
        ("guidor_right", tile(3263, 3406, 0), tile(3264, 3406, 0)),
    ] {
        for (direction, from, to) in [("east", west, east), ("west", east, west)] {
            let mut accounts = vec![
                ("complete", &complete),
                ("unresolved", &observed),
                ("direct", &direct),
            ];
            if gate.starts_with("shipyard") {
                accounts.extend([("password", &password), ("unreleased", &unreleased)]);
            } else if gate.starts_with("guidor") {
                accounts.push(("searching", &searching));
            }
            for (account, state) in accounts {
                cases.push(first(
                    &world,
                    Case {
                        name: &format!("gate_{gate}_{direction}_{account}"),
                        from,
                        goals: &[to],
                        options: legacy,
                        state,
                        account,
                    },
                ));
            }
        }
    }
    for (name, from, to) in [
        (
            "legends_guild_inside_to_outside",
            tile(2726, 3368, 0),
            tile(2729, 3348, 0),
        ),
        (
            "legends_guild_outside_to_inside",
            tile(2729, 3348, 0),
            tile(2726, 3368, 0),
        ),
    ] {
        let mut row = first(
            &world,
            Case {
                name,
                from,
                goals: &[to],
                options: legacy,
                state: &legends_started,
                account: "legends_started",
            },
        );
        row["crossing_edges"] = json!(world
            .graph
            .edges
            .iter()
            .filter(|edge| [2391, 2392].contains(&edge.loc_id) && legends_started.allows(edge))
            .map(|edge| json!({"edge": format!("{edge:?}"), "allowed": true}))
            .collect::<Vec<_>>());
        cases.push(row);
    }

    let legends_exit = tile(2729, 3348, 0);
    let sindarpos = tile(2850, 3497, 0);
    let blockers = find_blocking_zones(
        &world.collision,
        &world.graph,
        legends_exit,
        sindarpos,
        FindOptions::default(),
        &legends_started,
        &[],
    )
    .ok_or("Legends exit to Sindarpos has no blocking-zone witness")?;
    let zone_grants = ZoneExempt::named(&blockers)
        .map_err(|error| format!("too many Legends route zones: {error:?}"))?;
    let mut refused = first(
        &world,
        Case {
            name: "legends_exit_to_sindarpos_no_zone_grant",
            from: legends_exit,
            goals: &[sindarpos],
            options: FindOptions::default(),
            state: &legends_started,
            account: "legends_started",
        },
    );
    refused["zone_witness"] = json!(blockers
        .iter()
        .map(|key| format!("{key:?}"))
        .collect::<Vec<_>>());
    cases.push(refused);
    cases.push(first(
        &world,
        Case {
            name: "legends_exit_to_sindarpos_named_zone_grants",
            from: legends_exit,
            goals: &[sindarpos],
            options: FindOptions {
                zones: zone_grants,
                ..FindOptions::default()
            },
            state: &legends_started,
            account: "legends_started_named_zones",
        },
    ));
    cases.push(first(
        &world,
        Case {
            name: "legends_guild_to_sindarpos_named_zone_grants",
            from: tile(2726, 3368, 0),
            goals: &[sindarpos],
            options: FindOptions {
                zones: zone_grants,
                ..FindOptions::default()
            },
            state: &legends_started,
            account: "legends_started_named_zones",
        },
    ));

    let kharazi_center = tile(2806, 2930, 0);
    let mut kharazi_goals = Vec::new();
    for x in kharazi_center.x - 3..=kharazi_center.x + 3 {
        for z in kharazi_center.z - 3..=kharazi_center.z + 3 {
            let goal = tile(x, z, kharazi_center.level);
            if world.collision.standable(goal) {
                kharazi_goals.push(goal);
            }
        }
    }
    cases.push(many(
        &world,
        Case {
            name: "legends_gandius_to_kharazi_radius3",
            from: tile(2971, 2969, 0),
            goals: &kharazi_goals,
            options: legacy,
            state: &legends_ready,
            account: "legends_complete_tools",
        },
    ));
    cases.push(first(
        &world,
        Case {
            name: "legends_gandius_to_exact_blocked_target",
            from: tile(2971, 2969, 0),
            goals: &[kharazi_center],
            options: legacy,
            state: &legends_ready,
            account: "legends_complete_tools",
        },
    ));
    let mut no_machette = legends_ready.clone();
    no_machette.inv.remove(&975);
    cases.push(many(
        &world,
        Case {
            name: "legends_gandius_to_kharazi_without_machette",
            from: tile(2971, 2969, 0),
            goals: &kharazi_goals,
            options: legacy,
            state: &no_machette,
            account: "legends_complete_no_machette",
        },
    ));
    let mut obstacle_row = |name: String,
                            id: i32,
                            from: WorldTile,
                            to: WorldTile,
                            state: &WorldState,
                            account: &str,
                            options: FindOptions| {
        let mut row = first(
            &world,
            Case {
                name: &name,
                from,
                goals: &[to],
                options,
                state,
                account,
            },
        );
        row["crossing_edges"] = json!(world
            .graph
            .edges
            .iter()
            .filter(|edge| edge.loc_id == id && edge.takeoff == Some(from) && edge.to == to)
            .map(|edge| json!({"edge": format!("{edge:?}"), "allowed": state.allows(edge)}))
            .collect::<Vec<_>>());
        cases.push(row);
    };
    for (direction, from, to) in [
        ("north", tile(2817, 3561, 0), tile(2817, 3564, 0)),
        ("south", tile(2817, 3564, 0), tile(2817, 3561, 0)),
    ] {
        obstacle_row(
            format!("obstacle_stile_{direction}"),
            3730,
            from,
            to,
            &observed,
            "unresolved",
            legacy,
        );
    }
    for x in [2856, 2857] {
        for (direction, from, to) in [
            ("north", tile(x, 3611, 0), tile(x, 3613, 0)),
            ("south", tile(x, 3613, 0), tile(x, 3611, 0)),
        ] {
            for (account, progress, agility, worn) in [
                ("ready", 10, 15, true),
                ("stage9", 9, 15, true),
                ("agility14", 10, 14, true),
                ("carried_boots", 10, 15, false),
                ("unresolved", -1, 15, true),
            ] {
                let mut state = if progress < 0 {
                    observed.clone()
                } else {
                    quest_state(&data, &[("troll", "troll_quest", progress)])?
                };
                state.stats.insert(16, agility);
                state.inv.insert(3105, 1);
                if worn {
                    state.worn.insert(3105);
                }
                obstacle_row(
                    format!("obstacle_lower_{x}_{direction}_{account}"),
                    3748,
                    from,
                    to,
                    &state,
                    account,
                    legacy,
                );
            }
        }
    }
    for (west, east, top_west, zs) in [
        (2858, 2861, true, [3626, 3627]),
        (2877, 2880, false, [3622, 3623]),
    ] {
        for z in zs {
            for (direction, id, from, to) in [
                (
                    "east",
                    if top_west { 3790 } else { 3791 },
                    tile(west, z, 0),
                    tile(east, z, 0),
                ),
                (
                    "west",
                    if top_west { 3791 } else { 3790 },
                    tile(east, z, 0),
                    tile(west, z, 0),
                ),
            ] {
                for agility in [14, 15] {
                    let mut state = observed.clone();
                    state.stats.insert(16, agility);
                    obstacle_row(
                        format!("obstacle_upper_{west}_{z}_{direction}_agility{agility}"),
                        id,
                        from,
                        to,
                        &state,
                        "skill_only",
                        legacy,
                    );
                }
            }
        }
    }
    for (id, from, to) in [
        (2049, tile(2556, 3299, 0), tile(2559, 3299, 0)),
        (2048, tile(2556, 3300, 0), tile(2559, 3300, 0)),
        (2048, tile(2559, 3299, 0), tile(2556, 3299, 0)),
        (2049, tile(2559, 3300, 0), tile(2556, 3300, 0)),
    ] {
        for (account, stage) in [
            ("stage15", 15),
            ("stage16", 16),
            ("stage17", 17),
            ("unresolved", -1),
        ] {
            let state = if stage < 0 {
                observed.clone()
            } else {
                quest_state(&data, &[("biohazard", "biohazard", stage)])?
            };
            obstacle_row(
                format!("obstacle_wall_{}_{}_{account}", from.x, from.z),
                id,
                from,
                to,
                &state,
                account,
                legacy,
            );
        }
    }
    for agility in [17, 18] {
        let mut state = observed.clone();
        state.stats.insert(16, agility);
        obstacle_row(
            format!("obstacle_window_agility{agility}"),
            2299,
            tile(2548, 3119, 0),
            tile(2548, 3117, 1),
            &state,
            "skill_only",
            legacy,
        );
    }
    for (account, stage) in [("stage12", 12), ("stage13", 13), ("unresolved", -1)] {
        let state = if stage < 0 {
            observed.clone()
        } else {
            quest_state(&data, &[("itwatchtower", "itwatchtower", stage)])?
        };
        obstacle_row(
            format!("obstacle_tower_ladder_{account}"),
            2796,
            tile(2549, 3112, 1),
            tile(2549, 3112, 2),
            &state,
            account,
            legacy,
        );
    }
    for (id, z) in [(2866, 9873), (2865, 9874)] {
        for (direction, from, to) in [
            ("east", tile(2902, z, 0), tile(2903, z, 0)),
            ("west", tile(2903, z, 0), tile(2902, z, 0)),
        ] {
            for (account, worn) in [("worn_gloves", true), ("carried_gloves", false)] {
                let mut state = observed.clone();
                state.inv.insert(1059, 1);
                if worn {
                    state.worn.insert(1059);
                }
                obstacle_row(
                    format!("obstacle_cellar_{z}_{direction}_{account}"),
                    id,
                    from,
                    to,
                    &state,
                    account,
                    legacy,
                );
            }
        }
    }
    let mut troll = quest_state(&data, &[("troll", "troll_quest", 10)])?;
    troll.stats.insert(16, 15);
    troll.worn.insert(3105);
    troll.combat_level = Some(3);
    troll.quests.insert("Death Plateau".into());
    obstacle_row(
        "obstacle_troll_hut_to_arena_strict".into(),
        0,
        tile(2820, 3555, 0),
        tile(2896, 3618, 0),
        &troll,
        "stage10_boots_agility15_combat3",
        FindOptions::default(),
    );
    let mut tower = quest_state(&data, &[("itwatchtower", "itwatchtower", 12)])?;
    tower.stats.insert(16, 18);
    tower.combat_level = Some(3);
    obstacle_row(
        "obstacle_tower_street_to_wizard_strict".into(),
        0,
        tile(2612, 3093, 0),
        tile(2544, 3112, 2),
        &tower,
        "stage12_agility18_combat3",
        FindOptions::default(),
    );
    // Reused locs and additional operable stands follow the same handlers.
    // Keep these input rows on both packs, even when the baseline has no edge.
    let mut reused = quest_state(
        &data,
        &[
            ("troll", "troll_quest", 10),
            ("itwatchtower", "itwatchtower", 12),
        ],
    )?;
    reused.worn.extend([3105, 1059]);
    for (id, from, to) in [
        (3730, tile(2816, 3563, 0), tile(2817, 3561, 0)),
        (3730, tile(2816, 3562, 0), tile(2817, 3564, 0)),
        (3730, tile(2818, 3562, 0), tile(2817, 3564, 0)),
        (3748, tile(2822, 3635, 0), tile(2820, 3635, 0)),
        (3748, tile(2820, 3635, 0), tile(2822, 3635, 0)),
        (3748, tile(2833, 3629, 0), tile(2833, 3627, 0)),
        (3748, tile(2833, 3627, 0), tile(2833, 3629, 0)),
        (3748, tile(2834, 3629, 0), tile(2834, 3627, 0)),
        (3748, tile(2834, 3627, 0), tile(2834, 3629, 0)),
        (3748, tile(2911, 3686, 0), tile(2909, 3686, 0)),
        (3748, tile(2909, 3686, 0), tile(2911, 3686, 0)),
        (3748, tile(2911, 3687, 0), tile(2909, 3687, 0)),
        (3748, tile(2909, 3687, 0), tile(2911, 3687, 0)),
        (2299, tile(2932, 4719, 0), tile(2548, 3117, 1)),
        (2796, tile(2548, 3111, 1), tile(2549, 3112, 2)),
        (2796, tile(2550, 3111, 1), tile(2549, 3112, 2)),
        (2796, tile(2932, 4711, 1), tile(2549, 3112, 2)),
        (2796, tile(2933, 4712, 1), tile(2549, 3112, 2)),
        (2796, tile(2934, 4711, 1), tile(2549, 3112, 2)),
    ] {
        obstacle_row(
            format!("obstacle_reused_{id}_{}_{}_{}", from.x, from.z, from.level),
            id,
            from,
            to,
            &reused,
            "ready",
            legacy,
        );
    }
    let mut transports: Vec<_> = world
        .graph
        .edges
        .iter()
        .map(|edge| format!("{edge:?}"))
        .collect();
    transports.sort_unstable();
    let result = json!({
        "schema": "274bot-route-parity-v1",
        "revision": 289,
        "cases": cases,
        "transports": transports,
    });
    std::fs::write(
        &args[1],
        serde_json::to_vec_pretty(&result).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}
