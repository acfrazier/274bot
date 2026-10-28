use super::*;
use client::client::{Client, ClientConfig};
use client::config::if_type::{ComponentType, IfTypeMut};
use client::config::IfType;
use client::dash3d::ClientPlayer;
use client::io::{ClientProt, ServerProt};
use nav_planner::grid::StepGrid;
use nav_planner::tile::Tile;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use crate::{Companion, Scenario, ScenarioSettings, Seed, Step, StepKind, Sustain};

fn cfg() -> ClientConfig {
    ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: crate::NO_CACHE_DIR.into(),
        members: true,
        lowmem: false,
    }
}

/// A synthetic client that has already seeded: ingame, scene 2, a
/// mainland build base, and the family gens bumped so the first
/// rebuild counts as a game tick.
fn seeded_client() -> Client {
    let mut c = Client::new(cfg());
    c.ingame = true;
    c.set_cheat_admission(client::CheatAdmission::Granted);
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(20, 20));
    for prot in [
        ServerProt::PLAYER_INFO,
        ServerProt::REBUILD_NORMAL,
        ServerProt::UPDATE_STAT,
    ] {
        c.bump_gens(prot);
    }
    c
}

fn stat_scenario(min: i32, budget: u32) -> Scenario {
    Scenario {
        name: "run-energy",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "set run energy",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    c.runenergy = 99;
                    true
                }),
            },
            wait: wait(Proof::Stat { id: 16, min }, budget),
        }],
        proof: Proof::Stat { id: 16, min },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

#[test]
fn stop_script_requires_a_host_observed_idle_slot() {
    let mut scenario = stat_scenario(0, 100);
    scenario.steps[0].kind = StepKind::StopScript;
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    let mut c = seeded_client();
    assert_running_for_game_ticks(
        &mut runner,
        &mut c,
        4,
        "Stop must not pass on unchanged game stats",
    );
    assert!(
        runner.on_stop_script(),
        "without a host Stop observation the runner must retain its stop step"
    );
    runner.observe_script_idle(false);
    assert_running_for_game_ticks(&mut runner, &mut c, 2, "a still-running slot is not Stop");
    runner.observe_script_idle(true);
    advance_game_ticks_until_done(&mut runner, &mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

/// A failing Await appends its read-only terminal summary to the
/// receipt message, on its own tick budget and on the whole-run
/// deadline alike; an Await without one keeps the plain message.
#[test]
fn await_terminal_summary_reaches_the_failure_receipt() {
    fn await_runner(
        summary: Option<Box<dyn Fn() -> String + Send + Sync>>,
        budget: u32,
        deadline: Duration,
    ) -> ScenarioRunner {
        let mut scenario = stat_scenario(0, budget);
        scenario.steps[0].kind = StepKind::Await {
            evidence: "never_ready",
            ready: Box::new(|_| false),
            summary,
        };
        scenario.settings.deadline = deadline;
        let mut runner = ScenarioRunner::new(scenario);
        runner.set_scene_settle(Duration::ZERO);
        runner
    }
    let summary = || -> Option<Box<dyn Fn() -> String + Send + Sync>> {
        Some(Box::new(|| "unmet=[alpha beta] s0 disp=1".to_string()))
    };

    let mut runner = await_runner(summary(), 3, Duration::from_secs(3600));
    let mut c = seeded_client();
    advance_game_ticks_until_done(&mut runner, &mut c);
    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("budget failure expected: {:?}", runner.status());
    };
    assert!(
        message.contains("never_ready") || message.contains("not seen within 3"),
        "{message}"
    );
    assert!(
        message.contains("[unmet=[alpha beta] s0 disp=1]"),
        "{message}"
    );

    let mut runner = await_runner(summary(), 100_000, Duration::ZERO);
    let mut c = seeded_client();
    for _ in 0..10 {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
        if matches!(runner.status(), RunnerStatus::Failed(_)) {
            break;
        }
    }
    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("deadline failure expected: {:?}", runner.status());
    };
    assert!(message.contains("deadline"), "{message}");
    assert!(
        message.contains("[unmet=[alpha beta] s0 disp=1]"),
        "{message}"
    );

    let mut runner = await_runner(None, 3, Duration::from_secs(3600));
    let mut c = seeded_client();
    advance_game_ticks_until_done(&mut runner, &mut c);
    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("budget failure expected: {:?}", runner.status());
    };
    assert!(!message.contains("unmet=["), "{message}");
}

