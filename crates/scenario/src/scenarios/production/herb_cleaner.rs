use super::*;
pub(crate) const UNIDENTIFIED_MARENTILL_ID: i32 = 201;
const MARRENTILL_ID: i32 = 251;
const HERB_CLEANER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "herbs",
    value: ScriptInjectValue::StrList(&[]),
}];

const HERB_CLEANER_NAMED_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "herbs",
    value: ScriptInjectValue::StrList(&["Guam leaf"]),
}];

const HERB_CLEANER_EMPTY_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "herbs",
    value: ScriptInjectValue::StrList(&["Guam leaf", "Marrentill"]),
}];
pub(crate) fn herb_cleaner_scenario() -> Scenario {
    herb_cleaner_variant("herb_cleaner", HERB_CLEANER_INJECT, 3, false)
}

pub(crate) fn herb_cleaner_named_scenario() -> Scenario {
    herb_cleaner_variant("herb_cleaner_named", HERB_CLEANER_NAMED_INJECT, 5, true)
}

/// Frozen reference fixture: one sub-full pack of guam, Marrentill selected
/// but absent, then the script's own eventual empty-bank Stop.
pub(crate) fn herb_cleaner_empty_bank_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Herblore 20 and exactly 20 unidentified guam before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat herblore 20");
                cheat(c, "givebank unidentified_guam 20");
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
    steps.push(drain_advancestat());
    for (step_name, arm) in [
        (
            "confirm Herblore 20 before Start",
            Proof::Stat {
                id: HERBLORE_STAT,
                min: 20,
            },
        ),
        (
            "confirm no unidentified guam in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 0,
            },
        ),
        (
            "confirm no clean guam in pack before Start",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ),
        (
            "confirm no unidentified marrentill in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 0,
            },
        ),
        (
            "confirm no clean marrentill in pack before Start",
            Proof::ItemIdAtMost {
                id: MARRENTILL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge exactly 20 unidentified guam",
        Proof::BankItemId {
            id: UNIDENTIFIED_GUAM_ID,
            count: 20,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge Marrentill is absent from the loaded seed bank",
        Proof::BankItemIdAtMost {
            id: UNIDENTIFIED_MARENTILL_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch script-created clean guam without requiring a full pack",
        Proof::ItemId {
            id: GUAM_LEAF_ID,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch Herblore XP from post-Start guam cleaning",
        xp,
    ));
    Scenario {
        name: "herb_cleaner_empty_bank",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(420),
            start_script: Some("HerbCleaner"),
            script_settings_inject: Some(HERB_CLEANER_EMPTY_INJECT),
            terminal_shot: Some("herb_cleaner_empty_bank"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Empty pack, banked unidentified guam. Named also banks marrentill unids
/// that must stay put. Identify fills the pack, then deposit-all restocks.
fn herb_cleaner_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    level: i32,
    named: bool,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 2,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Herblore and unidentified bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat herblore {level}"));
                cheat(c, "givebank unidentified_guam 30");
                if named {
                    cheat(c, "givebank unidentified_marentill 4");
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
            "confirm Herblore before Start",
            Proof::Stat {
                id: HERBLORE_STAT,
                min: level,
            },
        ),
        (
            "confirm no seeded unidentified guam in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded clean guam before Start",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unidentified marrentill in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded clean marrentill before Start",
            Proof::ItemIdAtMost {
                id: MARRENTILL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact unidentified guam seed bank",
        Proof::BankItemId {
            id: UNIDENTIFIED_GUAM_ID,
            count: 30,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded clean guam in bank",
        Proof::BankItemIdAtMost {
            id: GUAM_LEAF_ID,
            count: 0,
        },
    ));
    if named {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact untouched marrentill seed bank",
            Proof::BankItemId {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 4,
            },
        ));
        steps.push(bank_fletcher_watch(
            "acknowledge no seeded clean marrentill in bank",
            Proof::BankItemIdAtMost {
                id: MARRENTILL_ID,
                count: 0,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        ("watch Herblore XP from identifying guam", first_xp),
        (
            "watch a full pack of exact clean guam before the bank cycle",
            Proof::ItemId {
                id: GUAM_LEAF_ID,
                count: 28,
            },
        ),
        (
            "watch exact unidentified guam consumed",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 0,
            },
        ),
        (
            "watch the script-created clean guam enter a fresh bank",
            Proof::BankItemId {
                id: GUAM_LEAF_ID,
                count: 28,
            },
        ),
        (
            "watch a restock of exact unidentified guam",
            Proof::ItemId {
                id: UNIDENTIFIED_GUAM_ID,
                count: 1,
            },
        ),
        (
            "watch exact unidentified guam bank stock decrease",
            Proof::BankItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 2,
            },
        ),
    ];
    if named {
        watch.push((
            "watch the filtered marrentill unids stay in bank",
            Proof::BankItemId {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 4,
            },
        ));
        watch.push((
            "watch no filtered marrentill enter the pack",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        ("watch the script close its herb bank", Proof::BankClosed),
        (
            "watch another exact clean guam after restock",
            Proof::ItemId {
                id: GUAM_LEAF_ID,
                count: 1,
            },
        ),
        ("watch Herblore XP beyond the first pack", further_xp),
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
            start_script: Some("HerbCleaner"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
