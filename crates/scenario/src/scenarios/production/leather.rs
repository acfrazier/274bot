use super::*;
pub(crate) const NEEDLE_ID: i32 = 1733;
pub(crate) const THREAD_ID: i32 = 1734;
pub(crate) const LEATHER_GLOVES_ID: i32 = 1059;
const LEATHER_CHAPS_ID: i32 = 1095;
const HARDLEATHER_BODY_ID: i32 = 1131;
const DRAGONHIDE_BODY_ID: i32 = 1135;
const DRAGONHIDE_CHAPS_ID: i32 = 1099;
pub(crate) const LEATHER_CERT_ID: i32 = 1742;
pub(crate) const HARD_LEATHER_CERT_ID: i32 = 1744;
/// Green dragon leather (selected289 `dragon_leather` / alias `dragon_leather`).
pub(crate) const GREEN_DRAGON_LEATHER_ID: i32 = 1745;
pub(crate) const GREEN_DRAGON_LEATHER_CERT_ID: i32 = 1746;
/// Enough banked leather for two 26-slot trips, plus four pieces.
const LEATHER_CRAFTER_TWO_TRIP_SEED: i32 = 56;
/// Banked coins for missing-thread `fundThread` (default threadPerTrip 100 ×
/// THREAD_MAX_PRICE 3 = 300 needed; 1000 leaves headroom).
pub(crate) const LEATHER_THREAD_SHOP_COIN_SEED: i32 = 1000;
/// Canonical fundThread withdrawal ceiling: threadPerTrip default 100 × max price 3.
pub(crate) const LEATHER_THREAD_SHOP_FUND: i32 = 300;
pub(crate) const LEATHER_CRAFTER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "leatherType",
    value: ScriptInjectValue::Str("Leather"),
}];
const LEATHER_CRAFTER_HARD_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "leatherType",
    value: ScriptInjectValue::Str("Hard leather"),
}];
const LEATHER_CRAFTER_GREEN_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "leatherType",
    value: ScriptInjectValue::Str("Green dragon leather"),
}];
pub(crate) fn leather_crafter_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter",
        LEATHER_CRAFTER_INJECT,
        1,
        "leather",
        "cert_leather",
        SOFT_LEATHER_ID,
        LEATHER_CERT_ID,
        28,
        LEATHER_GLOVES_ID,
        HARDLEATHER_BODY_ID,
    )
}

pub(crate) fn leather_crafter_hard_body_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter_hard_body",
        LEATHER_CRAFTER_HARD_INJECT,
        28,
        "hard_leather",
        "cert_hard_leather",
        HARD_LEATHER_ID,
        HARD_LEATHER_CERT_ID,
        28,
        HARDLEATHER_BODY_ID,
        LEATHER_GLOVES_ID,
    )
}

/// Green body at Crafting 63: multi3 make-X / chat count-dialog branch.
/// 3 leather per body; needle+thread leave 26 slots so a trip may leave 2 leather.
pub(crate) fn leather_crafter_green_body_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter_green_body",
        LEATHER_CRAFTER_GREEN_INJECT,
        63,
        "dragon_leather",
        "cert_dragon_leather",
        GREEN_DRAGON_LEATHER_ID,
        GREEN_DRAGON_LEATHER_CERT_ID,
        LEATHER_CRAFTER_TWO_TRIP_SEED,
        DRAGONHIDE_BODY_ID,
        DRAGONHIDE_CHAPS_ID,
    )
}

/// Soft leather chaps at Crafting 18: selected289 uses make-10 button 8645.
pub(crate) fn leather_crafter_chaps_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter_chaps",
        LEATHER_CRAFTER_INJECT,
        18,
        "leather",
        "cert_leather",
        SOFT_LEATHER_ID,
        LEATHER_CERT_ID,
        LEATHER_CRAFTER_TWO_TRIP_SEED,
        LEATHER_CHAPS_ID,
        LEATHER_GLOVES_ID,
    )
}