/// A passing Await records its read-only terminal summary as the
/// pass receipt's message, so observation values in the summary
/// reach the receipt on a pass too. An Await without one keeps a
/// pass receipt with no message.
#[test]
fn await_terminal_summary_reaches_the_pass_receipt() {
    fn passing_runner(summary: Option<Box<dyn Fn() -> String + Send + Sync>>) -> ScenarioRunner {
        let mut scenario = stat_scenario(0, 100);
        scenario.steps[0].kind = StepKind::Await {
            evidence: "already_ready",
            ready: Box::new(|_| true),
            summary,
        };
        let mut runner = ScenarioRunner::new(scenario);
        runner.set_scene_settle(Duration::ZERO);
        runner
    }

    let mut runner = passing_runner(Some(Box::new(|| {
        "kq_witness unmet=[] restock_overlap=false".to_string()
    })));
    let mut c = seeded_client();
    advance_game_ticks_until_done(&mut runner, &mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    let evidence = runner.evidence().expect("pass evidence");
    assert_eq!(evidence.outcome, "PASS");
    assert_eq!(
        evidence.message.as_deref(),
        Some("kq_witness unmet=[] restock_overlap=false"),
        "the pass receipt records the summary's observation values"
    );

    let mut runner = passing_runner(None);
    let mut c = seeded_client();
    advance_game_ticks_until_done(&mut runner, &mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    let evidence = runner.evidence().expect("pass evidence");
    assert_eq!(evidence.message, None);
}

#[test]
fn wait_script_stop_is_none_unless_the_scenario_sets_it() {
    let v1 = ScenarioRunner::new(crate::get("bone_burier").expect("v1"));
    assert_eq!(v1.wait_script_stop(), None);
    let v2 = ScenarioRunner::new(crate::get("bone_burier_v2_ts").expect("v2"));
    assert_eq!(
        v2.wait_script_stop(),
        Some("confirmed loaded current-generation bank exhaustion")
    );
}

fn jail_watch_waits_for_key(name: &str, key: i32, other_key: i32) {
    let mut scenario = crate::get(name).unwrap();
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    scenario.steps.drain(..start);
    scenario.seed.mainland = false;
    scenario.settings.require_mainland_base = false;
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    let mut c = seeded_client();
    set_world_tile(
        &mut c,
        WorldTile {
            x: 2884,
            z: 9798,
            level: 0,
        },
    );
    set_inv(&mut c, &[]);
    for _ in 0..80 {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
        assert!(
            matches!(runner.status(), RunnerStatus::Running { .. }),
            "{name}: the dungeon walk must not start the clean-stop grace"
        );
    }
    for stacks in [&[(other_key, 1)][..], &[(key, 0)][..]] {
        set_inv(&mut c, stacks);
        runner.tick(&mut c);
        assert!(
            matches!(runner.status(), RunnerStatus::Running { .. }),
            "{name}: the wrong key or an empty stack is not completion"
        );
    }
    set_inv(&mut c, &[(key, 1)]);
    advance_game_ticks_until_done(&mut runner, &mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed, "{name}");
}

#[test]
fn acquire_key_watch_waits_for_the_jail_key() {
    jail_watch_waits_for_key("acquire_key_v2_ts", 1591, 1590);
}

#[test]
fn cell_watch_waits_for_the_dusty_key_not_the_seeded_jail_key() {
    jail_watch_waits_for_key("cell_v2_ts", 1590, 1591);
}

/// The catalog scenario from its StartScript step on, over a synthetic
/// client standing on the mainland landing the seed releases on.
fn from_start(name: &str) -> (ScenarioRunner, Client) {
    let mut scenario = crate::get(name).unwrap();
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    scenario.steps.drain(..start);
    scenario.seed.mainland = false;
    scenario.settings.require_mainland_base = false;
    let mut runner = ScenarioRunner::with_world(scenario, None);
    runner.set_scene_settle(Duration::ZERO);
    let mut c = seeded_client();
    set_world_tile(&mut c, crate::MAINLAND_LANDING);
    (runner, c)
}

fn assert_running_for_game_ticks(
    runner: &mut ScenarioRunner,
    c: &mut Client,
    game_ticks: usize,
    why: &str,
) {
    for _ in 0..game_ticks {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(c);
        assert!(
            matches!(runner.status(), RunnerStatus::Running { .. }),
            "{why}: {:?}",
            runner.status()
        );
    }
}

fn landing_offset(dx: i32) -> WorldTile {
    WorldTile {
        x: crate::MAINLAND_LANDING.x + dx,
        ..crate::MAINLAND_LANDING
    }
}

#[test]
fn walking_cards_wait_for_their_own_walk_off_the_landing() {
    // (scenario, a walk still short of the card's outcome, its outcome)
    for (name, short, outcome) in [
        ("hold_spot_v2_ts", 1, 2),
        ("retreat_spot_v2_ts", 1, 6),
        ("walk_spot_v2_ts", 12, 13),
        ("enter_lair_v2_ts", 6, 9),
        ("leave_lair_v2_ts", 2, 3),
    ] {
        let (mut runner, mut c) = from_start(name);
        assert_running_for_game_ticks(
            &mut runner,
            &mut c,
            40,
            &format!("{name}: the seeded landing is not the card's walk"),
        );
        assert!(
            !runner.wants_script_paint(),
            "{name}: a card without a receipt watch never reads paint"
        );
        set_world_tile(&mut c, landing_offset(short));
        assert_running_for_game_ticks(
            &mut runner,
            &mut c,
            4,
            &format!("{name}: {short} tiles is short of the card's destination"),
        );
        set_world_tile(&mut c, landing_offset(outcome));
        advance_game_ticks_until_done(&mut runner, &mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed, "{name}");
    }
}

#[test]
fn bank_card_waits_for_its_walk_to_the_falador_bank() {
    let (mut runner, mut c) = from_start("bank_v2_ts");
    let bank = |z| WorldTile {
        x: 2946,
        z,
        level: 0,
    };
    // The seeded Falador fountain, then a walk one tile short of radius 3.
    for (tile, why) in [
        (
            WorldTile {
                x: 2949,
                z: 3381,
                level: 0,
            },
            "the seeded fountain is not the bank trip",
        ),
        (bank(3373), "Chebyshev 4 is outside the bank's radius"),
    ] {
        set_world_tile(&mut c, tile);
        assert_running_for_game_ticks(&mut runner, &mut c, 20, why);
    }
    set_world_tile(&mut c, bank(3372));
    advance_game_ticks_until_done(&mut runner, &mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn prayer_cards_wait_for_protect_from_melee_on() {
    use client::config::{Cache, VarpType};
    for name in ["prayer_v1_ts", "prayer_v2_ts"] {
        let (mut runner, mut c) = from_start(name);
        // The seed's state: prayer 43, every transmitted overlay varp off.
        c.cache = Arc::new(Cache {
            varps: (0..98).map(|_| VarpType::default()).collect(),
            ..Default::default()
        });
        c.var = vec![0; 98];
        c.stat_effective_level[5] = 43;
        c.stat_base_level[5] = 43;
        c.bump_gens(ServerProt::VARP_SYNC);
        c.bump_gens(ServerProt::UPDATE_STAT);
        assert_running_for_game_ticks(
            &mut runner,
            &mut c,
            40,
            &format!("{name}: seeded prayer 43 with every overlay off"),
        );
        c.var[96] = 1;
        c.bump_gens(ServerProt::VARP_SYNC);
        assert_running_for_game_ticks(
            &mut runner,
            &mut c,
            4,
            &format!("{name}: Protect from Missiles is not the card's prayer"),
        );
        c.var[96] = 0;
        c.var[97] = 1;
        c.bump_gens(ServerProt::VARP_SYNC);
        advance_game_ticks_until_done(&mut runner, &mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed, "{name}");
    }
}

#[test]
fn observing_cards_pass_only_on_their_own_painted_receipt() {
    for (name, prefix) in [
        ("line_of_sight_v2_ts", "los-receipt:"),
        ("actor_observation_v2_ts", "actor-receipt:"),
        ("fight_field_v2_ts", "fight-field-receipt:"),
    ] {
        let (mut runner, mut c) = from_start(name);
        let receipt = format!("{prefix}{{\"ok\":true}}");
        // Paint published before StartScript belongs to no card of this run.
        runner.observe_script_paint([receipt.as_str()]);
        assert_running_for_game_ticks(&mut runner, &mut c, 1, &format!("{name}: StartScript"));
        assert!(runner.wants_script_paint(), "{name}");
        runner.observe_script_paint(["watching", "result pending"]);
        assert_running_for_game_ticks(
            &mut runner,
            &mut c,
            40,
            &format!("{name}: a card that has not painted its receipt"),
        );
        runner.observe_script_paint(["line of sight v2", receipt.as_str()]);
        assert!(!runner.wants_script_paint(), "{name}: latched");
        advance_game_ticks_until_done(&mut runner, &mut c);
        assert_eq!(runner.status(), RunnerStatus::Passed, "{name}");
        let evidence = runner.evidence().expect("evidence");
        assert_eq!(evidence.receipt.as_deref(), Some(receipt.as_str()));
    }
}

/// The selected cache's items: every named item by its real id, and the
/// seeding cheats' own aliases (`give jail_key 1`), so a seeded "Jail key"
/// is object 1591 and seeded "Bones" are 526, exactly as the engine gives
/// them, not merely the lowest id that shares the name.
struct GameItems {
    data: Arc<api::game_data::SelectedGameData>,
    names: Arc<ObjNames>,
}

impl GameItems {
    fn load() -> Self {
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let objs = data
            .items()
            .iter()
            .filter_map(|item| {
                item.name.as_ref().map(|name| client::config::ObjType {
                    id: item.id,
                    name: name.clone(),
                    ..Default::default()
                })
            })
            .collect::<Vec<_>>();
        let names = Arc::new(ObjNames::from_objs(&objs));
        Self { data, names }
    }

    fn seeded_id(&self, name: &str) -> Option<i32> {
        let alias = name.to_lowercase().replace(' ', "_");
        self.data
            .item_by_alias(&alias)
            .map(|item| item.id)
            .or_else(|| self.names.by_name(name))
    }
}

/// The state a scenario's own seed has certified at Start: a fresh account
/// (level 1, Hitpoints 10, full run energy), on the mainland landing when it
/// seeds there, then its fixture prerequisites — the pre-Start arms prepare
/// and run-prepared acknowledge — applied in order. A position arm keeps
/// every tile it admits as a candidate, so a radius cannot hide a proof the
/// seed already stands in.
struct SeededState {
    client: Client,
    tiles: Vec<WorldTile>,
}

impl SeededState {
    fn of(scenario: &Scenario, items: &GameItems) -> Self {
        let mut client = seeded_client();
        for skill in 0..client.stat_base_level.len() {
            let (level, xp) = if skill == 3 { (10, 1154) } else { (1, 0) };
            client.stat_base_level[skill] = level;
            client.stat_effective_level[skill] = level;
            client.stat_xp[skill] = xp;
        }
        client.runenergy = 100;
        client.bump_gens(ServerProt::UPDATE_STAT);
        client.bump_gens(ServerProt::UPDATE_RUNENERGY);
        let mut tiles = vec![crate::MAINLAND_LANDING];
        let mut stacks: Vec<(i32, i32)> = Vec::new();
        let mut varps: Vec<(i32, i32)> = Vec::new();
        let mut side_tabs = Vec::new();
        let around = |x: i32, z: i32, level: i32, radius: i32| {
            (-radius..=radius)
                .flat_map(|dx| {
                    (-radius..=radius).map(move |dz| WorldTile {
                        x: x + dx,
                        z: z + dz,
                        level,
                    })
                })
                .collect::<Vec<_>>()
        };
        let mut stack = |id: i32, count: i32| {
            stacks.retain(|(held, _)| *held != id);
            stacks.push((id, count));
        };
        for arm in crate::fixture_prereqs_of(scenario) {
            match arm {
                // `arrived` also admits a stand next to a solid destination.
                Proof::Arrived { x, z, level } => tiles = around(x, z, level, 1),
                Proof::ArrivedNear {
                    x,
                    z,
                    level,
                    radius,
                } => tiles = around(x, z, level, radius),
                Proof::Stat { id: 16, min } => client.runenergy = min,
                Proof::Stat { id, min } => {
                    client.stat_base_level[id as usize] = min;
                    client.stat_effective_level[id as usize] = min;
                }
                Proof::Item { name, count } => stack(
                    items
                        .seeded_id(name)
                        .unwrap_or_else(|| panic!("{}: no item named {name}", scenario.name)),
                    count,
                ),
                Proof::ItemId { id, count } => stack(id, count),
                Proof::SideTabAvailable { index } => side_tabs.push(index as usize),
                Proof::Varp { id, min: value } | Proof::VarpExact { id, value } => {
                    varps.retain(|(held, _)| *held != id);
                    varps.push((id, value));
                }
                other => panic!(
                    "{}: model the pre-Start arm {other:?} in SeededState before trusting \
                     its terminal proof",
                    scenario.name
                ),
            }
        }
        set_inv(&mut client, &stacks);
        let inv = client
            .iface_id(|f| f.r#type == ComponentType::TYPE_INV)
            .expect("synthetic inventory component");
        for index in side_tabs {
            client.side_icon[index] = inv as i32;
        }
        client.bump_gens(ServerProt::IF_SETICON);
        if let Some(max) = varps.iter().map(|(id, _)| *id).max() {
            client.cache = Arc::new(client::config::Cache {
                varps: (0..=max)
                    .map(|_| client::config::VarpType::default())
                    .collect(),
                ..Default::default()
            });
            client.var = vec![0; max as usize + 1];
            for (id, value) in &varps {
                client.var[*id as usize] = *value;
            }
            client.bump_gens(ServerProt::VARP_SYNC);
        }
        set_world_tile(&mut client, tiles[0]);
        Self { client, tiles }
    }

    /// A seeded tile on which `proof` already holds, XP gains measured from
    /// the seeded XP exactly as StartScript anchors them.
    fn satisfies(&mut self, proof: Proof, names: &ObjNames) -> Option<WorldTile> {
        let baselines = (0..self.client.stat_xp.len() as i32)
            .map(|skill| (skill, self.client.stat_xp[skill as usize]))
            .collect::<Vec<_>>();
        let fresh = match proof {
            Proof::FreshStatXpGain { id, .. } => Some((id, self.client.stat_xp[id as usize])),
            _ => None,
        };
        for &tile in &self.tiles.clone() {
            set_world_tile(&mut self.client, tile);
            let mut snapshot = GameSnapshot::new();
            snapshot.rebuild(&self.client);
            if proof.check_with_xp_context(&snapshot, Some(names), Some(&baselines), fresh) {
                return Some(tile);
            }
        }
        set_world_tile(&mut self.client, self.tiles[0]);
        None
    }
}

/// Catalog-wide: a card the host waits on for a clean stop must also be
/// waited on for its own work. Every scenario with `wait_script_stop` needs
/// a post-Start watch and a terminal proof its own pre-Start seed cannot
/// already satisfy, or it passes on the Start snapshot and the clean-stop
/// grace becomes the only check. The seeded state is evaluated, not pattern
/// matched: seeded levels (prayer 43), carried keys and seeded positions
/// count.
#[test]
fn clean_stop_scenarios_are_not_satisfied_by_their_own_seed() {
    let items = GameItems::load();
    let names = Arc::clone(&items.names);
    let mut checked = Vec::new();
    let mut vacuous = Vec::new();
    for name in crate::names() {
        let Some(scenario) = crate::get(name) else {
            continue;
        };
        if scenario.settings.wait_script_stop.is_none() {
            continue;
        }
        checked.push(name);
        let Some(start) = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
        else {
            vacuous.push(format!("{name}: a clean stop without a StartScript step"));
            continue;
        };
        if start + 1 == scenario.steps.len() {
            vacuous.push(format!("{name}: no post-Start watch"));
        }
        let mut seeded = SeededState::of(&scenario, &items);
        if let Some(tile) = seeded.satisfies(scenario.proof, &names) {
            vacuous.push(format!(
                "{name}: terminal proof {} holds on its pre-Start seed at {tile:?}",
                scenario.proof.name()
            ));
        }
        if scenario.steps[start + 1..]
            .iter()
            .all(|step| seeded.satisfies(step.wait.arm, &names).is_some())
        {
            vacuous.push(format!(
                "{name}: every post-Start watch holds on its pre-Start seed"
            ));
        }

        // The runner itself, from Start with the seeded state frozen (the
        // card doing nothing), must never report PASS.
        let mut from_start = crate::get(name).unwrap();
        from_start.steps.drain(..start);
        from_start.seed.mainland = false;
        from_start.settings.require_mainland_base = false;
        let mut runner = ScenarioRunner::with_world(from_start, None);
        runner.set_scene_settle(Duration::ZERO);
        runner.set_obj_names(Arc::clone(&names));
        for _ in 0..300 {
            seeded.client.bump_gens(ServerProt::PLAYER_INFO);
            runner.tick(&mut seeded.client);
            match runner.status() {
                RunnerStatus::Passed => {
                    vacuous.push(format!("{name}: the runner passed on the seed alone"));
                    break;
                }
                RunnerStatus::Failed(_) => break,
                RunnerStatus::Seeding | RunnerStatus::Running { .. } => {}
            }
        }
    }
    assert!(vacuous.is_empty(), "{vacuous:#?}");
    for name in [
        "line_of_sight_v2_ts",
        "actor_observation_v2_ts",
        "fight_field_v2_ts",
        "hold_spot_v2_ts",
        "retreat_spot_v2_ts",
        "walk_spot_v2_ts",
        "enter_lair_v2_ts",
        "leave_lair_v2_ts",
        "bank_v2_ts",
        "prayer_v1_ts",
        "prayer_v2_ts",
        "acquire_key_v2_ts",
        "cell_v2_ts",
    ] {
        assert!(checked.contains(&name), "{name} lost its clean-stop wait");
    }
}

#[test]
fn terminal_snapshot_is_released_after_shot_and_evidence() {
    for failed in [false, true] {
        let mut client = seeded_client();
        client.runenergy = 73;
        let mut runner = ScenarioRunner::with_world(stat_scenario(1, 10), None);
        runner.snapshot.rebuild(&client);
        let terminal = serde_json::to_value(&runner.snapshot).unwrap();
        let empty = serde_json::to_value(GameSnapshot::new()).unwrap();
        assert_ne!(terminal, empty);
        let shots = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&shots);
        runner.set_terminal_shot("terminal");
        runner.set_shot_sink(Box::new(move |label, snapshot| {
            captured
                .lock()
                .unwrap()
                .push((label.to_owned(), serde_json::to_value(snapshot).unwrap()));
        }));
        if failed {
            runner.finish_fail("original failure");
        } else {
            runner.finish_pass();
        }
        assert_eq!(
            *shots.lock().unwrap(),
            vec![("terminal".to_owned(), terminal)]
        );
        let evidence = runner.evidence().unwrap();
        assert_eq!(evidence.tile, Some([3220, 3220, 0]));
        assert_eq!(evidence.scene, 2);
        assert_eq!(evidence.stat.as_ref().unwrap().runenergy, 73);
        assert_eq!(evidence.outcome, if failed { "FAIL" } else { "PASS" });
        assert_eq!(
            evidence.message.as_deref(),
            failed.then_some("original failure")
        );
        let retained_evidence = evidence.to_json();
        let retained_status = format!("{:?}", runner.status());
        runner.tick_with_hold(&mut client, false);
        assert_eq!(serde_json::to_value(&runner.snapshot).unwrap(), empty);
        client.runenergy = 1;
        client.ingame = false;
        runner.tick_with_hold(&mut client, true);
        runner.tick(&mut client);
        assert_eq!(runner.evidence().unwrap().to_json(), retained_evidence);
        assert_eq!(format!("{:?}", runner.status()), retained_status);
        assert_eq!(shots.lock().unwrap().len(), 1);
        assert_eq!(serde_json::to_value(&runner.snapshot).unwrap(), empty);
    }
}

#[test]
fn follow_failure_keeps_snapshot_for_existing_same_client_cycle_budget_failure() {
    let mut client = seeded_client();
    let mut runner = ScenarioRunner::with_world(stat_scenario(999, 1), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.phase = Phase::Running;
    runner.step_sent = true;
    runner.route = Some(Route {
        dest: WorldTile {
            x: 3221,
            z: 3220,
            level: 0,
        },
        legs: vec![nav_planner::router::Leg::Walk {
            tiles: vec![
                WorldTile {
                    x: 3220,
                    z: 3220,
                    level: 0,
                },
                WorldTile {
                    x: 3221,
                    z: 3220,
                    level: 0,
                },
            ],
        }],
        ticks: 1.0,
    });
    let shots = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&shots);
    runner.set_terminal_shot("failure");
    runner.set_shot_sink(Box::new(move |_, snap| {
        captured.lock().unwrap().push(snap.tile());
    }));
    runner.tick(&mut client);
    // Existing behavior: follow refusal does not short-circuit the budget
    // check. Memory cleanup must not change either callback's snapshot.
    assert_eq!(*shots.lock().unwrap(), vec![Some((3220, 3220, 0)); 2]);
    let evidence = runner.evidence().unwrap();
    assert_eq!(evidence.tile, Some([3220, 3220, 0]));
    assert!(evidence
        .message
        .as_ref()
        .unwrap()
        .contains("not seen within 1 game tick"));
}

#[test]
fn new_reads_deadline_and_terminal_shot_from_settings() {
    let runner = ScenarioRunner::new(crate::get("nav_full").unwrap());
    assert_eq!(runner.deadline(), Duration::from_secs(360));
    assert_eq!(runner.terminal_shot(), Some("nav_full terminal"));
}

#[test]
fn record_position_names_the_run_before_the_first_tick() {
    let scenario = crate::get("nav_full").unwrap();
    let total = scenario.steps.len();
    let runner = ScenarioRunner::new(scenario);
    assert_eq!(runner.scenario_name(), "nav_full");
    assert_eq!(runner.record_step(), (0, total, "seeding"));
    assert_eq!(runner.total_ticks(), 0);
}

#[test]
fn hold_skips_follow_and_keeps_the_armed_route() {
    use nav_planner::router::Route;

    let mut c = seeded_client();
    let dest = WorldTile {
        x: 4,
        z: 0,
        level: 0,
    };
    let mut runner = ScenarioRunner::new(Scenario {
        name: "hold-follow",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "walk",
            kind: StepKind::Walk { dest },
            wait: wait(
                Proof::Arrived {
                    x: 4,
                    z: 0,
                    level: 0,
                },
                100,
            ),
        }],
        proof: Proof::Arrived {
            x: 4,
            z: 0,
            level: 0,
        },
        companions: vec![],
        settings: ScenarioSettings::default(),
    });
    runner.set_scene_settle(Duration::ZERO);
    // Skip seeding: force Running with an already-armed route.
    runner.phase = Phase::Running;
    runner.route = Some(Route {
        dest,
        legs: vec![],
        ticks: 0.0,
    });
    runner.step_sent = true;
    runner.tick_with_hold(&mut c, true);
    assert!(
        runner.route.is_some(),
        "hold must not consume the latched scenario route"
    );
}

#[test]
fn new_leaves_the_mainland_base_gate_off_by_default() {
    let mut c = Client::new(cfg());
    let mut runner = ScenarioRunner::new(stat_scenario(1, 10));
    runner.set_scene_settle(Duration::ZERO);
    c.ingame = true;
    c.scene_state = 2;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "default gate off: scene 2 on a tutorial-scale base starts steps"
    );
}

#[test]
fn perform_send_then_arm_fires_and_proof_passes() {
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::new(stat_scenario(1, 10));
    runner.set_scene_settle(Duration::ZERO);
    assert_eq!(runner.status(), RunnerStatus::Seeding);
    // Client cycle 1: seed completes, send runs (runenergy 99 lands on the
    // client), and the stale snapshot still shows 0.
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
    // Client cycle 2: the stat bump refreshes the snapshot; the arm fires and
    // the proof passes on the same poll.
    c.bump_gens(ServerProt::UPDATE_RUNENERGY);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    let ev = runner.evidence().expect("evidence at PASS");
    assert_eq!(ev.outcome, "PASS");
    assert_eq!(ev.predicate, "stat(16)>=1");
    assert_eq!(ev.tile, Some([3220, 3220, 0]));
}

#[test]
fn perform_budget_fails_when_the_arm_never_fires() {
    let mut c = seeded_client();
    // The no-op send leaves runenergy at 0; min 999 never holds.
    let mut runner = ScenarioRunner::new(stat_scenario(999, 3));
    runner.set_scene_settle(Duration::ZERO);
    for _ in 0..5 {
        c.bump_gens(ServerProt::UPDATE_RUNENERGY);
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
    }
    match runner.status() {
        RunnerStatus::Failed(msg) => {
            assert!(
                msg.contains("not seen within 3 game ticks"),
                "fail message names the arm and budget: {msg}"
            );
            assert!(msg.contains("stat(16)>=999"), "names the arm: {msg}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    let ev = runner.evidence().expect("evidence at FAIL");
    assert_eq!(ev.outcome, "FAIL");
    assert_eq!(ev.predicate, "stat(16)>=999");
    assert!(ev.message.is_some());
}

#[test]
fn poll_sustain_sends_the_cheat_when_energy_is_at_or_below_the_line() {
    let mut scenario = stat_scenario(1, 10);
    scenario.settings.sustains = vec![Sustain::poll(
        Proof::StatAtMost { id: 16, max: 25 },
        "~energy",
    )];
    let mut c = seeded_client();
    c.runenergy = 10;
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert_eq!(
        c.out.data().first().copied(),
        Some(ClientProt::CLIENT_CHEAT.id as u8),
        "~energy is CLIENT_CHEAT, not a host set_run"
    );
}

fn fast_speed_scenario(arm: Proof, budget: u32) -> Scenario {
    let mut scenario = stat_scenario(0, budget);
    scenario.name = "quester-fast";
    scenario.steps[0].kind = StepKind::Perform {
        send: Box::new(|_, _| true),
    };
    scenario.steps[0].wait = wait(arm, budget);
    scenario.proof = arm;
    scenario.settings.nav.engine_speed_ms = Some(300);
    scenario.settings.teardown_world_speed_ms = Some(600);
    scenario
}

fn contains_outgoing_ascii(client: &Client, text: &str) -> bool {
    client
        .out
        .data()
        .windows(text.len())
        .any(|window| window == text.as_bytes())
}

fn outgoing_ascii_count(client: &Client, text: &str) -> usize {
    client
        .out
        .data()
        .windows(text.len())
        .filter(|window| *window == text.as_bytes())
        .count()
}

fn push_system_chat(client: &mut Client, text: &str) {
    client.add_chat(0, text, "");
    client.bump_gens(ServerProt::MESSAGE_GAME);
}

#[test]
fn world_speed_requires_fresh_confirmation_and_resets_after_pass() {
    let mut runner = ScenarioRunner::new(fast_speed_scenario(
        Proof::WorldSpeedChanged { ms: 300 },
        10,
    ));
    runner.set_scene_settle(Duration::ZERO);
    let mut client = seeded_client();

    // An older confirmation of a different rate neither satisfies the step
    // nor suppresses the requested change.
    push_system_chat(&mut client, "World speed was changed to 600ms");
    runner.tick(&mut client);
    assert!(contains_outgoing_ascii(&client, "speed 300"));
    assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
    assert!(!contains_outgoing_ascii(&client, "speed 600"));

    // Only a fresh 300 confirmation newer than the command completes the
    // step; the runner then restores 600 during terminal cleanup.
    push_system_chat(&mut client, "World speed was changed to 300ms");
    runner.tick(&mut client);
    assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
    runner.tick(&mut client);
    assert!(contains_outgoing_ascii(&client, "speed 600"));
    assert!(matches!(runner.status(), RunnerStatus::Running { .. }));

    push_system_chat(&mut client, "World speed was changed to 600ms");
    runner.tick(&mut client);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn world_speed_resets_after_failed_run() {
    let mut runner = ScenarioRunner::new(fast_speed_scenario(Proof::Stat { id: 16, min: 999 }, 1));
    runner.set_scene_settle(Duration::ZERO);
    let mut client = seeded_client();

    runner.tick(&mut client);
    assert!(contains_outgoing_ascii(&client, "speed 300"));
    assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
    runner.tick(&mut client);
    assert!(contains_outgoing_ascii(&client, "speed 600"));

    push_system_chat(&mut client, "World speed was changed to 600ms");
    runner.tick(&mut client);
    match runner.status() {
        RunnerStatus::Failed(message) => {
            assert!(message.contains("not seen within 1 game tick"), "{message}");
        }
        other => panic!("expected failed run after speed reset, got {other:?}"),
    }
}

#[test]
fn default_scenario_sends_no_world_speed_command() {
    let mut runner = ScenarioRunner::new(stat_scenario(0, 10));
    runner.set_scene_settle(Duration::ZERO);
    let mut client = seeded_client();
    runner.tick(&mut client);
    assert_eq!(
        client.cheat_packets_sent(),
        0,
        "a scenario that asks for no speed sends no cheat"
    );
    assert!(!contains_outgoing_ascii(&client, "speed "));
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn explicit_non_default_tick_sends_exactly_one_world_speed_command() {
    let mut runner = ScenarioRunner::new(fast_speed_scenario(
        Proof::WorldSpeedChanged { ms: 300 },
        10,
    ));
    runner.set_scene_settle(Duration::ZERO);
    let mut client = seeded_client();
    runner.tick(&mut client);
    runner.tick(&mut client);
    assert_eq!(
        outgoing_ascii_count(&client, "speed 300"),
        1,
        "the initial request is sent once, not once per client cycle"
    );
    assert_eq!(outgoing_ascii_count(&client, "speed 600"), 0);
}

#[test]
fn explicit_default_tick_sends_even_with_an_older_matching_confirmation() {
    let mut scenario = stat_scenario(0, 10);
    scenario.settings.nav.engine_speed_ms = Some(600);
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    let mut client = seeded_client();
    push_system_chat(&mut client, "World speed was changed to 600ms");
    runner.tick(&mut client);
    assert_eq!(
        outgoing_ascii_count(&client, "speed 600"),
        1,
        "a matching confirmation already in the ring does not suppress the request"
    );
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn world_speed_confirm_step_requires_a_line_newer_than_its_send() {
    let mut scenario = stat_scenario(0, 10);
    scenario.steps[0].kind = StepKind::Perform {
        send: Box::new(|_, _| true),
    };
    scenario.steps[0].wait = wait(Proof::WorldSpeedChanged { ms: 600 }, 10);
    scenario.settings.nav.engine_speed_ms = Some(600);
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    let mut client = seeded_client();

    // The ring already reports the requested rate. It predates this run's
    // send, so it cannot satisfy the confirm step.
    push_system_chat(&mut client, "World speed was changed to 600ms");
    runner.tick(&mut client);
    assert_eq!(outgoing_ascii_count(&client, "speed 600"), 1);
    assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
    runner.tick(&mut client);
    assert!(
        matches!(runner.status(), RunnerStatus::Running { .. }),
        "the pre-existing confirmation is stale"
    );

    // Only a confirmation newer than the send confirms the step.
    push_system_chat(&mut client, "World speed was changed to 600ms");
    runner.tick(&mut client);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn seeding_waits_for_ingame_scene2_and_mainland_base() {
    let mut c = Client::new(cfg());
    // The gate is opt-in: this test exercises it, so the scenario
    // turns it on. `new_leaves_the_mainland_base_gate_off_by_default`
    // covers the default-off path.
    let mut scenario = stat_scenario(1, 10);
    scenario.settings.require_mainland_base = true;
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Seeding);

    // Ingame with a tutorial-scale base must still hold.
    c.ingame = true;
    c.scene_state = 2;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Seeding,
        "a sub-3000 base is still seeding (tutorial island)"
    );

    // The queued hop may leave the initial ready frame.
    c.scene_state = 1;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Seeding);

    // A mainland build base and the requested landing release the seed.
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(20, 20));
    c.bump_gens(ServerProt::PLAYER_INFO);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
}

#[test]
fn mainland_seed_requires_a_completed_hop_before_releasing() {
    let mut c = Client::new(cfg());
    c.ingame = true;
    c.scene_state = 2;
    // The observed tutorial frame can carry a mainland-looking build
    // base and a tile inside the full-world pack.
    c.map_build_base_x = 3072;
    c.map_build_base_z = 3072;
    c.local_player = Some(ClientPlayer::at(22, 34));
    c.bump_gens(ServerProt::PLAYER_INFO);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut scenario = stat_scenario(1, 10);
    scenario.seed.mainland = true;
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);

    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Seeding);

    // An unrelated rebuild on the tutorial island is not the requested
    // mainland arrival, even though its base lies inside the full pack.
    c.local_player = Some(ClientPlayer::at(20, 20));
    c.bump_gens(ServerProt::PLAYER_INFO);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Seeding);

    // The actual mainland landing releases the seed, without requiring
    // an intermediate scene_state != 2 frame.
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.bump_gens(ServerProt::PLAYER_INFO);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
}

// --- Whole-world `Walk` steps (find + Traveller::follow) ---

fn walk_scenario(dest: WorldTile, budget: u32) -> Scenario {
    Scenario {
        name: "walk",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "walk the corridor",
            kind: StepKind::Walk { dest },
            wait: wait(
                Proof::Arrived {
                    x: dest.x,
                    z: dest.z,
                    level: dest.level,
                },
                budget,
            ),
        }],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

/// A `Walk` step routes exactly like `Follow`: `nav_planner::router::find`
/// over the whole-world `NavWorld` (collision + transport graph),
/// driven by `Traveller::follow` one leg per delivered frame.
#[test]
fn walk_step_arms_find_and_proofs_arrival() {
    let mut c = follow_client();
    let dest = WorldTile {
        x: 3205,
        z: 3230,
        level: 0,
    };
    let grid = StepGrid::fixture_rect_at(
        Tile {
            x: 3205,
            z: 3200,
            level: 0,
        },
        1,
        40,
    );
    let world = NavWorld::from_grid(&grid);
    let mut runner = ScenarioRunner::with_world(walk_scenario(dest, 120), Some(Arc::new(world)));
    runner.set_scene_settle(Duration::ZERO);

    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "send armed the whole-world route"
    );
    assert!(
        runner.armed_route().is_some(),
        "the headed paint reads this route as the red baked path"
    );

    // Advance the player north one tile per client cycle; the follow polls its
    // settle on every delivered frame and the arm fires on the exact
    // destination tile.
    let mut steps = 0;
    while runner.status() != RunnerStatus::Passed {
        if steps > 80 {
            panic!("walk never arrived; status={:?}", runner.status());
        }
        steps += 1;
        c.local_player = Some(ClientPlayer::at(5, steps));
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
    }
    let ev = runner.evidence().expect("evidence at PASS");
    assert_eq!(ev.tile, Some([3205, 3230, 0]));
    assert_eq!(ev.predicate, "arrived(3205,3230,0)");
    assert!(ev.ticks > 0, "the walk counted delivered frames");
}

