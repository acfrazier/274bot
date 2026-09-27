use super::*;
pub(crate) const HAMMER_ID: i32 = 2347;
pub(crate) const BRONZE_BAR_CERT_ID: i32 = 2350;
/// Selected 289 `obj.pack`: `cert_steel_bar` = 2354 (unnoted steel bar 2353).
pub(crate) const STEEL_BAR_CERT_ID: i32 = 2354;
/// Selected 289 `obj.pack`: mithril_bar=2359, cert_mithril_bar=2360.
pub(crate) const MITHRIL_BAR_CERT_ID: i32 = 2360;
const BRONZE_DAGGER_ID: i32 = 1205;
/// Mithril dagger product id (stock smithing dbrow lvl 50).
const MITHRIL_DAGGER_ID: i32 = 1209;
const BRONZE_PLATEBODY_ID: i32 = 1117;
/// Selected 289 nails (steel-only anvil product, stackable, out 2/bar).
const STEEL_NAILS_ID: i32 = 1539;
/// Nails recipe levelrequired (steel bar tier is 30; product gate is 34).
const STEEL_NAILS_SMITHING: i32 = 34;
/// Stock nails `product_amount` — stack output must prove ≥ this count.
pub(crate) const STEEL_NAILS_OUTPUT: i32 = 2;
const SMITHING_BOT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Bronze"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Dagger"),
    },
];
const SMITHING_BOT_PLATEBODY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Bronze"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Platebody"),
    },
];
const SMITHING_BOT_NAILS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Steel"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Nails"),
    },
];
const SMITHING_BOT_MITHRIL_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Mithril"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Dagger"),
    },
];
pub(crate) fn smithing_bot_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot",
        SMITHING_BOT_INJECT,
        1,
        BRONZE_DAGGER_ID,
        BRONZE_PLATEBODY_ID,
        BRONZE_BAR_ID,
        BRONZE_BAR_CERT_ID,
        "bronze_bar",
        "cert_bronze_bar",
        28,
        1,
    )
}

pub(crate) fn smithing_bot_platebody_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot_platebody",
        SMITHING_BOT_PLATEBODY_INJECT,
        18,
        BRONZE_PLATEBODY_ID,
        BRONZE_DAGGER_ID,
        BRONZE_BAR_ID,
        BRONZE_BAR_CERT_ID,
        "bronze_bar",
        "cert_bronze_bar",
        30,
        1,
    )
}

/// Steel Nails: steel-only special panel slot, stackable out=2/bar, F2P.
/// 28 steel bars → first trip 27 bars (+ hammer) then remaining 1 bar restock.
pub(crate) fn smithing_bot_nails_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot_nails",
        SMITHING_BOT_NAILS_INJECT,
        STEEL_NAILS_SMITHING,
        STEEL_NAILS_ID,
        BRONZE_DAGGER_ID,
        STEEL_BAR_ID,
        STEEL_BAR_CERT_ID,
        "steel_bar",
        "cert_steel_bar",
        28,
        STEEL_NAILS_OUTPUT,
    )
}

/// Mithril Dagger: 1-bar F2P product on a higher metal tier (lvl 50 / id 1209).
pub(crate) fn smithing_bot_mithril_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot_mithril",
        SMITHING_BOT_MITHRIL_INJECT,
        MITHRIL_SMITHING,
        MITHRIL_DAGGER_ID,
        BRONZE_DAGGER_ID,
        MITHRIL_BAR_ID,
        MITHRIL_BAR_CERT_ID,
        "mithril_bar",
        "cert_mithril_bar",
        28,
        1,
    )
}

#[allow(clippy::too_many_arguments)] // scenario factory bundles inject/skills/products
fn smithing_bot_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    smithing: i32,
    product_id: i32,
    wrong_id: i32,
    bar_id: i32,
    bar_cert_id: i32,
    bar_alias: &'static str,
    bar_note_alias: &'static str,
    bar_quantity: i32,
    product_count: i32,
) -> Scenario {
    let anvil = Proof::ArrivedNear {
        x: VARROCK_ANVIL.x,
        z: VARROCK_ANVIL.z,
        level: VARROCK_ANVIL.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: product_count,
    };
    let xp = Proof::StatXpGain {
        id: SMITHING_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: SMITHING_STAT,
        min: 1,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Smithing, banked hammer/bars, and tele to Varrock West before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: HAMMER_ID,
                debug_alias: "hammer",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: bar_id,
                debug_alias: bar_alias,
                note_alias: Some(bar_note_alias),
                quantity: bar_quantity,
                note_id: Some(bar_cert_id),
            },
        ],
        "smithing",
        smithing,
    ));
    for (step_name, arm) in [
        (
            "confirm Smithing before Start",
            Proof::Stat {
                id: SMITHING_STAT,
                min: smithing,
            },
        ),
        (
            "confirm the exact noted bar seed in pack before deposit",
            Proof::ItemId {
                id: bar_cert_id,
                count: bar_quantity,
            },
        ),
        (
            "confirm the hammer seed in pack before deposit",
            Proof::ItemId {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "bound the noted bar seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: bar_cert_id,
                count: bar_quantity,
            },
        ),
        (
            "bound the hammer seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong product in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the bar seed bank",
        Proof::BankItemIdAtMost {
            id: bar_id,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the hammer and native note seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: HAMMER_ID,
                debug_alias: "hammer",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: bar_id,
                debug_alias: bar_alias,
                note_alias: Some(bar_note_alias),
                quantity: bar_quantity,
                note_id: Some(bar_cert_id),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the hammer seed bank",
            Proof::BankItemId {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the exact bar seed bank",
            Proof::BankItemId {
                id: bar_id,
                count: bar_quantity,
            },
        ),
        (
            "confirm the hammer seed was removed from pack",
            Proof::ItemIdAtMost {
                id: HAMMER_ID,
                count: 0,
            },
        ),
        (
            "confirm the noted bar seed was removed from pack",
            Proof::ItemIdAtMost {
                id: bar_cert_id,
                count: 0,
            },
        ),
        (
            "bound the hammer seed bank count",
            Proof::BankItemIdAtMost {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "bound the bar seed bank count",
            Proof::BankItemIdAtMost {
                id: bar_id,
                count: bar_quantity,
            },
        ),
        (
            "confirm no noted bars remain in bank",
            Proof::BankItemIdAtMost {
                id: bar_cert_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch arrival at the Varrock anvil after Start", anvil),
        ("watch Smithing XP from the anvil panel after Start", xp),
        ("watch the selected smithing product after Start", product),
        (
            "watch script-smithed product enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: product_count,
            },
        ),
        (
            "watch the pack empty of product after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of bars",
            Proof::ItemId {
                id: bar_id,
                count: 1,
            },
        ),
        ("watch the smithing bank close", Proof::BankClosed),
        ("watch return to the anvil after restock", anvil),
        ("watch further Smithing XP after restock", further_xp),
    ] {
        // Deposit arms on the first product while ≤26 bars may still forge.
        // SMITHING_PRODUCT_DEPOSIT_WATCH_TICKS is runner dirty increments
        // (not engine p_delay ticks); see constant docs. Other arms keep
        // the ordinary gold watch; do not loosen the global deadline.
        let budget_ticks = if matches!(arm, Proof::BankItemId { .. }) {
            SMITHING_PRODUCT_DEPOSIT_WATCH_TICKS
        } else {
            SCRIPT_GOLD_WATCH_TICKS
        };
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait { arm, budget_ticks },
        });
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
            start_script: Some("SmithingBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
