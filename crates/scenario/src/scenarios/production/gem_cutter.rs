use super::*;
pub(crate) const UNCUT_SAPPHIRE_ID: i32 = 1623;
pub(crate) const SAPPHIRE_ID: i32 = 1607;
pub(crate) const UNCUT_OPAL_ID: i32 = 1625;
pub(crate) const CHISEL_ID: i32 = 1755;
pub(crate) const CRUSHED_GEMSTONE_ID: i32 = 1633;
const GEM_CUTTER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "gems",
    value: ScriptInjectValue::StrList(&[]),
}];

const GEM_CUTTER_NAMED_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "gems",
    value: ScriptInjectValue::StrList(&["Sapphire"]),
}];
pub(crate) fn gem_cutter_scenario() -> Scenario {
    gem_cutter_variant("gem_cutter", GEM_CUTTER_INJECT, false)
}

pub(crate) fn gem_cutter_named_scenario() -> Scenario {
    gem_cutter_variant("gem_cutter_named", GEM_CUTTER_NAMED_INJECT, true)
}

/// Empty pack, banked chisel plus uncut sapphires. Named also banks uncut
/// opal that must stay put. Deposit keeps the chisel.
fn gem_cutter_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    named: bool,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 2,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Crafting and uncut sapphire bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 20");
                cheat(c, "givebank chisel 1");
                cheat(c, "givebank uncut_sapphire 28");
                if named {
                    cheat(c, "givebank uncut_opal 4");
                }
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
            "confirm Crafting 20 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 20,
            },
        ),
        (
            "confirm no seeded chisel in pack before Start",
            Proof::ItemIdAtMost {
                id: CHISEL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded uncut sapphire in pack before Start",
            Proof::ItemIdAtMost {
                id: UNCUT_SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded cut sapphire before Start",
            Proof::ItemIdAtMost {
                id: SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "confirm no crushed gemstone before Start",
            Proof::ItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded uncut opal in pack before Start",
            Proof::ItemIdAtMost {
                id: UNCUT_OPAL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact uncut sapphire seed bank",
        Proof::BankItemId {
            id: UNCUT_SAPPHIRE_ID,
            count: 28,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact chisel seed bank",
            Proof::BankItemId {
                id: CHISEL_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded cut sapphire in bank",
            Proof::BankItemIdAtMost {
                id: SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no crushed gemstone in bank",
            Proof::BankItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    if named {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact untouched uncut opal seed bank",
            Proof::BankItemId {
                id: UNCUT_OPAL_ID,
                count: 4,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        ("watch Crafting XP from cutting sapphire", first_xp),
        (
            "watch a chisel-kept pack of exact cut sapphires",
            Proof::ItemId {
                id: SAPPHIRE_ID,
                count: 27,
            },
        ),
        (
            "watch the chisel remain in pack",
            Proof::ItemId {
                id: CHISEL_ID,
                count: 1,
            },
        ),
        (
            "watch exact uncut sapphires consumed",
            Proof::ItemIdAtMost {
                id: UNCUT_SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "watch no crushed gemstone from sapphire",
            Proof::ItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
        (
            "watch the script-created sapphires enter a fresh bank",
            Proof::BankItemId {
                id: SAPPHIRE_ID,
                count: 27,
            },
        ),
        (
            "watch a restock of exact uncut sapphire",
            Proof::ItemId {
                id: UNCUT_SAPPHIRE_ID,
                count: 1,
            },
        ),
        (
            "watch the chisel stay out of the deposit",
            Proof::ItemId {
                id: CHISEL_ID,
                count: 1,
            },
        ),
        (
            "watch exact uncut sapphire bank stock decrease",
            Proof::BankItemIdAtMost {
                id: UNCUT_SAPPHIRE_ID,
                count: 1,
            },
        ),
    ];
    if named {
        watch.push((
            "watch the filtered uncut opal stay in bank",
            Proof::BankItemId {
                id: UNCUT_OPAL_ID,
                count: 4,
            },
        ));
        watch.push((
            "watch no filtered uncut opal enter the pack",
            Proof::ItemIdAtMost {
                id: UNCUT_OPAL_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        ("watch the script close its gem bank", Proof::BankClosed),
        (
            "watch another exact cut sapphire after restock",
            Proof::ItemId {
                id: SAPPHIRE_ID,
                count: 1,
            },
        ),
        ("watch Crafting XP beyond the first pack", further_xp),
        (
            "watch crushed gemstone stay empty",
            Proof::ItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
    ]);
    for (step_name, arm) in watch {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("GemCutter"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