#[test]
fn walk_step_fails_without_a_nav_world() {
    let mut c = follow_client();
    let dest = WorldTile {
        x: 3205,
        z: 3230,
        level: 0,
    };
    let scenario = walk_scenario(dest, 120);
    // `with_world(None)` injects no router world: the walk step must
    // fail with a clear pack message.
    let mut runner = ScenarioRunner::with_world(scenario, None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    match runner.status() {
        RunnerStatus::Failed(msg) => {
            assert!(
                msg.contains("no nav world"),
                "clear world error names the pack: {msg}"
            )
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

// --- Whole-world `Follow` steps (find + Traveller::follow) ---

/// A synthetic client with a connected stream (so the snapshot is
/// `attached` and `Interactions::walk` passes its preconditions),
/// seeded on a mainland base, standing at world (3205, 3200) — scene
/// (5, 0), off the fresh client's `_BOUNDS` border columns.
fn follow_client() -> Client {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let stream =
        client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    // Keep the listener alive so the connect stays established.
    std::mem::forget(listener);
    let mut c = Client::new(cfg());
    c.stream = Some(stream);
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(5, 0));
    for prot in [
        ServerProt::PLAYER_INFO,
        ServerProt::REBUILD_NORMAL,
        ServerProt::UPDATE_STAT,
    ] {
        c.bump_gens(prot);
    }
    c
}

fn follow_scenario(dest: WorldTile, budget: u32) -> Scenario {
    Scenario {
        name: "follow",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "follow the corridor",
            kind: StepKind::Follow { dest },
            wait: wait(
                Proof::Arrived {
                    x: dest.x,
                    z: dest.z,
                    level: dest.level,
                },
                budget,
            ),
        }],
        proof: Proof::Arrived {
            x: dest.x,
            z: dest.z,
            level: dest.level,
        },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

/// A 1×40 corridor along z at x=3205 (scene x=5, off the fresh
/// client's `_BOUNDS` border columns): the route is forced, so the
/// follow's hops cannot wander like a 0-cost Dijkstra does on an open
/// plane. The `find`-then-`follow` mechanics are what is under test.
#[test]
fn follow_step_arms_find_and_proofs_arrival() {
    let mut c = follow_client();
    let dest = WorldTile {
        x: 3205,
        z: 3230,
        level: 0,
    };
    let grid = StepGrid::fixture_rect_at(
        Tile {
            x: 3205,
            z: 3200,
            level: 0,
        },
        1,
        40,
    );
    let world = NavWorld::from_grid(&grid);
    let mut runner = ScenarioRunner::with_world(follow_scenario(dest, 120), Some(Arc::new(world)));
    runner.set_scene_settle(Duration::ZERO);

    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "send armed the whole-world route"
    );

    // Advance the player north one tile per client cycle; the follow polls its
    // settle on every delivered frame and the arm fires on the exact
    // destination tile.
    let mut steps = 0;
    while runner.status() != RunnerStatus::Passed {
        if steps > 80 {
            panic!("follow never arrived; status={:?}", runner.status());
        }
        steps += 1;
        c.local_player = Some(ClientPlayer::at(5, steps));
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
    }
    let ev = runner.evidence().expect("evidence at PASS");
    assert_eq!(ev.tile, Some([3205, 3230, 0]));
    assert_eq!(ev.predicate, "arrived(3205,3230,0)");
    assert!(ev.ticks > 0, "the walk counted delivered frames");
}

#[test]
fn follow_step_fails_without_a_nav_world() {
    let mut c = follow_client();
    let dest = WorldTile {
        x: 3205,
        z: 3230,
        level: 0,
    };
    // `with_world(None)` injects no router world.
    let mut runner = ScenarioRunner::with_world(follow_scenario(dest, 120), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    match runner.status() {
        RunnerStatus::Failed(msg) => {
            assert!(
                msg.contains("no nav world"),
                "clear world error names the pack: {msg}"
            )
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

// --- Dialog-janitor steps (`DrainDialogs`) ---

/// A `DrainDialogs` step whose arm never holds: the fixture has no
/// chat modal, so every re-send is a harmless refusal. The step must
/// keep re-sending each client cycle (never count as "already sent")
/// and fail when its observed game-tick budget expires, not on the first send.
#[test]
fn drain_dialogs_resends_every_client_cycle_and_fails_on_game_tick_budget() {
    let scenario = Scenario {
        name: "drain",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "drain",
            kind: StepKind::DrainDialogs { choice: 1 },
            wait: wait(Proof::Stat { id: 16, min: 999 }, 3),
        }],
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut c = follow_client();
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    // Client cycles 1-4: the janitor re-sends each cycle (no chat choice
    // on the fixture -> refused, which is fine) while game ticks advance.
    for _ in 0..4 {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
    }
    match runner.status() {
        RunnerStatus::Failed(msg) => {
            assert!(
                msg.contains("not seen within 3 game ticks"),
                "the drain step fails on its game-tick budget: {msg}"
            );
        }
        other => panic!("expected Failed after the budget, got {other:?}"),
    }
}

// --- Scene-settle gate (the 377 `waitSceneReady` + settle idea) ---

/// A perform step whose send lands run energy 99 **and** drops the
/// scene to 1 (a cheat tele's rebuild): the arm `stat(16) >= 99`
/// holds once the rebuilt snapshot decodes the 99, so any progress
/// during the hold must be the gate, not the arm.
fn scene_drop_scenario(budget: u32) -> Scenario {
    Scenario {
        name: "scene-settle",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "tele and land run energy",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    c.runenergy = 99;
                    c.scene_state = 1; // the tele's scene rebuild
                    true
                }),
            },
            wait: wait(Proof::Stat { id: 16, min: 99 }, budget),
        }],
        proof: Proof::Stat { id: 16, min: 99 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

/// With `set_scene_settle(Duration::ZERO)` the hold is only
/// `scene_state == 2`: a perform send that drops the scene to 1 must
/// not advance while the scene stays 1 (even though the arm holds on
/// the rebuilt snapshot), and the arm fires the tick the scene
/// returns to 2.
#[test]
fn scene_settle_zero_holds_while_scene_state_is_not_2() {
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::new(scene_drop_scenario(100));
    runner.set_scene_settle(Duration::ZERO);
    // Tick 1: seed completes; the send drops the scene to 1.
    runner.tick(&mut c);
    assert_eq!(
        c.scene_state, 1,
        "the send dropped the scene (tele rebuild)"
    );
    assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
    // Scene still 1: the snapshot now decodes run energy 99 (the arm
    // would hold), but the runner must hold — no step advance.
    c.bump_gens(ServerProt::UPDATE_RUNENERGY);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "holds while scene_state != 2 even when the arm holds"
    );
    // Scene returns to 2: with a zero settle the gate releases and
    // the arm fires on the same game tick.
    c.scene_state = 2;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    let ev = runner.evidence().expect("evidence at PASS");
    assert_eq!(ev.outcome, "PASS");
    assert_eq!(ev.predicate, "stat(16)>=99");
}

/// The wall-clock settle: with a non-zero `scene_settle` the runner
/// holds even while the scene is 2, until the scene has held 2 for
/// the settle. The first step's send is held too, so a tele's rebuild
/// cannot be stepped onto.
#[test]
fn scene_settle_holds_steps_until_the_scene_has_held_2_for_the_settle() {
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::new(scene_drop_scenario(100));
    runner.set_scene_settle(Duration::from_millis(30));
    // Tick 1: the scene just became 2 (the seed completed); the
    // settle has not elapsed, so the step's send is held back.
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
    assert_eq!(c.runenergy, 0, "the send is held until the settle elapses");
    // After the settle: the send fires and drops the scene to 1.
    std::thread::sleep(Duration::from_millis(40));
    runner.tick(&mut c);
    assert_eq!(c.runenergy, 99, "the send fired once the settle elapsed");
    assert_eq!(c.scene_state, 1);
    // Scene 1: hold, regardless of the elapsed settle.
    c.bump_gens(ServerProt::UPDATE_RUNENERGY);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "a scene drop below 2 resets the settle clock"
    );
    // Scene back to 2: the arm holds on the snapshot, but the settle
    // has not elapsed again, so the step still does not advance.
    c.scene_state = 2;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "the step waits out the settle even after the scene returns to 2"
    );
    // Once the settle elapses, the arm fires and the run passes.
    std::thread::sleep(Duration::from_millis(40));
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

// --- Companion slots (fleet runs) ---

/// A 2-profile scenario: profile 0 is the driven slot, profile 1 is a
/// companion whose per-frame hook flips a captured flag. Ticking the
/// driven slot runs the walker steps; ticking the companion slot runs
/// the hook.
#[test]
fn companion_slots_tick_their_hook_while_the_driven_slot_runs() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let flipped = Arc::new(AtomicBool::new(false));
    let hook = Arc::clone(&flipped);
    let scenario = Scenario {
        name: "companion",
        seed: Seed {
            profiles: vec![("test", "test"), ("test2", "test2")],
            mainland: false,
        },
        steps: vec![Step {
            name: "set run energy",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    c.runenergy = 99;
                    true
                }),
            },
            wait: wait(Proof::Stat { id: 16, min: 1 }, 10),
        }],
        proof: Proof::Stat { id: 16, min: 1 },
        companions: vec![Companion {
            profile: 1,
            per_frame: Box::new(move |_| {
                hook.store(true, Ordering::Relaxed);
            }),
        }],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    assert_eq!(runner.companion_profile_name(0), "test2");
    assert_eq!(runner.companion_for("test2"), Some(0));
    assert_eq!(runner.companion_for("nobody"), None);

    // Driven slot: the walker's steps run to PASS.
    let mut c = seeded_client();
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 1 });
    c.bump_gens(ServerProt::UPDATE_RUNENERGY);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);

    // Companion slot: the hook flips the flag on tick.
    assert!(!flipped.load(Ordering::Relaxed));
    let index = runner
        .companion_for("test2")
        .expect("the companion slot resolves by profile name");
    runner.companion_tick(index, &mut c);
    assert!(flipped.load(Ordering::Relaxed));
}

/// Live boots mint per-run usernames: the runner drives/companions the
/// minted names (never the registry's `test`/`test2`), so the vault's
/// fresh accounts are the slots its per-frame hooks tick.
#[test]
fn live_names_replace_the_seed_names_for_dispatch() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let flipped = Arc::new(AtomicBool::new(false));
    let hook = Arc::clone(&flipped);
    let scenario = Scenario {
        name: "companion",
        seed: Seed {
            profiles: vec![("test", "test"), ("test2", "test2")],
            mainland: false,
        },
        steps: vec![Step {
            name: "set run energy",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    c.runenergy = 99;
                    true
                }),
            },
            wait: wait(Proof::Stat { id: 16, min: 1 }, 10),
        }],
        proof: Proof::Stat { id: 16, min: 1 },
        companions: vec![Companion {
            profile: 1,
            per_frame: Box::new(move |_| {
                hook.store(true, Ordering::Relaxed);
            }),
        }],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(scenario);
    let live = vec!["live1_0".to_string(), "live1_1".to_string()];
    runner.set_live_names(&live);
    runner.set_scene_settle(Duration::ZERO);
    assert_eq!(runner.profile_name(), "live1_0");
    assert_eq!(runner.companion_profile_name(0), "live1_1");
    assert!(runner.drives("live1_0"));
    assert!(
        !runner.drives("test"),
        "a live runner must not drive `test`"
    );
    assert_eq!(runner.companion_for("live1_1"), Some(0));
    assert_eq!(runner.companion_for("test2"), None);
}

#[test]
fn terminal_shot_fires_the_sink_on_pass_and_fail() {
    type SinkEvents = Arc<Mutex<Vec<(String, Option<(i32, i32, i32)>)>>>;
    let sink_events: SinkEvents = Arc::new(Mutex::new(Vec::new()));
    let pass_sink = Arc::clone(&sink_events);
    let mut pass = ScenarioRunner::new(stat_scenario(1, 10));
    pass.set_scene_settle(Duration::ZERO);
    pass.set_terminal_shot("t-pass");
    pass.set_shot_sink(Box::new(move |label, snap| {
        pass_sink
            .lock()
            .unwrap()
            .push((label.to_string(), snap.tile()));
    }));
    let mut c = seeded_client();
    pass.tick(&mut c);
    c.bump_gens(ServerProt::UPDATE_RUNENERGY);
    pass.tick(&mut c);
    assert_eq!(pass.status(), RunnerStatus::Passed);
    assert_eq!(
        sink_events.lock().unwrap().as_slice(),
        &[("t-pass".to_string(), Some((3220, 3220, 0)))],
        "the terminal shot fires once on PASS"
    );

    let fail_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let fail_sink = Arc::clone(&fail_events);
    let mut fail = ScenarioRunner::new(stat_scenario(999, 1));
    fail.set_scene_settle(Duration::ZERO);
    fail.set_terminal_shot("t-fail");
    fail.set_shot_sink(Box::new(move |label, _| {
        fail_sink.lock().unwrap().push(label.to_string());
    }));
    let mut c = seeded_client();
    fail.tick(&mut c);
    assert!(matches!(fail.status(), RunnerStatus::Failed(_)));
    assert_eq!(
        fail_events.lock().unwrap().as_slice(),
        &["t-fail".to_string()],
        "the terminal shot fires once on FAIL"
    );
}

