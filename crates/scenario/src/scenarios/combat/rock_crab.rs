use super::*;
pub(crate) const ROCK_CRAB_FOOD: i32 = 8;
const CASKET_ID: i32 = 405;
const NOTED_CASKET_ID: i32 = 406;
const NOTED_UNCUT_SAPPHIRE_ID: i32 = 1624;

/// Default source-script reset tile: inside the native visibility window but
/// outside the wake radius, so dormant `Rocks` can be observed before Start.
const ROCK_CRAB_SAFE_STAND: WorldTile = WorldTile {
    x: 2712,
    z: 3707,
    level: 0,
};
const ROCK_CRAB_BANK_RET: WorldTile = WorldTile {
    x: 2710,
    z: 3717,
    level: 0,
};
const ROCK_CRAB_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Rock Crab food",
    carry: &[("Lobster", ROCK_CRAB_FOOD as u32)],
}];
const ROCK_CRAB_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Off"),
    },
];
const ROCK_CRAB_RANGE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Rock Crab food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("range"),
    },
    ScriptSettingInject {
        id: "rangeStyle",
        value: ScriptInjectValue::Str("rapid"),
    },
    ScriptSettingInject {
        id: "bow",
        value: ScriptInjectValue::Str("Maple shortbow"),
    },
    ScriptSettingInject {
        id: "ammo",
        value: ScriptInjectValue::Str("Bronze arrow"),
    },
    ScriptSettingInject {
        id: "ammoWithdraw",
        value: ScriptInjectValue::Num(RANGE_AMMO as f64),
    },
    ScriptSettingInject {
        id: "minStack",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "collectRange",
        value: ScriptInjectValue::Num(12.0),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Off"),
    },
];
const ROCK_CRAB_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Rock Crab food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("melee"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankStrategy",
        value: ScriptInjectValue::Str("Loot count"),
    },
    ScriptSettingInject {
        id: "bankEveryItems",
        value: ScriptInjectValue::Num(1.0),
    },
];
const ROCK_CRAB_LOOT_EMPTY: &[i32] = &[
    UNCUT_SAPPHIRE_ID,
    NOTED_UNCUT_SAPPHIRE_ID,
    CASKET_ID,
    NOTED_CASKET_ID,
];
pub(crate) const ROCK_CRAB_BANK_DEPOSIT: [i32; 2] = [UNCUT_SAPPHIRE_ID, CASKET_ID];
/// RockCrab default melee/strength at spot 1. Ordinary bank policy Off.
/// Catalog requires native Rocks activation into Rock Crab, and the melee
/// fixture arrives already wearing the scoped scimitar: the frozen card's own
/// `GearEquip` refuses a carried melee fixture, so a packed 1331 would fight
/// unarmed (the native pre-Start wear is the proof). Banking is not this cell.
/// SolveClue stays injected off.
pub(crate) fn rock_crab_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
        name: "rock_crab",
        card: "RockCrab",
        tele: ROCK_CRAB_SAFE_STAND,
        radius: 2,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: ROCK_CRAB_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear: Some((
            "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport",
            COMBAT_SCIMITAR_ID,
        )),
        loot_empty: ROCK_CRAB_LOOT_EMPTY,
        inject: ROCK_CRAB_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    });
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat core has a Start step");
    scenario.steps.insert(
        start,
        bank_fletcher_watch(
            "acknowledge dormant Rocks in the supported field before Start",
            Proof::NpcNameNear {
                name: "Rocks",
                x: ROCK_CRAB_SPOT.x,
                z: ROCK_CRAB_SPOT.z,
                level: ROCK_CRAB_SPOT.level,
                radius: 50,
            },
        ),
    );
    scenario
}

pub(crate) fn rock_crab_range_scenario() -> Scenario {
    let mut scenario = combat_range_scenario(
        "rock_crab_range",
        "RockCrab",
        ROCK_CRAB_SAFE_STAND,
        2,
        "lobster",
        LOBSTER_ID,
        ROCK_CRAB_FOOD,
        ROCK_CRAB_RANGE_INJECT,
        true,
    );
    apply_combat_qualification_budget(
        &mut scenario,
        COMBAT_QUALIFICATION_DEADLINE,
        COMBAT_QUALIFICATION_WATCH_TICKS,
    );
    scenario.settings.fixture_loadouts = Some(ROCK_CRAB_FIXTURE_LOADOUTS);
    scenario
}
fn acknowledge_dormant_rocks_before_start(scenario: &mut Scenario) {
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat bank has a Start step");
    scenario.steps.insert(
        start,
        bank_fletcher_watch(
            "acknowledge dormant Rocks in the supported field before Start",
            Proof::NpcNameNear {
                name: "Rocks",
                x: ROCK_CRAB_SPOT.x,
                z: ROCK_CRAB_SPOT.z,
                level: ROCK_CRAB_SPOT.level,
                radius: 50,
            },
        ),
    );
}
/// RockCrab PeriodicBank `Loot count` at Seers. `bankEveryItems=1` ends the
/// trip on the first listed drop (uncut sapphire 1623 or casket 405). The
/// PeriodicBank task deposits matching loot and returns to `currentSpot()`;
/// it does not restock food, so this cell seeds no bank stock at all: the
/// frozen card's `BankRun` (the food-gone withdraw, a different trip) is the
/// only reader of a bank food window and eight lobster outlast the cell.
/// The `Scenario Rock Crab food` loadout pins `scriptFood` to those eight
/// Lobster: unpinned, the card takes the operator's first saved loadout
/// (Swordfish here), counts zero food and walks to Seers before any fight.
pub(crate) fn rock_crab_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "rock_crab_bank",
        "RockCrab",
        ROCK_CRAB_SAFE_STAND,
        2,
        "lobster",
        LOBSTER_ID,
        ROCK_CRAB_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        ROCK_CRAB_LOOT_EMPTY,
        ROCK_CRAB_BANK_INJECT,
        0,
        "",
        0,
        &[
            (
                "watch listed RockCrab loot enter a fresh Seers bank",
                Proof::BankItemIdAny {
                    ids: &ROCK_CRAB_BANK_DEPOSIT,
                    count: 1,
                },
            ),
            (
                "watch the periodic bank at the Seers booth",
                Proof::ArrivedNear {
                    x: SEERS_BANK.x,
                    z: SEERS_BANK.z,
                    level: SEERS_BANK.level,
                    radius: 6,
                },
            ),
            ("watch the periodic bank close", Proof::BankClosed),
            (
                "watch return to the nearest RockCrab spot after banking",
                Proof::ArrivedNear {
                    x: ROCK_CRAB_BANK_RET.x,
                    z: ROCK_CRAB_BANK_RET.z,
                    level: ROCK_CRAB_BANK_RET.level,
                    radius: 6,
                },
            ),
            (
                "watch fresh Strength XP after the bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    acknowledge_dormant_rocks_before_start(&mut scenario);
    scenario.settings.fixture_loadouts = Some(ROCK_CRAB_FIXTURE_LOADOUTS);
    scenario
}
