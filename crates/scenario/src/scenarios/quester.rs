//! Quester qualification fixtures. Cheats are allowed here only.
use super::script_basics::script_live_seed_steps;
use crate::*;

const COOK_DEADLINE: Duration = Duration::from_secs(900);
const COOK_WATCH: u32 = 3600;
const COOK_CARD: &str = "Quester";
const COOK_KITCHEN: WorldTile = WorldTile {
    x: 3209,
    z: 3215,
    level: 0,
};

const SHEEP_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quest",
    value: ScriptInjectValue::Str("sheep"),
}];
const RUNE_MYSTERIES_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quest",
    value: ScriptInjectValue::Str("runemysteries"),
}];
const ROMEO_AND_JULIET_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quest",
    value: ScriptInjectValue::Str("romeojuliet"),
}];
// Q2 route threshold: the Sheep and Rune legs cross bearded dark wizards
// (vislevel 20, huntmode `ranged`) and young dark wizards (vislevel 7,
// huntmode `ranged`). Engine `Npc.huntPlayers` skips a player whose combat
// exceeds twice the hunter's vislevel (`Npc.ts`, `check_nottoostrong =
// outside_wilderness` in `all.hunt [ranged]`), so the bearded wizard stays
// passive only at combat 41+, the young at 15+, and Aubury's Mugger
// (vislevel 6, `cowardly`) at 13+. Sheep and Rune therefore stage the
// tested combat-41 minimum from the minme fresh base (all stats 1,
// Hitpoints 10 — `ClientCheatHandler.ts` `minme`) and hold Start until the
// posted stats prove it. "Minimum" is the combat threshold, not
// individually minimal skills: Attack/Strength/Defence 40 follows the
// existing combat-prep precedent (`combat.rs` `COMBAT_ATTACK_LEVEL`), and
// Hitpoints 20 keeps the total at exactly 41 while adding a survivability
// margin. This covers only the passive-threshold route hazard: Draynor
// jail guards (vislevel 26, huntmode `aggressive_melee`,
// `check_nottoostrong = off`) hunt at any combat level and remain a route
// hazard until nav zones land. Romeo & Juliet stays at the fresh base —
// no hunter reaches its stands — and is the unchanged baseline.
const S2_ATTACK_STAT_ID: i32 = 0;
const S2_DEFENCE_STAT_ID: i32 = 1;
const S2_STRENGTH_STAT_ID: i32 = 2;
const S2_HITPOINTS_STAT_ID: i32 = 3;
/// Staged Attack/Strength/Defence: the existing combat-prep precedent
/// (`combat.rs` `COMBAT_ATTACK_LEVEL`). No maxme: nothing above 40.
const S2_SAFE_COMBAT_SKILL: i32 = 40;
/// Staged Hitpoints. Engine `Player.getCombatLevel` posts exactly combat
/// 41 for the tuple: base 0.25*(40+20) = 15, melee 0.325*(40+40) = 26,
/// floor(41) = 41 — past twice the bearded wizard's 20. Dropping any one
/// of the four staged stats by a level posts 40, still hunted.
const S2_SAFE_HITPOINTS: i32 = 20;
/// Fresh-base reset every S2 quest fixture emits before any seed.
fn s2_reset_step() -> Step {
    Step {
        name: "reset fixture stats to the fresh base",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "minme");
                true
            }),
        },
        wait: Wait {
            // Posted Defence back at base 1 before any seed lands. Fails
            // closed while a reused account still carries a staged profile.
            arm: Proof::StatAtMost {
                id: S2_DEFENCE_STAT_ID,
                max: 1,
            },
            budget_ticks: 80,
        },
    }
}
/// Shared combat-41 profile for the Sheep and Rune fixtures: one raise per
/// tuple stat plus an observed guard per stat, so Start waits until the
/// posted levels prove exactly combat 41 (see `S2_SAFE_HITPOINTS`). Observe
/// before teleport/relog/Start; relog clears the stat-level-up UI. The
/// guards say nothing about the jail guards, which hunt at any combat.
fn s2_combat_profile_steps() -> Vec<Step> {
    const PROFILE: [(i32, &str, i32); 4] = [
        (S2_ATTACK_STAT_ID, "attack", S2_SAFE_COMBAT_SKILL),
        (S2_DEFENCE_STAT_ID, "defence", S2_SAFE_COMBAT_SKILL),
        (S2_STRENGTH_STAT_ID, "strength", S2_SAFE_COMBAT_SKILL),
        (S2_HITPOINTS_STAT_ID, "hitpoints", S2_SAFE_HITPOINTS),
    ];
    PROFILE
        .iter()
        .map(|(id, skill, level)| {
            let id = *id;
            let skill = *skill;
            let level = *level;
            Step {
                name: "stage combat-41 profile and observe it before Start",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &format!("setstat {skill} {level}"));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat { id, min: level },
                    budget_ticks: 80,
                },
            }
        })
        .collect()
}
/// Splice S2 safe-stat staging into a quest fixture ahead of its `setvar`
/// stage seed. `profile` holds the quest's raises (empty for Romeo &
/// Juliet, which stays at the fresh base). Cook fixtures are out of scope
/// and keep the shared `quester_stage` path untouched.
fn stage_s2_safe_stats(scenario: &mut Scenario, profile: Vec<Step>) {
    let seed = scenario
        .steps
        .iter()
        .position(|step| step.name == "reset quest stage")
        .expect("quester fixture has a quest-stage seed step");
    let mut prep = vec![s2_reset_step()];
    prep.extend(profile);
    scenario.steps.splice(seed..seed, prep);
}
/// Generic builder: jump a quest to a stage key with optional items, then
/// Start Quester. `varp`/`value` seed via `setvar` (fixture only). The pack
/// is always cleared first, even when `items` is empty, so a reused fixture
/// account cannot carry quest items into the run.
pub fn quester_stage(
    name: &'static str,
    quest_display: &'static str,
    varp: &'static str,
    value: i32,
    items: &'static [(&'static str, i32)],
    stand: WorldTile,
) -> Scenario {
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "reset quest stage",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setvar {varp} {value}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 40,
        },
    });
    steps.push(Step {
        name: "clear and seed inventory",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                for (alias, qty) in items {
                    cheat(c, &format!("give {alias} {qty}"));
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 80,
        },
    });
    steps.push(Step {
        name: "stand at the quest start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(stand.level, stand.x, stand.z)).is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: stand.x,
                z: stand.z,
                level: stand.level,
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "relog so the quest tab colour matches the seeded varp",
        kind: StepKind::Relog,
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 600,
        },
    });
    steps.push(start_compiled_step());
    steps.push(Step {
        name: "watch the quest tab turn complete",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::QuestDone {
                name: quest_display,
            },
            budget_ticks: COOK_WATCH,
        },
    });
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::QuestDone {
            name: quest_display,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: COOK_DEADLINE,
            start_script: Some(COOK_CARD),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Fresh account Cook's Assistant, colour-only, no journal.
pub(crate) fn quester_cook_scenario() -> Scenario {
    quester_stage(
        "quester_cook",
        "Cook's Assistant",
        "cookquest",
        0,
        &[],
        COOK_KITCHEN,
    )
}

pub(crate) fn quester_sheep_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_sheep",
        "Sheep Shearer",
        "sheep",
        0,
        &[("coins", 100)],
        WorldTile {
            x: 3197,
            z: 3266,
            level: 0,
        },
    );
    scenario.settings.script_settings_inject = Some(SHEEP_SETTINGS);
    // Bearded-dark-wizard leg: reset, then the combat-41 profile whose
    // observed guards hold Start until the posted stats prove combat 41.
    // No wool or weapon seed: the Path shears, spins and hands in from
    // zero, buying shears with the seeded coins when none are banked.
    stage_s2_safe_stats(&mut scenario, s2_combat_profile_steps());
    scenario
}