#[test]
fn deadline_fails_a_stuck_run() {
    let mut c = Client::new(cfg());
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.local_player = Some(ClientPlayer::at(20, 20));
    c.bump_gens(ServerProt::PLAYER_INFO);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut runner = ScenarioRunner::new(stat_scenario(999, 999));
    runner.set_scene_settle(Duration::ZERO);
    runner.set_deadline(Duration::from_millis(1));
    let mut saw_failed = false;
    for _ in 0..20 {
        c.bump_gens(ServerProt::UPDATE_RUNENERGY);
        runner.tick(&mut c);
        if matches!(runner.status(), RunnerStatus::Failed(_)) {
            saw_failed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(saw_failed, "the deadline must fail a stuck run");
    match runner.status() {
        RunnerStatus::Failed(msg) => assert!(msg.contains("deadline"), "msg: {msg}"),
        other => panic!("expected Failed, got {other:?}"),
    }
    assert!(runner.deadline_exceeded(), "the deadline ended the run");
}

/// R3-F1 (M0 round 5): a step failure read after the deadline is still a
/// step failure. `deadline_exceeded` reports the recorded cause, not the
/// time at which the caller reads the status.
#[test]
fn a_step_failure_read_after_the_deadline_is_not_a_deadline_failure() {
    let mut runner = ScenarioRunner::new(stat_scenario(999, 1));
    runner.set_scene_settle(Duration::ZERO);
    runner.set_deadline(Duration::ZERO);
    let mut c = seeded_client();
    runner.tick(&mut c);
    let RunnerStatus::Failed(msg) = runner.status() else {
        panic!("expected Failed, got {:?}", runner.status());
    };
    assert!(msg.contains("not seen within 1 game tick"), "msg: {msg}");
    assert!(runner.started.elapsed() > runner.deadline());
    assert!(
        !runner.deadline_exceeded(),
        "a step failure is not a deadline failure"
    );
}

/// A one-step shot scenario: no send, the arm holds on the first
/// dirty tick (stat(16) >= 0 with the seeded client), and the proof
/// mirrors the arm.
fn shot_scenario(label: &'static str) -> Scenario {
    Scenario {
        name: "shot",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "shot the courtyard",
            kind: StepKind::Shot { label },
            wait: wait(Proof::Stat { id: 16, min: 0 }, 10),
        }],
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn shot_step_fires_the_sink_once_with_label_and_terminal_snapshot() {
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::new(shot_scenario("arrive courtyard"));
    runner.set_scene_settle(Duration::ZERO);
    type Fired = Arc<Mutex<Vec<(String, Option<(i32, i32, i32)>)>>>;
    let fired: Fired = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&fired);
    runner.set_shot_sink(Box::new(move |label, snap| {
        sink.lock().unwrap().push((label.to_string(), snap.tile()));
    }));
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    let fired = fired.lock().unwrap();
    assert_eq!(fired.len(), 1, "the sink fires once per shot step");
    assert_eq!(fired[0].0, "arrive courtyard");
    assert_eq!(
        fired[0].1,
        Some((3220, 3220, 0)),
        "the sink sees the terminal snapshot's player tile"
    );
}

fn start_script_scenario() -> Scenario {
    Scenario {
        name: "start-script",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![
            Step {
                name: "last seed wait",
                kind: StepKind::Shot { label: "seed" },
                wait: wait(Proof::Stat { id: 16, min: 0 }, 1),
            },
            Step {
                name: "start the catalog card",
                kind: StepKind::StartScript,
                wait: wait(Proof::Stat { id: 16, min: 0 }, 1),
            },
        ],
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

#[test]
fn start_script_step_is_a_client_noop_and_the_arm_holds_immediately() {
    let mut c = seeded_client();
    let energy = c.runenergy;
    let mut runner = ScenarioRunner::new(start_script_scenario());
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert!(
        runner.on_start_script(),
        "after the last seed wait, the live pumps see StartScript"
    );
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    assert_eq!(
        c.runenergy, energy,
        "StartScript is a no-op on the client (no cheat, no packet)"
    );
    assert!(
        !runner.on_start_script(),
        "the step has advanced; pumps must not Start again"
    );
}

fn compiled_start_scenario(budget: u32) -> Scenario {
    Scenario {
        name: "compiled-start",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "start the compiled card",
            kind: StepKind::StartScript,
            wait: wait(Proof::ScriptRunning, budget),
        }],
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

#[test]
fn compiled_start_script_waits_until_running() {
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::new(compiled_start_scenario(50));
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert!(
        runner.on_start_script(),
        "StartScript is current before the compiled Start settles"
    );
    assert_running_for_game_ticks(
        &mut runner,
        &mut c,
        8,
        "ScriptRunning must not hold before observe_script_running",
    );
    runner.observe_script_running();
    c.bump_gens(ServerProt::PLAYER_INFO);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn compiled_start_script_fails_with_the_rejection_reason() {
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::new(compiled_start_scenario(50));
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert!(runner.on_start_script());
    runner.fail_start("start rejected: Sherlock clue facts unavailable");
    match runner.status() {
        RunnerStatus::Failed(message) => {
            assert!(
                message.contains("start rejected: Sherlock clue facts unavailable"),
                "{message}"
            );
            assert!(message.contains("start the compiled card"), "{message}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn shot_step_with_the_default_sink_is_a_noop_and_passes() {
    // The headless twin leaves the default (no-op) sink: a Shot step
    // must still run to PASS without a window to capture.
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::new(shot_scenario("arrive courtyard"));
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn shot_step_writes_png_and_json_through_a_test_sink() {
    let dir = temp_dir("shot");
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::new(shot_scenario("arrive courtyard"));
    runner.set_scene_settle(Duration::ZERO);
    let sink_dir = dir.clone();
    let now = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_787_616_000);
    runner.set_shot_sink(Box::new(move |label, snap| {
        let json = serde_json::to_string_pretty(snap).expect("terminal snapshot serializes");
        // A known 4x4 RGBA buffer: no wgpu surface needed here.
        let rgba: Vec<u8> = (0..64).map(|i| (i * 4) as u8).collect();
        crate::shot::write_shot_at(&sink_dir, label, &rgba, 4, 4, &json, now)
            .expect("test sink writes the shot");
    }));
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    let png = dir.join("2026-08-25T00-00-00_arrive_courtyard.png");
    let json = dir.join("2026-08-25T00-00-00_arrive_courtyard.json");
    assert!(png.exists(), "one <label>.png written: {png:?}");
    assert!(json.exists(), "one <label>.json written: {json:?}");
    let header = std::fs::read(&png).unwrap();
    assert_eq!(&header[..8], b"\x89PNG\r\n\x1a\n");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(v["tile"], serde_json::json!([3220, 3220, 0]));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn relog_presses_the_logout_button_while_ingame() {
    let mut c = seeded_client();
    let com = IfType {
        client_code: api::interact::CC_LOGOUT,
        ..Default::default()
    };
    c.set_iface(2458, com);
    let scenario = Scenario {
        name: "relog",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "relog",
            kind: StepKind::Relog,
            wait: wait(
                Proof::QuestDone {
                    name: "The Grand Tree",
                },
                10,
            ),
        }],
        proof: Proof::QuestDone {
            name: "The Grand Tree",
        },
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert_eq!(
        c.out.data().first().copied(),
        Some(ClientProt::IF_BUTTON.id as u8),
        "clean logout is IF_BUTTON, not a socket drop"
    );
    assert!(
        matches!(runner.status(), RunnerStatus::Running { .. }),
        "still waiting to come back; we do not DC-wait"
    );
}

fn relog_before_start_with_arm(arm: Proof) -> (Client, ScenarioRunner) {
    let mut client = seeded_client();
    client.set_iface(
        2458,
        IfType {
            client_code: api::interact::CC_LOGOUT,
            ..Default::default()
        },
    );
    client.side_icon[3] = 100;
    client.bump_gens(ServerProt::IF_SETICON);
    let mut scenario = compiled_start_scenario(20);
    scenario.steps.insert(
        0,
        Step {
            name: "relog before native Start",
            kind: StepKind::Relog,
            wait: wait(arm, 20),
        },
    );
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    (client, runner)
}

fn relog_before_start() -> (Client, ScenarioRunner) {
    relog_before_start_with_arm(Proof::SideTabAvailable { index: 3 })
}

#[test]
fn relog_stale_side_tab_cannot_start_native_before_logout_and_new_login() {
    let (mut client, mut runner) = relog_before_start();
    runner.tick(&mut client);
    assert!(
        !runner.on_start_script(),
        "the old session's already-available tab cannot complete Relog"
    );
    let after_logout_request = client.out.pos;
    for _ in 0..3 {
        client.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut client);
        assert!(!runner.on_start_script());
    }
    assert_eq!(client.out.pos, after_logout_request);

    client.ingame = false;
    runner.tick(&mut client);
    assert!(!runner.on_start_script());

    client.ingame = true;
    client.gens.session = client.gens.session.wrapping_add(1);
    client.side_icon[3] = -1;
    client.bump_gens(ServerProt::IF_SETICON);
    runner.tick(&mut client);
    assert!(!runner.on_start_script(), "new login still needs its tab");
    client.side_icon[3] = 100;
    client.bump_gens(ServerProt::IF_SETICON);
    runner.tick(&mut client);
    assert!(runner.on_start_script());
}

#[test]
fn relog_logged_out_arm_advances_on_the_observed_off_world_frame() {
    let (mut client, mut runner) = relog_before_start_with_arm(Proof::LoggedOut);

    runner.tick(&mut client);
    assert!(
        !runner.on_start_script(),
        "the old online session and its stale available tab cannot complete LoggedOut"
    );
    let logout_request_len = client.out.pos;
    for _ in 0..3 {
        client.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut client);
        assert!(!runner.on_start_script());
    }
    assert_eq!(client.out.pos, logout_request_len);

    client.ingame = false;
    client.scene_state = 0;
    client.bump_gens(ServerProt::PLAYER_INFO);
    runner.tick(&mut client);
    assert!(
        runner.on_start_script(),
        "the explicit LoggedOut arm can enter StartScript before the new login posts colours"
    );
    assert_eq!(runner.record_step().2, "start the compiled card");
}

#[test]
fn relog_new_session_identity_allows_start_without_an_off_world_sample() {
    let (mut client, mut runner) = relog_before_start();
    runner.tick(&mut client);
    assert!(!runner.on_start_script());
    let after_logout_request = client.out.pos;
    client.gens.session = client.gens.session.wrapping_add(1);
    client.bump_gens(ServerProt::IF_SETICON);
    runner.tick(&mut client);
    assert!(runner.on_start_script());
    assert_eq!(client.out.pos, after_logout_request);
}

const AIR_RUINS: (i32, i32, i32) = (2988, 3294, 0);
const AIR_RUNE_ID: i32 = 556;
const RUNE_ESSENCE_ID: i32 = 1436;
const RUNECRAFT_STAT: i32 = 20;
const CRAFT_XP: i32 = 5;

fn set_inv(c: &mut Client, stacks: &[(i32, i32)]) {
    let types: Vec<i32> = stacks.iter().map(|(id, _)| id + 1).collect();
    let numbers: Vec<i32> = stacks.iter().map(|(_, n)| *n).collect();
    match c.iface_id(|f| f.r#type == ComponentType::TYPE_INV) {
        Some(id) => {
            let inv = c.iface_mut(id).unwrap();
            inv.link_obj_type = Some(types);
            inv.link_obj_number = Some(numbers);
        }
        None => {
            let id = c.push_iface(IfType {
                r#type: ComponentType::TYPE_INV,
                ..Default::default()
            });
            c.set_iface_mut(
                id,
                IfTypeMut {
                    link_obj_type: Some(types),
                    link_obj_number: Some(numbers),
                    ..Default::default()
                },
            );
        }
    }
    c.bump_gens(ServerProt::UPDATE_INV_FULL);
}

fn ruins_client() -> Client {
    let mut c = seeded_client();
    c.map_build_base_x = AIR_RUINS.0;
    c.map_build_base_z = AIR_RUINS.1;
    c.local_player = Some(ClientPlayer::at(0, 0));
    c.stat_xp[RUNECRAFT_STAT as usize] = 0;
    c.stat_effective_level[RUNECRAFT_STAT as usize] = 1;
    c.stat_base_level[RUNECRAFT_STAT as usize] = 1;
    set_inv(&mut c, &[(RUNE_ESSENCE_ID, 27)]);
    c.bump_gens(ServerProt::PLAYER_INFO);
    c.bump_gens(ServerProt::UPDATE_STAT);
    c
}

fn apply_simultaneous_craft(c: &mut Client) {
    set_inv(c, &[(AIR_RUNE_ID, 27)]);
    c.stat_xp[RUNECRAFT_STAT as usize] = CRAFT_XP;
    c.bump_gens(ServerProt::UPDATE_STAT);
}

fn rune_watch_scenario(arms: &[Proof]) -> Scenario {
    Scenario {
        name: "rune-observation-order",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: arms
            .iter()
            .enumerate()
            .map(|(i, arm)| Step {
                name: match i {
                    0 => "watch arrival at the selected mysterious ruins after Start",
                    1 if matches!(arm, Proof::StatXpGain { .. }) => {
                        "watch Runecraft XP from the selected craft"
                    }
                    1 => "watch essence become the selected rune after altar entry",
                    2 if matches!(arm, Proof::StatXpGain { .. }) => {
                        "watch Runecraft XP from the selected craft"
                    }
                    _ => "watch essence become the selected rune after altar entry",
                },
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: wait(*arm, 4),
            })
            .collect(),
        proof: Proof::ItemId {
            id: AIR_RUNE_ID,
            count: 1,
        },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

fn rc_baseline(runner: &ScenarioRunner) -> Option<i32> {
    runner
        .xp_baselines
        .iter()
        .find(|(id, _)| *id == RUNECRAFT_STAT)
        .map(|(_, xp)| *xp)
}

fn advance_game_ticks_until_done(runner: &mut ScenarioRunner, c: &mut Client) {
    for _ in 0..8 {
        if matches!(
            runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            return;
        }
        // Advance synthetic game time by posting `PLAYER_INFO`; the
        // `runner.tick` call below is one client-cycle poll.
        c.bump_gens(ServerProt::UPDATE_STAT);
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(c);
    }
}

#[test]
fn runecraft_xp_watch_before_product_catches_simultaneous_craft() {
    let ruins = Proof::ArrivedNear {
        x: AIR_RUINS.0,
        z: AIR_RUINS.1,
        level: AIR_RUINS.2,
        radius: 4,
    };
    let crafted = Proof::ItemId {
        id: AIR_RUNE_ID,
        count: 1,
    };
    let xp = Proof::StatXpGain {
        id: RUNECRAFT_STAT,
        min: 1,
    };

    let mut late_client = ruins_client();
    let mut late = ScenarioRunner::with_world(rune_watch_scenario(&[ruins, crafted, xp]), None);
    late.set_scene_settle(Duration::ZERO);
    late.tick(&mut late_client);
    assert_eq!(
        late.status(),
        RunnerStatus::Running { step: 1, total: 3 },
        "ruins arrival must arm the crafted-item watch first in the late order"
    );
    assert_eq!(rc_baseline(&late), None);
    apply_simultaneous_craft(&mut late_client);
    late.tick(&mut late_client);
    assert_eq!(
        rc_baseline(&late),
        Some(CRAFT_XP),
        "item-then-XP captures the already-applied craft XP as its baseline"
    );
    advance_game_ticks_until_done(&mut late, &mut late_client);
    match late.status() {
        RunnerStatus::Failed(msg) => {
            assert!(
                msg.contains("stat_xp_gain(20)>=1"),
                "late XP watch must name the missed arm: {msg}"
            );
            assert!(
                msg.contains("not seen within"),
                "late XP watch must miss the same-snapshot gain: {msg}"
            );
        }
        other => panic!("late item-then-XP order must fail, got {other:?}"),
    }

    let mut corrected_client = ruins_client();
    let mut corrected =
        ScenarioRunner::with_world(rune_watch_scenario(&[ruins, xp, crafted]), None);
    corrected.set_scene_settle(Duration::ZERO);
    corrected.tick(&mut corrected_client);
    assert_eq!(
        corrected.status(),
        RunnerStatus::Running { step: 1, total: 3 },
        "ruins arrival must arm the XP watch before product"
    );
    assert_eq!(rc_baseline(&corrected), Some(0));
    apply_simultaneous_craft(&mut corrected_client);
    corrected.tick(&mut corrected_client);
    assert_eq!(
        rc_baseline(&corrected),
        Some(0),
        "XP armed at ruins keeps the pre-craft baseline"
    );
    advance_game_ticks_until_done(&mut corrected, &mut corrected_client);
    assert_eq!(corrected.status(), RunnerStatus::Passed);

    let mut seed_client = ruins_client();
    let mut seed_only =
        ScenarioRunner::with_world(rune_watch_scenario(&[ruins, xp, crafted]), None);
    seed_only.set_scene_settle(Duration::ZERO);
    seed_only.tick(&mut seed_client);
    advance_game_ticks_until_done(&mut seed_only, &mut seed_client);
    match seed_only.status() {
        RunnerStatus::Failed(msg) => {
            assert!(
                msg.contains("stat_xp_gain(20)>=1"),
                "seed-only essence at ruins must not satisfy XP: {msg}"
            );
        }
        other => panic!("seed-only input must not pass, got {other:?}"),
    }
}

#[test]
fn ardy_cakes_fight_observes_both_skills_regardless_of_gain_order() {
    for (combat_before_start, thieving_first, thieving_gain, stall_food) in [
        (false, false, true, true),
        (false, true, true, true),
        (true, false, true, true),
        (false, false, false, false),
        (false, false, true, false),
    ] {
        let mut scenario = crate::get("ardy_cakes_fight").unwrap();
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        // Keep the real Start and proof sequence; preparation is already
        // acknowledged, just as it is when the live pump starts the card.
        scenario.steps.drain(..start);
        scenario.seed.mainland = false;
        scenario.settings.require_mainland_base = false;
        for step in &mut scenario.steps {
            step.wait.budget_ticks = 4;
        }
        let mut c = seeded_client();
        c.stat_xp[2] = 37_224;
        c.stat_xp[17] = 388;
        if combat_before_start {
            c.stat_xp[2] += 96;
        }
        let mut runner = ScenarioRunner::with_world(scenario, None);
        runner.set_scene_settle(Duration::ZERO);
        runner.tick(&mut c);

        // A roaming Guard can catch the first steal or a later one.
        // Either skill may gain XP before its individual watch begins.
        for thieving in [thieving_first, !thieving_first] {
            if thieving {
                if thieving_gain {
                    c.stat_xp[17] += 16;
                }
                if stall_food {
                    // A bread roll, not Cake: the widened stall-food arm
                    // must not gate on the exact stolen food.
                    set_inv(&mut c, &[(2309, 1)]);
                }
            } else if !combat_before_start {
                c.stat_xp[2] += 96;
            }
            c.bump_gens(ServerProt::UPDATE_STAT);
            runner.tick(&mut c);
        }
        advance_game_ticks_until_done(&mut runner, &mut c);

        if combat_before_start || !thieving_gain || !stall_food {
            assert!(
                matches!(runner.status(), RunnerStatus::Failed(_)),
                "seeded combat or missing post-Start stall evidence must not qualify"
            );
        } else {
            assert_eq!(
                runner.status(),
                RunnerStatus::Passed,
                "both post-Start XP gains must survive either watch order"
            );
        }
    }
}

const ARDY_FIGHT_STAND: WorldTile = WorldTile {
    x: 2668,
    z: 3312,
    level: 0,
};
const ARDY_FIGHT_BANK: WorldTile = WorldTile {
    x: 2655,
    z: 3286,
    level: 0,
};

fn ardy_fight_after_start() -> (ScenarioRunner, Client) {
    use client::client::ClientNpc;
    use client::config::{Cache, NpcType};

    let mut scenario = crate::get("ardy_cakes_fight").unwrap();
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    scenario.steps.drain(..start);
    scenario.seed.mainland = false;
    scenario.settings.require_mainland_base = false;
    let mut c = seeded_client();
    set_world_tile(&mut c, ARDY_FIGHT_STAND);
    set_inv(&mut c, &[]);
    c.cache = Arc::new(Cache {
        npcs: vec![NpcType {
            id: 0,
            name: "Guard".into(),
            size: 1,
            ..Default::default()
        }],
        ..Default::default()
    });
    for column in &mut c.collision[0].flags {
        column.fill(0);
    }
    let mut npc = ClientNpc {
        r#type: Some(0),
        ..Default::default()
    };
    npc.entity.route_x[0] = ARDY_FIGHT_STAND.x - c.map_build_base_x + 2;
    npc.entity.route_z[0] = ARDY_FIGHT_STAND.z - c.map_build_base_z;
    npc.entity.x = npc.entity.route_x[0] * 128 + 64;
    npc.entity.z = npc.entity.route_z[0] * 128 + 64;
    npc.entity.face_entity = -1;
    c.npc[3] = Some(Box::new(npc));
    c.npc_ids[0] = 3;
    // The pre-Start guard has wandered away; expose it again only on return.
    c.npc_count = 0;
    c.set_iface(
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601]),
            ..Default::default()
        },
    );
    c.set_iface(
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    let mut runner = ScenarioRunner::with_world(scenario, None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    (runner, c)
}

fn ardy_fight_bank_visit(runner: &mut ScenarioRunner, c: &mut Client) {
    set_world_tile(c, ARDY_FIGHT_BANK);
    c.main_modal_id = 600;
    c.bump_gens(ServerProt::IF_OPENMAIN);
    advance_one_game_tick(runner, c, false);
}

fn ardy_fight_catchless_first_session(runner: &mut ScenarioRunner, c: &mut Client) {
    c.stat_xp[17] += 16;
    c.bump_gens(ServerProt::UPDATE_STAT);
    set_inv(c, &[(1891, 6)]);
    for _ in 0..80 {
        advance_one_game_tick(runner, c, false);
    }
    ardy_fight_bank_visit(runner, c);
    set_inv(c, &[]);
    c.main_modal_id = -1;
    c.bump_gens(ServerProt::IF_CLOSE);
    advance_one_game_tick(runner, c, false);
    // The bank trip can outlast the old 150-dirty Strength watch.
    for _ in 0..75 {
        advance_one_game_tick(runner, c, false);
    }
    set_world_tile(c, ARDY_FIGHT_STAND);
}

#[test]
fn ardy_fight_accepts_combat_in_the_second_stall_session() {
    let (mut runner, mut c) = ardy_fight_after_start();
    ardy_fight_catchless_first_session(&mut runner, &mut c);
    assert!(
        matches!(runner.status(), RunnerStatus::Running { .. }),
        "one catch-less session must not end the combat watch: {:?}",
        runner.status()
    );
    c.npc_count = 1;
    c.bump_gens(ServerProt::NPC_INFO);
    advance_one_game_tick(&mut runner, &mut c, false);
    c.stat_xp[2] += 4;
    c.bump_gens(ServerProt::UPDATE_STAT);
    set_inv(&mut c, &[(1891, 1)]);
    advance_game_ticks_until_done(&mut runner, &mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn ardy_fight_rejects_a_second_unqualified_stall_session() {
    for (combat_xp, guard_ready) in [(false, true), (true, false)] {
        let (mut runner, mut c) = ardy_fight_after_start();
        ardy_fight_catchless_first_session(&mut runner, &mut c);
        c.npc_count = i32::from(guard_ready);
        c.bump_gens(ServerProt::NPC_INFO);
        advance_one_game_tick(&mut runner, &mut c, false);
        if combat_xp {
            c.stat_xp[2] += 4;
            c.bump_gens(ServerProt::UPDATE_STAT);
        }
        set_inv(&mut c, &[(1891, 6)]);
        advance_game_ticks_until_done(&mut runner, &mut c);
        assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
        ardy_fight_bank_visit(&mut runner, &mut c);
        assert!(
            matches!(runner.status(), RunnerStatus::Failed(_)),
            "a second catch-less trip or missing renewed Guard readiness must fail closed"
        );
    }
}

#[test]
fn fresh_xp_watch_does_not_consume_a_prior_step_gain() {
    const SKILL: i32 = 17;
    let cumulative = Proof::StatXpGain { id: SKILL, min: 1 };
    let fresh = Proof::FreshStatXpGain { id: SKILL, min: 1 };
    let scenario = Scenario {
        name: "fresh-xp",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![
            Step {
                name: "start the catalog card",
                kind: StepKind::StartScript,
                wait: wait(Proof::IngameScene2, 1),
            },
            Step {
                name: "watch the earlier gain",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: wait(cumulative, 4),
            },
            Step {
                name: "watch a later fresh gain",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: wait(fresh, 4),
            },
        ],
        proof: fresh,
        companions: vec![],
        settings: ScenarioSettings::default(),
    };
    let mut c = seeded_client();
    c.stat_xp[SKILL as usize] = 100;
    let mut runner = ScenarioRunner::with_world(scenario, None);
    runner.set_scene_settle(Duration::ZERO);

    runner.tick(&mut c);
    c.stat_xp[SKILL as usize] = 101;
    c.bump_gens(ServerProt::UPDATE_STAT);
    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 2, total: 3 },
        "the first gain advances only to the fresh watch"
    );

    c.bump_gens(ServerProt::UPDATE_STAT);
    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 2, total: 3 },
        "unchanged XP cannot reuse the prior step gain"
    );

    c.ingame = false;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(
        runner.fresh_xp_baseline, None,
        "leaving the session clears the step-local baseline"
    );
    c.ingame = true;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    c.bump_gens(ServerProt::UPDATE_STAT);
    runner.tick(&mut c);
    assert_eq!(runner.fresh_xp_baseline, Some((SKILL, 101)));

    c.stat_xp[SKILL as usize] = 102;
    c.bump_gens(ServerProt::UPDATE_STAT);
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

const TEST_LAMP_ID: i32 = 2528;
const TEST_STRENGTH_STAT: i32 = 2;
const TEST_PRAYER_STAT: i32 = 5;
const TEST_LAMP_AWARD: &str = "Your wish has been granted!";

fn lamp_episode_scenario(budget_game_ticks: u32) -> Scenario {
    let fresh_prayer = Proof::FreshStatXpGain {
        id: TEST_PRAYER_STAT,
        min: 1,
    };
    Scenario {
        name: "lamp-episode",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![
            Step {
                name: "observe one authentic lamp redemption episode",
                kind: StepKind::ObserveLampRedemption {
                    lamp_id: TEST_LAMP_ID,
                    reward_stat: TEST_STRENGTH_STAT,
                },
                wait: wait(Proof::NoActiveContinue, budget_game_ticks),
            },
            Step {
                name: "watch fresh script work after release",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: wait(fresh_prayer, 4),
            },
        ],
        proof: fresh_prayer,
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

fn set_continue_text(c: &mut Client, open: bool, text: &str) {
    use client::config::if_type::ButtonType;

    const ROOT: usize = 6206;
    const CONTINUE: usize = 6210;
    c.set_iface(
        ROOT,
        IfType {
            id: ROOT as i32,
            layer_id: ROOT as i32,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![CONTINUE as i32]),
            ..Default::default()
        },
    );
    c.set_iface(
        CONTINUE,
        IfType {
            id: CONTINUE as i32,
            layer_id: ROOT as i32,
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        CONTINUE,
        IfTypeMut {
            text: text.into(),
            button_type: ButtonType::BUTTON_CONTINUE,
            ..Default::default()
        },
    );
    c.chat_modal_id = if open { ROOT as i32 } else { -1 };
    c.bump_gens(if open {
        ServerProt::IF_OPENCHAT
    } else {
        ServerProt::IF_CLOSE
    });
}

fn set_continue(c: &mut Client, open: bool) {
    set_continue_text(c, open, "");
}

#[test]
fn lamp_episode_latches_same_frame_reward_then_requires_drain_release_and_fresh_work() {
    let mut c = seeded_client();
    c.stat_xp[TEST_STRENGTH_STAT as usize] = 100;
    c.stat_xp[TEST_PRAYER_STAT as usize] = 200;
    set_inv(&mut c, &[(TEST_LAMP_ID, 1)]);
    let mut runner = ScenarioRunner::with_world(lamp_episode_scenario(12), None);
    runner.set_scene_settle(Duration::ZERO);

    runner.tick_with_hold(&mut c, false);
    assert_eq!(runner.status(), RunnerStatus::Running { step: 0, total: 2 });

    c.bump_gens(ServerProt::PLAYER_INFO);
    runner.tick_with_hold(&mut c, true);

    set_inv(&mut c, &[]);
    c.stat_xp[TEST_STRENGTH_STAT as usize] = 110;
    c.bump_gens(ServerProt::UPDATE_STAT);
    set_continue_text(&mut c, true, TEST_LAMP_AWARD);
    runner.tick_with_hold(&mut c, true);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 2 },
        "reward, consumption, and active dialogue do not release a held episode"
    );

    set_continue(&mut c, false);
    runner.tick_with_hold(&mut c, true);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 2 },
        "dialogue drain alone does not release a still-held episode"
    );

    c.bump_gens(ServerProt::PLAYER_INFO);
    runner.tick_with_hold(&mut c, false);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 1, total: 2 },
        "native hold release advances to the post-event work gate"
    );

    c.bump_gens(ServerProt::UPDATE_STAT);
    runner.tick_with_hold(&mut c, false);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 1, total: 2 },
        "pre-release Prayer XP cannot satisfy the fresh post-release gate"
    );
    c.stat_xp[TEST_PRAYER_STAT as usize] = 201;
    c.bump_gens(ServerProt::UPDATE_STAT);
    runner.tick_with_hold(&mut c, false);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn lamp_episode_accepts_source_named_award_after_split_reward_packets() {
    let mut c = seeded_client();
    c.stat_xp[TEST_STRENGTH_STAT as usize] = 100;
    c.stat_xp[TEST_PRAYER_STAT as usize] = 200;
    set_inv(&mut c, &[(TEST_LAMP_ID, 1)]);
    let mut runner = ScenarioRunner::with_world(lamp_episode_scenario(12), None);
    runner.set_scene_settle(Duration::ZERO);

    runner.tick_with_hold(&mut c, true);

    // xplamp_confirm publishes the durable reward effects before mesbox;
    // a client frame may therefore observe these without the award IF.
    set_inv(&mut c, &[]);
    c.stat_xp[TEST_STRENGTH_STAT as usize] = 110;
    c.bump_gens(ServerProt::UPDATE_STAT);
    runner.tick_with_hold(&mut c, true);

    // The authentic award continuation arrives on a later frame, after
    // the reward/consumption edges have already been retained.
    set_continue_text(&mut c, true, TEST_LAMP_AWARD);
    runner.tick_with_hold(&mut c, true);
    set_continue_text(&mut c, false, "");
    runner.tick_with_hold(&mut c, true);
    c.bump_gens(ServerProt::PLAYER_INFO);
    runner.tick_with_hold(&mut c, false);

    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 1, total: 2 },
        "source-identified split award must complete the same held episode"
    );
}

