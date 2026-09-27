use super::*;
const NOTED_COPPER_ORE_ID: i32 = 437;
const NOTED_TIN_ORE_ID: i32 = 439;
const NOTED_IRON_ORE_ID: i32 = 441;
const NOTED_BRONZE_BAR_ID: i32 = 2350;
const NOTED_STEEL_BAR_ID: i32 = 2354;
pub(crate) const SMELT_ORE_SEED: i32 = 56;
pub(crate) const STEEL_COAL_SEED: i32 = 112;
pub(crate) const AL_KHARID_FURNACE: WorldTile = WorldTile {
    x: 3275,
    z: 3185,
    level: 0,
};
const SMELTER_BOT_BRONZE_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Bronze"),
}];

const SMELTER_BOT_STEEL_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Steel"),
}];
pub(crate) fn smelter_bot_scenario() -> Scenario {
    smelter_bot_variant(SmelterSpec {
        name: "smelter_bot",
        inject: SMELTER_BOT_BRONZE_INJECT,
        smithing: 1,
        primary_alias: "copper_ore",
        primary_id: COPPER_ORE_ID,
        primary_seed: SMELT_ORE_SEED,
        secondary_alias: "tin_ore",
        secondary_id: TIN_ORE_ID,
        secondary_seed: SMELT_ORE_SEED,
        product_id: BRONZE_BAR_ID,
        wrong_product_id: STEEL_BAR_ID,
        noted_primary_id: NOTED_COPPER_ORE_ID,
        noted_secondary_id: NOTED_TIN_ORE_ID,
        noted_product_id: NOTED_BRONZE_BAR_ID,
    })
}

pub(crate) fn smelter_bot_steel_scenario() -> Scenario {
    smelter_bot_variant(SmelterSpec {
        name: "smelter_bot_steel",
        inject: SMELTER_BOT_STEEL_INJECT,
        smithing: 30,
        primary_alias: "iron_ore",
        primary_id: IRON_ORE_ID,
        primary_seed: SMELT_ORE_SEED,
        secondary_alias: "coal",
        secondary_id: COAL_ID,
        secondary_seed: STEEL_COAL_SEED,
        product_id: STEEL_BAR_ID,
        wrong_product_id: BRONZE_BAR_ID,
        noted_primary_id: NOTED_IRON_ORE_ID,
        noted_secondary_id: NOTED_COAL_ID,
        noted_product_id: NOTED_STEEL_BAR_ID,
    })
}

struct SmelterSpec {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    smithing: i32,
    primary_alias: &'static str,
    primary_id: i32,
    primary_seed: i32,
    secondary_alias: &'static str,
    secondary_id: i32,
    secondary_seed: i32,
    product_id: i32,
    wrong_product_id: i32,
    noted_primary_id: i32,
    noted_secondary_id: i32,
    noted_product_id: i32,
}

/// Empty pack at Al-Kharid bank. Banked ores, never bars. Script withdraws
/// the recipe, smelts at the real furnace, deposits the exact bar, restocks
/// ore, returns and smelts again. Smithing main panel is not this hop.
fn smelter_bot_variant(spec: SmelterSpec) -> Scenario {
    let SmelterSpec {
        name,
        inject,
        smithing,
        primary_alias,
        primary_id,
        primary_seed,
        secondary_alias,
        secondary_id,
        secondary_seed,
        product_id,
        wrong_product_id,
        noted_primary_id,
        noted_secondary_id,
        noted_product_id,
    } = spec;
    let furnace = Proof::ArrivedNear {
        x: AL_KHARID_FURNACE.x,
        z: AL_KHARID_FURNACE.z,
        level: AL_KHARID_FURNACE.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let bank = AL_KHARID_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed smithing, banked ores, and tele to Al-Kharid bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat smithing {smithing}"));
                cheat(c, &format!("givebank {primary_alias} {primary_seed}"));
                cheat(c, &format!("givebank {secondary_alias} {secondary_seed}"));
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
            "confirm Smithing before Start",
            Proof::Stat {
                id: SMITHING_STAT,
                min: smithing,
            },
        ),
        (
            "confirm no seeded primary ore in pack before Start",
            Proof::ItemIdAtMost {
                id: primary_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded secondary ore in pack before Start",
            Proof::ItemIdAtMost {
                id: secondary_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded bars in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong bars in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded iron bars in pack before Start",
            Proof::ItemIdAtMost {
                id: IRON_BAR_ID,
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
        "open and acknowledge the exact primary-ore seed bank",
        Proof::BankItemId {
            id: primary_id,
            count: primary_seed,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact secondary-ore seed bank",
        Proof::BankItemId {
            id: secondary_id,
            count: secondary_seed,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bars in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted primary in bank",
        Proof::BankItemIdAtMost {
            id: noted_primary_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted secondary in bank",
        Proof::BankItemIdAtMost {
            id: noted_secondary_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the Al-Kharid furnace after Start",
            furnace,
        ),
        (
            "watch Smithing XP from the furnace after Start",
            Proof::StatXpGain {
                id: SMITHING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bar after Start", product),
        (
            "watch the withdrawn primary ore finish converting",
            Proof::ItemIdAtMost {
                id: primary_id,
                count: 0,
            },
        ),
        (
            "watch script-smelted bars enter a fresh Al-Kharid bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bars after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of exact primary ore",
            Proof::ItemId {
                id: primary_id,
                count: 1,
            },
        ),
        ("watch the script close its smelt bank", Proof::BankClosed),
        (
            "watch return to the Al-Kharid furnace after restock",
            furnace,
        ),
        ("watch another exact bar after restock", product),
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
            start_script: Some("SmelterBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
