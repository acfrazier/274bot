use super::*;
pub(crate) const EYE_OF_NEWT_ID: i32 = 221;
pub(crate) const GUAM_UNF_ID: i32 = 91;
pub(crate) const ATTACK_POTION_3_ID: i32 = 121;
pub(crate) const RANARR_WEED_ID: i32 = 257;
pub(crate) const RANARR_UNF_ID: i32 = 99;
pub(crate) const SNAPE_GRASS_ID: i32 = 231;
pub(crate) const PRAYER_POTION_3_ID: i32 = 139;
pub(crate) const POTION_BATCH_SEED: i32 = 42;
pub(crate) const GUAM_HERBLORE: i32 = 3;
/// Source has no herblore field. Seed high enough that Ranarr mixing is
/// not refused; this is not a generated-data requirement.
pub(crate) const RANARR_HERBLORE: i32 = 38;
const POTION_MAKER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "herb",
        value: ScriptInjectValue::Str("Custom"),
    },
    ScriptSettingInject {
        id: "herbCustom",
        value: ScriptInjectValue::Str("Guam leaf"),
    },
    ScriptSettingInject {
        id: "secondary",
        value: ScriptInjectValue::Str("Custom"),
    },
    ScriptSettingInject {
        id: "secondaryCustom",
        value: ScriptInjectValue::Str("Eye of newt"),
    },
];

const POTION_MAKER_NAMED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "herb",
        value: ScriptInjectValue::Str("Ranarr weed"),
    },
    ScriptSettingInject {
        id: "secondary",
        value: ScriptInjectValue::Str("Snape grass"),
    },
];
pub(crate) fn potion_maker_scenario() -> Scenario {
    potion_maker_variant(
        "potion_maker",
        POTION_MAKER_INJECT,
        PotionMakerRecipe {
            herb_id: GUAM_LEAF_ID,
            unf_id: GUAM_UNF_ID,
            secondary_id: EYE_OF_NEWT_ID,
            finished_id: ATTACK_POTION_3_ID,
            wrong_unf_id: RANARR_UNF_ID,
            wrong_finished_id: PRAYER_POTION_3_ID,
            herb_alias: "guam_leaf",
            secondary_alias: "eye_of_newt",
            herblore: GUAM_HERBLORE,
            named: false,
        },
    )
}

pub(crate) fn potion_maker_named_scenario() -> Scenario {
    potion_maker_variant(
        "potion_maker_named",
        POTION_MAKER_NAMED_INJECT,
        PotionMakerRecipe {
            herb_id: RANARR_WEED_ID,
            unf_id: RANARR_UNF_ID,
            secondary_id: SNAPE_GRASS_ID,
            finished_id: PRAYER_POTION_3_ID,
            wrong_unf_id: GUAM_UNF_ID,
            wrong_finished_id: ATTACK_POTION_3_ID,
            herb_alias: "ranarr_weed",
            secondary_alias: "snape_grass",
            herblore: RANARR_HERBLORE,
            named: true,
        },
    )
}

fn potion_maker_live_seed_steps() -> Vec<Step> {
    vec![
        Step {
            name: "stick tutorial skip and complete Druidic Ritual",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, "setvar tutorial 1000");
                    // Frozen quest def: druidquest is the server-only
                    // permanent varp; COMPLETE = 4. Relog paints the journal.
                    cheat(c, "setvar druidquest 4");
                    cheat(c, "getvar tutorial");
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Chat {
                    needle: "get tutorial: 1000",
                },
                budget_ticks: 200,
            },
        },
        Step {
            name: "relog so the inv tab binds and journal refreshes",
            kind: StepKind::Relog,
            wait: Wait {
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 600,
            },
        },
        bank_fletcher_watch(
            "acknowledge Druidic Ritual before Start",
            Proof::QuestDone {
                name: "Druidic Ritual",
            },
        ),
    ]
}

/// Empty pack at Varrock West. Banked herb, water and secondary. Script
/// withdraws a batch, spam-uses herb onto water, withdraws the secondary,
/// finishes, deposits, restocks and makes further product. Named selector
/// also banks leftover Guam that must stay put.
#[derive(Clone, Copy)]
struct PotionMakerRecipe {
    herb_id: i32,
    unf_id: i32,
    secondary_id: i32,
    finished_id: i32,
    wrong_unf_id: i32,
    wrong_finished_id: i32,
    herb_alias: &'static str,
    secondary_alias: &'static str,
    herblore: i32,
    named: bool,
}