#[test]
fn lamp_episode_rejects_preexisting_and_later_unrelated_dialogues() {
    let mut c = seeded_client();
    c.stat_xp[TEST_STRENGTH_STAT as usize] = 100;
    set_inv(&mut c, &[(TEST_LAMP_ID, 1)]);
    set_continue_text(&mut c, true, TEST_LAMP_AWARD);
    let mut runner = ScenarioRunner::with_world(lamp_episode_scenario(7), None);
    runner.set_scene_settle(Duration::ZERO);

    runner.tick_with_hold(&mut c, true);
    set_inv(&mut c, &[]);
    c.stat_xp[TEST_STRENGTH_STAT as usize] = 110;
    c.bump_gens(ServerProt::UPDATE_STAT);
    runner.tick_with_hold(&mut c, true);
    set_continue(&mut c, false);
    runner.tick_with_hold(&mut c, true);
    set_continue(&mut c, true);
    runner.tick_with_hold(&mut c, true);
    set_continue(&mut c, false);
    runner.tick_with_hold(&mut c, false);

    for _ in 0..8 {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick_with_hold(&mut c, false);
    }
    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("preexisting and later unrelated dialogues must fail");
    };
    assert!(
        message.contains("lamp redemption episode"),
        "the failed native episode must be named: {message}"
    );
}

#[test]
fn lamp_episode_rejects_wrong_hold_and_elapsed_only_progress() {
    let mut c = seeded_client();
    c.stat_xp[TEST_STRENGTH_STAT as usize] = 100;
    set_inv(&mut c, &[(TEST_LAMP_ID, 1)]);
    let mut runner = ScenarioRunner::with_world(lamp_episode_scenario(5), None);
    runner.set_scene_settle(Duration::ZERO);

    runner.tick_with_hold(&mut c, false);
    set_inv(&mut c, &[]);
    c.stat_xp[TEST_STRENGTH_STAT as usize] = 110;
    c.bump_gens(ServerProt::UPDATE_STAT);
    set_continue_text(&mut c, true, TEST_LAMP_AWARD);
    runner.tick_with_hold(&mut c, false);
    set_continue(&mut c, false);
    runner.tick_with_hold(&mut c, false);

    for _ in 0..6 {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick_with_hold(&mut c, false);
    }
    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("wrong native hold and elapsed ticks must fail");
    };
    assert!(message.contains("hold=false"), "{message}");
    assert!(
        message.contains("dialogue=false"),
        "an award outside a held episode must not bind: {message}"
    );
}

const TEST_MAZE_SPAWNS: &[WorldTile] = &[
    WorldTile {
        x: 2891,
        z: 4597,
        level: 0,
    },
    WorldTile {
        x: 2933,
        z: 4597,
        level: 0,
    },
    WorldTile {
        x: 2933,
        z: 4555,
        level: 0,
    },
    WorldTile {
        x: 2891,
        z: 4555,
        level: 0,
    },
];
const TEST_MAZE_SHRINE: WorldTile = WorldTile {
    x: 2911,
    z: 4575,
    level: 0,
};

fn maze_episode_scenario(budget_game_ticks: u32) -> Scenario {
    maze_episode_scenario_with(budget_game_ticks, None)
}

fn maze_episode_scenario_with(budget_game_ticks: u32, trigger: Option<&'static str>) -> Scenario {
    Scenario {
        name: "maze-episode",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "observe ordered Maze completion",
            kind: StepKind::ObserveMazeCompletion {
                spawns: TEST_MAZE_SPAWNS,
                shrine: TEST_MAZE_SHRINE,
                shrine_radius: 4,
                min_progress: 8,
                entry_shot: "maze entered",
                trigger,
            },
            wait: wait(Proof::IngameScene2, budget_game_ticks),
        }],
        proof: Proof::IngameScene2,
        companions: vec![],
        settings: ScenarioSettings {
            terminal_shot: Some("maze final"),
            ..ScenarioSettings::default()
        },
    }
}

fn maze_out_contains(c: &Client, needle: &str) -> bool {
    let bytes = &c.out.data()[..c.out.pos];
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn maze_out_count(c: &Client, needle: &str) -> usize {
    let bytes = &c.out.data()[..c.out.pos];
    bytes
        .windows(needle.len())
        .filter(|window| *window == needle.as_bytes())
        .count()
}

fn set_world_tile(c: &mut Client, tile: WorldTile) {
    c.map_build_base_x = (tile.x >> 6) << 6;
    c.map_build_base_z = (tile.z >> 6) << 6;
    c.minusedlevel = tile.level;
    c.local_player = Some(ClientPlayer::at(
        tile.x - c.map_build_base_x,
        tile.z - c.map_build_base_z,
    ));
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    c.bump_gens(ServerProt::PLAYER_INFO);
}

fn advance_one_game_tick(runner: &mut ScenarioRunner, c: &mut Client, hold: bool) {
    c.bump_gens(ServerProt::PLAYER_INFO);
    runner.tick_with_hold(c, hold);
}

fn drive_test_maze_to_shrine(runner: &mut ScenarioRunner, c: &mut Client) {
    set_world_tile(c, TEST_MAZE_SPAWNS[0]);
    runner.tick_with_hold(c, true);
    for tile in [
        WorldTile {
            x: 2891,
            z: 4590,
            level: 0,
        },
        WorldTile {
            x: 2896,
            z: 4588,
            level: 0,
        },
        WorldTile {
            x: 2910,
            z: 4576,
            level: 0,
        },
    ] {
        set_world_tile(c, tile);
        runner.tick_with_hold(c, true);
    }
}

#[test]
fn maze_episode_refreshes_pre_entry_baseline_to_last_ready_outside_snapshot() {
    const TILE_A: WorldTile = WorldTile {
        x: 3220,
        z: 3220,
        level: 0,
    };
    const TILE_B: WorldTile = WorldTile {
        x: 3221,
        z: 3220,
        level: 0,
    };

    let mut c = seeded_client();
    set_inv(&mut c, &[(995, 100)]);
    let mut runner = ScenarioRunner::with_world(maze_episode_scenario(30), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick_with_hold(&mut c, false);

    set_world_tile(&mut c, TILE_B);
    set_inv(&mut c, &[(995, 105)]);
    runner.tick_with_hold(&mut c, false);
    drive_test_maze_to_shrine(&mut runner, &mut c);

    set_world_tile(&mut c, TILE_A);
    set_inv(&mut c, &[(995, 101)]);
    runner.tick_with_hold(&mut c, false);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "the stale observer-entry tile A must not count as the server return"
    );

    set_world_tile(&mut c, TILE_B);
    runner.tick_with_hold(&mut c, false);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "inventory above A's baseline but below B's must not count as the reward"
    );

    set_inv(&mut c, &[(995, 106)]);
    advance_one_game_tick(&mut runner, &mut c, false);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn maze_episode_recovers_baseline_after_observer_begins_unready() {
    const RETURN_TILE: WorldTile = WorldTile {
        x: 3221,
        z: 3220,
        level: 0,
    };

    let mut c = seeded_client();
    set_inv(&mut c, &[(995, 100)]);
    c.scene_state = 1;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut runner = ScenarioRunner::with_world(maze_episode_scenario(30), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.snapshot.rebuild(&c);
    runner.phase = Phase::Running;
    runner.begin_step();
    assert_eq!(
        runner.maze_episode.as_ref().unwrap().return_tile,
        None,
        "the unready observer start has no usable baseline"
    );

    c.scene_state = 2;
    set_world_tile(&mut c, RETURN_TILE);
    runner.tick_with_hold(&mut c, false);
    drive_test_maze_to_shrine(&mut runner, &mut c);

    set_world_tile(&mut c, RETURN_TILE);
    set_inv(&mut c, &[(995, 101)]);
    runner.tick_with_hold(&mut c, false);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn maze_episode_accepts_immediate_held_entry_without_npc_snapshot() {
    const RETURN_TILE: WorldTile = WorldTile {
        x: 3220,
        z: 3220,
        level: 0,
    };
    const NE_SPAWN: WorldTile = WorldTile {
        x: 2933,
        z: 4597,
        level: 0,
    };

    let mut c = seeded_client();
    set_inv(&mut c, &[(995, 100)]);
    let mut runner =
        ScenarioRunner::with_world(maze_episode_scenario_with(30, Some("~macro_event 8")), None);
    runner.set_scene_settle(Duration::ZERO);

    runner.tick_with_hold(&mut c, false);
    let episode = runner.maze_episode.as_ref().expect("observer armed");
    assert_eq!(
        episode.return_tile,
        Some(RETURN_TILE),
        "pre-entry baseline must be captured before the trigger send"
    );
    assert_eq!(episode.reward_baseline, Some(100));
    assert_eq!(
        maze_out_count(&c, "~macro_event 8"),
        1,
        "the armed observer sends the authentic trigger once"
    );
    assert!(!maze_out_contains(&c, "mazeend"));
    assert_eq!(
        episode.entry, None,
        "the trigger send must not invent an entry"
    );

    set_world_tile(&mut c, NE_SPAWN);
    runner.tick_with_hold(&mut c, true);
    assert_eq!(
        runner.maze_episode.as_ref().unwrap().entry,
        Some(NE_SPAWN),
        "held canonical NE entry must count without any NPC snapshot"
    );
    assert_eq!(
        runner.maze_episode.as_ref().unwrap().return_tile,
        Some(RETURN_TILE),
        "immediate entry must keep the pre-send mainland baseline"
    );

    for tile in [
        WorldTile {
            x: 2933,
            z: 4590,
            level: 0,
        },
        WorldTile {
            x: 2925,
            z: 4585,
            level: 0,
        },
        WorldTile {
            x: 2911,
            z: 4576,
            level: 0,
        },
    ] {
        set_world_tile(&mut c, tile);
        runner.tick_with_hold(&mut c, true);
    }

    set_world_tile(&mut c, RETURN_TILE);
    set_inv(&mut c, &[(995, 101)]);
    runner.tick_with_hold(&mut c, true);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "return and reward cannot pass while native hold remains"
    );
    advance_one_game_tick(&mut runner, &mut c, false);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    assert_eq!(maze_out_count(&c, "~macro_event 8"), 1);
}

#[test]
fn maze_episode_requires_held_entry_progress_shrine_return_reward_and_release() {
    let mut c = seeded_client();
    set_inv(&mut c, &[(995, 100)]);
    let mut runner = ScenarioRunner::with_world(maze_episode_scenario(20), None);
    runner.set_scene_settle(Duration::ZERO);
    type Captures = Arc<Mutex<Vec<(String, Option<(i32, i32, i32)>)>>>;
    let captures: Captures = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&captures);
    runner.set_shot_sink(Box::new(move |label, snap| {
        sink.lock().unwrap().push((label.to_string(), snap.tile()));
    }));

    runner.tick_with_hold(&mut c, false);
    set_world_tile(&mut c, TEST_MAZE_SPAWNS[0]);
    runner.tick_with_hold(&mut c, true);
    assert_eq!(
        captures.lock().unwrap().as_slice(),
        &[("maze entered".to_string(), Some((2891, 4597, 0)))],
        "the entry capture reports the server-selected spawn"
    );

    set_world_tile(
        &mut c,
        WorldTile {
            x: 2891,
            z: 4590,
            level: 0,
        },
    );
    runner.tick_with_hold(&mut c, true);
    set_world_tile(
        &mut c,
        WorldTile {
            x: 2896,
            z: 4588,
            level: 0,
        },
    );
    runner.tick_with_hold(&mut c, true);
    set_world_tile(
        &mut c,
        WorldTile {
            x: 2910,
            z: 4576,
            level: 0,
        },
    );
    runner.tick_with_hold(&mut c, true);

    set_world_tile(
        &mut c,
        WorldTile {
            x: 3220,
            z: 3220,
            level: 0,
        },
    );
    set_inv(&mut c, &[(995, 101)]);
    runner.tick_with_hold(&mut c, true);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 1 },
        "server return and reward cannot pass while native hold remains"
    );
    advance_one_game_tick(&mut runner, &mut c, false);

    assert_eq!(runner.status(), RunnerStatus::Passed);
    assert_eq!(
        captures.lock().unwrap().as_slice(),
        &[
            ("maze entered".to_string(), Some((2891, 4597, 0))),
            ("maze final".to_string(), Some((3220, 3220, 0))),
        ]
    );
    let evidence = runner.evidence().expect("terminal evidence");
    assert_eq!(evidence.scene, 2);
    assert_eq!(evidence.tile, Some([3220, 3220, 0]));
}

#[test]
fn maze_episode_rejects_near_shrine_without_canonical_entry() {
    let mut c = seeded_client();
    set_inv(&mut c, &[(995, 100)]);
    let mut runner = ScenarioRunner::with_world(maze_episode_scenario(4), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick_with_hold(&mut c, false);

    set_world_tile(&mut c, TEST_MAZE_SHRINE);
    runner.tick_with_hold(&mut c, true);
    set_world_tile(
        &mut c,
        WorldTile {
            x: 3220,
            z: 3220,
            level: 0,
        },
    );
    set_inv(&mut c, &[(995, 101)]);
    for _ in 0..5 {
        advance_one_game_tick(&mut runner, &mut c, false);
    }

    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("near-shrine and outside planted states must fail");
    };
    assert!(message.contains("entry=None"), "{message}");
    assert!(message.contains("shrine=false"), "{message}");
}

#[test]
fn maze_episode_rejects_return_without_real_progress_and_shrine_order() {
    let mut c = seeded_client();
    set_inv(&mut c, &[(995, 100)]);
    let mut runner = ScenarioRunner::with_world(maze_episode_scenario(5), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick_with_hold(&mut c, false);

    set_world_tile(&mut c, TEST_MAZE_SPAWNS[3]);
    runner.tick_with_hold(&mut c, true);
    set_world_tile(
        &mut c,
        WorldTile {
            x: 3220,
            z: 3220,
            level: 0,
        },
    );
    set_inv(&mut c, &[(995, 101)]);
    for _ in 0..6 {
        advance_one_game_tick(&mut runner, &mut c, false);
    }

    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("entry followed by a planted outside state must fail");
    };
    assert!(message.contains("progress=false"), "{message}");
    assert!(message.contains("shrine=false"), "{message}");
    assert!(message.contains("returned=false"), "{message}");
}

#[test]
fn maze_episode_exact_return_rejects_whoops_fallback_when_baseline_is_fountain() {
    let mut c = seeded_client();
    set_inv(&mut c, &[(995, 100)]);
    let mut runner = ScenarioRunner::with_world(maze_episode_scenario(8), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick_with_hold(&mut c, false);
    assert_eq!(
        runner.maze_episode.as_ref().unwrap().return_tile,
        Some(WorldTile {
            x: 3220,
            z: 3220,
            level: 0,
        }),
        "the mainland hop tile is the blocked fountain baseline"
    );

    drive_test_maze_to_shrine(&mut runner, &mut c);
    set_world_tile(
        &mut c,
        WorldTile {
            x: 3221,
            z: 3218,
            level: 0,
        },
    );
    set_inv(&mut c, &[(995, 101)]);
    for _ in 0..9 {
        advance_one_game_tick(&mut runner, &mut c, false);
    }

    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("Whoops fallback must not satisfy an exact fountain baseline");
    };
    assert!(message.contains("returned=false"), "{message}");
    assert!(message.contains("entry=Some"), "{message}");
    assert!(message.contains("shrine=true"), "{message}");
}

