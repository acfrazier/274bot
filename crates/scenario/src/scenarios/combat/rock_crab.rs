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
pub(super) const ROCK_CRAB_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Rock Crab food",
    carry: &[("Lobster", ROCK_CRAB_FOOD as u32)],
}];
const ROCK_CRAB_INJECT: &[ScriptSettingInject] = &[
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
/// The old seeded-cargo route sizing used a calibration from
/// `bank-cell-windows.md`: 1.25 dirty snapshots per 0.6-second game tick.
/// The hostile RockCrab spot `(2704, 3726)` to Seers `(2725, 3491)` is
/// 235 Chebyshev tiles. MossGiant's old 74-tile envelope was 100 dirty
/// snapshots including its booth interaction, so the inferred interaction
/// component was `100 - (74 * 1.25) = 7.5`; RockCrab was estimated at
/// `235 * 1.25 + 7.5 = 301.25`, rounded to the current 320-game-tick cap
/// (192s at 600ms per game tick) for each travel arm.
const ROCK_CRAB_BANK_ROUTE_WATCH_TICKS: u32 = 320;
/// Two 235-tile walks at 0.6 seconds/tile take 282 seconds; the 360-second
/// whole-run deadline leaves 78 seconds for seed/start, the same booth
/// interaction, and close/return evidence without expanding the earned-drop
/// RNG witness. Both 320-game-tick route caps allow 384 seconds together, so
/// the wall can pre-empt them if both reach their maxima. Retain the measured
/// route estimate rather than raising the deadline without live evidence.
const ROCK_CRAB_BANK_SEEDED_DEADLINE: Duration = Duration::from_secs(360);

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
/// RockCrab startup PeriodicBank witness. An uncut Sapphire is seeded in the
/// backpack so the frozen card walks from the field to Seers immediately,
/// deposits that cargo, closes the bank, and returns toward the crabs.
pub(crate) fn rock_crab_bank_seeded_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "rock_crab_bank_seeded",
        "RockCrab",
        ROCK_CRAB_SAFE_STAND,
        2,
        "lobster",
        LOBSTER_ID,
        ROCK_CRAB_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[("uncut_sapphire", UNCUT_SAPPHIRE_ID, 1)],
        &[NOTED_UNCUT_SAPPHIRE_ID, NOTED_CASKET_ID],
        ROCK_CRAB_BANK_INJECT,
        0,
        "",
        0,
        &[
            (
                "watch seeded Sapphire leave the backpack after PeriodicBank deposit",
                Proof::ItemIdAtMost {
                    id: UNCUT_SAPPHIRE_ID,
                    count: 0,
                },
            ),
            (
                "watch seeded Sapphire enter a fresh Seers bank",
                Proof::BankItemId {
                    id: UNCUT_SAPPHIRE_ID,
                    count: 1,
                },
            ),
            (
                "watch seeded RockCrab cargo reach the Seers bank after deposit",
                Proof::ArrivedNear {
                    x: SEERS_BANK.x,
                    z: SEERS_BANK.z,
                    level: SEERS_BANK.level,
                    radius: 6,
                },
            ),
            (
                "watch seeded RockCrab PeriodicBank close",
                Proof::BankClosed,
            ),
            (
                "watch return toward the nearest RockCrab spot after seeded banking",
                Proof::ArrivedNear {
                    x: ROCK_CRAB_BANK_RET.x,
                    z: ROCK_CRAB_BANK_RET.z,
                    level: ROCK_CRAB_BANK_RET.level,
                    radius: 6,
                },
            ),
            (
                "watch fresh Strength XP after the seeded bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    scenario.settings.deadline = ROCK_CRAB_BANK_SEEDED_DEADLINE;
    acknowledge_dormant_rocks_before_start(&mut scenario);
    scenario.settings.fixture_loadouts = Some(ROCK_CRAB_FIXTURE_LOADOUTS);
    for step in &mut scenario.steps {
        if step.name == "watch seeded Sapphire leave the backpack after PeriodicBank deposit"
            || step.name == "watch seeded RockCrab cargo reach the Seers bank after deposit"
            || step.name == "watch return toward the nearest RockCrab spot after seeded banking"
        {
            step.wait.budget_ticks = ROCK_CRAB_BANK_ROUTE_WATCH_TICKS;
        }
    }
    scenario
}