fn potion_maker_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    recipe: PotionMakerRecipe,
) -> Scenario {
    let PotionMakerRecipe {
        herb_id,
        unf_id,
        secondary_id,
        finished_id,
        wrong_unf_id,
        wrong_finished_id,
        herb_alias,
        secondary_alias,
        herblore,
        named,
    } = recipe;
    let first_xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    };
    let finished = Proof::ItemId {
        id: finished_id,
        count: 1,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = potion_maker_live_seed_steps();
    steps.push(Step {
        name: "seed Herblore and potion ingredients before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat herblore {herblore}"));
                cheat(c, &format!("givebank {herb_alias} {POTION_BATCH_SEED}"));
                cheat(c, &format!("givebank vial_water {POTION_BATCH_SEED}"));
                cheat(
                    c,
                    &format!("givebank {secondary_alias} {POTION_BATCH_SEED}"),
                );
                if named {
                    cheat(c, "givebank guam_leaf 14");
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
    let mut before_start = vec![
        (
            "confirm Herblore before Start",
            Proof::Stat {
                id: HERBLORE_STAT,
                min: herblore,
            },
        ),
        (
            "confirm no seeded herb in pack before Start",
            Proof::ItemIdAtMost {
                id: herb_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded water vials in pack before Start",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded secondary in pack before Start",
            Proof::ItemIdAtMost {
                id: secondary_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded unfinished potion before Start",
            Proof::ItemIdAtMost {
                id: unf_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded finished potion before Start",
            Proof::ItemIdAtMost {
                id: finished_id,
                count: 0,
            },
        ),
        (
            "confirm no wrong unfinished potion before Start",
            Proof::ItemIdAtMost {
                id: wrong_unf_id,
                count: 0,
            },
        ),
        (
            "confirm no wrong finished potion before Start",
            Proof::ItemIdAtMost {
                id: wrong_finished_id,
                count: 0,
            },
        ),
    ];
    if named {
        before_start.push((
            "confirm no seeded leftover Guam in pack before Start",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ));
    }
    for (step_name, arm) in before_start {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(open_seed_booth(
        "open and acknowledge the exact herb seed bank",
        WorldTile {
            x: VARROCK_WEST_BANK.x + 1,
            z: VARROCK_WEST_BANK.z,
            level: VARROCK_WEST_BANK.level,
        },
        Proof::BankItemId {
            id: herb_id,
            count: POTION_BATCH_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact water-vial seed bank",
        Proof::BankItemId {
            id: VIAL_OF_WATER_ID,
            count: POTION_BATCH_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact secondary seed bank",
        Proof::BankItemId {
            id: secondary_id,
            count: POTION_BATCH_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded unfinished potion in bank",
        Proof::BankItemIdAtMost {
            id: unf_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded finished potion in bank",
        Proof::BankItemIdAtMost {
            id: finished_id,
            count: 0,
        },
    ));
    if named {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact leftover Guam seed bank",
            Proof::BankItemId {
                id: GUAM_LEAF_ID,
                count: 14,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        (
            "watch herb plus water become the exact unfinished potion",
            Proof::ItemId {
                id: unf_id,
                count: 1,
            },
        ),
        (
            "watch the withdrawn herb consumed into unfinished potions",
            Proof::ItemIdAtMost {
                id: herb_id,
                count: 0,
            },
        ),
        (
            "watch the withdrawn water vials consumed into unfinished potions",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
        (
            "watch the unfinished potions become the exact finished potion",
            finished,
        ),
        (
            "watch unfinished potions consumed into the finished product",
            Proof::ItemIdAtMost {
                id: unf_id,
                count: 0,
            },
        ),
        ("watch Herblore XP from finishing potions", first_xp),
        (
            "watch no wrong unfinished potion",
            Proof::ItemIdAtMost {
                id: wrong_unf_id,
                count: 0,
            },
        ),
        (
            "watch no wrong finished potion",
            Proof::ItemIdAtMost {
                id: wrong_finished_id,
                count: 0,
            },
        ),
        (
            "watch script-created finished potions enter a fresh bank",
            Proof::BankItemId {
                id: finished_id,
                count: 1,
            },
        ),
        (
            "watch a restock of exact water vials",
            Proof::ItemId {
                id: VIAL_OF_WATER_ID,
                count: 1,
            },
        ),
        (
            "watch a restock of the exact herb",
            Proof::ItemId {
                id: herb_id,
                count: 1,
            },
        ),
        (
            "watch finished potions leave the pack across deposit/restock",
            Proof::ItemIdAtMost {
                id: finished_id,
                count: 0,
            },
        ),
    ];
    if named {
        watch.push((
            "watch leftover Guam stay in bank",
            Proof::BankItemId {
                id: GUAM_LEAF_ID,
                count: 14,
            },
        ));
        watch.push((
            "watch leftover Guam never enter the pack",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        (
            "watch the script close its potion restock bank",
            Proof::BankClosed,
        ),
        (
            "watch another exact unfinished potion after restock",
            Proof::ItemId {
                id: unf_id,
                count: 1,
            },
        ),
        (
            "watch another exact finished potion after restock",
            finished,
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
        proof: finished,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("PotionMaker"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