fn fresh_tutorial_scenario() -> Scenario {
    Scenario {
        name: "fresh-tutorial",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![Step {
            name: "reseed tutorial after relog and verify fresh value",
            kind: StepKind::Repeat {
                send: Box::new(|_, _| true),
            },
            wait: crate::Wait {
                arm: Proof::FreshTutorial,
                budget_ticks: 200,
            },
        }],
        proof: Proof::IngameScene2,
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

#[test]
fn fresh_tutorial_needs_a_reply_newer_than_the_step_start_baseline() {
    let mut c = seeded_client();
    // A pre-step `1000` line latches into the first snapshot; the runner
    // baselines it at step start, so it must not satisfy the wait.
    c.add_chat(0, "get tutorial: 1000", "");
    c.bump_gens(ServerProt::MESSAGE_GAME);
    let mut runner = ScenarioRunner::with_world(fresh_tutorial_scenario(), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert!(
        matches!(runner.status(), RunnerStatus::Running { .. }),
        "a stale pre-step line at the baseline must not satisfy FreshTutorial: {:?}",
        runner.status()
    );
    // The next game tick's resend without a new reply still waits.
    advance_one_game_tick(&mut runner, &mut c, false);
    assert!(
        matches!(runner.status(), RunnerStatus::Running { .. }),
        "the reseed send tick must not advance on the stale head: {:?}",
        runner.status()
    );
    // A strictly newer `1000` reply in the same session satisfies.
    c.add_chat(0, "get tutorial: 1000", "");
    c.bump_gens(ServerProt::MESSAGE_GAME);
    advance_one_game_tick(&mut runner, &mut c, false);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn fresh_tutorial_leaving_the_session_clears_the_baseline() {
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::with_world(fresh_tutorial_scenario(), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert!(
        runner.fresh_chat_baseline.is_some(),
        "the step-start snapshot must latch a baseline"
    );
    c.ingame = false;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(
        runner.fresh_chat_baseline, None,
        "leaving the session clears the step-local tutorial baseline"
    );
}

#[test]
fn fresh_tutorial_relog_with_a_retained_old_reply_keeps_waiting() {
    let mut c = seeded_client();
    // The pre-relog reply: the same line the reseed step later asks the
    // new session to send again (`reseed_after_relog` in
    // `clue_walk_arrive_live`, whose wait is FreshTutorial).
    c.add_chat(0, "get tutorial: 1000", "");
    c.bump_gens(ServerProt::MESSAGE_GAME);
    let mut runner = ScenarioRunner::with_world(fresh_tutorial_scenario(), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert!(
        matches!(runner.status(), RunnerStatus::Running { .. }),
        "the pre-relog line at the baseline must not satisfy the reseed: {:?}",
        runner.status()
    );

    // Relog: leave the session (the step baseline clears), then return
    // with the old line still retained in the snapshot ring. The runner
    // re-baselines on that retained line, so it stays stale.
    c.ingame = false;
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    runner.tick(&mut c);
    assert_eq!(runner.fresh_chat_baseline, None);
    c.ingame = true;
    advance_one_game_tick(&mut runner, &mut c, false);
    advance_one_game_tick(&mut runner, &mut c, false);
    assert!(
        matches!(runner.status(), RunnerStatus::Running { .. }),
        "a retained pre-relog line must not pass after the relog: {:?}",
        runner.status()
    );

    // Only a strictly newer reply from the new session satisfies it.
    c.add_chat(0, "get tutorial: 1000", "");
    c.bump_gens(ServerProt::MESSAGE_GAME);
    advance_one_game_tick(&mut runner, &mut c, false);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

/// The brimhaven_agility watch chain in miniature: a preceding watch
/// (one Coin stands in for obstacle XP), the first-tag chat prompt, the
/// first tag's no-ticket state, and the later earned ticket
/// (`scenarios/production/agility.rs` keeps the same milestones). The
/// prompt arm takes its chat baseline from the preceding watch's start,
/// so a prompt sent while that watch runs counts and an older one does
/// not.
fn brimhaven_tag_ticket_scenario() -> Scenario {
    let watch = |name: &'static str, arm: Proof| Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: wait(arm, 10),
    };
    Scenario {
        name: "brimhaven-tag-ticket",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![
            watch(
                "watch obstacle XP from a real hop before the first Tag",
                Proof::ItemId { id: 995, count: 1 },
            ),
            watch(
                "watch the first Tag prompt for the next pillar",
                Proof::FreshChatSincePreviousStep {
                    needle: "tag the next",
                },
            ),
            watch(
                "confirm the first Tag grants no ticket",
                Proof::ItemIdAtMost { id: 2996, count: 0 },
            ),
            watch(
                "watch a later Tag grant the first ticket",
                Proof::ItemId { id: 2996, count: 1 },
            ),
        ],
        proof: Proof::ItemId { id: 2996, count: 1 },
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

/// The content's two consecutive first-tag messages, in send order
/// (`agilityarena.rs2:72-73`): the second line pushes the matching first
/// line off the chat head in the same burst.
fn send_first_tag_prompt(c: &mut Client) {
    c.add_chat(
        0,
        "You get tickets by tagging more than one pillar in a row, tag the next",
        "",
    );
    c.add_chat(0, "pillar!", "");
    c.bump_gens(ServerProt::MESSAGE_GAME);
}

#[test]
fn brimhaven_two_line_tag_prompt_passes_then_waits_for_the_ticket() {
    let mut c = seeded_client();
    set_inv(&mut c, &[]);
    let mut runner = ScenarioRunner::with_world(brimhaven_tag_ticket_scenario(), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 0, total: 4 },
        "no obstacle progress yet: the preceding watch must wait"
    );

    // The prompt bursts while the preceding watch is still running, so
    // it is newer than that watch's start baseline but older than the
    // prompt step's own start.
    send_first_tag_prompt(&mut c);
    set_inv(&mut c, &[(995, 1)]);
    assert_running_for_game_ticks(
        &mut runner,
        &mut c,
        3,
        "the burst passes the preceding, prompt and no-ticket watches, then waits for a ticket",
    );
    assert_eq!(
        runner.status(),
        RunnerStatus::Running { step: 3, total: 4 },
        "the run must sit on the later-ticket watch, not pass on the prompt alone"
    );

    set_inv(&mut c, &[(995, 1), (2996, 1)]);
    advance_game_ticks_until_done(&mut runner, &mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

#[test]
fn brimhaven_prompt_from_before_the_previous_watch_is_stale_history() {
    let mut c = seeded_client();
    // No ticket is seeded: with one held, the later no-ticket step
    // could never pass, so the run would fail even if the prompt step
    // wrongly accepted the stale line.
    set_inv(&mut c, &[(995, 1)]);
    // The matching line is already in the ring before the preceding
    // watch (and therefore before its baseline) begins. It must never
    // satisfy the prompt step.
    send_first_tag_prompt(&mut c);
    let mut runner = ScenarioRunner::with_world(brimhaven_tag_ticket_scenario(), None);
    runner.set_scene_settle(Duration::ZERO);
    for _ in 0..20 {
        advance_one_game_tick(&mut runner, &mut c, false);
    }
    match runner.status() {
        RunnerStatus::Failed(msg) => {
            assert!(
                msg.contains("watch the first Tag prompt for the next pillar"),
                "a prompt from before the previous-watch baseline must fail at the prompt step: {msg}"
            );
            assert!(
                msg.contains("fresh_chat_since_previous_step"),
                "the failure must name the prompt arm: {msg}"
            );
            assert!(
                msg.contains("not seen within 10 game ticks"),
                "the prompt step must exhaust its own 10-game-tick budget: {msg}"
            );
        }
        other => panic!(
            "a prompt received before the previous-watch baseline must not pass, got {other:?}"
        ),
    }
}

#[test]
fn brimhaven_run_without_a_tag_prompt_fails() {
    let mut c = seeded_client();
    // No ticket is seeded: with one held, the later no-ticket step
    // could never pass, so the run would fail even if the prompt step
    // were skipped or wrongly satisfied.
    set_inv(&mut c, &[(995, 1)]);
    c.add_chat(0, "pillar!", "");
    c.bump_gens(ServerProt::MESSAGE_GAME);
    let mut runner = ScenarioRunner::with_world(brimhaven_tag_ticket_scenario(), None);
    runner.set_scene_settle(Duration::ZERO);
    for _ in 0..20 {
        advance_one_game_tick(&mut runner, &mut c, false);
    }
    match runner.status() {
        RunnerStatus::Failed(msg) => {
            assert!(
                msg.contains("watch the first Tag prompt for the next pillar"),
                "a run with no tag prompt must fail at the prompt step: {msg}"
            );
            assert!(
                msg.contains("fresh_chat_since_previous_step"),
                "the failure must name the prompt arm: {msg}"
            );
            assert!(
                msg.contains("not seen within 10 game ticks"),
                "the prompt step must exhaust its own 10-game-tick budget: {msg}"
            );
        }
        other => panic!("a run with no tag prompt must fail, got {other:?}"),
    }
}

#[test]
fn brimhaven_run_without_ticket_progress_fails() {
    let mut c = seeded_client();
    set_inv(&mut c, &[]);
    let mut runner = ScenarioRunner::with_world(brimhaven_tag_ticket_scenario(), None);
    runner.set_scene_settle(Duration::ZERO);
    runner.tick(&mut c);
    send_first_tag_prompt(&mut c);
    set_inv(&mut c, &[(995, 1)]);
    for _ in 0..24 {
        advance_one_game_tick(&mut runner, &mut c, false);
    }
    assert!(
        matches!(runner.status(), RunnerStatus::Failed(_)),
        "the prompt alone must not pass without the later ticket, got {:?}",
        runner.status()
    );
}

fn visited_within_scenario() -> Scenario {
    let watch = |name: &'static str, min: i32, max: i32| Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: wait(
            Proof::VisitedWithin {
                min_x: min,
                max_x: max,
                min_z: min,
                max_z: max,
                level: 0,
            },
            200,
        ),
    };
    Scenario {
        name: "visited-within",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: false,
        },
        steps: vec![
            watch("visit the first box", 3218, 3222),
            watch("visit the second box", 3228, 3232),
        ],
        proof: Proof::IngameScene2,
        companions: vec![],
        settings: ScenarioSettings::default(),
    }
}

#[test]
fn visited_within_latches_inside_the_box_and_clears_at_the_next_step() {
    let tile = |x: i32, z: i32| WorldTile { x, z, level: 0 };
    let mut c = seeded_client();
    let mut runner = ScenarioRunner::with_world(visited_within_scenario(), None);
    runner.set_scene_settle(Duration::ZERO);

    // One tile short of the box (min_x - 1) must not latch.
    set_world_tile(&mut c, tile(3217, 3220));
    runner.tick(&mut c);
    advance_one_game_tick(&mut runner, &mut c, false);
    assert!(!runner.visited_within, "min_x - 1 must not latch");
    assert!(
        matches!(runner.status(), RunnerStatus::Running { step: 0, .. }),
        "an outside tile must not advance the step: {:?}",
        runner.status()
    );
    // One tile past the box (max_x + 1) must not latch either.
    set_world_tile(&mut c, tile(3223, 3220));
    advance_one_game_tick(&mut runner, &mut c, false);
    assert!(!runner.visited_within, "max_x + 1 must not latch");
    assert!(
        matches!(runner.status(), RunnerStatus::Running { step: 0, .. }),
        "an outside tile must not advance the step: {:?}",
        runner.status()
    );

    // A correct receipt passes: the inclusive min corner latches and
    // the step advances.
    set_world_tile(&mut c, tile(3218, 3218));
    for _ in 0..5 {
        if matches!(runner.status(), RunnerStatus::Running { step: 1, .. }) {
            break;
        }
        advance_one_game_tick(&mut runner, &mut c, false);
    }
    assert!(
        matches!(runner.status(), RunnerStatus::Running { step: 1, .. }),
        "an inside tile must latch and advance to the next step: {:?}",
        runner.status()
    );
    assert!(
        !runner.visited_within,
        "a later step starts with the visited latch clear"
    );
    advance_one_game_tick(&mut runner, &mut c, false);
    assert!(
        matches!(runner.status(), RunnerStatus::Running { step: 1, .. }),
        "the first box's latch must not satisfy the second step: {:?}",
        runner.status()
    );

    // Standing in the second box latches the second step and passes.
    set_world_tile(&mut c, tile(3230, 3230));
    for _ in 0..8 {
        if runner.status() == RunnerStatus::Passed {
            break;
        }
        advance_one_game_tick(&mut runner, &mut c, false);
    }
    assert_eq!(runner.status(), RunnerStatus::Passed);
}

// Behavioural oracle tests for the combat bank cells: the real catalog
// scenario is driven from its StartScript step on with a synthetic client
// whose observations change tick by tick. Each cell has one correct sequence
// that passes and one bad sequence per named behaviour; a bad sequence is the
// correct one with only that behaviour changed, and must fail at the step
// that watches it.

const ORACLE_STRENGTH_STAT: usize = 2;
const ORACLE_COINS_ID: i32 = 995;

/// What one cell's driver does differently from the correct sequence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flaw {
    Correct,
    /// The supply event never happens: no looted bones reach the pack
    /// (hill giant, ardy fighter) or the carried Lobsters never run out
    /// (green dragon).
    NoSupplyEvent,
    /// The bot reaches an open bank, but the loot is not in it.
    NotDeposited,
    /// The bot never reaches the bank: no bank window, no deposit/restock.
    OutboundStall,
    /// The bot banks but never comes back near the field.
    ReturnStall,
    /// The Strength XP is gained before the return, none after it.
    StaleXp,
    /// The death line is posted after Start, then recovery proceeds.
    DiedAfterStart,
    /// The death line is in the chat before Start; nothing dies afterwards.
    DeathLineBeforeStart,
}

struct OracleCell {
    name: &'static str,
    field: WorldTile,
    bank: WorldTile,
    /// Pack when the card starts.
    carried: &'static [(i32, i32)],
    /// Pack once the supply event has happened.
    supplied: &'static [(i32, i32)],
    /// Bank rows at the booth on the correct trip.
    bank_rows: &'static [(i32, i32)],
    /// Pack after the bank visit on the correct trip.
    restocked: &'static [(i32, i32)],
    /// Item the correct trip deposits (and that the `NotDeposited` bad trip
    /// keeps in the pack), if the cell has a deposit.
    deposited: Option<(i32, i32)>,
    supply_step: &'static str,
    /// The first step that needs the bank trip (deposit, or restock).
    visit_step: &'static str,
    return_step: &'static str,
    xp_step: &'static str,
    /// The card selects the native Strength mode (varp 43 = 1) before its
    /// first Strength XP, and the cell watches that selection.
    selects_style: bool,
}

const HILL_ORACLE: OracleCell = OracleCell {
    name: "hill_giant_bank",
    field: WorldTile {
        x: 3110,
        z: 9832,
        level: 0,
    },
    bank: WorldTile {
        x: 3185,
        z: 3436,
        level: 0,
    },
    carried: &[(333, 8)],
    supplied: &[(333, 8), (532, 1)],
    bank_rows: &[(532, 1)],
    restocked: &[(333, 12)],
    deposited: Some((532, 1)),
    supply_step: "watch the first looted Giant drop in pack before the bank trip",
    visit_step: "watch the trip's looted drop enter a fresh Varrock West bank",
    return_step: "watch return to the giant pit after banking",
    xp_step: "watch fresh Strength XP after the bank return",
    selects_style: false,
};

/// The same trip when the card takes the Giant's Limpwurt root first: it is
/// in HillGiant's default loot list beside the Big bones, so it ends the
/// one-slot trip just as well (`r3a-hill_giant_bank`).
const HILL_LIMPWURT_ORACLE: OracleCell = OracleCell {
    supplied: &[(333, 8), (225, 1)],
    bank_rows: &[(225, 1)],
    deposited: Some((225, 1)),
    ..HILL_ORACLE
};

const ARDY_ORACLE: OracleCell = OracleCell {
    name: "ardy_fighter_bank",
    field: WorldTile {
        x: 2661,
        z: 3306,
        level: 0,
    },
    bank: ARDY_FIGHT_BANK,
    carried: &[],
    supplied: &[(526, 1)],
    bank_rows: &[(526, 1)],
    restocked: &[],
    deposited: Some((526, 1)),
    supply_step: "watch the Guard's looted Bones in pack before the bank trip",
    visit_step: "watch a Guard drop enter a fresh East Ardougne bank",
    return_step: "watch return to the market anchor after banking",
    xp_step: "watch fresh Strength XP after the bank return",
    selects_style: true,
};

const GREEN_ORACLE: OracleCell = OracleCell {
    name: "green_dragon_bank",
    field: WorldTile {
        x: 3096,
        z: 3814,
        level: 0,
    },
    bank: WorldTile {
        x: 3096,
        z: 3494,
        level: 0,
    },
    carried: &[(379, 12)],
    supplied: &[],
    bank_rows: &[(379, 100)],
    restocked: &[(379, 20)],
    deposited: None,
    supply_step: "watch the carried Lobsters run out before the bank trip",
    visit_step: "watch the restock of Lobster to the card's foodWithdraw 20",
    return_step: "watch return to the dragon field after banking",
    xp_step: "watch fresh Strength XP after the bank return",
    selects_style: false,
};

const ROCK_SEEDED_ORACLE: OracleCell = OracleCell {
    name: "rock_crab_bank_seeded",
    field: WorldTile {
        x: 2710,
        z: 3717,
        level: 0,
    },
    bank: WorldTile {
        x: 2725,
        z: 3491,
        level: 0,
    },
    carried: &[(379, 8), (1623, 1)],
    supplied: &[(379, 8), (1623, 1)],
    bank_rows: &[(1623, 1)],
    restocked: &[(379, 8)],
    deposited: Some((1623, 1)),
    supply_step: "watch seeded Sapphire leave the backpack after PeriodicBank deposit",
    visit_step: "watch seeded Sapphire enter a fresh Seers bank",
    return_step: "watch return toward the nearest RockCrab spot after seeded banking",
    xp_step: "watch fresh Strength XP after the seeded bank return",
    selects_style: false,
};

const CHICKEN_ORACLE: OracleCell = OracleCell {
    name: "chicken_killer_bank",
    field: WorldTile {
        x: 3029,
        z: 3294,
        level: 0,
    },
    bank: WorldTile {
        x: 3013,
        z: 3355,
        level: 0,
    },
    carried: &[],
    supplied: &[(526, 1)],
    bank_rows: &[(526, 1)],
    restocked: &[],
    deposited: Some((526, 1)),
    supply_step: "watch exact Bones 526 looted after Start",
    visit_step: "watch script-looted Bones enter a fresh bank",
    return_step: "watch return to the Falador chicken anchor within radius 6",
    xp_step: "watch fresh Strength XP after the bank return",
    selects_style: false,
};

#[test]
fn chicken_bank_oracle_passes_without_feathers() {
    oracle_assert_passes(&CHICKEN_ORACLE, Flaw::Correct);
}

#[test]
fn chicken_bank_oracle_rejects_no_pickup() {
    oracle_assert_fails_at(
        &CHICKEN_ORACLE,
        Flaw::NoSupplyEvent,
        CHICKEN_ORACLE.supply_step,
    );
}

#[test]
fn chicken_bank_oracle_rejects_no_deposit() {
    oracle_assert_fails_at(
        &CHICKEN_ORACLE,
        Flaw::NotDeposited,
        CHICKEN_ORACLE.visit_step,
    );
}

#[test]
fn chicken_bank_oracle_rejects_a_stalled_outbound_route() {
    oracle_assert_fails_at(
        &CHICKEN_ORACLE,
        Flaw::OutboundStall,
        CHICKEN_ORACLE.visit_step,
    );
}

#[test]
fn chicken_bank_oracle_rejects_a_stalled_return_route() {
    oracle_assert_fails_at(
        &CHICKEN_ORACLE,
        Flaw::ReturnStall,
        CHICKEN_ORACLE.return_step,
    );
}

#[test]
fn chicken_bank_oracle_rejects_no_fresh_xp() {
    oracle_assert_fails_at(&CHICKEN_ORACLE, Flaw::StaleXp, CHICKEN_ORACLE.xp_step);
}

fn chicken_bank_at_first_return() -> (ScenarioRunner, Client) {
    let cell = &CHICKEN_ORACLE;
    let (mut runner, mut c) = from_start(cell.name);
    set_inv(&mut c, cell.carried);
    oracle_settle(&mut runner, &mut c, 2);
    oracle_gain_strength(&mut c);
    oracle_settle(&mut runner, &mut c, 2);
    set_inv(&mut c, cell.supplied);
    oracle_settle(&mut runner, &mut c, 2);
    set_world_tile(&mut c, cell.bank);
    oracle_open_bank(&mut c, cell.bank_rows);
    set_inv(&mut c, &[]);
    oracle_settle(&mut runner, &mut c, 3);
    oracle_close_bank(&mut c);
    oracle_settle(&mut runner, &mut c, 2);
    set_world_tile(&mut c, cell.field);
    oracle_settle(&mut runner, &mut c, 2);
    assert_eq!(runner.current_step().name, cell.xp_step);
    (runner, c)
}

fn chicken_bank_extra_return(runner: &mut ScenarioRunner, c: &mut Client, count: i32) {
    set_inv(c, &[(526, 1)]);
    set_world_tile(c, CHICKEN_ORACLE.bank);
    oracle_open_bank(c, &[(526, count)]);
    set_inv(c, &[]);
    oracle_settle(runner, c, 1);
    oracle_close_bank(c);
    oracle_settle(runner, c, 1);
    set_world_tile(c, CHICKEN_ORACLE.field);
    oracle_settle(runner, c, 1);
}

#[test]
fn chicken_bank_oracle_restarts_after_an_extra_bank_return_then_accepts_fresh_xp() {
    let (mut runner, mut c) = chicken_bank_at_first_return();
    oracle_settle(&mut runner, &mut c, 100);
    set_world_tile(&mut c, CHICKEN_ORACLE.bank);
    oracle_open_bank(&mut c, &[(526, 2)]);
    oracle_settle(&mut runner, &mut c, 1);
    oracle_close_bank(&mut c);
    oracle_settle(&mut runner, &mut c, 100);
    set_world_tile(&mut c, CHICKEN_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 1);
    assert_eq!(runner.game_ticks_waited, 0);
    assert_eq!(runner.combat_bank_returns.unwrap().restarts, 1);
    let budget = runner.current_step().wait.budget_ticks;
    oracle_settle(&mut runner, &mut c, (budget - 2) as usize);
    assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
    oracle_gain_strength(&mut c);
    assert_eq!(oracle_finish(&mut runner, &mut c), RunnerStatus::Passed);
}

#[test]
fn chicken_bank_oracle_accepts_fresh_combat_after_chasing_outside_the_return_radius() {
    let (mut runner, mut c) = chicken_bank_at_first_return();
    set_world_tile(
        &mut c,
        WorldTile {
            x: 3019,
            z: 3288,
            level: 0,
        },
    );
    oracle_gain_strength(&mut c);
    assert_eq!(oracle_finish(&mut runner, &mut c), RunnerStatus::Passed);
}

#[test]
fn chicken_bank_oracle_rejects_no_xp_after_the_last_bank_return() {
    let (mut runner, mut c) = chicken_bank_at_first_return();
    chicken_bank_extra_return(&mut runner, &mut c, 2);
    let RunnerStatus::Failed(message) = oracle_finish(&mut runner, &mut c) else {
        panic!("the latest return must still require fresh XP");
    };
    assert!(
        message.contains("not seen within 150 game ticks"),
        "{message}"
    );
}

#[test]
fn chicken_bank_oracle_rejects_xp_first_observed_at_the_latest_return() {
    let (mut runner, mut c) = chicken_bank_at_first_return();
    set_world_tile(&mut c, CHICKEN_ORACLE.bank);
    oracle_open_bank(&mut c, &[(526, 2)]);
    oracle_settle(&mut runner, &mut c, 1);
    oracle_close_bank(&mut c);
    oracle_settle(&mut runner, &mut c, 1);
    oracle_gain_strength(&mut c);
    set_world_tile(&mut c, CHICKEN_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 1);
    assert!(matches!(
        oracle_finish(&mut runner, &mut c),
        RunnerStatus::Failed(_)
    ));
}

#[test]
fn chicken_bank_oracle_rejects_banking_past_the_restart_limit() {
    let (mut runner, mut c) = chicken_bank_at_first_return();
    for count in 2..=4 {
        chicken_bank_extra_return(&mut runner, &mut c, count);
        assert!(matches!(runner.status(), RunnerStatus::Running { .. }));
    }
    assert_eq!(runner.combat_bank_returns.unwrap().restarts, 3);
    set_world_tile(&mut c, CHICKEN_ORACLE.bank);
    oracle_open_bank(&mut c, &[(526, 5)]);
    oracle_gain_strength(&mut c);
    oracle_settle(&mut runner, &mut c, 1);
    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("a fourth restart must fail even if XP rises at the bank");
    };
    assert!(message.contains("restart limit spent"), "{message}");
}

#[test]
fn chicken_bank_oracle_does_not_restart_on_an_unfinished_bank_return() {
    let (mut runner, mut c) = chicken_bank_at_first_return();
    set_world_tile(&mut c, CHICKEN_ORACLE.bank);
    oracle_open_bank(&mut c, &[(526, 2)]);
    oracle_settle(&mut runner, &mut c, 1);
    assert!(matches!(
        oracle_finish(&mut runner, &mut c),
        RunnerStatus::Failed(_)
    ));
    assert_eq!(runner.combat_bank_returns.unwrap().restarts, 0);
}

#[test]
fn rock_crab_seeded_bank_oracle_passes_deposit_return_and_fresh_combat() {
    oracle_assert_passes(&ROCK_SEEDED_ORACLE, Flaw::Correct);
}

#[test]
fn rock_crab_seeded_bank_oracle_rejects_cargo_kept_in_the_pack() {
    oracle_assert_fails_at(
        &ROCK_SEEDED_ORACLE,
        Flaw::NotDeposited,
        ROCK_SEEDED_ORACLE.supply_step,
    );
}

#[test]
fn rock_crab_seeded_bank_oracle_rejects_a_stalled_outbound_route() {
    oracle_assert_fails_at(
        &ROCK_SEEDED_ORACLE,
        Flaw::OutboundStall,
        ROCK_SEEDED_ORACLE.supply_step,
    );
}

#[test]
fn rock_crab_seeded_bank_oracle_rejects_a_stalled_return_route() {
    oracle_assert_fails_at(
        &ROCK_SEEDED_ORACLE,
        Flaw::ReturnStall,
        ROCK_SEEDED_ORACLE.return_step,
    );
}

#[test]
fn rock_crab_seeded_bank_oracle_rejects_xp_only_before_the_return() {
    oracle_assert_fails_at(
        &ROCK_SEEDED_ORACLE,
        Flaw::StaleXp,
        ROCK_SEEDED_ORACLE.xp_step,
    );
}

fn oracle_settle(runner: &mut ScenarioRunner, c: &mut Client, game_ticks: usize) {
    for _ in 0..game_ticks {
        advance_one_game_tick(runner, c, false);
    }
}

fn oracle_gain_strength(c: &mut Client) {
    c.stat_xp[ORACLE_STRENGTH_STAT] += 4;
    c.bump_gens(ServerProt::UPDATE_STAT);
}

/// Open a withdraw window whose full contents are `rows`, the way the server
/// does: modal open, then a full inventory update for the grid component.
fn oracle_open_bank(c: &mut Client, rows: &[(i32, i32)]) {
    use client::io::Packet;
    c.set_iface(
        600,
        IfType {
            id: 600,
            layer_id: 600,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![601]),
            ..Default::default()
        },
    );
    c.set_iface(
        601,
        IfType {
            id: 601,
            layer_id: 600,
            r#type: ComponentType::TYPE_INV,
            iop: [Some("Withdraw 1".into()), None, None, None, None],
            ..Default::default()
        },
    );
    c.set_iface_mut(
        601,
        IfTypeMut {
            link_obj_type: Some(vec![0; 8]),
            link_obj_number: Some(vec![0; 8]),
            ..Default::default()
        },
    );
    c.handle_packet(ServerProt::IF_OPENMAIN, &mut Packet::new(vec![2, 88]));
    let mut full = vec![2, 89, rows.len() as u8];
    for &(id, count) in rows {
        let stored = id + 1;
        full.extend([(stored >> 8) as u8, (stored & 0xff) as u8, count as u8]);
    }
    c.handle_packet(ServerProt::UPDATE_INV_FULL, &mut Packet::new(full));
}

fn oracle_close_bank(c: &mut Client) {
    c.main_modal_id = -1;
    c.bump_gens(ServerProt::IF_CLOSE);
}

/// Tick until the run ends or the largest budget in the scenario has been
/// spent with a margin, and return the status.
fn oracle_finish(runner: &mut ScenarioRunner, c: &mut Client) -> RunnerStatus {
    let cap = runner
        .scenario
        .steps
        .iter()
        .map(|step| step.wait.budget_ticks)
        .max()
        .unwrap()
        + 5;
    for _ in 0..cap {
        if matches!(
            runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            break;
        }
        advance_one_game_tick(runner, c, false);
    }
    runner.status()
}

/// The correct observation sequence for `cell`, with only `flaw` changed.
fn oracle_drive(cell: &OracleCell, flaw: Flaw) -> RunnerStatus {
    let (mut runner, mut c) = from_start(cell.name);
    set_inv(&mut c, cell.carried);
    if cell.selects_style {
        with_combat_style_varp(&mut c);
    }
    if flaw == Flaw::DeathLineBeforeStart {
        push_system_chat(&mut c, crate::random_episode::DEATH_MESSAGE);
    }
    oracle_settle(&mut runner, &mut c, 2);
    if flaw == Flaw::DiedAfterStart {
        push_system_chat(&mut c, crate::random_episode::DEATH_MESSAGE);
    }
    if cell.selects_style {
        set_combat_style(&mut c, 1);
        oracle_settle(&mut runner, &mut c, 1);
    }
    oracle_gain_strength(&mut c);
    oracle_settle(&mut runner, &mut c, 2);
    if flaw != Flaw::NoSupplyEvent {
        set_inv(&mut c, cell.supplied);
    }
    oracle_settle(&mut runner, &mut c, 2);
    if flaw != Flaw::OutboundStall {
        set_world_tile(&mut c, cell.bank);
        let rows: &[(i32, i32)] = if flaw == Flaw::NotDeposited {
            &[(ORACLE_COINS_ID, 100)]
        } else {
            cell.bank_rows
        };
        oracle_open_bank(&mut c, rows);
        let mut pack = cell.restocked.to_vec();
        if let (Flaw::NotDeposited, Some(kept)) = (flaw, cell.deposited) {
            pack.push(kept);
        }
        set_inv(&mut c, &pack);
        oracle_settle(&mut runner, &mut c, 3);
        oracle_close_bank(&mut c);
        oracle_settle(&mut runner, &mut c, 2);
    }
    if flaw == Flaw::StaleXp {
        oracle_gain_strength(&mut c);
    }
    if flaw != Flaw::ReturnStall {
        set_world_tile(&mut c, cell.field);
    }
    oracle_settle(&mut runner, &mut c, 2);
    if flaw != Flaw::StaleXp {
        oracle_gain_strength(&mut c);
    }
    oracle_finish(&mut runner, &mut c)
}

fn oracle_assert_fails_at(cell: &OracleCell, flaw: Flaw, step: &str) {
    let budget = from_start(cell.name)
        .0
        .scenario
        .steps
        .iter()
        .find(|candidate| candidate.name == step)
        .unwrap_or_else(|| panic!("{}: no step named {step:?}", cell.name))
        .wait
        .budget_ticks;
    match oracle_drive(cell, flaw) {
        RunnerStatus::Failed(msg) => {
            assert!(
                msg.contains(step),
                "{} {flaw:?} must fail at {step:?}: {msg}",
                cell.name
            );
            assert!(
                msg.contains(&format!("not seen within {budget} game ticks")),
                "{} {flaw:?} must exhaust the step's own {budget}-game-tick budget: {msg}",
                cell.name
            );
        }
        other => panic!(
            "{} {flaw:?} must fail at {step:?}, got {other:?}",
            cell.name
        ),
    }
}

fn oracle_assert_passes(cell: &OracleCell, flaw: Flaw) {
    assert_eq!(
        oracle_drive(cell, flaw),
        RunnerStatus::Passed,
        "{} {flaw:?}",
        cell.name
    );
}

#[test]
fn hill_giant_bank_oracle_passes_the_full_trip() {
    oracle_assert_passes(&HILL_ORACLE, Flaw::Correct);
}

#[test]
fn hill_giant_bank_oracle_passes_a_limpwurt_first_trip() {
    oracle_assert_passes(&HILL_LIMPWURT_ORACLE, Flaw::Correct);
}

#[test]
fn hill_giant_bank_oracle_rejects_a_limpwurt_kept_in_the_pack() {
    oracle_assert_fails_at(
        &HILL_LIMPWURT_ORACLE,
        Flaw::NotDeposited,
        HILL_LIMPWURT_ORACLE.visit_step,
    );
}

#[test]
fn hill_giant_bank_oracle_rejects_xp_without_looted_bones() {
    oracle_assert_fails_at(&HILL_ORACLE, Flaw::NoSupplyEvent, HILL_ORACLE.supply_step);
}

#[test]
fn hill_giant_bank_oracle_rejects_loot_without_deposit() {
    oracle_assert_fails_at(&HILL_ORACLE, Flaw::NotDeposited, HILL_ORACLE.visit_step);
}

#[test]
fn hill_giant_bank_oracle_rejects_a_stalled_outbound_route() {
    oracle_assert_fails_at(&HILL_ORACLE, Flaw::OutboundStall, HILL_ORACLE.visit_step);
}

#[test]
fn hill_giant_bank_oracle_rejects_a_stalled_return_route() {
    oracle_assert_fails_at(&HILL_ORACLE, Flaw::ReturnStall, HILL_ORACLE.return_step);
}

#[test]
fn hill_giant_bank_oracle_rejects_xp_gained_only_before_the_return() {
    oracle_assert_fails_at(&HILL_ORACLE, Flaw::StaleXp, HILL_ORACLE.xp_step);
}

#[test]
fn ardy_fighter_bank_oracle_passes_the_full_trip() {
    oracle_assert_passes(&ARDY_ORACLE, Flaw::Correct);
}

#[test]
fn ardy_fighter_bank_oracle_rejects_xp_without_looted_bones() {
    oracle_assert_fails_at(&ARDY_ORACLE, Flaw::NoSupplyEvent, ARDY_ORACLE.supply_step);
}

#[test]
fn ardy_fighter_bank_oracle_rejects_loot_without_deposit() {
    oracle_assert_fails_at(&ARDY_ORACLE, Flaw::NotDeposited, ARDY_ORACLE.visit_step);
}

#[test]
fn ardy_fighter_bank_oracle_rejects_a_stalled_outbound_route() {
    oracle_assert_fails_at(&ARDY_ORACLE, Flaw::OutboundStall, ARDY_ORACLE.visit_step);
}

#[test]
fn ardy_fighter_bank_oracle_rejects_a_stalled_return_route() {
    oracle_assert_fails_at(&ARDY_ORACLE, Flaw::ReturnStall, ARDY_ORACLE.return_step);
}

#[test]
fn ardy_fighter_bank_oracle_rejects_xp_gained_only_before_the_return() {
    oracle_assert_fails_at(&ARDY_ORACLE, Flaw::StaleXp, ARDY_ORACLE.xp_step);
}

/// The transmitted `com_mode` varp table of the seeded account: mode 0
/// (Accurate) until the card presses its style.
fn with_combat_style_varp(c: &mut Client) {
    use client::config::{Cache, VarpType};
    c.cache = Arc::new(Cache {
        varps: (0..44).map(|_| VarpType::default()).collect(),
        ..Default::default()
    });
    c.var = vec![0; 44];
    c.bump_gens(ServerProt::VARP_SYNC);
}

fn set_combat_style(c: &mut Client, mode: i32) {
    c.var[43] = mode;
    c.bump_gens(ServerProt::VARP_SYNC);
}

const ARDY_STYLE_STEP: &str =
    "watch the card select its Strength style across pre-style Loot-count trips";
const ARDY_FIGHT_STEP: &str = "watch Strength XP from the selected melee style after Start";
const ARDY_ATTACK_STAT: usize = 0;
const ARDY_BONES: i32 = 526;
const ARDY_CAKE: i32 = 1891;
const ARDY_THIEVING_STAT: usize = 17;

/// One successful steal: Thieving XP (`stealing.dbrow` `experience` 160 =
/// 16 XP) and the stolen `food` in the pack.
fn ardy_steal(c: &mut Client, food: i32) {
    c.stat_xp[ARDY_THIEVING_STAT] += 16;
    c.bump_gens(ServerProt::UPDATE_STAT);
    set_inv(c, &[(food, 1)]);
}

fn ardy_bank_after_start() -> (ScenarioRunner, Client) {
    let (mut runner, mut c) = from_start(ARDY_ORACLE.name);
    set_inv(&mut c, &[]);
    with_combat_style_varp(&mut c);
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 2);
    (runner, c)
}

fn ardy_budget(step: &str) -> u32 {
    from_start(ARDY_ORACLE.name)
        .0
        .scenario
        .steps
        .iter()
        .find(|candidate| candidate.name == step)
        .unwrap_or_else(|| panic!("no step named {step:?}"))
        .wait
        .budget_ticks
}

/// One pre-style trip as the live receipts show it: a caught fight in the
/// card's unselected Accurate mode with no food in the pack, the Guard's
/// Bones, then the booth, where the deposit raises the banked Bones to
/// `banked`. `spend` game ticks pass before the deposit.
fn ardy_pre_style_trip(runner: &mut ScenarioRunner, c: &mut Client, banked: i32, spend: usize) {
    set_world_tile(c, ARDY_ORACLE.field);
    c.stat_xp[ARDY_ATTACK_STAT] += 40;
    c.bump_gens(ServerProt::UPDATE_STAT);
    oracle_settle(runner, c, 1);
    set_inv(c, &[(ARDY_BONES, 1)]);
    oracle_settle(runner, c, spend);
    set_world_tile(c, ARDY_ORACLE.bank);
    oracle_open_bank(c, &[(ARDY_BONES, banked - 1)]);
    oracle_settle(runner, c, 1);
    set_inv(c, &[]);
    oracle_open_bank(c, &[(ARDY_BONES, banked)]);
    oracle_settle(runner, c, 1);
    oracle_close_bank(c);
    oracle_settle(runner, c, 1);
}

fn assert_ardy_running(runner: &ScenarioRunner, context: &str) {
    assert!(
        matches!(runner.status(), RunnerStatus::Running { .. }),
        "{context}: {:?}",
        runner.status()
    );
}

/// The `base-d7` receipt (`FIX-ARDY-STYLE-025/runs/base-d7/timeline.txt`):
/// two Guard catches and one public Bones pile before the style, with the
/// deposits at +95, +160 and +279 game ticks after Start. The card had
/// stocked a Cake at +324 and was about to select its style when the old
/// 320-game-tick watch failed it. Projected on: style at +326, first
/// Strength hit at +333, then the ordinary bank chain.
#[test]
fn ardy_fighter_bank_style_watch_accepts_the_base_d7_receipt() {
    let (mut runner, mut c) = ardy_bank_after_start();
    // Each trip takes `spend + 3` game ticks to its observed deposit and one
    // more to the close.
    let mut now = 0;
    for (banked, deposit_at) in [(1, 95), (2, 160), (3, 279)] {
        ardy_pre_style_trip(&mut runner, &mut c, banked, deposit_at - now - 3);
        now = deposit_at + 1;
        assert_ardy_running(&runner, &format!("deposit {banked} at +{deposit_at}"));
    }
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 324 - now);
    set_inv(&mut c, &[(ARDY_CAKE, 1)]);
    oracle_settle(&mut runner, &mut c, 2);
    set_combat_style(&mut c, 1);
    oracle_settle(&mut runner, &mut c, 7);
    oracle_gain_strength(&mut c);
    oracle_settle(&mut runner, &mut c, 2);
    set_inv(&mut c, &[(ARDY_BONES, 1)]);
    oracle_settle(&mut runner, &mut c, 2);
    set_world_tile(&mut c, ARDY_ORACLE.bank);
    oracle_open_bank(&mut c, &[(ARDY_BONES, 4)]);
    set_inv(&mut c, &[]);
    oracle_settle(&mut runner, &mut c, 2);
    oracle_close_bank(&mut c);
    oracle_settle(&mut runner, &mut c, 2);
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 2);
    oracle_gain_strength(&mut c);
    assert_eq!(oracle_finish(&mut runner, &mut c), RunnerStatus::Passed);
}

