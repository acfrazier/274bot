use super::*;
const NOTED_HERB_ID: i32 = 200;
pub(crate) const CHAOS_DRUID_FOOD: i32 = 12;
const CHAOS_DRUID_BANK_FOOD: i32 = 8;
const CHAOS_DRUID_FIELD: WorldTile = WorldTile {
    x: 3110,
    z: 9936,
    level: 0,
};
const CHAOS_DRUID_TOWER_FIELD: WorldTile = WorldTile {
    x: 2562,
    z: 3356,
    level: 0,
};
const CHAOS_DRUID_YANILLE_FIELD: WorldTile = WorldTile {
    x: 2580,
    z: 9501,
    level: 0,
};
// Keep the seeded food independent of the operator's saved first loadout.
pub(super) const CHAOS_DRUID_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Chaos Druid food",
    carry: &[("Lobster", 12)],
}];
const CHAOS_DRUID_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Chaos Druid food"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Edgeville Dungeon"),
    },
    ScriptSettingInject {
        id: "combatStyleIndex",
        value: ScriptInjectValue::Str("1"),
    },
];
const CHAOS_DRUID_TOWER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Chaos Druid food"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Chaos Druid Tower"),
    },
    ScriptSettingInject {
        id: "combatStyleIndex",
        value: ScriptInjectValue::Str("1"),
    },
];
const CHAOS_DRUID_YANILLE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Chaos Druid food"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Yanille Dungeon"),
    },
    ScriptSettingInject {
        id: "combatStyleIndex",
        value: ScriptInjectValue::Str("1"),
    },
];
const CHAOS_DRUID_LOOT_EMPTY: &[i32] = &[
    UNIDENTIFIED_GUAM_ID,
    NATURE_RUNE_ID,
    LAW_RUNE_ID,
    NOTED_HERB_ID,
];
/// ChaosDruidKiller Edgeville dungeon core. Own bank is not this cell.
/// Style index 1 on the wielded weapon; lobster x12 so tripPrepared holds.
pub(crate) fn chaos_druid_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "chaos_druid",
        card: "ChaosDruidKiller",
        tele: CHAOS_DRUID_FIELD,
        radius: 14,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: CHAOS_DRUID_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear: None,
        loot_empty: CHAOS_DRUID_LOOT_EMPTY,
        inject: CHAOS_DRUID_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

/// Chaos Druid Tower surface camp. Same Chaos druid identity and Herb/Law/Nature
/// loot as Edgeville; Thieving 46 is the door/approach prerequisite, prepared and
/// acknowledged before Start. Banking is not this cell.
pub(crate) fn chaos_druid_tower_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "chaos_druid_tower",
        card: "ChaosDruidKiller",
        tele: CHAOS_DRUID_TOWER_FIELD,
        radius: 4,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: CHAOS_DRUID_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear: None,
        loot_empty: CHAOS_DRUID_LOOT_EMPTY,
        inject: CHAOS_DRUID_TOWER_INJECT,
        complete_quest: None,
        thieving: 46,
        agility: 0,
    })
}

/// Yanille Dungeon warrior room. Target display is Chaos druid warrior; Agility 40
/// is the room prerequisite. Approach web/ledge is not this cell.
pub(crate) fn chaos_druid_yanille_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "chaos_druid_yanille",
        card: "ChaosDruidKiller",
        tele: CHAOS_DRUID_YANILLE_FIELD,
        radius: 8,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: CHAOS_DRUID_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear: None,
        loot_empty: CHAOS_DRUID_LOOT_EMPTY,
        inject: CHAOS_DRUID_YANILLE_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 40,
    })
}
pub(crate) fn chaos_druid_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "chaos_druid_bank",
        "ChaosDruidKiller",
        CHAOS_DRUID_FIELD,
        14,
        "lobster",
        LOBSTER_ID,
        CHAOS_DRUID_BANK_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        CHAOS_DRUID_LOOT_EMPTY,
        CHAOS_DRUID_INJECT,
        0,
        "lobster",
        12,
        &[
            (
                "watch the prepare-trip restock of exactly twelve Lobster",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: CHAOS_DRUID_FOOD,
                },
            ),
            ("watch the Edgeville bank close", Proof::BankClosed),
            (
                "watch return through the trapdoor into the druid field",
                Proof::ArrivedNear {
                    x: CHAOS_DRUID_FIELD.x,
                    z: CHAOS_DRUID_FIELD.z,
                    level: CHAOS_DRUID_FIELD.level,
                    // Match the script's WalkNear return radius; broad proximity
                    // can pass while the final hop is still cancelled.
                    radius: 4,
                },
            ),
            (
                "watch fresh Strength XP inside the field after the return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    apply_combat_qualification_budget(
        &mut scenario,
        COMBAT_QUALIFICATION_DEADLINE,
        COMBAT_QUALIFICATION_WATCH_TICKS,
    );
    scenario
}