/// Fresh Duke Horacio start: the fixture stands on the castle's upper floor
/// by the Duke and plays the whole quest; nothing is seeded.
pub(crate) fn quester_rune_mysteries_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_rune_mysteries",
        "Rune Mysteries Quest",
        "runemysteries",
        0,
        &[],
        WorldTile {
            x: 3212,
            z: 3220,
            level: 1,
        },
    );
    scenario.settings.script_settings_inject = Some(RUNE_MYSTERIES_SETTINGS);
    // Same bearded-dark-wizard leg as Sheep: reset, then the combat-41
    // profile whose observed guards hold Start until the posted stats prove
    // combat 41. The stand is the Path's own `start-duke` anchor
    // (paths/289/runemysteries.json, upstairs level 1).
    stage_s2_safe_stats(&mut scenario, s2_combat_profile_steps());
    scenario
}

pub(crate) fn quester_romeo_and_juliet_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_romeo_and_juliet",
        "Romeo & Juliet",
        "rjquest",
        0,
        &[],
        WorldTile {
            x: 3211,
            z: 3425,
            level: 0,
        },
    );
    scenario.settings.script_settings_inject = Some(ROMEO_AND_JULIET_SETTINGS);
    // Unchanged baseline: no hunter reaches its stands, so the fresh base
    // (Tier A, passive 3) is the safe profile — reset, no raise.
    stage_s2_safe_stats(&mut scenario, Vec::new());
    scenario
}

