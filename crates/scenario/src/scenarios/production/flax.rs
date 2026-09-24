use super::*;
pub(crate) const FLAX_ID: i32 = 1779;

pub(crate) const FLAX_FIELD: WorldTile = WorldTile {
    x: 2741,
    z: 3444,
    level: 0,
};
const NOTED_FLAX_ID: i32 = 1780;
const NOTED_BOW_STRING_ID: i32 = 1778;
pub(crate) const BALL_OF_WOOL_ID: i32 = 1759;
pub(crate) const FLAX_SPIN_SEED: i32 = 56;
const FLAX_SPINNER_BANK: WorldTile = WorldTile {
    x: 2722,
    z: 3493,
    level: 0,
};
pub(crate) const FLAX_SPINNER_WHEEL: WorldTile = WorldTile {
    x: 2711,
    z: 3471,
    level: 1,
};
/// FlaxAIO BANK_STAND; the spinner booth seed is 2722,3493,0.
const FLAX_AIO_BANK: WorldTile = WorldTile {
    x: 2725,
    z: 3493,
    level: 0,
};
const FLAX_AIO_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "picking",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "spinning",
        value: ScriptInjectValue::Bool(true),
    },
];
const FLAX_AIO_PICK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "picking",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "spinning",
        value: ScriptInjectValue::Bool(false),
    },
];
const FLAX_AIO_SPIN_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "picking",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "spinning",
        value: ScriptInjectValue::Bool(true),
    },
];
pub(crate) fn flax_picker_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "clear the pack and tele to the default Seers flax field",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm no seeded flax in pack before Start",
        Proof::ItemIdAtMost {
            id: FLAX_ID,
            count: 0,
        },
    ));
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch a full pack of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        (
            "watch script-created flax enter a fresh Seers bank",
            Proof::BankItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        ("watch the flax bank close after deposit", Proof::BankClosed),
        (
            "watch return to the flax field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 12,
            },
        ),
        (
            "watch further exact flax after return",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_picker",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: FLAX_ID,
            count: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxPicker"),
            terminal_shot: Some("flax_picker"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
pub(crate) fn flax_spinner_scenario() -> Scenario {
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let bank = FLAX_SPINNER_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed crafting, banked flax, and tele to the Seers flax bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 10");
                cheat(c, &format!("givebank flax {FLAX_SPIN_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Crafting 10 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 10,
            },
        ),
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded ball of wool in pack before Start",
            Proof::ItemIdAtMost {
                id: BALL_OF_WOOL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted flax in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_BOW_STRING_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact flax seed bank",
        Proof::BankItemId {
            id: FLAX_ID,
            count: FLAX_SPIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bow string in bank",
        Proof::BankItemIdAtMost {
            id: BOW_STRING_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted flax in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_FLAX_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the upstairs spinning wheel after Start",
            wheel,
        ),
        (
            "watch Crafting XP from the spinning wheel after Start",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bow string 1777 after Start", product),
        (
            "watch the withdrawn flax finish converting",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "watch script-spun bow string enter a fresh Seers bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bow string after deposit",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "watch a restock of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
        ("watch the script close its spin bank", Proof::BankClosed),
        (
            "watch return to the upstairs spinning wheel after restock",
            wheel,
        ),
        ("watch another exact bow string after restock", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_spinner",
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
            start_script: Some("FlaxSpinner"),
            terminal_shot: Some("flax_spinner"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn flax_aio_empty_pack_confirms() -> Vec<(&'static str, Proof)> {
    vec![
        (
            "confirm Crafting 10 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 10,
            },
        ),
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded ball of wool in pack before Start",
            Proof::ItemIdAtMost {
                id: BALL_OF_WOOL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted flax in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_BOW_STRING_ID,
                count: 0,
            },
        ),
    ]
}

/// Empty pack at the Seers flax field. FlaxAIO own script, both flags on.
/// Pick 1779, climb, makeX 1779→1777 with Crafting XP, deposit strings,
/// closed return to the field, further Pick. Not FlaxPicker+FlaxSpinner.
pub(crate) fn flax_aio_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let flax = Proof::ItemId {
        id: FLAX_ID,
        count: 1,
    };
    let product = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed crafting and tele to the Seers flax field before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 10");
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in flax_aio_empty_pack_confirms() {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch exact flax 1779 from the Seers field after Start",
            flax,
        ),
        (
            "watch arrival at the upstairs spinning wheel after picking",
            wheel,
        ),
        (
            "watch Crafting XP from the spinning wheel after Start",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bow string 1777 after Start", product),
        (
            "watch the picked flax finish converting",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "watch script-spun bow string enter a fresh Seers bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bow string after deposit",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        ("watch the script close its flax bank", Proof::BankClosed),
        (
            "watch return to the flax field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 12,
            },
        ),
        ("watch further exact flax after return", flax),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_aio",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: flax,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxAIO"),
            script_settings_inject: Some(FLAX_AIO_INJECT),
            terminal_shot: Some("flax_aio"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// FlaxAIO pick-only. Full flax pack, deposit 1779, closed return, further
/// Pick. Spun 1777 must not qualify.
pub(crate) fn flax_aio_pick_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "clear the pack and tele to the Seers flax field before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted flax in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_FLAX_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch a full pack of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        (
            "watch script-created flax enter a fresh Seers bank",
            Proof::BankItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        (
            "watch the pack empty of flax after deposit",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        ("watch the flax bank close after deposit", Proof::BankClosed),
        (
            "watch return to the flax field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 12,
            },
        ),
        (
            "watch further exact flax after return",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_aio_pick",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: FLAX_ID,
            count: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxAIO"),
            script_settings_inject: Some(FLAX_AIO_PICK_INJECT),
            terminal_shot: Some("flax_aio_pick"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// FlaxAIO spin-only. Banked flax at FlaxAIO's own booth stand, wheel
/// conversion+XP, string deposit, restock, closed return upstairs, further
/// spin. Wool is not this core.
pub(crate) fn flax_aio_spin_scenario() -> Scenario {
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let bank = FLAX_AIO_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed crafting, banked flax, and tele to FlaxAIO's Seers bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 10");
                cheat(c, &format!("givebank flax {FLAX_SPIN_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in flax_aio_empty_pack_confirms() {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact flax seed bank",
        Proof::BankItemId {
            id: FLAX_ID,
            count: FLAX_SPIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bow string in bank",
        Proof::BankItemIdAtMost {
            id: BOW_STRING_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted flax in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_FLAX_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the upstairs spinning wheel after Start",
            wheel,
        ),
        (
            "watch Crafting XP from the spinning wheel after Start",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bow string 1777 after Start", product),
        (
            "watch the withdrawn flax finish converting",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "watch script-spun bow string enter a fresh Seers bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bow string after deposit",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "watch a restock of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
        ("watch the script close its spin bank", Proof::BankClosed),
        (
            "watch return to the upstairs spinning wheel after restock",
            wheel,
        ),
        ("watch another exact bow string after restock", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_aio_spin",
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
            start_script: Some("FlaxAIO"),
            script_settings_inject: Some(FLAX_AIO_SPIN_INJECT),
            terminal_shot: Some("flax_aio_spin"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