/// Missing-thread purchase + return: level-1 Leather gloves with default
/// `threadPerTrip` 100. Bank holds needle, coins, and raw soft leather only —
/// absolutely no thread or crafted product in pack or bank before Start.
/// Script `fundThread` withdraws up to `threadPerTrip * THREAD_MAX_PRICE`
/// (100×3), walks nearest Dommik (3322,3194), `Shop.buy(Thread)`, closes,
/// returns to the remembered Al-Kharid bank stand, restocks leather, crafts.
///
/// Post-Start scope is purchase / return / resumed craft — not a second full
/// product bank cycle. `BankClosed` is the existing bank-modal gate only; there
/// is no `ShopClosed` proof, so shop-close is not asserted here.
pub(crate) fn leather_crafter_thread_shop_scenario() -> Scenario {
    let product = Proof::ItemId {
        id: LEATHER_GLOVES_ID,
        count: 1,
    };
    let xp = Proof::FreshStatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let bank = AL_KHARID_BANK;
    let returned = Proof::ArrivedNear {
        x: bank.x,
        z: bank.z,
        level: bank.level,
        radius: 8,
    };
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Crafting, banked needle/coins/leather, and tele to Al-Kharid before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: COINS_ID,
                debug_alias: "coins",
                note_alias: None,
                quantity: LEATHER_THREAD_SHOP_COIN_SEED,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: SOFT_LEATHER_ID,
                debug_alias: "leather",
                note_alias: Some("cert_leather"),
                quantity: 28,
                note_id: Some(LEATHER_CERT_ID),
            },
        ],
        "crafting",
        1,
    ));
    for (step_name, arm) in [
        (
            "confirm Crafting before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        (
            "confirm the exact noted leather seed in pack before deposit",
            Proof::ItemId {
                id: LEATHER_CERT_ID,
                count: 28,
            },
        ),
        (
            "confirm the needle seed in pack before deposit",
            Proof::ItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "confirm the exact coin seed in pack before deposit",
            Proof::ItemId {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "bound the noted leather seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: LEATHER_CERT_ID,
                count: 28,
            },
        ),
        (
            "bound the needle seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the coin seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "confirm zero thread in pack before deposit",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded gloves product in pack before Start",
            Proof::ItemIdAtMost {
                id: LEATHER_GLOVES_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong product in pack before Start",
            Proof::ItemIdAtMost {
                id: HARDLEATHER_BODY_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the leather seed bank",
        Proof::BankItemIdAtMost {
            id: SOFT_LEATHER_ID,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the native leather seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: COINS_ID,
                debug_alias: "coins",
                note_alias: None,
                quantity: LEATHER_THREAD_SHOP_COIN_SEED,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: SOFT_LEATHER_ID,
                debug_alias: "leather",
                note_alias: Some("cert_leather"),
                quantity: 28,
                note_id: Some(LEATHER_CERT_ID),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact leather seed bank",
            Proof::BankItemId {
                id: SOFT_LEATHER_ID,
                count: 28,
            },
        ),
        (
            "acknowledge the needle seed bank",
            Proof::BankItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the exact coin seed bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "confirm noted leather removal",
            Proof::ItemIdAtMost {
                id: LEATHER_CERT_ID,
                count: 0,
            },
        ),
        (
            "confirm needle removal",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 0,
            },
        ),
        (
            "confirm coin removal",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm zero thread in pack after deposit",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm zero thread in bank before Start",
            Proof::BankItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no gloves product in bank before Start",
            Proof::BankItemIdAtMost {
                id: LEATHER_GLOVES_ID,
                count: 0,
            },
        ),
        (
            "confirm no wrong product in bank before Start",
            Proof::BankItemIdAtMost {
                id: HARDLEATHER_BODY_ID,
                count: 0,
            },
        ),
        (
            "bound the leather seed bank count",
            Proof::BankItemIdAtMost {
                id: SOFT_LEATHER_ID,
                count: 28,
            },
        ),
        (
            "bound the needle seed bank count",
            Proof::BankItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the coin seed bank count",
            Proof::BankItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "confirm no noted leather remains in bank",
            Proof::BankItemIdAtMost {
                id: LEATHER_CERT_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch thread acquired from the zero-thread baseline after Start",
            Proof::ItemId {
                id: THREAD_ID,
                count: 1,
            },
        ),
        (
            // fundThread withdraws at most threadPerTrip*THREAD_MAX_PRICE (300).
            // After Shop.buy the pack holds strictly less than that withdrawal
            // while leftover coins remain, matching climbing_boots / shop_buyout
            // ItemIdAtMost spend arms. Not a ShopClosed proof.
            "watch coin expenditure after thread purchase",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_FUND - 1,
            },
        ),
        (
            "watch return near the original Al-Kharid bank after acquisition",
            returned,
        ),
        (
            "watch the leather bank close after restock",
            Proof::BankClosed,
        ),
        ("watch fresh Crafting XP after purchase return", xp),
        ("watch exact leather gloves after resumed craft", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "leather_crafter_thread_shop",
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
            start_script: Some("LeatherCrafter"),
            script_settings_inject: Some(LEATHER_CRAFTER_INJECT),
            terminal_shot: Some("leather_crafter_thread_shop"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[allow(clippy::too_many_arguments)] // scenario factory bundles inject/skills/products
fn leather_crafter_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    crafting: i32,
    leather_alias: &'static str,
    leather_note_alias: &'static str,
    leather_id: i32,
    leather_note: i32,
    leather_qty: i32,
    product_id: i32,
    wrong_id: i32,
) -> Scenario {
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let bank = AL_KHARID_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Crafting, banked needle/thread/leather, and tele to Al-Kharid before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: THREAD_ID,
                debug_alias: "thread",
                note_alias: None,
                quantity: 100,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: leather_id,
                debug_alias: leather_alias,
                note_alias: Some(leather_note_alias),
                quantity: leather_qty,
                note_id: Some(leather_note),
            },
        ],
        "crafting",
        crafting,
    ));
    for (step_name, arm) in [
        (
            "confirm Crafting before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: crafting,
            },
        ),
        (
            "confirm the exact noted leather seed in pack before deposit",
            Proof::ItemId {
                id: leather_note,
                count: leather_qty,
            },
        ),
        (
            "confirm the needle seed in pack before deposit",
            Proof::ItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "confirm the thread seed in pack before deposit",
            Proof::ItemId {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "bound the noted leather seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: leather_note,
                count: leather_qty,
            },
        ),
        (
            "bound the needle seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the thread seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 100,
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
        "open and acknowledge the leather seed bank",
        Proof::BankItemIdAtMost {
            id: leather_id,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the native leather seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: THREAD_ID,
                debug_alias: "thread",
                note_alias: None,
                quantity: 100,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: leather_id,
                debug_alias: leather_alias,
                note_alias: Some(leather_note_alias),
                quantity: leather_qty,
                note_id: Some(leather_note),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact leather seed bank",
            Proof::BankItemId {
                id: leather_id,
                count: leather_qty,
            },
        ),
        (
            "acknowledge the needle seed bank",
            Proof::BankItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the thread seed bank",
            Proof::BankItemId {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "confirm noted leather removal",
            Proof::ItemIdAtMost {
                id: leather_note,
                count: 0,
            },
        ),
        (
            "confirm needle removal",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 0,
            },
        ),
        (
            "confirm thread removal",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "bound the leather seed bank count",
            Proof::BankItemIdAtMost {
                id: leather_id,
                count: leather_qty,
            },
        ),
        (
            "bound the needle seed bank count",
            Proof::BankItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the thread seed bank count",
            Proof::BankItemIdAtMost {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "confirm no noted leather remains in bank",
            Proof::BankItemIdAtMost {
                id: leather_note,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Crafting XP after Start", xp),
        ("watch the leather product after Start", product),
        (
            "watch script-crafted product enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
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
            "watch a restock of leather",
            Proof::ItemId {
                id: leather_id,
                count: 1,
            },
        ),
        ("watch the leather bank close", Proof::BankClosed),
        ("watch further Crafting XP after restock", further_xp),
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
            start_script: Some("LeatherCrafter"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