/// Resume Cook from in-progress with the three products already held.
pub(crate) fn quester_cook_resume_scenario() -> Scenario {
    quester_stage(
        "quester_cook_resume",
        "Cook's Assistant",
        "cookquest",
        1,
        &[("egg", 1), ("bucket_milk", 1), ("pot_flour", 1)],
        COOK_KITCHEN,
    )
}

/// Start during relog, before the new session has posted its quest-tab colours.
pub(crate) fn quester_cook_login_scenario() -> Scenario {
    let mut scenario = quester_cook_resume_scenario();
    scenario.name = "quester_cook_login";
    scenario.settings.terminal_shot = Some("quester_cook_login");
    let relog = scenario
        .steps
        .iter()
        .rposition(|step| matches!(step.kind, StepKind::Relog))
        .unwrap();
    scenario.steps[relog].wait.arm = Proof::LoggedOut;
    scenario.steps.remove(relog + 1);
    scenario
}

/// Stop after an observed egg, then restart from live colour with that item held.
pub(crate) fn quester_cook_restart_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_cook_restart",
        "Cook's Assistant",
        "cookquest",
        1,
        &[],
        COOK_KITCHEN,
    );
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    scenario.steps.splice(
        start + 1..start + 1,
        [
            Step {
                name: "watch Quester acquire an egg before Stop",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::Item {
                        name: "Egg",
                        count: 1,
                    },
                    budget_ticks: COOK_WATCH,
                },
            },
            Step {
                name: "stop Quester mid-quest",
                kind: StepKind::StopScript,
                wait: Wait {
                    arm: Proof::ScriptIdle,
                    budget_ticks: 10,
                },
            },
            start_compiled_step(),
        ],
    );
    scenario
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::snapshot::GameSnapshot;
    use client::client::{Client, ClientConfig};
    use client::dash3d::ClientPlayer;
    use client::io::ServerProt;

    /// Staged tuple level per stat, mirroring the fixture constants above.
    const STAGED_LEVELS: [i32; 4] = [
        S2_SAFE_COMBAT_SKILL,
        S2_SAFE_COMBAT_SKILL,
        S2_SAFE_COMBAT_SKILL,
        S2_SAFE_HITPOINTS,
    ];

    /// Cheat-admitted client for exercising fixture staging commands.
    fn fixture_client() -> Client {
        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.set_cheat_admission(client::CheatAdmission::Granted);
        client.map_build_base_x = 3200;
        client.map_build_base_z = 3200;
        client.local_player = Some(ClientPlayer::at(20, 12));
        client
    }

    fn start_index(scenario: &Scenario) -> usize {
        scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("quester fixture Starts the compiled card")
    }

    /// Concatenated cheat bytes every pre-Start `Perform` send emits — the
    /// actual fixture operations, not step names.
    fn prestart_cheats(scenario: &Scenario) -> String {
        let start = start_index(scenario);
        let mut client = fixture_client();
        let mut written = String::new();
        for step in &scenario.steps[..start] {
            if let StepKind::Perform { send } = &step.kind {
                let snapshot = GameSnapshot::new();
                let before = client.out.pos;
                assert!(
                    send(&mut client, &snapshot),
                    "pre-Start send '{}' must succeed",
                    step.name
                );
                written.push_str(&String::from_utf8_lossy(
                    &client.out.data()[before..client.out.pos],
                ));
            }
        }
        written
    }

    /// Snapshot with the given posted effective melee levels (fresh base
    /// 1/1/1/10 after `minme`, staged tuple after the profile `setstat`s).
    fn posted_melee_snapshot(
        attack: i32,
        defence: i32,
        strength: i32,
        hitpoints: i32,
    ) -> GameSnapshot {
        let mut client = fixture_client();
        client.stat_effective_level[S2_ATTACK_STAT_ID as usize] = attack;
        client.stat_effective_level[S2_DEFENCE_STAT_ID as usize] = defence;
        client.stat_effective_level[S2_STRENGTH_STAT_ID as usize] = strength;
        client.stat_effective_level[S2_HITPOINTS_STAT_ID as usize] = hitpoints;
        client.bump_gens(ServerProt::UPDATE_STAT);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        snapshot
    }

    fn fresh_base_snapshot() -> GameSnapshot {
        posted_melee_snapshot(1, 1, 1, 10)
    }

    fn staged_snapshot() -> GameSnapshot {
        posted_melee_snapshot(
            STAGED_LEVELS[0],
            STAGED_LEVELS[1],
            STAGED_LEVELS[2],
            STAGED_LEVELS[3],
        )
    }

    /// Staged tuple with one stat a level below staged.
    fn staged_minus_one(stat: i32) -> GameSnapshot {
        let mut levels = STAGED_LEVELS;
        levels[stat as usize] -= 1;
        posted_melee_snapshot(levels[0], levels[1], levels[2], levels[3])
    }

    /// Combat `combat_level.rs2` posts for a melee profile off the minme
    /// base (Ranged/Magic 1, Prayer 1, so Prayer/2 floors to 0 and the melee
    /// term wins): `(10 * (def + hp) + 13 * (atk + str)) / 40` in integer
    /// division. Matches engine `Player.getCombatLevel` for these inputs.
    fn melee_combat_level(attack: i32, defence: i32, strength: i32, hitpoints: i32) -> i32 {
        (10 * (defence + hitpoints) + 13 * (attack + strength)) / 40
    }

    #[test]
    fn sheep_and_rune_fixtures_hold_start_until_posted_stats_prove_combat_41() {
        // The staged tuple posts exactly combat 41 — past twice the bearded
        // dark wizard's 20 — while any single stat one below posts 40 and
        // the fresh base posts 3. The retired Defence-40 profile posted 13,
        // which the bearded wizard still hunts.
        assert_eq!(
            melee_combat_level(
                S2_SAFE_COMBAT_SKILL,
                S2_SAFE_COMBAT_SKILL,
                S2_SAFE_COMBAT_SKILL,
                S2_SAFE_HITPOINTS
            ),
            41,
            "staged tuple posts exactly combat 41: no inflation"
        );
        assert_eq!(
            melee_combat_level(39, 40, 40, 20),
            40,
            "attack one below still posts 40"
        );
        assert_eq!(
            melee_combat_level(40, 39, 40, 20),
            40,
            "defence one below still posts 40"
        );
        assert_eq!(
            melee_combat_level(40, 40, 39, 20),
            40,
            "strength one below still posts 40"
        );
        assert_eq!(
            melee_combat_level(40, 40, 40, 19),
            40,
            "hitpoints one below still posts 40"
        );
        assert_eq!(
            melee_combat_level(1, 1, 1, 10),
            3,
            "minme fresh base posts combat 3"
        );
        assert_eq!(
            melee_combat_level(1, 40, 1, 10),
            13,
            "retired single-stat profile posted only 13"
        );
        for scenario in [
            quester_sheep_scenario(),
            quester_rune_mysteries_scenario(),
        ] {
            let start = start_index(&scenario);
            let stand = scenario
                .steps
                .iter()
                .position(|step| step.name == "stand at the quest start")
                .expect("quester fixture stands at the quest start");
            let relog = scenario
                .steps
                .iter()
                .rposition(|step| matches!(step.kind, StepKind::Relog))
                .expect("quester fixture relogs before Start");
            // One observed guard per tuple stat, all before the stand: the
            // guards, teleport, relog and Start run in that order.
            let expected = [
                (S2_ATTACK_STAT_ID, S2_SAFE_COMBAT_SKILL),
                (S2_DEFENCE_STAT_ID, S2_SAFE_COMBAT_SKILL),
                (S2_STRENGTH_STAT_ID, S2_SAFE_COMBAT_SKILL),
                (S2_HITPOINTS_STAT_ID, S2_SAFE_HITPOINTS),
            ];
            assert!(
                stand < relog && relog < start,
                "guards, teleport, relog, Start must run in that order"
            );
            for (id, min) in expected {
                let arm = Proof::Stat { id, min };
                assert!(
                    scenario.steps[..stand]
                        .iter()
                        .any(|step| step.wait.arm == arm),
                    "{} guards Start on every posted tuple stat",
                    scenario.name
                );
                assert!(
                    !arm.check(&fresh_base_snapshot(), None),
                    "{}: fresh base must not release the guard",
                    scenario.name
                );
                assert!(
                    arm.check(&staged_snapshot(), None),
                    "{}: staged tuple releases the guard",
                    scenario.name
                );
                assert!(
                    !arm.check(&staged_minus_one(id), None),
                    "{}: one level below staged must not release the guard",
                    scenario.name
                );
                for (other_id, other_min) in expected {
                    if other_id == id {
                        continue;
                    }
                    assert!(
                        Proof::Stat {
                            id: other_id,
                            min: other_min
                        }
                        .check(&staged_minus_one(id), None),
                        "{}: every other guard stays released at the boundary",
                        scenario.name
                    );
                }
            }
            // The fixture really emits reset-then-profile before any seed,
            // from the minme base with no maxme.
            let written = prestart_cheats(&scenario);
            let reset = written.find("minme").unwrap_or_else(|| {
                panic!("{} resets to the fresh base", scenario.name)
            });
            for cheat in [
                "setstat attack 40",
                "setstat defence 40",
                "setstat strength 40",
                "setstat hitpoints 20",
            ] {
                let staged = written.find(cheat).unwrap_or_else(|| {
                    panic!("{} stages {cheat}: {written}", scenario.name)
                });
                assert!(
                    reset < staged,
                    "{}: reset precedes the profile: {written}",
                    scenario.name
                );
            }
            assert_eq!(
                written.matches("setstat ").count(),
                4,
                "{}: exactly the tuple raises: {written}",
                scenario.name
            );
            assert!(
                !written.contains("maxme"),
                "{}: no maxme: {written}",
                scenario.name
            );
        }
    }

    #[test]
    fn rune_fixture_starts_fresh_at_duke_upstairs() {
        let scenario = quester_rune_mysteries_scenario();
        let written = prestart_cheats(&scenario);
        assert!(
            written.contains("setvar runemysteries 0"),
            "true Duke start seeds stage 0: {written}"
        );
        assert!(
            !written.contains("give "),
            "stage-0 start seeds no items — the quest grants them: {written}"
        );
        let stand = scenario
            .steps
            .iter()
            .find(|step| step.name == "stand at the quest start")
            .expect("quester fixture stands at the quest start");
        assert!(
            stand.wait.arm
                == Proof::Arrived {
                    x: 3212,
                    z: 3220,
                    level: 1
                },
            "Duke upstairs exact floor: the Path `start-duke` anchor"
        );
    }

    #[test]
    fn sheep_fixture_seeds_no_wool_and_keeps_shears_money() {
        let scenario = quester_sheep_scenario();
        let written = prestart_cheats(&scenario);
        assert!(
            written.contains("setvar sheep 0"),
            "sheep starts at stage 0: {written}"
        );
        assert!(
            written.contains("give coins"),
            "coins for shears survive: {written}"
        );
        assert!(
            !written.contains("wool"),
            "no wool seed — shear, spin and hand in from zero: {written}"
        );
        assert!(!written.contains("sword"), "no weapon seed: {written}");
    }

    #[test]
    fn quester_stage_clears_inventory_even_with_empty_items() {
        let empty = quester_stage(
            "probe_empty",
            "Probe",
            "probequest",
            0,
            &[],
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
        );
        let written = prestart_cheats(&empty);
        assert!(
            written.contains("~clearinv"),
            "empty-items fixture still clears the pack so reuse cannot carry quest items: {written}"
        );
        let seeded = quester_stage(
            "probe_seeded",
            "Probe",
            "probequest",
            0,
            &[("coins", 100)],
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
        );
        let written = prestart_cheats(&seeded);
        let clear = written
            .find("~clearinv")
            .expect("seeded fixture clears first: {written}");
        let give = written
            .find("give coins 100")
            .expect("seeded fixture seeds after clearing: {written}");
        assert!(clear < give, "clear precedes the seed: {written}");
    }

    #[test]
    fn romeo_fixture_stays_at_fresh_base() {
        assert_fresh_base_fixture(&quester_romeo_and_juliet_scenario(), "setvar rjquest 0");
    }

    /// Fresh-base profile: reset before seeding and never raise a stat.
    fn assert_fresh_base_fixture(scenario: &Scenario, setvar: &str) {
        let written = prestart_cheats(scenario);
        let reset = written
            .find("minme")
            .expect("fixture resets to the fresh base");
        assert!(
            !written.contains("setstat "),
            "{} raises no stat: {written}",
            scenario.name
        );
        let stage = written
            .find(setvar)
            .unwrap_or_else(|| panic!("stage seed kept: {written}"));
        assert!(reset < stage, "reset precedes the stage seed: {written}");
        for step in &scenario.steps[..start_index(scenario)] {
            if let Proof::Stat { id, min } = step.wait.arm {
                assert!(
                    id == 16 || min <= 1,
                    "{} stages no raised stat arm: {}",
                    scenario.name,
                    step.wait.arm.name()
                );
            }
        }
    }
}
