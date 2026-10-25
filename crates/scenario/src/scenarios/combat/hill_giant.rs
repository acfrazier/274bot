use super::*;
pub(crate) const HILL_GIANT_FOOD: i32 = 8;
const HILL_GIANT_BANK_RESTOCK: i32 = 4;
const HILL_GIANT_PIT: WorldTile = WorldTile {
    x: 3110,
    z: 9832,
    level: 0,
};
pub(super) const HILL_GIANT_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Hill Giant food",
    carry: &[("Trout", HILL_GIANT_FOOD as u32)],
}];
const HILL_GIANT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Hill Giant food"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const HILL_GIANT_LOOT_EMPTY: &[i32] = &[
    BIG_BONES_ID,
    NOTED_BIG_BONES_ID,
    LIMPWURT_ROOT_ID,
    NOTED_LIMPWURT_ROOT_ID,
];
/// HillGiant's always-on trip end, reached on the first loot slot so the cell
/// does not need fourteen giant drops. `meleeStyle`/`buryBones` as the core.
const HILL_GIANT_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Hill Giant food"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "lootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
];
/// Deposit-only trip end with the one loot slot already narrowed: only Big
/// bones may fill it and shared junk is off, so the deposit watch cannot land
/// on a chance Limpwurt root (`giant.rs2:51-52`) or a shared gem
/// (`bankCommonJunk`; `HillGiant.ts:35,43-44,85`).
const HILL_GIANT_LOOT_DEPOSIT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Hill Giant food"),
    },
    ScriptSettingInject {
        id: "meleeStyle",
        value: ScriptInjectValue::Str("strength"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "lootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Big bones"]),
    },
    ScriptSettingInject {
        id: "bankCommonJunk",
        value: ScriptInjectValue::Bool(false),
    },
];
/// HillGiant default melee in the pit. Target display is Giant. The Brass key
/// is prepared because this inside-pit cell does not qualify the key-fetch or
/// entrance branch. Blank weapon. DeathRecovery and banking stay idle.
pub(crate) fn hill_giant_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "hill_giant",
        card: "HillGiant",
        tele: HILL_GIANT_PIT,
        radius: 16,
        food_alias: "trout",
        food_id: TROUT_ID,
        food_count: HILL_GIANT_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[("edgevilledungeonkey", BRASS_KEY_ID, 1)],
        wear: None,
        loot_empty: HILL_GIANT_LOOT_EMPTY,
        inject: HILL_GIANT_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}
/// HillGiant's default loot list (`HillGiant.ts:35` `DEFAULT_LOOT`): the
/// trip-ending drop is whichever of these the card takes first. Big bones are
/// the Giant's guaranteed death drop (`giant.rs2` adds `npc_param(death_drop)`
/// before the random table) and a Limpwurt root lands on the same tile on
/// random 76..86 of 128 (`giant.rs2:51-52`); `LootCorpse` takes the
/// `nearest()` of the two, so either can fill the one loot slot.
const HILL_GIANT_TRIP_LOOT: [i32; 2] = [BIG_BONES_ID, LIMPWURT_ROOT_ID];
/// HillGiant's always-on trip end (`lootSlots=1`): one looted Giant drop ends
/// the trip, Varrock West banks it and withdraws trout back, then the pit
/// fight resumes.
///
/// The looted drop is watched in the pack before the deposit, so each leg has
/// its own ordinary 150-dirty watch. One watch from the first Strength hit to
/// the deposit had to cover the rest of a 35-HP kill and the whole trip:
/// Engine A 2026-10-19 (`FIX-024/combat-bank/diagA3`) measured 118 dirties
/// from the first hit to the looted bones and 113 more to the Varrock West
/// deposit, 231 in total. Both watches accept the card's whole loot class:
/// `r3a-hill_giant_bank` took the Limpwurt root first and carried it to the
/// Varrock West booth, which a Big-bones-only watch failed.
/// Whole-cell cap for this cell only; other cells keep the shared 180 s cap.
/// Completed passes 147.9, 160.3 and 177.3 s (`NIGHTLY-QUAL-1025/report.md`,
/// cell 3): slowest completed pass plus 20 %, rounded up (212.76 -> 213 s).
/// A failed run is censored and does not size it; the watches are unchanged.
const HILL_GIANT_BANK_DEADLINE: Duration = Duration::from_secs(213);

