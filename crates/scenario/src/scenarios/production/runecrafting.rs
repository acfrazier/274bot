use super::*;
pub(crate) const RUNECRAFT_STAT: i32 = 20;
pub(crate) const RUNE_ESSENCE_ID: i32 = 1436;
pub(crate) const NOTED_ESSENCE_ID: i32 = 1437;
pub(crate) const AIR_TALISMAN_ID: i32 = 1438;
const EARTH_TALISMAN_ID: i32 = 1440;
pub(crate) const EARTH_RUNE_ID: i32 = 557;
pub(crate) const RUNE_ESSENCE_SEED: i32 = 200;
const RUNECRAFTER_AIR_RUINS: WorldTile = WorldTile {
    x: 2988,
    z: 3294,
    level: 0,
};
const RUNECRAFTER_EARTH_RUINS: WorldTile = WorldTile {
    x: 3303,
    z: 3477,
    level: 0,
};
pub(crate) const MULECRAFTER_AIR_RUINS: WorldTile = WorldTile {
    x: 2983,
    z: 3288,
    level: 0,
};
const RUNE_CRAFTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "rune",
        value: ScriptInjectValue::Str("Air runes"),
    },
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("Solo"),
    },
];

const RUNE_CRAFTER_EARTH_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "rune",
        value: ScriptInjectValue::Str("Earth runes"),
    },
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("Solo"),
    },
];

const MULE_CRAFTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "rune",
        value: ScriptInjectValue::Str("Air rune"),
    },
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("Crafter"),
    },
    ScriptSettingInject {
        id: "partner",
        value: ScriptInjectValue::Str(""),
    },
    ScriptSettingInject {
        id: "bankFill",
        value: ScriptInjectValue::Bool(true),
    },
];
pub(crate) fn rune_crafter_scenario() -> Scenario {
    rune_craft_variant(RuneCraftSpec {
        name: "rune_crafter",
        start_script: "RuneCrafter",
        inject: RUNE_CRAFTER_INJECT,
        bank: FALADOR_EAST_BANK,
        ruins: RUNECRAFTER_AIR_RUINS,
        rc_level: 1,
        talisman_alias: "air_talisman",
        talisman_id: AIR_TALISMAN_ID,
        rune_id: AIR_RUNE_ID,
        wrong_rune_id: EARTH_RUNE_ID,
    })
}

pub(crate) fn rune_crafter_earth_scenario() -> Scenario {
    rune_craft_variant(RuneCraftSpec {
        name: "rune_crafter_earth",
        start_script: "RuneCrafter",
        inject: RUNE_CRAFTER_EARTH_INJECT,
        bank: VARROCK_EAST_BANK,
        ruins: RUNECRAFTER_EARTH_RUINS,
        rc_level: 9,
        talisman_alias: "earth_talisman",
        talisman_id: EARTH_TALISMAN_ID,
        rune_id: EARTH_RUNE_ID,
        wrong_rune_id: AIR_RUNE_ID,
    })
}

pub(crate) fn mule_crafter_scenario() -> Scenario {
    rune_craft_variant(RuneCraftSpec {
        name: "mule_crafter",
        start_script: "MuleCrafter",
        inject: MULE_CRAFTER_INJECT,
        bank: FALADOR_EAST_BANK,
        ruins: MULECRAFTER_AIR_RUINS,
        rc_level: 1,
        talisman_alias: "air_talisman",
        talisman_id: AIR_TALISMAN_ID,
        rune_id: AIR_RUNE_ID,
        wrong_rune_id: EARTH_RUNE_ID,
    })
}

fn runecraft_open_seed_bank(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).open_nearest_booth(),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

/// Empty pack at the selected bank. Banked unnoted essence 1436 and the
/// selected talisman; never crafted runes or noted 1437. Script withdraws,
/// uses the talisman on the selected Mysterious ruins, Craft-rune, banks the
/// produced runes, restocks essence and crafts again. Trade/paired modes stay
/// pending.
struct RuneCraftSpec {
    name: &'static str,
    start_script: &'static str,
    inject: &'static [ScriptSettingInject],
    bank: WorldTile,
    ruins: WorldTile,
    rc_level: i32,
    talisman_alias: &'static str,
    talisman_id: i32,
    rune_id: i32,
    wrong_rune_id: i32,
}

fn rune_craft_variant(spec: RuneCraftSpec) -> Scenario {
    let RuneCraftSpec {
        name,
        start_script,
        inject,
        bank,
        ruins,
        rc_level,
        talisman_alias,
        talisman_id,
        rune_id,
        wrong_rune_id,
    } = spec;
    let ruins_near = Proof::ArrivedNear {
        x: ruins.x,
        z: ruins.z,
        level: ruins.level,
        radius: 4,
    };
    let crafted = Proof::ItemId {
        id: rune_id,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed runecraft, banked essence, talisman, and tele before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat runecraft {rc_level}"));
                cheat(c, &format!("givebank blankrune {RUNE_ESSENCE_SEED}"));
                cheat(c, &format!("givebank {talisman_alias} 1"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm runecraft level before Start",
            Proof::Stat {
                id: RUNECRAFT_STAT,
                min: rc_level,
            },
        ),
        (
            "confirm no seeded essence in pack before Start",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted essence in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded selected runes in pack before Start",
            Proof::ItemIdAtMost {
                id: rune_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong runes in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_rune_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded talisman in pack before Start",
            Proof::ItemIdAtMost {
                id: talisman_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(runecraft_open_seed_bank(
        "open and acknowledge the exact unnoted essence seed bank",
        Proof::BankItemId {
            id: RUNE_ESSENCE_ID,
            count: RUNE_ESSENCE_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact talisman seed bank",
        Proof::BankItemId {
            id: talisman_id,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted essence in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_ESSENCE_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded selected runes in bank",
        Proof::BankItemIdAtMost {
            id: rune_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded wrong runes in bank",
        Proof::BankItemIdAtMost {
            id: wrong_rune_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch script withdrawal of unnoted essence after Start",
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: 1,
            },
        ),
        (
            "watch arrival at the selected mysterious ruins after Start",
            ruins_near,
        ),
        (
            "watch Runecraft XP from the selected craft",
            Proof::StatXpGain {
                id: RUNECRAFT_STAT,
                min: 1,
            },
        ),
        (
            "watch essence become the selected rune after altar entry",
            crafted,
        ),
        (
            "watch the withdrawn essence finish converting",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: 0,
            },
        ),
        ("watch portal exit back to the selected ruins", ruins_near),
        (
            "watch script-created runes enter a fresh bank",
            Proof::BankItemId {
                id: rune_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of runes after deposit",
            Proof::ItemIdAtMost {
                id: rune_id,
                count: 0,
            },
        ),
        (
            "watch a restock of unnoted essence",
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: 1,
            },
        ),
        (
            "watch the script close its runecraft bank",
            Proof::BankClosed,
        ),
        (
            "watch return to the selected ruins after restock",
            ruins_near,
        ),
        ("watch another exact selected rune after restock", crafted),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: crafted,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(start_script),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
