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
// Q2 stages skill gates plus passive-at combat levels ≤40. Sheep/Romeo have
// no gates or ambient hostiles (Tier A, passive 3), so reset without raises.
// Rune's Aubury Mugger is level 6: combat >2*6 prevents acquisition (Tier B,
// passive 13). After minme, Defence 40 alone posts combat 13; Defence 39 posts
// 12. The journal live fixture already qualifies this minimal single-stat
// profile, leaving Attack/Strength unchanged. Observe the posted stat before
// teleport/relog/Start; relog clears the stat-level-up UI.
const S2_DEFENCE_STAT_ID: i32 = 1;
const S2_RUNE_SAFE_DEFENCE: i32 = 40;
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
/// Rune Mysteries passive-at-13 profile: the single minimal raise plus the
/// observed guard that holds Start until the posted stat proves combat 13.
fn s2_rune_profile_step() -> Step {
    Step {
        name: "stage Mugger-safe Defence 40 and observe it before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, &format!("setstat defence {S2_RUNE_SAFE_DEFENCE}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: S2_DEFENCE_STAT_ID,
                min: S2_RUNE_SAFE_DEFENCE,
            },
            budget_ticks: 80,
        },
    }
}
/// Splice S2 safe-stat staging into a quest fixture ahead of its `setvar`
/// stage seed. `profile` is `Some` only for the quest whose passive level is
/// above the fresh base (Rune Mysteries). Cook fixtures are out of scope and
/// keep the shared `quester_stage` path untouched.
fn stage_s2_safe_stats(scenario: &mut Scenario, profile: Option<Step>) {
    let seed = scenario
        .steps
        .iter()
        .position(|step| step.name == "reset quest stage")
        .expect("quester fixture has a quest-stage seed step");
    let mut prep = vec![s2_reset_step()];
    if let Some(profile) = profile {
        prep.push(profile);
    }
    scenario.steps.splice(seed..seed, prep);
}
/// Generic builder: jump a quest to a stage key with optional items, then
/// Start Quester. `varp`/`value` seed via `setvar` (fixture only).
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
    if !items.is_empty() {
        steps.push(Step {
            name: "seed items",
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
    }
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
        &[("bronze_sword", 1), ("coins", 100), ("wool", 19)],
        WorldTile {
            x: 3197,
            z: 3266,
            level: 0,
        },
    );
    scenario.settings.script_settings_inject = Some(SHEEP_SETTINGS);
    // Tier A passive at 3: fresh base is the safe profile — reset, no raise.
    stage_s2_safe_stats(&mut scenario, None);
    scenario
}

/// Exercise the package-to-notes half of Rune Mysteries without depending on
/// the castle's unsupported exterior-to-upper-floor navigation.
pub(crate) fn quester_rune_mysteries_scenario() -> Scenario {
    let mut scenario = quester_stage(
        "quester_rune_mysteries",
        "Rune Mysteries Quest",
        "runemysteries",
        3,
        &[("research_package", 1)],
        WorldTile {
            x: 3253,
            z: 3402,
            level: 0,
        },
    );
    scenario.settings.script_settings_inject = Some(RUNE_MYSTERIES_SETTINGS);
    // Tier B passive at 13: reset, then the minimal Defence-40 profile whose
    // observed guard holds the Aubury teleport/relog/Start until combat 13.
    stage_s2_safe_stats(&mut scenario, Some(s2_rune_profile_step()));
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
    // Tier A passive at 3: fresh base is the safe profile — reset, no raise.
    stage_s2_safe_stats(&mut scenario, None);
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

    const DEFENCE_STAT_ID: i32 = 1;
    const RUNE_SAFE_DEFENCE: i32 = 40;

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

    /// Snapshot with the given posted effective Defence (fresh base 1 after
    /// `minme`, staged profile 40 after `setstat defence 40`).
    fn posted_defence_snapshot(defence_effective: i32) -> GameSnapshot {
        let mut client = fixture_client();
        client.stat_effective_level[DEFENCE_STAT_ID as usize] = defence_effective;
        client.bump_gens(ServerProt::UPDATE_STAT);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&mut client);
        snapshot
    }

    #[test]
    fn rune_fixture_holds_start_until_posted_defence_proves_combat_13() {
        let scenario = quester_rune_mysteries_scenario();
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
        // Observed guard: a step before the Aubury teleport whose arm watches
        // posted Defence 40 — combat 13 = 2×Mugger(6)+1, past the >2× rule.
        let guard = scenario.steps[..stand]
            .iter()
            .find(|step| {
                step.wait.arm
                    == Proof::Stat {
                        id: DEFENCE_STAT_ID,
                        min: RUNE_SAFE_DEFENCE,
                    }
            })
            .expect("rune fixture guards Start on posted Defence 40");
        assert!(
            stand < relog && relog < start,
            "guard, teleport, relog, Start must run in that order"
        );
        // The same arm against posted stats: the fresh base cannot release
        // Start, the staged profile can, and the 39 boundary still cannot
        // (Defence 39 posts combat 12 — 40 is minimal).
        assert!(
            !guard.wait.arm.check(&posted_defence_snapshot(1), None),
            "fresh-base Defence 1 must not release the guard: the bot cannot Start below combat 13"
        );
        assert!(
            !guard.wait.arm.check(&posted_defence_snapshot(39), None),
            "Defence 39 posts combat 12 and must not release the guard"
        );
        assert!(
            guard
                .wait
                .arm
                .check(&posted_defence_snapshot(RUNE_SAFE_DEFENCE), None),
            "posted Defence 40 releases the guard (journal fixture observes combat > 12 there)"
        );
        // The fixture really emits reset-then-profile before any seed.
        let written = prestart_cheats(&scenario);
        let reset = written
            .find("minme")
            .expect("rune fixture resets to the fresh base");
        let profile = written
            .find("setstat defence 40")
            .expect("rune fixture stages the minimal Defence profile");
        assert!(reset < profile, "reset precedes the profile: {written}");
        assert_eq!(
            written.matches("setstat ").count(),
            1,
            "exactly one stat raise: {written}"
        );
    }

    #[test]
    fn sheep_and_romeo_fixtures_reset_and_raise_nothing() {
        assert_fresh_base_fixture(&quester_sheep_scenario(), "setvar sheep 0");
        assert_fresh_base_fixture(&quester_romeo_and_juliet_scenario(), "setvar rjquest 0");
    }

    /// Tier-A profile: reset before seeding and never raise a stat.
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
