use super::*;
pub(crate) const COOKING_STAT: i32 = 7;
pub(crate) const RAW_SALMON_ID: i32 = 331;
pub(crate) const SALMON_ID: i32 = 329;
const NOTED_RAW_SALMON_ID: i32 = 332;
const NOTED_SALMON_ID: i32 = 330;
const RAW_LOBSTER_ID: i32 = 377;
pub(crate) const LOBSTER_ID: i32 = 379;
const NOTED_RAW_LOBSTER_ID: i32 = 378;
pub(crate) const NOTED_LOBSTER_ID: i32 = 380;
const BURNT_LOBSTER_ID: i32 = 381;
const BURNT_FISH_1_ID: i32 = 323;
const BURNT_FISH_2_ID: i32 = 343;
pub(crate) const COOKING_FIXTURE_LEVEL: i32 = 80;
pub(crate) const COOK_RAW_SEED: i32 = 56;
pub(crate) const CATHERBY_RANGE_STAND: WorldTile = WorldTile {
    x: 2817,
    z: 3443,
    level: 0,
};
const COOK_BOT_SALMON_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "fish",
        value: ScriptInjectValue::Str("Raw salmon"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Catherby"),
    },
    ScriptSettingInject {
        id: "surface",
        value: ScriptInjectValue::Str("Range"),
    },
];

const COOK_BOT_LOBSTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "fish",
        value: ScriptInjectValue::Str("Raw lobster"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Catherby"),
    },
    ScriptSettingInject {
        id: "surface",
        value: ScriptInjectValue::Str("Range"),
    },
];
pub(crate) fn cook_bot_scenario() -> Scenario {
    cook_bot_variant(CookSpec {
        name: "cook_bot",
        inject: COOK_BOT_SALMON_INJECT,
        raw_alias: "raw_salmon",
        raw_id: RAW_SALMON_ID,
        product_id: SALMON_ID,
        wrong_product_id: LOBSTER_ID,
        noted_raw_id: NOTED_RAW_SALMON_ID,
        noted_product_id: NOTED_SALMON_ID,
    })
}

pub(crate) fn cook_bot_lobster_scenario() -> Scenario {
    cook_bot_variant(CookSpec {
        name: "cook_bot_lobster",
        inject: COOK_BOT_LOBSTER_INJECT,
        raw_alias: "raw_lobster",
        raw_id: RAW_LOBSTER_ID,
        product_id: LOBSTER_ID,
        wrong_product_id: SALMON_ID,
        noted_raw_id: NOTED_RAW_LOBSTER_ID,
        noted_product_id: NOTED_LOBSTER_ID,
    })
}

struct CookSpec {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    raw_alias: &'static str,
    raw_id: i32,
    product_id: i32,
    wrong_product_id: i32,
    noted_raw_id: i32,
    noted_product_id: i32,
}

/// Empty pack at Catherby bank. Banked raw fish, never cooked/burnt/noted.
/// Cooking 80 is a fixture seed so ordinary burn randomness does not replace
/// the product contract; the script encodes no cook level. Range only — Fire
/// stays behind native fire work. Script withdraws, cooks on the Catherby
/// Range, deposits the exact product, restocks raw, returns and cooks again.
fn cook_bot_variant(spec: CookSpec) -> Scenario {
    let CookSpec {
        name,
        inject,
        raw_alias,
        raw_id,
        product_id,
        wrong_product_id,
        noted_raw_id,
        noted_product_id,
    } = spec;
    let range = Proof::ArrivedNear {
        x: CATHERBY_RANGE_STAND.x,
        z: CATHERBY_RANGE_STAND.z,
        level: CATHERBY_RANGE_STAND.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let bank = CATHERBY_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed cooking, banked raw fish, and tele to Catherby bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat cooking {COOKING_FIXTURE_LEVEL}"));
                cheat(c, &format!("givebank {raw_alias} {COOK_RAW_SEED}"));
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
            "confirm Cooking 80 before Start",
            Proof::Stat {
                id: COOKING_STAT,
                min: COOKING_FIXTURE_LEVEL,
            },
        ),
        (
            "confirm no seeded raw fish in pack before Start",
            Proof::ItemIdAtMost {
                id: raw_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded cooked product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong cooked fish in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded burnt fish 323 in pack before Start",
            Proof::ItemIdAtMost {
                id: BURNT_FISH_1_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded burnt fish in pack before Start",
            Proof::ItemIdAtMost {
                id: BURNT_FISH_2_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded burnt lobster in pack before Start",
            Proof::ItemIdAtMost {
                id: BURNT_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted raw in pack before Start",
            Proof::ItemIdAtMost {
                id: noted_raw_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted product in pack before Start",
            Proof::ItemIdAtMost {
                id: noted_product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact raw-fish seed bank",
        Proof::BankItemId {
            id: raw_id,
            count: COOK_RAW_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded cooked product in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted raw in bank",
        Proof::BankItemIdAtMost {
            id: noted_raw_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch arrival at the Catherby Range after Start", range),
        (
            "watch Cooking XP from the Catherby Range after Start",
            Proof::StatXpGain {
                id: COOKING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted cooked fish after Start", product),
        (
            "watch the withdrawn raw finish converting",
            Proof::ItemIdAtMost {
                id: raw_id,
                count: 0,
            },
        ),
        (
            "watch script-cooked fish enter a fresh Catherby bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of cooked fish after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of exact raw fish",
            Proof::ItemId {
                id: raw_id,
                count: 1,
            },
        ),
        ("watch the script close its cook bank", Proof::BankClosed),
        ("watch return to the Catherby Range after restock", range),
        ("watch another exact cooked fish after restock", product),
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
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("CookBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
