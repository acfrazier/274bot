use super::*;
const STEEL_ARROW_ID: i32 = 886;
const BODY_TALISMAN_ID: i32 = 1446;
pub(crate) const BLOOD_RUNE_ID: i32 = 565;
const CHAOS_RUNE_ID: i32 = 562;
/// ArdyFighter sets its combat style only when out of combat after the first
/// cake restock, so the first Guard fight can come before it, and a Guard drop
/// that lands first sends PeriodicBank (`bankEveryItems=1`) on a bank round
/// trip before the style is set (live `ardyfighter-window-r1`). Frozen rs2b0t at
/// `00d39a17` on this cell's fixture (A/S/HP 40, Defence 1, Thieving 5, Adamant
/// scimitar, foodTarget 1, empty food) set Strength at 7.5 s and 45.3 s; strong
/// accounts at 44.6 s and 76.6 s. Worst style time 76.6 s = 128 engine ticks at
/// 600 ms, + 30 for the first Strength hit, + 60 for one market↔East Ardougne
/// booth round trip (~20 tiles each way at one tile per tick, plus open and
/// deposit) = 218 ticks; at 1.25 runner dirties per engine tick (`moss_giant.rs`)
/// that is 273, rounded up to 320.
const ARDY_FIGHTER_BANK_STYLE_WATCH_TICKS: u32 = 320;
/// Seeded bank trip arithmetic uses the frozen Fight's possible pre-bank
/// combat hold: `90 s / 0.6 s = 150` engine ticks, plus the measured
/// market→East Ardougne approach (~23 tiles) and the existing 20-tick
/// open/waitBankReady component. Thus `(150 + 23 + 20) * 1.25 = 241.25`
/// runner dirties, rounded to 320 for the first deposit arm. Return is the
/// measured 20-tile path: `20 * 1.25 = 25`, rounded to 40.
const ARDY_FIGHTER_BANK_SEEDED_DEPOSIT_WATCH_TICKS: u32 = 320;
const ARDY_FIGHTER_BANK_SEEDED_RETURN_WATCH_TICKS: u32 = 40;
/// ArdyFighter's loadout setting only selects gear (`weaponOf` falls back to
/// the worn weapon); its food is ignored because the card eats what it
/// steals from the Baker's stall. The fixture exists so `selectedLoadout`
/// resolves instead of failing closed with no loadouts posted, and its
/// Cake carry mirrors the injected `foodTarget` of 1.
pub(super) const ARDY_FIGHTER_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Ardy Fighter food",
    carry: &[("Cake", 1)],
}];
const ARDY_FIGHTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Ardy Fighter food"),
    },
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "combatStyle",
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
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
];
const ARDY_FIGHTER_LOOT_EMPTY: &[i32] = &[CAKE_ID, BREAD_ID, CHOCOLATE_SLICE_ID, CHOCOLATE_CAKE_ID];
/// Earned bank cell: stall food plus the deposit class, the Guard's Bones
/// and their noted form. `bankEveryItems=1` would fire on pre-Start Bones,
/// so the class stays empty until a kill feeds it.
const ARDY_FIGHTER_BANK_LOOT_EMPTY: &[i32] = &[
    CAKE_ID,
    BREAD_ID,
    CHOCOLATE_SLICE_ID,
    CHOCOLATE_CAKE_ID,
    BONES_ID,
    NOTED_BONES_ID,
];
const ARDY_FIGHTER_BANK_SEEDED_LOOT_EMPTY: &[i32] = &[
    BREAD_ID,
    CHOCOLATE_SLICE_ID,
    CHOCOLATE_CAKE_ID,
    STEEL_ARROW_ID,
    BODY_TALISMAN_ID,
    BLOOD_RUNE_ID,
    CHAOS_RUNE_ID,
    NATURE_RUNE_ID,
];
/// ArdyFighter default Guard/strength. No fabricated cakes; the script
/// must steal cake/bread/chocolate slice after Start. PeriodicBank Off.
pub(crate) fn ardy_fighter_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
        name: "ardy_fighter",
        card: "ArdyFighter",
        tele: ARDOUGNE_GUARD,
        radius: 12,
        food_alias: "cake",
        food_id: CAKE_ID,
        food_count: 0,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear: Some((
            "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport",
            COMBAT_SCIMITAR_ID,
        )),
        loot_empty: ARDY_FIGHTER_LOOT_EMPTY,
        inject: ARDY_FIGHTER_INJECT,
        complete_quest: None,
        thieving: 5,
        agility: 0,
    });
    let style_watch = scenario
        .steps
        .iter_mut()
        .find(|step| step.name == "watch Strength XP from the selected melee style after Start")
        .expect("ardy fighter has a post-Start Strength watch");
    style_watch.wait.budget_ticks = ARDY_FIGHTER_BANK_STYLE_WATCH_TICKS;
    scenario
}
/// ArdyFighter's `bankStrategy=Loot count` PeriodicBank after it has looted a
/// Guard drop. `foodTarget=1` keeps the stall restock short so the cell has
/// room for the loot the bank trip deposits.
const ARDY_FIGHTER_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "combatStyle",
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
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
];
/// Earned-kill variant: the loot list is the Guard's guaranteed death drop
/// alone. Guard death unconditionally adds `npc_param(death_drop)` (Bones)
/// before its random table (content `guard.rs2`), so every kill feeds the
/// `bankEveryItems=1` trip end; the six DEFAULT_LOOT items are random-table
/// outcomes (1-14/128 each) and a cell must not gate on them. This mirrors
/// AutoFighter's Custom Bones bank inject. The seeded sibling keeps the
/// default loot list because its seeded Iron ore is one of those six.
const ARDY_FIGHTER_BANK_EARNED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "combatStyle",
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
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Bones"]),
    },
];
/// ArdyFighter's `bankStrategy=Loot count` trip on an earned kill: the loot
/// list is pinned to the Guard's guaranteed Bones death drop (see
/// `ARDY_FIGHTER_BANK_EARNED_INJECT`), so the first kill feeds the
/// `bankEveryItems=1` trip end deterministically. The PeriodicBank walks to
/// the East Ardougne booth, deposits the Bones and returns to the market
/// anchor for further work. Nothing in the deposit class is prepared:
/// `bankEveryItems=1` would otherwise treat a pre-Start listed item as the
/// trip end. The looted Bones are watched in the pack before the deposit,
/// so the kill and the trip each have their own ordinary watch. The seeded
/// sibling below remains the default-loot proof.
pub(crate) fn ardy_fighter_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "ardy_fighter_bank",
        "ArdyFighter",
        ARDOUGNE_GUARD,
        12,
        "cake",
        CAKE_ID,
        0,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        ARDY_FIGHTER_BANK_LOOT_EMPTY,
        ARDY_FIGHTER_BANK_EARNED_INJECT,
        5,
        "",
        0,
        &[
            (
                "watch the Guard's looted Bones in pack before the bank trip",
                Proof::ItemId {
                    id: BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch a Guard drop enter a fresh East Ardougne bank",
                Proof::BankItemId {
                    id: BONES_ID,
                    count: 1,
                },
            ),
            ("watch the periodic bank close", Proof::BankClosed),
            (
                "watch return to the market anchor after banking",
                Proof::ArrivedNear {
                    x: ARDOUGNE_GUARD.x,
                    z: ARDOUGNE_GUARD.z,
                    level: ARDOUGNE_GUARD.level,
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
    let style_watch = scenario
        .steps
        .iter_mut()
        .find(|step| step.name == "watch Strength XP from the selected melee style after Start")
        .expect("ardy fighter bank has a post-Start Strength watch");
    style_watch.wait.budget_ticks = ARDY_FIGHTER_BANK_STYLE_WATCH_TICKS;
    scenario
}

/// ArdyFighter startup PeriodicBank witness. One DEFAULT_LOOT Guard drop is
/// seeded with one Cake so only PeriodicBank owns the first trip instead of
/// `RestockCakes`; the seeded Iron ore leaves the pack, is banked in East
/// Ardougne, and the script returns to the market before fresh Strength XP.
pub(crate) fn ardy_fighter_bank_seeded_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "ardy_fighter_bank_seeded",
        "ArdyFighter",
        ARDOUGNE_GUARD,
        12,
        "cake",
        CAKE_ID,
        1,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[("iron_ore", IRON_ORE_ID, 1)],
        ARDY_FIGHTER_BANK_SEEDED_LOOT_EMPTY,
        ARDY_FIGHTER_BANK_INJECT,
        5,
        "",
        0,
        &[
            (
                "watch seeded Iron ore leave the pack after PeriodicBank deposit",
                Proof::ItemIdAtMost {
                    id: IRON_ORE_ID,
                    count: 0,
                },
            ),
            (
                "watch seeded Iron ore enter a fresh East Ardougne bank",
                Proof::BankItemId {
                    id: IRON_ORE_ID,
                    count: 1,
                },
            ),
            ("watch seeded PeriodicBank close", Proof::BankClosed),
            (
                "watch return to the market anchor after seeded banking",
                Proof::ArrivedNear {
                    x: ARDOUGNE_GUARD.x,
                    z: ARDOUGNE_GUARD.z,
                    level: ARDOUGNE_GUARD.level,
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
    for step in &mut scenario.steps {
        match step.name {
            "watch seeded Iron ore leave the pack after PeriodicBank deposit" => {
                step.wait.budget_ticks = ARDY_FIGHTER_BANK_SEEDED_DEPOSIT_WATCH_TICKS;
            }
            "watch return to the market anchor after seeded banking" => {
                step.wait.budget_ticks = ARDY_FIGHTER_BANK_SEEDED_RETURN_WATCH_TICKS;
            }
            "watch fresh Strength XP after the seeded bank return" => {
                step.wait.budget_ticks = ARDY_FIGHTER_BANK_STYLE_WATCH_TICKS;
            }
            _ => {}
        }
    }
    scenario
}
