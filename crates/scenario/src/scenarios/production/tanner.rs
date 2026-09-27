use super::*;
pub(crate) const COW_HIDE_ID: i32 = 1739;
pub(crate) const TANNER_HIDE_SEED: i32 = 28;
pub(crate) const TANNER_COIN_SEED: i32 = 5000;
const TANNER_STAND: WorldTile = WorldTile {
    x: 3277,
    z: 3191,
    level: 0,
};
const TANNER_BOT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "hideType",
        value: ScriptInjectValue::Str("Soft leather"),
    },
    ScriptSettingInject {
        id: "buyThread",
        value: ScriptInjectValue::Bool(false),
    },
];

const TANNER_BOT_HARD_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "hideType",
        value: ScriptInjectValue::Str("Hard leather"),
    },
    ScriptSettingInject {
        id: "buyThread",
        value: ScriptInjectValue::Bool(false),
    },
];
pub(crate) fn tanner_bot_scenario() -> Scenario {
    tanner_bot_variant(
        "tanner_bot",
        TANNER_BOT_INJECT,
        SOFT_LEATHER_ID,
        HARD_LEATHER_ID,
    )
}

pub(crate) fn tanner_bot_hard_scenario() -> Scenario {
    tanner_bot_variant(
        "tanner_bot_hard",
        TANNER_BOT_HARD_INJECT,
        HARD_LEATHER_ID,
        SOFT_LEATHER_ID,
    )
}

/// Empty pack at Al-Kharid bank. Banked cowhides 1739 and coins, never leather.
/// Script withdraws, tans at the Tanner widget (not a shop), deposits produced
/// leather, empties the pack of leather, restocks hides, returns and tans again.
/// Dommik thread-buy stays pending.
fn tanner_bot_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    product_id: i32,
    wrong_product_id: i32,
) -> Scenario {
    let tanner = Proof::ArrivedNear {
        x: TANNER_STAND.x,
        z: TANNER_STAND.z,
        level: TANNER_STAND.level,
        radius: 4,
    };
    let leather = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let bank = AL_KHARID_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed cowhides, coins, and tele to Al-Kharid bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("givebank cow_hide {TANNER_HIDE_SEED}"));
                cheat(c, &format!("givebank coins {TANNER_COIN_SEED}"));
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
            "confirm no seeded cowhides in pack before Start",
            Proof::ItemIdAtMost {
                id: COW_HIDE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded soft leather in pack before Start",
            Proof::ItemIdAtMost {
                id: SOFT_LEATHER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded hard leather in pack before Start",
            Proof::ItemIdAtMost {
                id: HARD_LEATHER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact cowhide seed bank",
        Proof::BankItemId {
            id: COW_HIDE_ID,
            count: TANNER_HIDE_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact coin seed bank",
        Proof::BankItemId {
            id: COINS_ID,
            count: TANNER_COIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded leather in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded wrong leather in bank",
        Proof::BankItemIdAtMost {
            id: wrong_product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch arrival at the Tanner after Start", tanner),
        (
            "watch cowhides become the selected leather at the Tanner",
            leather,
        ),
        (
            "watch the withdrawn cowhides finish converting",
            Proof::ItemIdAtMost {
                id: COW_HIDE_ID,
                count: 0,
            },
        ),
        (
            "watch script-created leather enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of leather after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of exact cowhides",
            Proof::ItemId {
                id: COW_HIDE_ID,
                count: 1,
            },
        ),
        ("watch the script close its tanner bank", Proof::BankClosed),
        ("watch return to the Tanner after restock", tanner),
        ("watch another exact leather after restock", leather),
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
        proof: leather,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("TannerBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
