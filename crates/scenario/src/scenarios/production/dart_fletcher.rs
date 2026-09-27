use super::*;
pub(crate) const BRONZE_DART_TIP_ID: i32 = 819;
pub(crate) const BRONZE_DART_ID: i32 = 806;
pub(crate) const IRON_DART_TIP_ID: i32 = 820;
pub(crate) const IRON_DART_ID: i32 = 807;
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
pub(crate) fn dart_fletcher_scenario() -> Scenario {
    dart_fletcher_variant(
        "dart_fletcher",
        DART_FLETCHER_INJECT,
        1,
        "bronze_dart_tip",
        BRONZE_DART_TIP_ID,
        BRONZE_DART_ID,
        IRON_DART_ID,
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
    )
}

/// No bank. Spam Feather on the selected tip; one action is 10 darts.
fn dart_fletcher_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    level: i32,
    tip_alias: &'static str,
    tip_id: i32,
    product_id: i32,
    wrong_product_id: i32,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: FLETCHING_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: FLETCHING_STAT,
        min: 2,
    };
    let courtyard = LUMBRIDGE_COURTYARD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Fletching and exact dart stacks before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat fletching {level}"));
                cheat(c, &format!("give {tip_alias} 100"));
                cheat(c, "give feather 100");
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
                count: 100,
            },
        ),
        (
            "confirm exact feathers before Start",
            Proof::ItemId {
                id: FEATHER_ID,
                count: 100,
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
                count: 90,
            },
        ),
        (
            "watch exact feathers consumed",
            Proof::ItemIdAtMost {
                id: FEATHER_ID,
                count: 90,
            },
        ),
        (
            "watch further exact product progress",
            Proof::ItemId {
                id: product_id,
                count: 20,
            },
        ),
        (
            "watch no wrong-tier dart product",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
                count: 0,
            },
        ),
        ("watch Fletching XP beyond one dart action", further_xp),
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