fn assert_ardy_fails(runner: &mut ScenarioRunner, c: &mut Client, step: &str, needle: &str) {
    match oracle_finish(runner, c) {
        RunnerStatus::Failed(msg) => {
            assert!(msg.contains(step), "must fail at {step:?}: {msg}");
            assert!(msg.contains(needle), "must name {needle:?}: {msg}");
        }
        other => panic!("must fail at {step:?}, got {other:?}"),
    }
}

/// A card that neither banks, steals nor selects its style fails at one leg.
/// So does a card that steals its food and then idles with it: the steal
/// restarts the leg once, and nothing restarts it again.
#[test]
fn ardy_fighter_bank_style_watch_rejects_an_idle_card() {
    let session = ardy_budget(ARDY_STYLE_STEP);
    let (mut runner, mut c) = ardy_bank_after_start();
    assert_ardy_fails(
        &mut runner,
        &mut c,
        ARDY_STYLE_STEP,
        &format!("not seen within {session} game ticks"),
    );
    let (mut runner, mut c) = ardy_bank_after_start();
    oracle_settle(&mut runner, &mut c, session as usize - 20);
    ardy_steal(&mut c, ARDY_CAKE);
    oracle_settle(&mut runner, &mut c, 30);
    assert_ardy_running(&runner, "the steal restarted the leg");
    assert_ardy_fails(
        &mut runner,
        &mut c,
        ARDY_STYLE_STEP,
        &format!("[pre-style trips=0 deposited=0 steals=1] not seen within {session} game ticks"),
    );
}

