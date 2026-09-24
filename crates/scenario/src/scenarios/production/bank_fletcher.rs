use super::*;
const BANK_FLETCHER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Willow logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Short bow"),
    },
];

const BANK_FLETCHER_SHAFTS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Arrow shafts"),
    },
];

const BANK_FLETCHER_HEADLESS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Headless arrows"),
    },
];

const BANK_FLETCHER_STRING_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Willow logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("String short bow"),
    },
];

const BANK_FLETCHER_CUT_STRING_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("cut+string"),
    },
    ScriptSettingInject {
        id: "material",
        value: ScriptInjectValue::Str("Willow logs"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Short bow"),
    },
];
const WILLOW_LOGS_ID: i32 = 1519;
const ARROW_SHAFT_ID: i32 = 52;
const HEADLESS_ARROW_ID: i32 = 53;
const UNSTRUNG_WILLOW_SHORTBOW_ID: i32 = 60;
const STRUNG_WILLOW_SHORTBOW_ID: i32 = 849;
/// Run BankFletcher through a full pack, product deposit, log withdrawal and
/// another product. All bank stock is prepared before the script starts.
pub(crate) fn bank_fletcher_scenario() -> Scenario {
    let xp = Proof::StatXpGain { id: 9, min: 1 };
    // Selected content awards 33.3 XP per willow shortbow: the 27 carried logs
    // alone can produce at most 899 integer XP. This requires a banked log.
    let second_batch_xp = Proof::StatXpGain { id: 9, min: 900 };
    let bank = VARROCK_WEST_BANK;
    Scenario {
        name: "bank_fletcher",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: {
            let mut steps = script_live_seed_steps();
            steps.push(Step {
                name: "seed knife, logs, and tele to Varrock West",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, "advancestat fletching 35");
                        cheat(c, "give knife 1");
                        cheat(c, "give willow_logs 27");
                        cheat(c, "givebank willow_logs 54");
                        cheat(c, &tele_args(bank.level, bank.x, bank.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Item {
                        name: "Knife",
                        count: 1,
                    },
                    budget_ticks: 120,
                },
            });
            steps.push(drain_advancestat());
            steps.push(start_catalog_step());
            steps.push(Step {
                name: "watch the script fletch willow logs",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: xp,
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            });
            for (name, arm) in [
                (
                    "watch the script deposit its first pack of bows",
                    Proof::BankItem {
                        name: "Willow shortbow",
                        count: 27,
                    },
                ),
                (
                    "watch the script withdraw its next pack of logs",
                    Proof::Item {
                        name: "Willow logs",
                        count: 27,
                    },
                ),
                (
                    "watch the banked logs decrease",
                    Proof::BankItemAtMost {
                        name: "Willow logs",
                        count: 27,
                    },
                ),
                ("watch the script close its bank", Proof::BankClosed),
                ("watch the script fletch a withdrawn log", second_batch_xp),
            ] {
                steps.push(Step {
                    name,
                    kind: StepKind::Perform {
                        send: Box::new(|_, _| true),
                    },
                    wait: Wait {
                        arm,
                        budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                    },
                });
            }
            steps
        },
        proof: second_batch_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BankFletcher"),
            script_settings_inject: Some(BANK_FLETCHER_INJECT),
            terminal_shot: Some("bank_fletcher terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[derive(Clone, Copy)]
struct BankFletcherOption {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    primary_alias: &'static str,
    primary_id: i32,
    primary_carried: i32,
    primary_banked: i32,
    primary_restocked: i32,
    secondary: Option<(&'static str, i32, i32, i32, i32)>,
    product_id: i32,
    first_product_count: i32,
    keep_knife: bool,
}

fn bank_fletcher_option_scenario(option: BankFletcherOption) -> Scenario {
    let BankFletcherOption {
        name,
        inject,
        primary_alias,
        primary_id,
        primary_carried,
        primary_banked,
        primary_restocked,
        secondary,
        product_id,
        first_product_count,
        keep_knife,
    } = option;
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed exact BankFletcher option inputs and bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat fletching 1");
                if keep_knife {
                    cheat(c, "give knife 1");
                }
                cheat(c, &format!("give {primary_alias} {primary_carried}"));
                cheat(c, &format!("givebank {primary_alias} {primary_banked}"));
                if let Some((alias, _, carried, banked, _)) = secondary {
                    cheat(c, &format!("give {alias} {carried}"));
                    cheat(c, &format!("givebank {alias} {banked}"));
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
    steps.push(bank_fletcher_watch(
        "confirm Fletching 1 before Start",
        Proof::Stat { id: 9, min: 1 },
    ));
    if keep_knife {
        steps.push(bank_fletcher_watch(
            "confirm the knife before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: 1,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "confirm the exact primary input before Start",
        Proof::ItemId {
            id: primary_id,
            count: primary_carried,
        },
    ));
    if let Some((_, id, carried, _, _)) = secondary {
        steps.push(bank_fletcher_watch(
            "confirm the exact secondary input before Start",
            Proof::ItemId { id, count: carried },
        ));
    }
    steps.push(bank_fletcher_watch(
        "confirm no seeded product in pack before Start",
        Proof::ItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact primary seed bank",
        Proof::BankItemId {
            id: primary_id,
            count: primary_banked,
        },
    ));
    if let Some((_, id, _, banked, _)) = secondary {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact secondary seed bank",
            Proof::BankItemId { id, count: banked },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded product in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Fletching XP from the first exact input batch",
            Proof::StatXpGain { id: 9, min: 1 },
        ),
        (
            "watch the exact first product batch",
            Proof::ItemId {
                id: product_id,
                count: first_product_count,
            },
        ),
        (
            "watch the primary input consumed",
            Proof::ItemIdAtMost {
                id: primary_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    if let Some((_, id, _, _, _)) = secondary {
        steps.push(bank_fletcher_watch(
            "watch the secondary input consumed",
            Proof::ItemIdAtMost { id, count: 0 },
        ));
    }
    steps.push(bank_fletcher_watch(
        "watch the exact product batch enter a fresh bank",
        Proof::BankItemId {
            id: product_id,
            count: first_product_count,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch the product leave the pack after deposit",
        Proof::ItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch fresh primary input restocked",
        Proof::ItemId {
            id: primary_id,
            count: primary_restocked,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch primary bank stock decrease on restock",
        Proof::BankItemIdAtMost {
            id: primary_id,
            count: primary_banked - primary_restocked,
        },
    ));
    if let Some((_, id, _, banked, restocked)) = secondary {
        steps.push(bank_fletcher_watch(
            "watch fresh secondary input restocked",
            Proof::ItemId {
                id,
                count: restocked,
            },
        ));
        steps.push(bank_fletcher_watch(
            "watch secondary bank stock decrease on restock",
            Proof::BankItemIdAtMost {
                id,
                count: banked - restocked,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "watch the script close its production bank",
        Proof::BankClosed,
    ));
    steps.push(bank_fletcher_watch(
        "watch fresh Fletching XP after the restock and closed return",
        Proof::FreshStatXpGain { id: 9, min: 1 },
    ));
    steps.push(bank_fletcher_watch(
        "watch further exact product after restock",
        Proof::ItemId {
            id: product_id,
            count: 1,
        },
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: product_id,
            count: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BankFletcher"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn bank_fletcher_shafts_scenario() -> Scenario {
    bank_fletcher_option_scenario(BankFletcherOption {
        name: "bank_fletcher_shafts",
        inject: BANK_FLETCHER_SHAFTS_INJECT,
        primary_alias: "logs",
        primary_id: LOGS_ID,
        primary_carried: 27,
        primary_banked: 54,
        primary_restocked: 27,
        secondary: None,
        product_id: ARROW_SHAFT_ID,
        first_product_count: 405,
        keep_knife: true,
    })
}

pub(crate) fn bank_fletcher_headless_scenario() -> Scenario {
    bank_fletcher_option_scenario(BankFletcherOption {
        name: "bank_fletcher_headless",
        inject: BANK_FLETCHER_HEADLESS_INJECT,
        primary_alias: "feather",
        primary_id: FEATHER_ID,
        primary_carried: 30,
        primary_banked: 60,
        primary_restocked: 60,
        secondary: Some(("arrow_shaft", ARROW_SHAFT_ID, 30, 60, 60)),
        product_id: HEADLESS_ARROW_ID,
        first_product_count: 30,
        keep_knife: false,
    })
}

/// String two carried pairs, bank the exact id-849 products, withdraw a fresh
/// unstacked 14+14 load, and string beyond the seeded pair. The old catalog has
/// no `mode`, so product selection is the shared old/new stringing contract.
pub(crate) fn bank_fletcher_string_scenario() -> Scenario {
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed exact willow stringing pairs and bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "advancestat fletching 35");
                cheat(c, "give unstrung_willow_shortbow 2");
                cheat(c, "give bow_string 2");
                cheat(c, "givebank unstrung_willow_shortbow 28");
                cheat(c, "givebank bow_string 28");
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
    for (name, arm) in [
        (
            "confirm Fletching 35 before Start",
            Proof::Stat { id: 9, min: 35 },
        ),
        (
            "confirm two exact unstrung willow shortbows before Start",
            Proof::ItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "confirm two exact bow strings before Start",
            Proof::ItemId {
                id: BOW_STRING_ID,
                count: 2,
            },
        ),
        (
            "confirm no seeded strung willow shortbow before Start",
            Proof::ItemIdAtMost {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact unstrung seed bank",
        Proof::BankItemId {
            id: UNSTRUNG_WILLOW_SHORTBOW_ID,
            count: 28,
        },
    ));
    for (name, arm) in [
        (
            "acknowledge the exact bow-string seed bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 28,
            },
        ),
        (
            "acknowledge no seeded strung bow in bank",
            Proof::BankItemIdAtMost {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch XP from both seeded stringing pairs",
            Proof::StatXpGain { id: 9, min: 66 },
        ),
        (
            "watch both seeded pairs become exact strung willow shortbows",
            Proof::ItemId {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch both seeded unstrung bows consumed",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "watch both seeded bow strings consumed",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "watch the exact strung pair enter a fresh bank",
            Proof::BankItemId {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch a fresh fourteen-bow withdrawal",
            Proof::ItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 14,
            },
        ),
        (
            "watch a fresh fourteen-string withdrawal",
            Proof::ItemId {
                id: BOW_STRING_ID,
                count: 14,
            },
        ),
        (
            "watch exact unstrung bank stock decrease",
            Proof::BankItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 14,
            },
        ),
        (
            "watch exact bow-string bank stock decrease",
            Proof::BankItemIdAtMost {
                id: BOW_STRING_ID,
                count: 14,
            },
        ),
        (
            "watch the script close its stringing bank",
            Proof::BankClosed,
        ),
        (
            "watch a newly withdrawn bow become exact id 849",
            Proof::ItemId {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 1,
            },
        ),
        (
            "watch stringing XP beyond the two seeded pairs",
            Proof::StatXpGain { id: 9, min: 67 },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "bank_fletcher_string",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::StatXpGain { id: 9, min: 67 },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BankFletcher"),
            script_settings_inject: Some(BANK_FLETCHER_STRING_INJECT),
            terminal_shot: Some("bank_fletcher_string terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// New-catalog combined mode: cut only the two seeded logs, observe their exact
/// id-60 products in bank, then withdraw those products with strings and finish
/// both as id 849. No unstrung or strung outcome is seeded.
pub(crate) fn bank_fletcher_cut_string_scenario() -> Scenario {
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed two willow logs and only banked strings before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "advancestat fletching 35");
                cheat(c, "give knife 1");
                cheat(c, "give willow_logs 2");
                cheat(c, "givebank bow_string 28");
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
    for (name, arm) in [
        (
            "confirm Fletching 35 before Start",
            Proof::Stat { id: 9, min: 35 },
        ),
        (
            "confirm the knife before Start",
            Proof::Item {
                name: "Knife",
                count: 1,
            },
        ),
        (
            "confirm two exact willow logs before Start",
            Proof::ItemId {
                id: WILLOW_LOGS_ID,
                count: 2,
            },
        ),
        (
            "confirm no seeded unstrung willow shortbow before Start",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded strung willow shortbow before Start",
            Proof::ItemIdAtMost {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact bow-string seed bank",
        Proof::BankItemId {
            id: BOW_STRING_ID,
            count: 28,
        },
    ));
    for (name, arm) in [
        (
            "acknowledge no extra willow logs in bank",
            Proof::BankItemIdAtMost {
                id: WILLOW_LOGS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded unstrung bow in bank",
            Proof::BankItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded strung bow in bank",
            Proof::BankItemIdAtMost {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch cut XP for both seeded logs",
            Proof::StatXpGain { id: 9, min: 66 },
        ),
        (
            "watch both willow logs become exact unstrung bows",
            Proof::ItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch both willow logs consumed in the cut phase",
            Proof::ItemIdAtMost {
                id: WILLOW_LOGS_ID,
                count: 0,
            },
        ),
        (
            "watch the script-created unstrung pair enter a fresh bank",
            Proof::BankItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch the same unstrung pair leave that bank",
            Proof::ItemId {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch the combined string load arrive",
            Proof::ItemId {
                id: BOW_STRING_ID,
                count: 14,
            },
        ),
        (
            "watch exact unstrung bank stock empty",
            Proof::BankItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "watch exact bow-string bank stock decrease",
            Proof::BankItemIdAtMost {
                id: BOW_STRING_ID,
                count: 14,
            },
        ),
        (
            "watch the combined script close its bank",
            Proof::BankClosed,
        ),
        (
            "watch XP advance from cutting into stringing",
            Proof::StatXpGain { id: 9, min: 100 },
        ),
        (
            "watch both script-created bows become exact id 849",
            Proof::ItemId {
                id: STRUNG_WILLOW_SHORTBOW_ID,
                count: 2,
            },
        ),
        (
            "watch both script-created id-60 bows consumed",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_WILLOW_SHORTBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "bank_fletcher_cut_string",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: STRUNG_WILLOW_SHORTBOW_ID,
            count: 2,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BankFletcher"),
            script_settings_inject: Some(BANK_FLETCHER_CUT_STRING_INJECT),
            terminal_shot: Some("bank_fletcher_cut_string terminal"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