pub(crate) fn hill_giant_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "hill_giant_bank",
        "HillGiant",
        HILL_GIANT_PIT,
        16,
        "trout",
        TROUT_ID,
        HILL_GIANT_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[("edgevilledungeonkey", BRASS_KEY_ID, 1)],
        HILL_GIANT_LOOT_EMPTY,
        HILL_GIANT_BANK_INJECT,
        0,
        "trout",
        12,
        &[
            (
                "watch the first looted Giant drop in pack before the bank trip",
                Proof::ItemIdAny {
                    ids: &HILL_GIANT_TRIP_LOOT,
                    count: 1,
                },
            ),
            (
                "watch the trip's looted drop enter a fresh Varrock West bank",
                Proof::BankItemIdAny {
                    ids: &HILL_GIANT_TRIP_LOOT,
                    count: 1,
                },
            ),
            (
                "watch the restock of Trout to the card's declared twelve",
                Proof::ItemId {
                    id: TROUT_ID,
                    count: HILL_GIANT_FOOD + HILL_GIANT_BANK_RESTOCK,
                },
            ),
            ("watch HillGiant close its bank", Proof::BankClosed),
            (
                "watch return to the giant pit after banking",
                Proof::ArrivedNear {
                    x: HILL_GIANT_PIT.x,
                    z: HILL_GIANT_PIT.z,
                    level: HILL_GIANT_PIT.level,
                    radius: 16,
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
    scenario.settings.deadline = HILL_GIANT_BANK_DEADLINE;
    scenario
}

/// Prepared HillGiant earned-kill full bank. Same source strength/`lootSlots=1`
/// inject as `hill_giant_bank`, but 70-stat Rune armour and no seeded bones or
/// limpwurt. Restock is the card's declared `foodWithdraw` 12. Seeded-cargo
/// upstream bank proof is not this cell.
pub(crate) fn hill_giant_bank_prepared_scenario() -> Scenario {
    let mut scenario = remaining_prepared_combat_bank_scenario(
        "hill_giant_bank_prepared",
        "HillGiant",
        HILL_GIANT_PIT,
        16,
        "trout",
        TROUT_ID,
        HILL_GIANT_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[("edgevilledungeonkey", BRASS_KEY_ID, 1)],
        HILL_GIANT_LOOT_EMPTY,
        HILL_GIANT_BANK_INJECT,
        "trout",
        12,
        REMAINING_COMBAT_PREPARED_LEVEL,
        COMBAT_QUALIFICATION_DEADLINE,
        COMBAT_QUALIFICATION_WATCH_TICKS,
        &[
            (
                "watch earned Giant loot enter a fresh Varrock West bank",
                Proof::BankItemIdAny {
                    ids: &HILL_GIANT_BANK_DEPOSIT,
                    count: 1,
                },
            ),
            (
                "watch the prepared restock of Trout to the card's declared twelve",
                Proof::ItemId {
                    id: TROUT_ID,
                    count: HILL_GIANT_FOOD + HILL_GIANT_BANK_RESTOCK,
                },
            ),
            ("watch prepared HillGiant close its bank", Proof::BankClosed),
            (
                "watch prepared return to the giant pit after banking",
                Proof::ArrivedNear {
                    x: HILL_GIANT_PIT.x,
                    z: HILL_GIANT_PIT.z,
                    level: HILL_GIANT_PIT.level,
                    radius: 16,
                },
            ),
            (
                "watch fresh Strength XP after the prepared bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

/// Deposit-only HillGiant qualification: same pit prep, food/melee/burial rows
/// and combat-first Strength XP as `hill_giant_bank`, with the loot list cut to
/// Big bones, then a script-looted Big bones in pack, then a fresh Varrock West
/// deposit under the ordinary 150-dirty bank watch.
/// Restock, close, return and second-fight watches remain on the frozen full-cycle cell.
pub(crate) fn hill_giant_loot_deposit_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "hill_giant_loot_deposit",
        "HillGiant",
        HILL_GIANT_PIT,
        16,
        "trout",
        TROUT_ID,
        HILL_GIANT_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[("edgevilledungeonkey", BRASS_KEY_ID, 1)],
        HILL_GIANT_LOOT_EMPTY,
        HILL_GIANT_LOOT_DEPOSIT_INJECT,
        0,
        "trout",
        12,
        &[
            (
                "watch looted Big bones in pack before the bank trip",
                Proof::ItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch the trip's Big bones enter a fresh Varrock West bank",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
        ],
    );
    scenario.proof = Proof::BankItemId {
        id: BIG_BONES_ID,
        count: 1,
    };
    scenario
}
pub(crate) const HILL_GIANT_BANK_DEPOSIT: [i32; 2] = [BIG_BONES_ID, LIMPWURT_ROOT_ID];