/// The `final-r10` receipt (`FIX-ARDY-STYLE-025/runs/final-r10/timeline.txt`):
/// three pre-style trips (deposits at +93, +165 and +293), then six owner
/// refusals, two successful steals that the card ate at once (+380, +389),
/// an emptied stall and a third catch (+406 steal, +451 Bones). The live run
/// failed the deposit-only leg at 515 game ticks, 11 tiles from the booth,
/// so its fourth deposit is set at +510. Projected on: back at the stall,
/// one steal held at +540, style at +542, first Strength hit at +549.
#[test]
fn ardy_fighter_bank_style_watch_accepts_the_final_r10_receipt() {
    let (mut runner, mut c) = ardy_bank_after_start();
    let mut now = 0;
    for (banked, deposit_at) in [(1, 93), (2, 165), (3, 293)] {
        ardy_pre_style_trip(&mut runner, &mut c, banked, deposit_at - now - 3);
        now = deposit_at + 1;
    }
    set_world_tile(&mut c, ARDY_ORACLE.field);
    for (steal_at, food) in [(380, 1901), (389, ARDY_CAKE)] {
        oracle_settle(&mut runner, &mut c, steal_at - now);
        ardy_steal(&mut c, food);
        oracle_settle(&mut runner, &mut c, 2);
        set_inv(&mut c, &[]);
        now = steal_at + 2;
    }
    oracle_settle(&mut runner, &mut c, 406 - now);
    now = 406;
    ardy_pre_style_trip(&mut runner, &mut c, 4, 510 - now - 3);
    now = 511;
    assert_ardy_running(&runner, "the fourth deposit at +510");
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 540 - now);
    ardy_steal(&mut c, ARDY_CAKE);
    oracle_settle(&mut runner, &mut c, 2);
    set_combat_style(&mut c, 1);
    oracle_settle(&mut runner, &mut c, 7);
    oracle_gain_strength(&mut c);
    oracle_settle(&mut runner, &mut c, 2);
    set_inv(&mut c, &[(ARDY_BONES, 1)]);
    oracle_settle(&mut runner, &mut c, 2);
    set_world_tile(&mut c, ARDY_ORACLE.bank);
    oracle_open_bank(&mut c, &[(ARDY_BONES, 5)]);
    set_inv(&mut c, &[]);
    oracle_settle(&mut runner, &mut c, 2);
    oracle_close_bank(&mut c);
    oracle_settle(&mut runner, &mut c, 2);
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 2);
    oracle_gain_strength(&mut c);
    assert_eq!(oracle_finish(&mut runner, &mut c), RunnerStatus::Passed);
}

/// The 2026-10-24 nightly receipt on `dev` `58080a889`
/// (`NIGHTLY/2026-10-24-dev-58080a889/cells/ardy_fighter_bank`): three
/// pre-style trips deposited four Bones and two successful steals were eaten,
/// then the final leg's restock was refused at both Baker's stands before a
/// catch. The receipt's final tile is the East Ardougne booth with that
/// catch's Bones still in the pack and no food held, so the fourth deposit
/// lands 213 game ticks after the third (the return, one refusal run at each
/// stand, the catch and the bank walk), beyond the former one-refusal-run
/// leg. Projected on: one steal held, the style, and the ordinary bank chain.
#[test]
fn ardy_fighter_bank_style_watch_accepts_the_nightly_1024_receipt() {
    let (mut runner, mut c) = ardy_bank_after_start();
    ardy_pre_style_trip(&mut runner, &mut c, 1, 90);
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 20);
    ardy_steal(&mut c, 1901);
    oracle_settle(&mut runner, &mut c, 2);
    set_inv(&mut c, &[]);
    ardy_pre_style_trip(&mut runner, &mut c, 2, 70);
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 20);
    ardy_steal(&mut c, ARDY_CAKE);
    oracle_settle(&mut runner, &mut c, 2);
    set_inv(&mut c, &[]);
    // The third trip deposits two Bones in one observed rise (banked 2 to 4).
    set_world_tile(&mut c, ARDY_ORACLE.field);
    c.stat_xp[ARDY_ATTACK_STAT] += 40;
    c.bump_gens(ServerProt::UPDATE_STAT);
    oracle_settle(&mut runner, &mut c, 1);
    set_inv(&mut c, &[(ARDY_BONES, 1)]);
    oracle_settle(&mut runner, &mut c, 90);
    set_world_tile(&mut c, ARDY_ORACLE.bank);
    oracle_open_bank(&mut c, &[(ARDY_BONES, 2)]);
    oracle_settle(&mut runner, &mut c, 1);
    set_inv(&mut c, &[]);
    oracle_open_bank(&mut c, &[(ARDY_BONES, 4)]);
    oracle_settle(&mut runner, &mut c, 1);
    oracle_close_bank(&mut c);
    oracle_settle(&mut runner, &mut c, 1);
    assert_ardy_running(&runner, "the third deposit (four Bones banked)");
    // Final leg: return and one refused run at each stand (no progress
    // signal), the catch, then the walk to the booth.
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 119);
    c.stat_xp[ARDY_ATTACK_STAT] += 40;
    c.bump_gens(ServerProt::UPDATE_STAT);
    oracle_settle(&mut runner, &mut c, 1);
    set_inv(&mut c, &[(ARDY_BONES, 1)]);
    oracle_settle(&mut runner, &mut c, 90);
    set_world_tile(&mut c, ARDY_ORACLE.bank);
    oracle_open_bank(&mut c, &[(ARDY_BONES, 4)]);
    oracle_settle(&mut runner, &mut c, 1);
    set_inv(&mut c, &[]);
    oracle_open_bank(&mut c, &[(ARDY_BONES, 5)]);
    oracle_settle(&mut runner, &mut c, 1);
    assert_ardy_running(&runner, "the fourth deposit after two refusal runs");
    oracle_close_bank(&mut c);
    oracle_settle(&mut runner, &mut c, 1);
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 20);
    ardy_steal(&mut c, ARDY_CAKE);
    oracle_settle(&mut runner, &mut c, 2);
    set_combat_style(&mut c, 1);
    oracle_settle(&mut runner, &mut c, 7);
    oracle_gain_strength(&mut c);
    oracle_settle(&mut runner, &mut c, 2);
    set_inv(&mut c, &[(ARDY_BONES, 1)]);
    oracle_settle(&mut runner, &mut c, 2);
    set_world_tile(&mut c, ARDY_ORACLE.bank);
    oracle_open_bank(&mut c, &[(ARDY_BONES, 6)]);
    set_inv(&mut c, &[]);
    oracle_settle(&mut runner, &mut c, 2);
    oracle_close_bank(&mut c);
    oracle_settle(&mut runner, &mut c, 2);
    set_world_tile(&mut c, ARDY_ORACLE.field);
    oracle_settle(&mut runner, &mut c, 2);
    oracle_gain_strength(&mut c);
    assert_eq!(oracle_finish(&mut runner, &mut c), RunnerStatus::Passed);
}

/// A trip session longer than content allows fails, even after an earlier
/// trip restarted the budget.
#[test]
fn ardy_fighter_bank_style_watch_rejects_a_session_beyond_one_content_trip() {
    let session = ardy_budget(ARDY_STYLE_STEP);
    let (mut runner, mut c) = ardy_bank_after_start();
    ardy_pre_style_trip(&mut runner, &mut c, 1, 20);
    set_world_tile(&mut c, ARDY_ORACLE.field);
    c.stat_xp[ARDY_ATTACK_STAT] += 40;
    c.bump_gens(ServerProt::UPDATE_STAT);
    set_inv(&mut c, &[(ARDY_BONES, 1)]);
    assert_ardy_fails(
        &mut runner,
        &mut c,
        ARDY_STYLE_STEP,
        &format!("[pre-style trips=1 deposited=1 steals=0] not seen within {session} game ticks"),
    );
}

/// Melee XP while the card holds stall food and has not selected Strength
/// (Accurate mode 0, or the wrong Controlled mode 2) fails at once.
#[test]
fn ardy_fighter_bank_style_watch_rejects_a_fight_without_the_selected_style() {
    for (mode, stat) in [(0, ARDY_ATTACK_STAT), (2, ORACLE_STRENGTH_STAT)] {
        let (mut runner, mut c) = ardy_bank_after_start();
        set_inv(&mut c, &[(1893, 1)]);
        set_combat_style(&mut c, mode);
        oracle_settle(&mut runner, &mut c, 3);
        c.stat_xp[stat] += 40;
        c.bump_gens(ServerProt::UPDATE_STAT);
        oracle_settle(&mut runner, &mut c, 1);
        match runner.status() {
            RunnerStatus::Failed(msg) => assert!(
                msg.contains(ARDY_STYLE_STEP)
                    && msg.contains("melee XP rose while stall food was held"),
                "mode {mode}: {msg}"
            ),
            other => panic!("mode {mode} must fail at once, got {other:?}"),
        }
    }
}

/// After the style, a card that waits instead of fighting fails at the
/// fight window.
#[test]
fn ardy_fighter_bank_style_watch_rejects_a_card_that_does_not_fight() {
    let fight = ardy_budget(ARDY_FIGHT_STEP);
    let (mut runner, mut c) = ardy_bank_after_start();
    set_inv(&mut c, &[(ARDY_CAKE, 1)]);
    set_combat_style(&mut c, 1);
    oracle_settle(&mut runner, &mut c, 2);
    assert_ardy_fails(
        &mut runner,
        &mut c,
        ARDY_FIGHT_STEP,
        &format!("not seen within {fight} game ticks"),
    );
}

#[test]
fn green_dragon_bank_oracle_passes_the_full_trip() {
    oracle_assert_passes(&GREEN_ORACLE, Flaw::Correct);
}

#[test]
fn green_dragon_bank_oracle_rejects_food_that_never_runs_out() {
    oracle_assert_fails_at(&GREEN_ORACLE, Flaw::NoSupplyEvent, GREEN_ORACLE.supply_step);
}

#[test]
fn green_dragon_bank_oracle_rejects_a_stalled_outbound_route() {
    oracle_assert_fails_at(&GREEN_ORACLE, Flaw::OutboundStall, GREEN_ORACLE.visit_step);
}

#[test]
fn green_dragon_bank_oracle_rejects_a_stalled_return_route() {
    oracle_assert_fails_at(&GREEN_ORACLE, Flaw::ReturnStall, GREEN_ORACLE.return_step);
}

#[test]
fn green_dragon_bank_oracle_rejects_xp_gained_only_before_the_return() {
    oracle_assert_fails_at(&GREEN_ORACLE, Flaw::StaleXp, GREEN_ORACLE.xp_step);
}

#[test]
fn green_dragon_bank_oracle_rejects_death_then_full_recovery() {
    // The recovery (restock, close, return, fresh XP) is the correct trip, so
    // without the death line this exact sequence passes.
    match oracle_drive(&GREEN_ORACLE, Flaw::DiedAfterStart) {
        RunnerStatus::Failed(msg) => assert!(
            msg.contains("died"),
            "a death after Start must fail the run by name: {msg}"
        ),
        other => panic!("a death after Start must fail the run, got {other:?}"),
    }
}

#[test]
fn green_dragon_bank_oracle_ignores_a_death_line_from_before_start() {
    oracle_assert_passes(&GREEN_ORACLE, Flaw::DeathLineBeforeStart);
}

fn guard_ground_item(id: i32, count: i32, tile: WorldTile) -> api::snapshot::GroundItemView {
    api::snapshot::GroundItemView {
        def: api::obj_names::ItemDefView {
            id,
            name: None,
            stackable: false,
            members: false,
            base_value: 0,
            noted: false,
            certificate_link: -1,
            certificate_template: -1,
        },
        count,
        actions: vec![],
        tile,
        distance: 0,
    }
}

fn auto_fighter_floor_guard() -> (ScenarioRunner, Client) {
    let mut scenario = crate::get("auto_fighter_bank").unwrap();
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    scenario.steps.truncate(start + 1);
    scenario.steps.retain(|step| {
        step.name == "wait for the Guard field's public Bones to clear before Start"
            || matches!(step.kind, StepKind::StartScript)
    });
    scenario.seed.mainland = false;
    scenario.settings.require_mainland_base = false;
    scenario.proof = Proof::Stat { id: 16, min: 0 };
    let mut runner = ScenarioRunner::new(scenario);
    runner.set_scene_settle(Duration::ZERO);
    let mut c = seeded_client();
    set_world_tile(
        &mut c,
        WorldTile {
            x: 2661,
            z: 3306,
            level: 0,
        },
    );
    runner.snapshot.rebuild(&c);
    (runner, c)
}

#[test]
fn auto_fighter_bank_does_not_start_over_public_bones() {
    let (mut runner, mut c) = auto_fighter_floor_guard();
    runner.snapshot.seed_ground_items(vec![guard_ground_item(
        526,
        1,
        WorldTile {
            x: 2661,
            z: 3306,
            level: 0,
        },
    )]);
    for _ in 0..4 {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
        assert!(
            !runner.on_start_script() && !runner.script_started,
            "public Bones must not trigger a bank trip before this account's first kill"
        );
    }
    assert!(
        matches!(runner.status(), RunnerStatus::Running { .. }),
        "{:?}",
        runner.status()
    );
}

#[test]
fn auto_fighter_bank_clean_floor_allows_start_and_records_zero_wait() {
    let (mut runner, mut c) = auto_fighter_floor_guard();
    runner.tick(&mut c);
    assert!(runner.on_start_script());
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    let guard = runner.evidence().unwrap().ground_guard.as_ref().unwrap();
    assert_eq!(guard.ticks_waited, 0);
    assert_eq!(guard.budget_game_ticks, 202);
    assert!(guard.items_seen.is_empty());
}

#[test]
fn auto_fighter_bank_floor_guard_records_loot_then_starts_after_removal() {
    let (mut runner, mut c) = auto_fighter_floor_guard();
    runner.snapshot.seed_ground_items(vec![guard_ground_item(
        526,
        2,
        WorldTile {
            x: 2661,
            z: 3306,
            level: 0,
        },
    )]);
    runner.tick(&mut c);
    assert!(!runner.script_started);
    for _ in 0..3 {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
    }
    runner.snapshot.seed_ground_items(vec![]);
    c.bump_gens(ServerProt::PLAYER_INFO);
    runner.tick(&mut c);
    assert!(runner.on_start_script());
    runner.tick(&mut c);
    assert_eq!(runner.status(), RunnerStatus::Passed);
    let evidence = runner.evidence().unwrap();
    let guard = evidence.ground_guard.as_ref().unwrap();
    assert_eq!(guard.ticks_waited, 4);
    assert_eq!(guard.items_seen.len(), 1);
    assert_eq!(guard.items_seen[0].id, 526);
    assert_eq!(guard.items_seen[0].count, 2);
    assert_eq!(guard.items_seen[0].tile, [2661, 3306, 0]);
    let json: serde_json::Value = serde_json::from_str(&evidence.to_json()).unwrap();
    assert_eq!(json["ground_guard"]["ticks_waited"], 4);
    assert_eq!(json["ground_guard"]["items_seen"][0]["id"], 526);
}

#[test]
fn auto_fighter_bank_floor_guard_counts_game_ticks_and_fails_at_content_cap() {
    let (mut runner, mut c) = auto_fighter_floor_guard();
    runner.snapshot.seed_ground_items(vec![guard_ground_item(
        526,
        1,
        WorldTile {
            x: 2661,
            z: 3306,
            level: 0,
        },
    )]);
    runner.tick(&mut c);
    for _ in 0..250 {
        c.bump_gens(ServerProt::MESSAGE_GAME);
        runner.tick(&mut c);
    }
    assert!(matches!(
        runner.status(),
        RunnerStatus::Running { step: 0, .. }
    ));
    assert_eq!(runner.ground_guard.as_ref().unwrap().ticks_waited, 0);
    for _ in 0..201 {
        c.bump_gens(ServerProt::PLAYER_INFO);
        runner.tick(&mut c);
    }
    assert!(matches!(
        runner.status(),
        RunnerStatus::Running { step: 0, .. }
    ));
    c.bump_gens(ServerProt::PLAYER_INFO);
    runner.tick(&mut c);
    let RunnerStatus::Failed(message) = runner.status() else {
        panic!("unclean field must fail at the content cap");
    };
    assert!(message.contains("fixture interference"));
    assert!(message.contains("202 observed game ticks"));
    let guard = runner.evidence().unwrap().ground_guard.as_ref().unwrap();
    assert_eq!(guard.ticks_waited, 202);
    assert_eq!(guard.items_seen.len(), 1);
    assert!(!runner.script_started);
}

#[test]
fn ground_item_absence_proof_bounds_ids_area_level_and_loaded_player() {
    let (mut runner, mut c) = auto_fighter_floor_guard();
    let proof = runner.scenario.steps[0].wait.arm;
    assert!(proof.check(&runner.snapshot, None));
    for (id, count, x, z, level, clean) in [
        (526, 1, 2661, 3306, 0, false),
        (526, 1, 2673, 3318, 0, false),
        (526, 1, 2674, 3306, 0, true),
        (526, 1, 2661, 3306, 1, true),
        (995, 1, 2661, 3306, 0, true),
        (526, 0, 2661, 3306, 0, true),
    ] {
        runner.snapshot.seed_ground_items(vec![guard_ground_item(
            id,
            count,
            WorldTile { x, z, level },
        )]);
        assert_eq!(proof.check(&runner.snapshot, None), clean);
    }
    runner.snapshot.seed_ground_items(vec![]);
    c.scene_state = 1;
    runner.snapshot.rebuild(&c);
    assert!(!proof.check(&runner.snapshot, None));
    c.scene_state = 2;
    set_world_tile(
        &mut c,
        WorldTile {
            x: 2690,
            z: 3306,
            level: 0,
        },
    );
    runner.snapshot.rebuild(&c);
    assert!(!proof.check(&runner.snapshot, None));
    c.ingame = false;
    runner.snapshot.rebuild(&c);
    assert!(!proof.check(&runner.snapshot, None));
}
