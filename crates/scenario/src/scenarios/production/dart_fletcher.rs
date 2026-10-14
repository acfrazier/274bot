use super::*;
pub(crate) const BRONZE_DART_TIP_ID: i32 = 819;
pub(crate) const BRONZE_DART_ID: i32 = 806;
pub(crate) const IRON_DART_TIP_ID: i32 = 820;
pub(crate) const IRON_DART_ID: i32 = 807;
pub(crate) const RUNE_DART_TIP_ID: i32 = 824;
pub(crate) const RUNE_DART_ID: i32 = 811;
const LUMBRIDGE_COURTYARD: WorldTile = WorldTile {
    x: 3220,
    z: 3212,
    level: 0,
};
const DART_FLETCHER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "tier",
    value: ScriptInjectValue::Str("Bronze"),
}];

const DART_FLETCHER_IRON_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "tier",
    value: ScriptInjectValue::Str("Iron"),
}];

const DART_FLETCHER_RUNE_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "tier",
    value: ScriptInjectValue::Str("Rune"),
}];

/// One hundred ten-dart actions: the sustained-throughput gate of the
/// Rune cell, in exact Rune darts held.
const RUNE_DART_SUSTAINED_DARTS: i32 = 1_000;
/// Seeded tips and feathers for the Rune cell: the sustained gate plus
/// actions already in flight when it lands.
const RUNE_DART_SUPPLY: i32 = 1_200;
/// 1,000 Rune darts at the plan's 18.8 XP per dart is 18,800 XP; the
/// gate keeps slack for the wire XP floor (`exp / 10`).
const RUNE_DART_SUSTAINED_XP: i32 = 18_000;
/// Runner dirty-snapshot increments for the sustained arms. The script
/// pays one server tick per loop and sends at most five actions per
/// loop, so 100 actions take 20-100 engine ticks; double the gold watch
/// keeps the 180s deadline the controlling bound.
const RUNE_DART_SUSTAINED_WATCH_TICKS: u32 = 300;
pub(crate) fn dart_fletcher_scenario() -> Scenario {
    dart_fletcher_variant(
        "dart_fletcher",
        DART_FLETCHER_INJECT,
        1,
        "bronze_dart_tip",
        BRONZE_DART_TIP_ID,
        BRONZE_DART_ID,
        IRON_DART_ID,
        100,
        20,
        2,
        SCRIPT_GOLD_WATCH_TICKS,
    )
}

pub(crate) fn dart_fletcher_iron_scenario() -> Scenario {
    dart_fletcher_variant(
        "dart_fletcher_iron",
        DART_FLETCHER_IRON_INJECT,
        22,
        "iron_dart_tip",
        IRON_DART_TIP_ID,
        IRON_DART_ID,
        BRONZE_DART_ID,
        100,
        20,
        2,
        SCRIPT_GOLD_WATCH_TICKS,
    )
}

/// Ported from rs2b0t `e2e/dartfletcher-test.ts` @2de6bb23.
/// Changed: wall-clock XP/hr and the tick-normalized 600ms projection
/// are dropped; sustained throughput is 1,000 Rune darts and 18,000
/// Fletching XP inside the ordinary 180s script deadline.
/// Changed: the exact observed XP/dart equality becomes count gates on
/// the product and both consumed stacks (the XP floor is not exact).
/// Kept: Rune tier at Fletching 99 with a supply far past the gate —
/// the Bronze/Iron cells stop at twenty darts.
pub(crate) fn dart_fletcher_rune_scenario() -> Scenario {
    dart_fletcher_variant(
        "dart_fletcher_rune",
        DART_FLETCHER_RUNE_INJECT,
        99,
        "rune_dart_tip",
        RUNE_DART_TIP_ID,
        RUNE_DART_ID,
        BRONZE_DART_ID,
        RUNE_DART_SUPPLY,
        RUNE_DART_SUSTAINED_DARTS,
        RUNE_DART_SUSTAINED_XP,
        RUNE_DART_SUSTAINED_WATCH_TICKS,
    )
}

fn dart_watch(name: &'static str, arm: Proof, budget_ticks: u32) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait { arm, budget_ticks },
    }
}

/// No bank. Spam Feather on the selected tip; one action is 10 darts.
#[allow(clippy::too_many_arguments)]
fn dart_fletcher_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    level: i32,
    tip_alias: &'static str,
    tip_id: i32,
    product_id: i32,
    wrong_product_id: i32,
    supply: i32,
    sustained_darts: i32,
    sustained_xp: i32,
    sustained_watch_ticks: u32,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: FLETCHING_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: FLETCHING_STAT,
        min: sustained_xp,
    };
    let courtyard = LUMBRIDGE_COURTYARD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Fletching and exact dart stacks before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat fletching {level}"));
                cheat(c, &format!("give {tip_alias} {supply}"));
                cheat(c, &format!("give feather {supply}"));
                cheat(c, &tele_args(courtyard.level, courtyard.x, courtyard.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: courtyard.x,
                z: courtyard.z,
                level: courtyard.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Fletching before Start",
            Proof::Stat {
                id: FLETCHING_STAT,
                min: level,
            },
        ),
        (
            "confirm exact dart tips before Start",
            Proof::ItemId {
                id: tip_id,
                count: supply,
            },
        ),
        (
            "confirm exact feathers before Start",
            Proof::ItemId {
                id: FEATHER_ID,
                count: supply,
            },
        ),
        (
            "confirm no seeded dart product before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no wrong-tier dart product before Start",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Fletching XP from dart fletching", first_xp),
        (
            "watch at least one ten-dart action by exact product id",
            Proof::ItemId {
                id: product_id,
                count: 10,
            },
        ),
        (
            "watch exact dart tips consumed",
            Proof::ItemIdAtMost {
                id: tip_id,
                count: supply - 10,
            },
        ),
        (
            "watch exact feathers consumed",
            Proof::ItemIdAtMost {
                id: FEATHER_ID,
                count: supply - 10,
            },
        ),
        (
            "watch no wrong-tier dart product",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(dart_watch(
        "watch sustained exact product progress",
        Proof::ItemId {
            id: product_id,
            count: sustained_darts,
        },
        sustained_watch_ticks,
    ));
    steps.push(dart_watch(
        "watch sustained Fletching XP from dart fletching",
        further_xp,
        sustained_watch_ticks,
    ));
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
            start_script: Some("DartFletcher"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
