use super::*;
pub(crate) const EMPTY_VIAL_ID: i32 = 229;
pub(crate) const VIAL_EMPTY_SEED: i32 = 56;
pub(crate) const FALADOR_EAST_BOOTH: WorldTile = WorldTile {
    x: 3013,
    z: 3354,
    level: 0,
};
const FALADOR_FOUNTAIN: WorldTile = WorldTile {
    x: 2949,
    z: 3381,
    level: 0,
};
const VIAL_FILLER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bank",
        value: ScriptInjectValue::Str("Falador West"),
    },
    ScriptSettingInject {
        id: "buyVials",
        value: ScriptInjectValue::Bool(false),
    },
];

const VIAL_FILLER_EAST_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bank",
        value: ScriptInjectValue::Str("Falador East"),
    },
    ScriptSettingInject {
        id: "buyVials",
        value: ScriptInjectValue::Bool(false),
    },
];
pub(crate) fn vial_filler_scenario() -> Scenario {
    vial_filler_variant(
        "vial_filler",
        VIAL_FILLER_INJECT,
        FALADOR_WEST_BANK,
        FALADOR_WEST_BOOTH,
    )
}

pub(crate) fn vial_filler_east_scenario() -> Scenario {
    vial_filler_variant(
        "vial_filler_east",
        VIAL_FILLER_EAST_INJECT,
        FALADOR_EAST_BANK,
        FALADOR_EAST_BOOTH,
    )
}

/// Empty pack at the selected Falador bank. Banked empty vials, no water.
/// Script withdraws, fills at the west fountain, deposits produced water
/// vials, empties the pack of water, restocks empties, returns and fills
/// again. Shop-buy stays pending.
fn vial_filler_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    bank: WorldTile,
    booth: WorldTile,
) -> Scenario {
    let fountain = Proof::ArrivedNear {
        x: FALADOR_FOUNTAIN.x,
        z: FALADOR_FOUNTAIN.z,
        level: FALADOR_FOUNTAIN.level,
        radius: 4,
    };
    let filled = Proof::ItemId {
        id: VIAL_OF_WATER_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed empty vials and tele to the selected Falador bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("givebank vial_empty {VIAL_EMPTY_SEED}"));
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
            "confirm no seeded empty vials in pack before Start",
            Proof::ItemIdAtMost {
                id: EMPTY_VIAL_ID,
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
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(open_seed_booth(
        "open and acknowledge the exact empty-vial seed bank",
        booth,
        Proof::BankItemId {
            id: EMPTY_VIAL_ID,
            count: VIAL_EMPTY_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded water vials in bank",
        Proof::BankItemIdAtMost {
            id: VIAL_OF_WATER_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the Falador fountain after Start",
            fountain,
        ),
        (
            "watch empty vials become water vials at the fountain",
            filled,
        ),
        (
            "watch the withdrawn empty vials finish filling",
            Proof::ItemIdAtMost {
                id: EMPTY_VIAL_ID,
                count: 0,
            },
        ),
        (
            "watch script-created water vials enter a fresh bank",
            Proof::BankItemId {
                id: VIAL_OF_WATER_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of water vials after deposit",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
        (
            "watch a restock of exact empty vials",
            Proof::ItemId {
                id: EMPTY_VIAL_ID,
                count: 1,
            },
        ),
        ("watch the script close its vial bank", Proof::BankClosed),
        (
            "watch return to the Falador fountain after restock",
            fountain,
        ),
        ("watch another exact water vial after restock", filled),
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
        proof: filled,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("VialFiller"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
