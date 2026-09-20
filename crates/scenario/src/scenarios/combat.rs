use super::production::*;
use crate::*;

pub(crate) const TROUT_ID: i32 = 333;
pub(crate) const COMBAT_SCIMITAR_ID: i32 = 1331;
const MIND_RUNE_ID: i32 = 558;
pub(crate) const BIG_BONES_ID: i32 = 532;
const NOTED_BIG_BONES_ID: i32 = 533;
pub(crate) const LIMPWURT_ROOT_ID: i32 = 225;
const NOTED_LIMPWURT_ROOT_ID: i32 = 226;
pub(crate) const LAW_RUNE_ID: i32 = 563;
pub(crate) const BONES_ID: i32 = 526;
const NOTED_BONES_ID: i32 = 527;
const NOTED_HERB_ID: i32 = 200;
pub(crate) const CHAOS_DRUID_FOOD: i32 = 12;
pub(crate) const MOSS_GIANT_FOOD: i32 = 10;
pub(crate) const HILL_GIANT_FOOD: i32 = 8;
pub(crate) const AUTO_FIGHTER_FOOD: i32 = 8;
/// Bank-cell preparation: MossGiant only banks once the pack's food is gone, so
/// the bank cell carries a shortfall instead of a full pack; ChaosDruidKiller's
/// own `tripPrepared` needs `foodWithdraw` (12) in the field, so 8 forces its
/// declared `prepare-trip` end.
const MOSS_GIANT_BANK_FOOD: i32 = 2;
const CHAOS_DRUID_BANK_FOOD: i32 = 8;
/// Restock lines the cards themselves withdraw to (MossGiant's declared
/// `foodWithdraw` default 20, AutoFighter's 10, HillGiant's 12).
const MOSS_GIANT_BANK_RESTOCK: i32 = 20;
const AUTO_FIGHTER_BANK_RESTOCK: i32 = 10;
const HILL_GIANT_BANK_RESTOCK: i32 = 4;
const AUTO_FIGHTER_MAGE_LEVEL: i32 = 13;
const AUTO_FIGHTER_MAGE_CASTS: i32 = 150;
const AUTO_FIGHTER_MAGE_AIR_RUNES: i32 = AUTO_FIGHTER_MAGE_CASTS * 2;
const AUTOCAST_MAGIC_VARP: i32 = 108;
const AUTOCAST_ARMED_VALUE: i32 = 3;
const RANGED_STAT: i32 = 4;
const COMBAT_MODE_VARP: i32 = 43;
const RAPID_COMBAT_MODE: i32 = 1;
const MAPLE_SHORTBOW_ID: i32 = 853;
pub(crate) const BRONZE_ARROW_ID: i32 = 882;
const RUNE_ARROW_ID: i32 = 892;
const RANGE_AMMO: i32 = 200;
const MOSS_GIANT_DART_SUPPLY: i32 = 80;
const MOSS_GIANT_DART_BANK_FOOD: i32 = 15;
const MOSS_GIANT_DART_RANGED: i32 = 50;
const MOSS_GIANT_DART_FOOD_WITHDRAW: i32 = 10;
const MOSS_GIANT_DART_FIELD_RADIUS: i32 = 12;
const CAMELOT_TELE_MAGIC: i32 = 45;
const CAMELOT_TELE_AIR: i32 = 5;
const CAMELOT_TELE_LAW: i32 = 1;
const CAMELOT_TELE_STOCK: i32 = 2;
const CAMELOT_AIR_CARRY: i32 = CAMELOT_TELE_AIR * (CAMELOT_TELE_STOCK + 1);
const CAMELOT_LAW_CARRY: i32 = CAMELOT_TELE_LAW * (CAMELOT_TELE_STOCK + 1);
/// Spare bank stock so the keep-list deposit can withdraw teleStock+1 again.
const CAMELOT_BANK_AIR: i32 = 20;
const CAMELOT_BANK_LAW: i32 = 5;
pub(crate) const ROCK_CRAB_FOOD: i32 = 8;
const GREEN_DRAGON_BASE_FOOD: i32 = 20;
const GREEN_DRAGON_FOOD: i32 = 12;
const GREEN_DRAGON_BANK_PREPARED_FOOD: i32 = 26;
const GREEN_DRAGON_BANK_PREPARED_RESTOCK: i32 = 27;
const FIRE_GIANT_FOOD: i32 = 12;
pub(crate) const FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD: i32 = 1;
pub(crate) const FIRE_GIANT_BANK_PREPARED_RESTOCK: i32 = 25;
pub(crate) const COMBAT_ATTACK_LEVEL: i32 = 40;
const REMAINING_COMBAT_PREPARED_LEVEL: i32 = 70;
const BANK_PRESSURE_PREPARED_LEVEL: i32 = 99;
const COMBAT_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(300);
/// Runner dirty-snapshot increments, not engine ticks or wall seconds. Live
/// combat receipts run near two dirty increments/second; 750 keeps the named
/// 300s wall as the controlling bound instead of the ordinary 150-dirty arm.
const COMBAT_QUALIFICATION_WATCH_TICKS: u32 = 750;
/// Measured GreenDragon pressure reached its loaded deposit 268s after the
/// post-Start baseline, before the roughly 320-tile return and resumed combat.
const GREEN_DRAGON_BANK_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(600);
/// Default-table loot fills more pack slots than the explicit inject sibling;
/// measured pressure trip + return needs a longer wall while keeping 2.5 dirty/s.
const GREEN_DRAGON_BANK_DEFAULT_PREPARED_QUALIFICATION_DEADLINE: Duration =
    Duration::from_secs(720);
const GREEN_DRAGON_BANK_DEFAULT_PREPARED_QUALIFICATION_WATCH_TICKS: u32 = 1800;
/// The measured GreenDragon teleport/Edgeville/return path predicts 475.2s
/// including fresh post-return XP and route margin.
const GREEN_DRAGON_TELE_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(480);
const GREEN_DRAGON_TELE_QUALIFICATION_WATCH_TICKS: u32 = 1200;
/// The pinned GreenDragon source only enters `Escape` when it is both foodless
/// and below `panicHp`. At 70/99 HP and a configured 98% threshold the branch
/// is deterministic while retaining enough absolute health for the south run.
const GREEN_DRAGON_TELE_PREPARED_LEVEL: i32 = 99;
const GREEN_DRAGON_TELE_PREPARED_HP: i32 = 70;
const GREEN_DRAGON_TELE_PANIC_PERCENT: i32 = 98;
/// FireGiant's one-food trigger plus barrel exit, bank leg, Waterfall re-entry
/// and fresh post-return XP is estimated at 259.4s before margin.
const FIRE_GIANT_BANK_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(600);
/// Keep the same 2.5 dirty increments/second allowance as the 300s/750 arm.
const BANK_QUALIFICATION_WATCH_TICKS: u32 = 1500;
pub(crate) const RUNE_SCIMITAR_ID: i32 = 1333;
pub(crate) const DRAGONFIRE_SHIELD_ID: i32 = 1540;
const DEFENCE_STAT: i32 = 1;
const RUNE_PLATELEGS_ID: i32 = 1079;
const RUNE_FULL_HELM_ID: i32 = 1163;
const DRAGON_DAGGER_ID: i32 = 1215;
const DRAGON_DAGGER_ATTACK_LEVEL: i32 = 60;
const SPECIAL_ENERGY_VARP: i32 = 300;
const DRAGON_DAGGER_SPECIAL_COST: i32 = 250;
const SUPER_ATTACK_3_ID: i32 = 145;
const SUPER_ATTACK_2_ID: i32 = 147;
const SUPER_STRENGTH_3_ID: i32 = 157;
const SUPER_STRENGTH_2_ID: i32 = 159;
pub(crate) const BRASS_KEY_ID: i32 = 983;
pub(crate) const DRAGON_BONES_ID: i32 = 536;
const NOTED_DRAGON_BONES_ID: i32 = 537;
pub(crate) const GREEN_DRAGONHIDE_ID: i32 = 1753;
const NOTED_GREEN_DRAGONHIDE_ID: i32 = 1754;
pub(crate) const GLARIALS_AMULET_ID: i32 = 295;
pub(crate) const ROPE_ID: i32 = 954;
const CASKET_ID: i32 = 405;
const NOTED_CASKET_ID: i32 = 406;
const NOTED_UNCUT_SAPPHIRE_ID: i32 = 1624;
const STEEL_ARROW_ID: i32 = 886;
const BODY_TALISMAN_ID: i32 = 1446;
pub(crate) const BLOOD_RUNE_ID: i32 = 565;
const CHAOS_RUNE_ID: i32 = 562;
/// The six verifiable Guard drops ArdyFighter lists in DEFAULT_LOOT
/// (`iron ore, steel arrow, body talisman, blood/chaos/nature rune`).
/// Its bank cell starts empty of this class.
pub(crate) const GUARD_DROP_IDS: [i32; 6] = [
    IRON_ORE_ID,
    STEEL_ARROW_ID,
    BODY_TALISMAN_ID,
    BLOOD_RUNE_ID,
    CHAOS_RUNE_ID,
    NATURE_RUNE_ID,
];

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
const MOSS_GIANT_SAFESPOT: WorldTile = WorldTile {
    x: 2553,
    z: 3406,
    level: 0,
};
const MOSS_GIANT_BANK: WorldTile = WorldTile {
    x: 2615,
    z: 3332,
    level: 0,
};
const HILL_GIANT_PIT: WorldTile = WorldTile {
    x: 3110,
    z: 9832,
    level: 0,
};
const ROCK_CRAB_SPOT: WorldTile = WorldTile {
    x: 2704,
    z: 3726,
    level: 0,
};
/// Default source-script reset tile: inside the native visibility window but
/// outside the wake radius, so dormant `Rocks` can be observed before Start.
const ROCK_CRAB_SAFE_STAND: WorldTile = WorldTile {
    x: 2712,
    z: 3707,
    level: 0,
};
const GREEN_DRAGON_FIELD: WorldTile = WorldTile {
    x: 3096,
    z: 3814,
    level: 0,
};
pub(crate) const FIRE_GIANT_ROOM: WorldTile = WorldTile {
    x: 2575,
    z: 9893,
    level: 0,
};
pub(crate) const FIRE_GIANT_RAFT: WorldTile = WorldTile {
    x: 2510,
    z: 3493,
    level: 0,
};
pub(crate) const FIRE_GIANT_WASH: WorldTile = WorldTile {
    x: 2527,
    z: 3413,
    level: 0,
};
const FIRE_GIANT_BANK: WorldTile = WorldTile {
    x: 2616,
    z: 3332,
    level: 0,
};
pub(crate) const VARROCK_TELE_LAND: WorldTile = WorldTile {
    x: 3213,
    z: 3424,
    level: 0,
};
const ROCK_CRAB_BANK_RET: WorldTile = WorldTile {
    x: 2710,
    z: 3717,
    level: 0,
};
const SEERS_BANK: WorldTile = WorldTile {
    x: 2725,
    z: 3491,
    level: 0,
};
const CAMELOT_TELE_LAND: WorldTile = WorldTile {
    x: 2757,
    z: 3478,
    level: 0,
};
pub(crate) const FIRE_RUNE_ID: i32 = 554;
pub(crate) const VARROCK_TELE_MAGIC: i32 = 25;
const VARROCK_TELE_LAW: i32 = 3;
const VARROCK_TELE_AIR: i32 = 9;
const VARROCK_TELE_FIRE: i32 = 3;
const GREEN_DRAGON_BANK_RESTOCK: i32 = 20;
const FIRE_GIANT_BANK_RESTOCK: i32 = 20;

// Keep the seeded food independent of the operator's saved first loadout.
const CHAOS_DRUID_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Chaos Druid food",
    carry: &[("Lobster", 12)],
}];
const MOSS_GIANT_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Moss Giant food",
    carry: &[("Lobster", MOSS_GIANT_FOOD as u32)],
}];
const HILL_GIANT_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Hill Giant food",
    carry: &[("Trout", HILL_GIANT_FOOD as u32)],
}];
const GREEN_DRAGON_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[
    FixtureLoadout {
        name: "Scenario Green Dragon food",
        carry: &[("Lobster", GREEN_DRAGON_BASE_FOOD as u32)],
    },
    FixtureLoadout {
        name: "Scenario Green Dragon trip food",
        carry: &[("Lobster", GREEN_DRAGON_FOOD as u32)],
    },
];
const FIRE_GIANT_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Fire Giant food",
    carry: &[("Lobster", FIRE_GIANT_FOOD as u32)],
}];

fn combat_fixture_loadouts(card: &str) -> Option<&'static [FixtureLoadout]> {
    match card {
        "ChaosDruidKiller" => Some(CHAOS_DRUID_FIXTURE_LOADOUTS),
        "MossGiant" => Some(MOSS_GIANT_FIXTURE_LOADOUTS),
        "HillGiant" => Some(HILL_GIANT_FIXTURE_LOADOUTS),
        "GreenDragon" => Some(GREEN_DRAGON_FIXTURE_LOADOUTS),
        "FireGiant" => Some(FIRE_GIANT_FIXTURE_LOADOUTS),
        _ => None,
    }
}

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
const MOSS_GIANT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Moss Giant food"),
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
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const MOSS_GIANT_DART_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("range"),
    },
    ScriptSettingInject {
        id: "bow",
        value: ScriptInjectValue::Str("Bronze dart"),
    },
    ScriptSettingInject {
        id: "ammo",
        value: ScriptInjectValue::Str("Rune arrow"),
    },
    ScriptSettingInject {
        id: "ammoWithdraw",
        value: ScriptInjectValue::Num(MOSS_GIANT_DART_SUPPLY as f64),
    },
    ScriptSettingInject {
        id: "rangeStyle",
        value: ScriptInjectValue::Str("rapid"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(MOSS_GIANT_DART_FOOD_WITHDRAW as f64),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
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
const AUTO_FIGHTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "food",
        value: ScriptInjectValue::Str("Trout"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_FOOD as f64),
    },
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "spot",
        value: ScriptInjectValue::Str("Start position"),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("None"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
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
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const AUTO_FIGHTER_MAGE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "spot",
        value: ScriptInjectValue::Str("Start position"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("mage"),
    },
    ScriptSettingInject {
        id: "spell",
        value: ScriptInjectValue::Str("Fire Strike"),
    },
    ScriptSettingInject {
        id: "runesWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_MAGE_CASTS as f64),
    },
    ScriptSettingInject {
        id: "food",
        value: ScriptInjectValue::Str("Trout"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_FOOD as f64),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("None"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const AUTO_FIGHTER_RANGE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "spot",
        value: ScriptInjectValue::Str("Start position"),
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
        id: "ammo",
        value: ScriptInjectValue::Str("Bronze arrow"),
    },
    ScriptSettingInject {
        id: "ammoWithdraw",
        value: ScriptInjectValue::Num(RANGE_AMMO as f64),
    },
    ScriptSettingInject {
        id: "food",
        value: ScriptInjectValue::Str("Trout"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_FOOD as f64),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("None"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
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
const GREEN_DRAGON_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon food"),
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
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_MAGE_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
    },
    ScriptSettingInject {
        id: "combatStyle",
        value: ScriptInjectValue::Str("mage"),
    },
    ScriptSettingInject {
        id: "spell",
        value: ScriptInjectValue::Str("Fire Strike"),
    },
    ScriptSettingInject {
        id: "staff",
        value: ScriptInjectValue::Str("Staff of fire"),
    },
    ScriptSettingInject {
        id: "runesWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_MAGE_CASTS as f64),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_SPECIAL_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
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
        id: "useSpecial",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Dragon dagger"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_POTIONS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
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
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const FIRE_GIANT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Fire Giant food"),
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
        id: "escapeTele",
        value: ScriptInjectValue::Str("Barrel (free)"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];
const FIRE_GIANT_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Fire Giant food"),
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
        id: "escapeTele",
        value: ScriptInjectValue::Str("Barrel (free)"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
];
const FIRE_GIANT_BANK_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Fire Giant food"),
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
        id: "escapeTele",
        value: ScriptInjectValue::Str("Barrel (free)"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(FIRE_GIANT_BANK_PREPARED_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Big bones"]),
    },
];
const FIRE_GIANT_CAMELOT_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Fire Giant food"),
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
        id: "escapeTele",
        value: ScriptInjectValue::Str("Camelot"),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(FIRE_GIANT_BANK_PREPARED_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Big bones"]),
    },
];
const ROCK_CRAB_BANK_INJECT: &[ScriptSettingInject] = &[
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
const GREEN_DRAGON_TELE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
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
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Teleport to Varrock"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_TELE_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
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
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Teleport to Varrock"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "panicHp",
        value: ScriptInjectValue::Num(GREEN_DRAGON_TELE_PANIC_PERCENT as f64),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const ARDY_FIGHTER_INJECT: &[ScriptSettingInject] = &[
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
const CHAOS_DRUID_LOOT_EMPTY: &[i32] = &[
    UNIDENTIFIED_GUAM_ID,
    NATURE_RUNE_ID,
    LAW_RUNE_ID,
    NOTED_HERB_ID,
];
const MOSS_GIANT_LOOT_EMPTY: &[i32] = &[BIG_BONES_ID, NOTED_BIG_BONES_ID];
const HILL_GIANT_LOOT_EMPTY: &[i32] = &[
    BIG_BONES_ID,
    NOTED_BIG_BONES_ID,
    LIMPWURT_ROOT_ID,
    NOTED_LIMPWURT_ROOT_ID,
];
const AUTO_FIGHTER_LOOT_EMPTY: &[i32] = &[BONES_ID, NOTED_BONES_ID];
const ROCK_CRAB_LOOT_EMPTY: &[i32] = &[
    UNCUT_SAPPHIRE_ID,
    NOTED_UNCUT_SAPPHIRE_ID,
    CASKET_ID,
    NOTED_CASKET_ID,
];
const GREEN_DRAGON_LOOT_EMPTY: &[i32] = &[
    DRAGON_BONES_ID,
    NOTED_DRAGON_BONES_ID,
    GREEN_DRAGONHIDE_ID,
    NOTED_GREEN_DRAGONHIDE_ID,
];
const FIRE_GIANT_LOOT_EMPTY: &[i32] = &[BIG_BONES_ID, NOTED_BIG_BONES_ID];
const ARDY_FIGHTER_LOOT_EMPTY: &[i32] = &[CAKE_ID, BREAD_ID, CHOCOLATE_SLICE_ID, CHOCOLATE_CAKE_ID];
/// Bank cell: stall food plus the deposit class. `bankEveryItems=1` would
/// fire on a pre-Start listed Guard drop, so the class stays empty until a
/// kill feeds it.
const ARDY_FIGHTER_BANK_LOOT_EMPTY: &[i32] = &[
    CAKE_ID,
    BREAD_ID,
    CHOCOLATE_SLICE_ID,
    CHOCOLATE_CAKE_ID,
    IRON_ORE_ID,
    STEEL_ARROW_ID,
    BODY_TALISMAN_ID,
    BLOOD_RUNE_ID,
    CHAOS_RUNE_ID,
    NATURE_RUNE_ID,
];

/// Shared melee-core seed: legal stats, food, ordinary weapon, empty loot,
/// then Start. Banking policy is explicit in the inject; these cells are not
/// bank-roundtrip proof. Scenario watch is selected-style XP; catalog adds
/// two engagements, a verified defeat, and exact loot where required.
struct CombatCorePlan {
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_alias: &'static str,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    /// Pre-Start wear step: the step's own name plus the item id its
    /// `Proof::EquipmentId` arm acknowledges. The label belongs to the item
    /// ("Dragonfire shield", "Adamant scimitar"), so it stays accurate when
    /// the helper is reused outside the shield cells.
    wear: Option<(&'static str, i32)>,
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    /// The native quest prerequisite completed before the stat/inventory
    /// reset and the hostile-field teleport (never a fabricated stage: the
    /// quest's own completion script runs and its journal row is
    /// acknowledged before Start).
    complete_quest: Option<NativeQuestPrereq>,
    thieving: i32,
    agility: i32,
}

#[derive(Clone, Copy)]
struct PreparedCombatPlan {
    level: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    wear: &'static [(&'static str, i32)],
}

const TIER40_RUNE_ARMOUR_GIVE: &[(&str, i32, i32)] = &[
    ("rune_chainbody", RUNE_CHAINBODY_ID, 1),
    ("rune_platelegs", RUNE_PLATELEGS_ID, 1),
    ("rune_full_helm", RUNE_FULL_HELM_ID, 1),
];

const TIER40_RUNE_ARMOUR_WEAR: &[(&str, i32)] = &[
    (
        "wear and acknowledge Rune chainbody before hostile-field teleport",
        RUNE_CHAINBODY_ID,
    ),
    (
        "wear and acknowledge Rune platelegs before hostile-field teleport",
        RUNE_PLATELEGS_ID,
    ),
    (
        "wear and acknowledge Rune full helm before hostile-field teleport",
        RUNE_FULL_HELM_ID,
    ),
];

fn combat_core_scenario(plan: CombatCorePlan) -> Scenario {
    combat_core_scenario_with_preparation(plan, None)
}

fn prepared_combat_core_scenario(
    plan: CombatCorePlan,
    preparation: PreparedCombatPlan,
) -> Scenario {
    let mut scenario = combat_core_scenario_with_preparation(plan, Some(preparation));
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

fn apply_combat_qualification_budget(
    scenario: &mut Scenario,
    deadline: Duration,
    watch_ticks: u32,
) {
    scenario.settings.deadline = deadline;
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat qualification has a Start step");
    for step in &mut scenario.steps[start + 1..] {
        step.wait.budget_ticks = step.wait.budget_ticks.max(watch_ticks);
    }
}

fn combat_core_scenario_with_preparation(
    plan: CombatCorePlan,
    preparation: Option<PreparedCombatPlan>,
) -> Scenario {
    let CombatCorePlan {
        name,
        card,
        tele,
        radius,
        food_alias,
        food_id,
        food_count,
        weapon_alias,
        weapon_id,
        extra_give,
        wear,
        loot_empty,
        inject,
        complete_quest,
        thieving,
        agility,
    } = plan;
    let xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    let combat_level = preparation
        .map(|prepared| prepared.level)
        .unwrap_or(COMBAT_ATTACK_LEVEL);
    if let Some(prereq) = complete_quest {
        steps.extend(quest_prereq_steps(prereq));
    }
    steps.push(Step {
        name: "prepare melee stats, food and gear on the safe tile before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setstat attack {combat_level}"));
                cheat(c, &format!("setstat strength {combat_level}"));
                cheat(c, &format!("setstat hitpoints {combat_level}"));
                if let Some(preparation) = preparation {
                    cheat(c, &format!("setstat defence {}", preparation.level));
                }
                if thieving > 0 {
                    cheat(c, &format!("setstat thieving {thieving}"));
                }
                if agility > 0 {
                    cheat(c, &format!("setstat agility {agility}"));
                }
                cheat(c, "~clearinv");
                cheat(c, &format!("give {weapon_alias} 1"));
                if food_count > 0 {
                    cheat(c, &format!("give {food_alias} {food_count}"));
                }
                for &(alias, _, count) in extra_give {
                    cheat(c, &format!("give {alias} {count}"));
                }
                if let Some(preparation) = preparation {
                    for &(alias, _, count) in preparation.extra_give {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: 0,
                min: combat_level,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge prepared Attack profile",
        Proof::Stat {
            id: 0,
            min: combat_level,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge prepared Strength profile",
        Proof::Stat {
            id: STRENGTH_STAT,
            min: combat_level,
        },
    ));
    if let Some(preparation) = preparation {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Defence profile",
            Proof::Stat {
                id: DEFENCE_STAT,
                min: preparation.level,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge prepared Hitpoints profile",
        Proof::Stat {
            id: 3,
            min: combat_level,
        },
    ));
    if thieving > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: thieving,
            },
        ));
    }
    if agility > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Agility before Start",
            Proof::Stat {
                id: AGILITY_STAT,
                min: agility,
            },
        ));
    }
    if food_count > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared food in pack before Start",
            Proof::ItemId {
                id: food_id,
                count: food_count,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge prepared weapon before Start",
        Proof::ItemId {
            id: weapon_id,
            count: 1,
        },
    ));
    for &(_, id, count) in extra_give {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared extra gear before Start",
            Proof::ItemId { id, count },
        ));
    }
    if let Some(preparation) = preparation {
        for &(_, id, count) in preparation.extra_give {
            steps.push(bank_fletcher_watch(
                "acknowledge prepared tier-40 armour before Start",
                Proof::ItemId { id, count },
            ));
        }
    }
    if let Some((label, id)) = wear {
        steps.push(wear_combat_item_step(label, id));
    }
    if let Some(preparation) = preparation {
        for &(label, id) in preparation.wear {
            steps.push(wear_combat_item_step(label, id));
        }
    }
    for &id in loot_empty {
        steps.push(bank_fletcher_watch(
            "confirm no seeded combat loot in pack before Start",
            Proof::ItemIdAtMost { id, count: 0 },
        ));
    }
    steps.push(Step {
        name: "teleport into the hostile field only after preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(tele.level, tele.x, tele.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch Strength XP from the selected melee style after Start",
        xp,
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(card),
            script_settings_inject: Some(inject),
            fixture_loadouts: combat_fixture_loadouts(card),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// A quest prerequisite the fixture completes through the **native** debug
/// quest-journal command `~quest` (`_test/scripts/cheats/cheat_quest.rs2`
/// `@debug_quests`): "Select Individual Quest." → the quest's row on the
/// paginated `p_choice5_header` list → "Complete.", which queues that
/// quest's own `[queue,<quest>_complete]` script. The path is targeted: the
/// required quest's completion is the only script queued, so no *other*
/// completion's rewards can arrive after Start (the demonstrated
/// `~completequests` leak: `QuestDone(Waterfall)` returned while ~60 queued
/// completions still owed stats/items/dialogs).
///
/// `dialog` is the quest's `quest_names_enum` label
/// (`content/scripts/general/configs/quest.enum`: index 34 `Lost City`,
/// index 50 `Waterfall Quest`) — the text the native list renders as
/// `<index>. <label>`; `journal` is the quest-tab row the native
/// `~send_quest_complete` helper paints green
/// (`content/scripts/player/interfaces/questlist.if` `[zanaris]`/`[waterfall]`,
/// `content/scripts/general/scripts/quests.rs2`), the acknowledgement the
/// fixture waits for.
///
/// The acknowledgement is the completion boundary itself, verified in clean
/// native content: `quest_zanaris.rs2` `[queue,zanaris_quest_complete]` sets
/// `%zanaris = ^zanaris_complete` (the var `levelrequire_zanaris_quest` gates
/// `opheld2 dragon_dagger` on, `levelrequire/scripts/levelrequire.rs2`) and
/// then calls `~send_quest_complete(questlist:zanaris, …)`; the FireGiant
/// counterpart `[queue,waterfall_quest_complete]`
/// (`quest_waterfall/scripts/quest_waterfall.rs2`) banks 2 diamonds, 2 gold
/// bars, 40 mithril seeds and 137,500 attack + strength XP before the same
/// helper. Every reward statement precedes the green, and `send_quest_complete`
/// is the last call in both scripts, so a fixture that waits for the green and
/// then resets stats/inventory cannot have quest rewards land after Start.
#[derive(Clone, Copy)]
pub(crate) struct NativeQuestPrereq {
    dialog: &'static str,
    journal: &'static str,
}

/// Waterfall Quest (`quest_names_enum` 50, journal row `[waterfall]`) — the
/// FireGiant dungeon entry's requirement.
pub(crate) const WATERFALL_QUEST_PREREQ: NativeQuestPrereq = NativeQuestPrereq {
    dialog: "Waterfall Quest",
    journal: "Waterfall Quest",
};

/// Lost City Of Zanaris (`quest_names_enum` 34, journal row `[zanaris]`):
/// `levelrequire_zanaris_quest_attack(60, last_slot)` gates
/// `opheld2 dragon_dagger` (`content/scripts/levelrequire/scripts/tier60.rs2`,
/// `content/scripts/levelrequire/scripts/levelrequire.rs2`).
pub(crate) const LOST_CITY_PREREQ: NativeQuestPrereq = NativeQuestPrereq {
    dialog: "Lost City",
    journal: "Lost City",
};

/// Shilo Village (`quest_names_enum` 43, journal row `[zombiequeen]` text
/// `Shilo Village`): `%zombiequeen >= ^zombiequeen_complete` (15) is the
/// wooden-gate membership (`quest_zombiequeen.rs2` `[oploc1,_shilo_woodengate]`).
pub(super) const SHILO_VILLAGE_PREREQ: NativeQuestPrereq = NativeQuestPrereq {
    dialog: "Shilo Village",
    journal: "Shilo Village",
};

/// The native quest-journal command and the labels of its individual-quest
/// path: the first dialog's "Select Individual Quest." branch button, the
/// paginated list's "Next." button and the per-quest "Complete."
/// confirmation. The labels are the dialog's own resume-button texts, so
/// the machine presses a button by what it says, never by page arithmetic.
const QUEST_JOURNAL_CHEAT: &str = "~quest";
const QUEST_SELECT_INDIVIDUAL: &str = "Select Individual Quest.";
const QUEST_PAGE_NEXT: &str = "Next.";
const QUEST_COMPLETE_LABEL: &str = "Complete.";

/// The dialog this machine has already answered: the chat modal it was
/// rendered in and the option texts it carried. `chat.rs2` reuses one
/// interface per dialog shape (`multi4`, `multi5`, `multi3`) and
/// re-registers the *same* components as resume buttons for the question
/// that follows (`if_addresumebutton(multi5:com_5)`), and the engine
/// resumes a paused script with the clicked component
/// (`IfButtonHandler` → `p_pausebutton` → `last_com`). A press repeated
/// into the replaced dialog is therefore not inert — the page list's
/// `Next.` would advance a second page. One press per distinct dialog;
/// the step then waits for the next one, like the player it stands in for.
#[derive(Default)]
pub(crate) struct QuestJournalState {
    answered: Option<(i32, String)>,
}

/// One tick of the native quest-journal dialog machine: continue a
/// `BUTTON_CONTINUE` chat IF (the completion's level-ups), else answer the
/// option dialog the individual-quest path waits on (Select Individual
/// Quest. → the required quest's page row → Complete.), else close the
/// modal a completion opened (the quest scroll `send_quest_complete` opens
/// just before it paints the row green). Returns `false` when an option
/// dialog cannot be placed: the preparation step fails explicitly instead
/// of guessing a button.
pub(crate) fn answer_quest_journal_dialogs(
    client: &mut Client,
    snapshot: &GameSnapshot,
    prereq: NativeQuestPrereq,
    state: &mut QuestJournalState,
) -> bool {
    if snapshot.chat_continue_component_id() != -1 {
        let mut ix = Interactions::new(snapshot, client);
        return matches!(
            ix.continue_dialog(),
            SendResult::Sent { .. } | SendResult::Refused { .. }
        );
    }
    if !snapshot.chat_options().is_empty() {
        let identity = (
            snapshot.modals().chat,
            snapshot
                .chat_options()
                .iter()
                .map(|option| option.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        if state.answered.as_ref() == Some(&identity) {
            // Already answered this dialog; the next one has not landed.
            return true;
        }
        let choose = |wanted: &dyn Fn(&str) -> bool| {
            snapshot
                .chat_options()
                .iter()
                .position(|option| wanted(option.text.as_str()))
        };
        let choice = choose(&|text| text == QUEST_SELECT_INDIVIDUAL)
            .or_else(|| choose(&|text| text == QUEST_COMPLETE_LABEL))
            .or_else(|| choose(&|text| text.ends_with(prereq.dialog)))
            .or_else(|| choose(&|text| text == QUEST_PAGE_NEXT));
        let Some(choice) = choice else {
            return false;
        };
        let mut ix = Interactions::new(snapshot, client);
        return match ix.answer_choice(choice as i32 + 1) {
            SendResult::Sent { .. } => {
                state.answered = Some(identity);
                true
            }
            // Nothing pressable in that snapshot: retry on the next tick.
            SendResult::Refused { .. } => true,
        };
    }
    let m = snapshot.modals();
    if m.main == -1 && m.side == -1 && m.chat == -1 && m.tutorial == -1 {
        return true;
    }
    let mut ix = Interactions::new(snapshot, client);
    matches!(
        ix.close_modal(),
        SendResult::Sent { .. } | SendResult::Refused { .. }
    )
}

/// Pre-Start native quest prerequisite: send `~quest`, answer its
/// individual-quest dialogs until the required quest's "Complete." queued
/// that quest's own completion script, then drain the completion's
/// scroll/level-up dialogs until the native helper paints the quest's
/// journal row green. That green row is the completion boundary: the stat
/// and inventory reset, the prepared-gear acknowledgements and the
/// hostile-field teleport all follow it, so no quest reward can arrive
/// after Start. The step's tick budget bounds a preparation that never
/// acknowledges — the run fails before Start instead of starting without
/// the prerequisite.
pub(super) fn quest_prereq_steps(prereq: NativeQuestPrereq) -> [Step; 2] {
    // The machine is re-entered once per tick by the runner's `Repeat`
    // arm; its "already answered this dialog" state is behind a mutex so
    // the step closure stays `Fn + Send + Sync` (the `StepKind` bound).
    let state = std::sync::Mutex::new(QuestJournalState::default());
    [
        Step {
            name: "open the native quest-journal prerequisite dialog before Start",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, QUEST_JOURNAL_CHEAT);
                    true
                }),
            },
            wait: Wait {
                arm: Proof::ChatChoice,
                budget_ticks: 20,
            },
        },
        Step {
            name: "answer the quest-journal dialogs until the required quest is acknowledged",
            kind: StepKind::Repeat {
                send: Box::new(move |c, snapshot| {
                    let Ok(mut state) = state.lock() else {
                        return false;
                    };
                    answer_quest_journal_dialogs(c, snapshot, prereq, &mut state)
                }),
            },
            wait: Wait {
                arm: Proof::QuestDone {
                    name: prereq.journal,
                },
                budget_ticks: 600,
            },
        },
    ]
}

pub(super) fn wear_combat_item_step(name: &'static str, id: i32) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).wear(id),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::EquipmentId { id },
            budget_ticks: 200,
        },
    }
}

/// Select and acknowledge the native aggressive melee style after wielding.
pub(crate) fn is_aggressive_combat_style(label: &str) -> bool {
    label.to_ascii_lowercase().contains("aggressive")
}

pub(super) fn select_strength_combat_style_step() -> Step {
    Step {
        name: "select and acknowledge native Strength combat style before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                let Some(root) = snapshot
                    .side_tabs()
                    .iter()
                    .find(|tab| tab.index == 0 && tab.available)
                    .map(|tab| tab.root_component_id)
                else {
                    return false;
                };
                let Some(style) =
                    api::query::widget_search::combat_style_labels(snapshot, root, 43)
                        .into_iter()
                        .find(|style| style.mode == 1 && is_aggressive_combat_style(&style.label))
                else {
                    return false;
                };
                let ctx = ReadContext::new(snapshot);
                let Some(widget) = ctx.component(style.component_id) else {
                    return false;
                };
                matches!(
                    Interactions::new(snapshot, c).press(widget),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::VarpExact { id: 43, value: 1 },
            budget_ticks: 200,
        },
    }
}

/// Prepared bow/quiver branch. The frozen script owns combat-mode selection,
/// target engagement, ammunition use and (for RockCrab) projectile recovery.
#[allow(clippy::too_many_arguments)]
fn combat_range_scenario(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    inject: &'static [ScriptSettingInject],
    dormant_rocks: bool,
) -> Scenario {
    let ranged_xp = Proof::StatXpGain {
        id: RANGED_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare Ranged stats, food, bow and arrows on the safe tile before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setstat ranged {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "~clearinv");
                cheat(c, "give maple_shortbow 1");
                cheat(c, &format!("give bronze_arrow {RANGE_AMMO}"));
                cheat(c, &format!("give {food_alias} {food_count}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: RANGED_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "acknowledge prepared Hitpoints before Start",
            Proof::Stat {
                id: 3,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "acknowledge prepared range food before Start",
            Proof::ItemId {
                id: food_id,
                count: food_count,
            },
        ),
        (
            "acknowledge Maple shortbow before wielding",
            Proof::ItemId {
                id: MAPLE_SHORTBOW_ID,
                count: 1,
            },
        ),
        (
            "acknowledge exact Bronze arrow stack before equipping",
            Proof::ItemId {
                id: BRONZE_ARROW_ID,
                count: RANGE_AMMO,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(wear_combat_item_step(
        "wield and acknowledge Maple shortbow before hostile-field teleport",
        MAPLE_SHORTBOW_ID,
    ));
    steps.push(wear_combat_item_step(
        "equip and acknowledge Bronze arrows before hostile-field teleport",
        BRONZE_ARROW_ID,
    ));
    steps.push(Step {
        name: "teleport into the ranged field only after preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(tele.level, tele.x, tele.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius,
            },
            budget_ticks: 200,
        },
    });
    if dormant_rocks {
        steps.push(bank_fletcher_watch(
            "acknowledge dormant Rocks in the supported field before Start",
            Proof::NpcNameNear {
                name: "Rocks",
                x: ROCK_CRAB_SPOT.x,
                z: ROCK_CRAB_SPOT.z,
                level: ROCK_CRAB_SPOT.level,
                radius: 50,
            },
        ));
    }
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch the frozen script select rapid ranged mode",
        Proof::Varp {
            id: COMBAT_MODE_VARP,
            min: RAPID_COMBAT_MODE,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch Ranged XP from actual ammunition combat after Start",
        ranged_xp,
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: ranged_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(card),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

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

/// MossGiant default melee at the safespot. Big bones 532 is catalog loot.
/// DeathRecovery stays idle. Banking is not this cell.
pub(crate) fn moss_giant_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "moss_giant",
        card: "MossGiant",
        tele: MOSS_GIANT_SAFESPOT,
        radius: 10,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: MOSS_GIANT_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear: None,
        loot_empty: MOSS_GIANT_LOOT_EMPTY,
        inject: MOSS_GIANT_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

/// Prepared MossGiant melee core. Original Defence-1 `moss_giant` stays
/// unchanged; this sibling uses the directed 70-stat Rune kit and the same
/// empty Big-bones guard plus source melee/strength inject. Fight-first
/// banking policy is not this cell.
pub(crate) fn moss_giant_prepared_scenario() -> Scenario {
    let mut scenario = prepared_combat_core_scenario(
        CombatCorePlan {
            name: "moss_giant_prepared",
            card: "MossGiant",
            tele: MOSS_GIANT_SAFESPOT,
            radius: 10,
            food_alias: "lobster",
            food_id: LOBSTER_ID,
            food_count: MOSS_GIANT_FOOD,
            weapon_alias: "rune_scimitar",
            weapon_id: RUNE_SCIMITAR_ID,
            extra_give: &[],
            wear: Some((
                "wield and acknowledge the prepared Rune scimitar before the hostile-field teleport",
                RUNE_SCIMITAR_ID,
            )),
            loot_empty: MOSS_GIANT_LOOT_EMPTY,
            inject: MOSS_GIANT_INJECT,
            complete_quest: None,
            thieving: 0,
            agility: 0,
        },
        PreparedCombatPlan {
            level: REMAINING_COMBAT_PREPARED_LEVEL,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        },
    );
    apply_combat_qualification_budget(
        &mut scenario,
        COMBAT_QUALIFICATION_DEADLINE,
        COMBAT_QUALIFICATION_WATCH_TICKS,
    );
    scenario
}

/// Bank-only MossGiant dart option. Pack and worn start empty; 806x80 and
/// 15 Lobster exist only in the still-open Ardougne North bank. The unused
/// Rune-arrow setting must stay absent. This is not combat_range_scenario.
pub(crate) fn moss_giant_dart_scenario() -> Scenario {
    let ranged_xp = Proof::StatXpGain {
        id: RANGED_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "clear pack, worn, and bank so darts start bank-only",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                cheat(c, "~clearinv inv");
                cheat(c, "~clearinv worn");
                cheat(c, "~clearbank");
                cheat(c, &format!("setstat ranged {MOSS_GIANT_DART_RANGED}"));
                cheat(c, &format!("setstat defence {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(
                    c,
                    &format!("givebank bronze_dart {MOSS_GIANT_DART_SUPPLY}"),
                );
                cheat(
                    c,
                    &format!("givebank lobster {MOSS_GIANT_DART_BANK_FOOD}"),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: RANGED_STAT,
                min: MOSS_GIANT_DART_RANGED,
            },
            budget_ticks: 200,
        },
    });
    for (name, arm) in [
        (
            "acknowledge dart Defence 40 before Start",
            Proof::Stat {
                id: DEFENCE_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "acknowledge dart Hitpoints 40 before Start",
            Proof::Stat {
                id: 3,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "acknowledge empty pack of Bronze darts before Start",
            Proof::ItemIdAtMost {
                id: BRONZE_DART_ID,
                count: 0,
            },
        ),
        (
            "acknowledge empty pack of Rune arrows before Start",
            Proof::ItemIdAtMost {
                id: RUNE_ARROW_ID,
                count: 0,
            },
        ),
        (
            "acknowledge empty pack of Lobster before Start",
            Proof::ItemIdAtMost {
                id: LOBSTER_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(Step {
        name: "teleport to Ardougne North with the bank-only dart fixture",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(MOSS_GIANT_BANK.level, MOSS_GIANT_BANK.x, MOSS_GIANT_BANK.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: MOSS_GIANT_BANK.x,
                z: MOSS_GIANT_BANK.z,
                level: MOSS_GIANT_BANK.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "open the Ardougne North booth so bank-only dart stock is visible",
        kind: StepKind::Repeat {
            send: Box::new(|c, snapshot| {
                if snapshot.bank_component_id() >= 0 && snapshot.bank_loaded() {
                    return true;
                }
                match Interactions::new(snapshot, c).open_nearest_booth() {
                    SendResult::Sent { .. } => true,
                    SendResult::Refused {
                        reason:
                            SendReason::SceneUnavailable
                            | SendReason::OffScene
                            | SendReason::StaleTarget,
                        ..
                    } => true,
                    SendResult::Refused { .. } => false,
                }
            }),
        },
        wait: Wait {
            arm: Proof::BankItemId {
                id: BRONZE_DART_ID,
                count: MOSS_GIANT_DART_SUPPLY,
            },
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    });
    for (name, arm) in [
        (
            "acknowledge banked Lobster 15 while the booth stays open",
            Proof::BankItemId {
                id: LOBSTER_ID,
                count: MOSS_GIANT_DART_BANK_FOOD,
            },
        ),
        (
            "acknowledge no Rune arrows in the open bank",
            Proof::BankItemIdAtMost {
                id: RUNE_ARROW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch the script withdraw and wear Bronze darts",
            Proof::EquipmentId {
                id: BRONZE_DART_ID,
            },
        ),
        (
            "watch travel to the moss-giant safespot after the dart withdraw",
            Proof::ArrivedNear {
                x: MOSS_GIANT_SAFESPOT.x,
                z: MOSS_GIANT_SAFESPOT.z,
                level: MOSS_GIANT_SAFESPOT.level,
                radius: MOSS_GIANT_DART_FIELD_RADIUS,
            },
        ),
        (
            "watch a Moss giant in the dart field",
            Proof::NpcNameNear {
                name: "Moss giant",
                x: MOSS_GIANT_SAFESPOT.x,
                z: MOSS_GIANT_SAFESPOT.z,
                level: MOSS_GIANT_SAFESPOT.level,
                radius: MOSS_GIANT_DART_FIELD_RADIUS,
            },
        ),
        (
            "watch the frozen script select rapid ranged mode",
            Proof::Varp {
                id: COMBAT_MODE_VARP,
                min: RAPID_COMBAT_MODE,
            },
        ),
        ("watch Ranged XP from worn Bronze darts after Start", ranged_xp),
        (
            "watch the unused Rune-arrow setting stay absent from the pack",
            Proof::ItemIdAtMost {
                id: RUNE_ARROW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    let mut scenario = Scenario {
        name: "moss_giant_dart",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: ranged_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: COMBAT_QUALIFICATION_DEADLINE,
            start_script: Some("MossGiant"),
            script_settings_inject: Some(MOSS_GIANT_DART_INJECT),
            fixture_loadouts: None,
            terminal_shot: Some("moss_giant_dart"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    };
    apply_combat_qualification_budget(
        &mut scenario,
        COMBAT_QUALIFICATION_DEADLINE,
        COMBAT_QUALIFICATION_WATCH_TICKS,
    );
    scenario
}

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

/// AutoFighter Guard at Start position. banking=None, clues/special off.
/// Gem-table loot is not required. DeathRecovery stays idle.
pub(crate) fn auto_fighter_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "auto_fighter",
        card: "AutoFighter",
        tele: ARDOUGNE_GUARD,
        radius: 8,
        food_alias: "trout",
        food_id: TROUT_ID,
        food_count: AUTO_FIGHTER_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[],
        wear: None,
        loot_empty: AUTO_FIGHTER_LOOT_EMPTY,
        inject: AUTO_FIGHTER_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

/// AutoFighter's supported Fire Strike branch. Staff and exact cast supplies
/// are prepared and acknowledged on a safe tile before the Guard teleport.
/// Post-Start arms require native autocast state, Magic XP, and both paid rune
/// types to be consumed; the catalog witness adds death and further-combat proof.
pub(crate) fn auto_fighter_mage_scenario() -> Scenario {
    let magic_xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare Fire Strike stats, food, staff and runes on the safe tile",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, &format!("setstat magic {AUTO_FIGHTER_MAGE_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "~clearinv");
                cheat(c, "give staff_of_fire 1");
                cheat(c, &format!("give trout {AUTO_FIGHTER_FOOD}"));
                cheat(c, &format!("give mindrune {AUTO_FIGHTER_MAGE_CASTS}"));
                cheat(c, &format!("give airrune {AUTO_FIGHTER_MAGE_AIR_RUNES}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: MAGIC_STAT,
                min: AUTO_FIGHTER_MAGE_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (name, arm) in [
        (
            "acknowledge prepared Hitpoints 40 before Start",
            Proof::Stat {
                id: 3,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "acknowledge eight Trout before Start",
            Proof::ItemId {
                id: TROUT_ID,
                count: AUTO_FIGHTER_FOOD,
            },
        ),
        (
            "acknowledge 150 Mind runes before Start",
            Proof::ItemId {
                id: MIND_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_CASTS,
            },
        ),
        (
            "acknowledge 300 Air runes before Start",
            Proof::ItemId {
                id: AIR_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_AIR_RUNES,
            },
        ),
        (
            "acknowledge Staff of fire before wielding",
            Proof::ItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(Step {
        name: "wield and acknowledge Staff of fire before hostile-field teleport",
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).wear(STAFF_OF_FIRE_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::EquipmentId {
                id: STAFF_OF_FIRE_ID,
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "teleport to the Ardougne Guard only after mage preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(ARDOUGNE_GUARD.level, ARDOUGNE_GUARD.x, ARDOUGNE_GUARD.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: ARDOUGNE_GUARD.x,
                z: ARDOUGNE_GUARD.z,
                level: ARDOUGNE_GUARD.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch native Fire Strike autocast become armed",
            Proof::Varp {
                id: AUTOCAST_MAGIC_VARP,
                min: AUTOCAST_ARMED_VALUE,
            },
        ),
        ("watch Magic XP from real Fire Strike combat", magic_xp),
        (
            "watch a Mind rune consumed by Fire Strike",
            Proof::ItemIdAtMost {
                id: MIND_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_CASTS - 1,
            },
        ),
        (
            "watch two Air runes consumed by Fire Strike",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_AIR_RUNES - 2,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "auto_fighter_mage",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: magic_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("AutoFighter"),
            script_settings_inject: Some(AUTO_FIGHTER_MAGE_INJECT),
            terminal_shot: Some("auto_fighter_mage"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn auto_fighter_range_scenario() -> Scenario {
    combat_range_scenario(
        "auto_fighter_range",
        "AutoFighter",
        ARDOUGNE_GUARD,
        6,
        "trout",
        TROUT_ID,
        AUTO_FIGHTER_FOOD,
        AUTO_FIGHTER_RANGE_INJECT,
        false,
    )
}

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
    scenario
}

/// GreenDragon melee in the wilderness field. Shield 1540 is worn with the
/// native fixture operation on the safe tile before hostile-field teleport;
/// this does not qualify the frozen GearEquip branch. Catalog Start baseline
/// and cycle both require worn 1540 plus real dragon combat and exact bones 536
/// or green hide 1753. Escape/bank is not this cell.
pub(crate) fn green_dragon_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
        name: "green_dragon",
        card: "GreenDragon",
        tele: GREEN_DRAGON_FIELD,
        radius: 22,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: GREEN_DRAGON_BASE_FOOD,
        weapon_alias: "rune_scimitar",
        weapon_id: RUNE_SCIMITAR_ID,
        extra_give: &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        wear: Some((
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        )),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    })
}

pub(crate) fn green_dragon_prepared_scenario() -> Scenario {
    prepared_combat_core_scenario(
        CombatCorePlan {
            name: "green_dragon_prepared",
            card: "GreenDragon",
            tele: GREEN_DRAGON_FIELD,
            radius: 22,
            food_alias: "lobster",
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_BASE_FOOD,
            weapon_alias: "rune_scimitar",
            weapon_id: RUNE_SCIMITAR_ID,
            extra_give: &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
            wear: Some((
                "wear and acknowledge Dragonfire shield before hostile-field teleport",
                DRAGONFIRE_SHIELD_ID,
            )),
            loot_empty: GREEN_DRAGON_LOOT_EMPTY,
            inject: GREEN_DRAGON_INJECT,
            complete_quest: None,
            thieving: 0,
            agility: 0,
        },
        PreparedCombatPlan {
            level: COMBAT_ATTACK_LEVEL,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        },
    )
}

/// Prepared GreenDragon Fire Strike option. Exact AutoFighter mage rune
/// constants stay intact. No rune armour or scimitar; shield stays worn.
pub(crate) fn green_dragon_mage_prepared_scenario() -> Scenario {
    let magic_xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "prepare 70-stat Fire Strike kit and shield on the safe tile",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &format!("setstat magic {REMAINING_COMBAT_PREPARED_LEVEL}"),
                );
                cheat(
                    c,
                    &format!("setstat defence {REMAINING_COMBAT_PREPARED_LEVEL}"),
                );
                cheat(
                    c,
                    &format!("setstat hitpoints {REMAINING_COMBAT_PREPARED_LEVEL}"),
                );
                cheat(c, "~clearinv");
                cheat(c, "give staff_of_fire 1");
                cheat(c, "give antidragonbreathshield 1");
                cheat(c, &format!("give lobster {GREEN_DRAGON_FOOD}"));
                cheat(c, &format!("give mindrune {AUTO_FIGHTER_MAGE_CASTS}"));
                cheat(c, &format!("give airrune {AUTO_FIGHTER_MAGE_AIR_RUNES}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: MAGIC_STAT,
                min: REMAINING_COMBAT_PREPARED_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (name, arm) in [
        (
            "acknowledge prepared Defence 70 before Start",
            Proof::Stat {
                id: DEFENCE_STAT,
                min: REMAINING_COMBAT_PREPARED_LEVEL,
            },
        ),
        (
            "acknowledge prepared Hitpoints 70 before Start",
            Proof::Stat {
                id: 3,
                min: REMAINING_COMBAT_PREPARED_LEVEL,
            },
        ),
        (
            "acknowledge exact lobster 12 before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: GREEN_DRAGON_FOOD,
            },
        ),
        (
            "acknowledge 150 Mind runes before Start",
            Proof::ItemId {
                id: MIND_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_CASTS,
            },
        ),
        (
            "acknowledge 300 Air runes before Start",
            Proof::ItemId {
                id: AIR_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_AIR_RUNES,
            },
        ),
        (
            "acknowledge Staff of fire before wielding",
            Proof::ItemId {
                id: STAFF_OF_FIRE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge Dragonfire shield before wearing",
            Proof::ItemId {
                id: DRAGONFIRE_SHIELD_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    for (name, id) in [
        (
            "wield and acknowledge Staff of fire before hostile-field teleport",
            STAFF_OF_FIRE_ID,
        ),
        (
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        ),
    ] {
        steps.push(wear_combat_item_step(name, id));
    }
    steps.push(Step {
        name: "teleport into the hostile field only after preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(
                    c,
                    &tele_args(
                        GREEN_DRAGON_FIELD.level,
                        GREEN_DRAGON_FIELD.x,
                        GREEN_DRAGON_FIELD.z,
                    ),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: GREEN_DRAGON_FIELD.x,
                z: GREEN_DRAGON_FIELD.z,
                level: GREEN_DRAGON_FIELD.level,
                radius: 22,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch native Fire Strike autocast become armed",
            Proof::Varp {
                id: AUTOCAST_MAGIC_VARP,
                min: AUTOCAST_ARMED_VALUE,
            },
        ),
        ("watch Magic XP from real Fire Strike combat", magic_xp),
        (
            "watch a Mind rune consumed by Fire Strike",
            Proof::ItemIdAtMost {
                id: MIND_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_CASTS - 1,
            },
        ),
        (
            "watch two Air runes consumed by Fire Strike",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: AUTO_FIGHTER_MAGE_AIR_RUNES - 2,
            },
        ),
        (
            "watch the Dragonfire shield stay worn during mage combat",
            Proof::EquipmentId {
                id: DRAGONFIRE_SHIELD_ID,
            },
        ),
        (
            "watch a Green dragon in the wilderness field",
            Proof::NpcNameNear {
                name: "Green dragon",
                x: GREEN_DRAGON_FIELD.x,
                z: GREEN_DRAGON_FIELD.z,
                level: GREEN_DRAGON_FIELD.level,
                radius: 22,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    let mut scenario = Scenario {
        name: "green_dragon_mage_prepared",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: magic_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: COMBAT_QUALIFICATION_DEADLINE,
            start_script: Some("GreenDragon"),
            script_settings_inject: Some(GREEN_DRAGON_MAGE_PREPARED_INJECT),
            fixture_loadouts: combat_fixture_loadouts("GreenDragon"),
            terminal_shot: Some("green_dragon_mage_prepared"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    };
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    apply_combat_qualification_budget(
        &mut scenario,
        COMBAT_QUALIFICATION_DEADLINE,
        COMBAT_QUALIFICATION_WATCH_TICKS,
    );
    scenario
}

fn green_dragon_special_scenario_with_preparation(
    name: &'static str,
    preparation: Option<PreparedCombatPlan>,
) -> Scenario {
    let prepared_level = preparation.map(|prepared| prepared.level);
    let plan = CombatCorePlan {
        name,
        card: "GreenDragon",
        tele: GREEN_DRAGON_FIELD,
        radius: 22,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: GREEN_DRAGON_FOOD,
        weapon_alias: "dragon_dagger",
        weapon_id: DRAGON_DAGGER_ID,
        extra_give: &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        wear: Some((
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        )),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_SPECIAL_INJECT,
        complete_quest: Some(LOST_CITY_PREREQ),
        thieving: 0,
        agility: 0,
    };
    let mut scenario = if let Some(preparation) = preparation {
        prepared_combat_core_scenario(plan, preparation)
    } else {
        combat_core_scenario(plan)
    };
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat core has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [
            Step {
                name: "prepare and acknowledge the Dragon-dagger Attack profile before Start",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        let attack = prepared_level.unwrap_or(DRAGON_DAGGER_ATTACK_LEVEL);
                        cheat(c, &format!("setstat attack {attack}"));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Stat {
                        id: 0,
                        min: prepared_level.unwrap_or(DRAGON_DAGGER_ATTACK_LEVEL),
                    },
                    budget_ticks: 200,
                },
            },
            wear_combat_item_step(
                "wield and acknowledge Dragon dagger before hostile-field teleport",
                DRAGON_DAGGER_ID,
            ),
            // The card only arms when `Special.energy() >= cost`, so the pool
            // has to already cover the dagger's 250 before Start; a low pool
            // must fail this acknowledgement instead of passing unarmed.
            bank_fletcher_watch(
                "acknowledge the worn dagger's special cost is covered before Start",
                Proof::Varp {
                    id: SPECIAL_ENERGY_VARP,
                    min: DRAGON_DAGGER_SPECIAL_COST,
                },
            ),
        ],
    );
    if preparation.is_some() {
        apply_combat_qualification_budget(
            &mut scenario,
            COMBAT_QUALIFICATION_DEADLINE,
            COMBAT_QUALIFICATION_WATCH_TICKS,
        );
    }
    scenario
}

pub(crate) fn green_dragon_special_scenario() -> Scenario {
    green_dragon_special_scenario_with_preparation("green_dragon_special", None)
}

pub(crate) fn green_dragon_special_prepared_scenario() -> Scenario {
    green_dragon_special_scenario_with_preparation(
        "green_dragon_special_prepared",
        Some(PreparedCombatPlan {
            level: REMAINING_COMBAT_PREPARED_LEVEL,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        }),
    )
}

fn green_dragon_potions_scenario_with_preparation(
    name: &'static str,
    preparation: Option<PreparedCombatPlan>,
) -> Scenario {
    let plan = CombatCorePlan {
        name,
        card: "GreenDragon",
        tele: GREEN_DRAGON_FIELD,
        radius: 22,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: GREEN_DRAGON_FOOD,
        weapon_alias: "rune_scimitar",
        weapon_id: RUNE_SCIMITAR_ID,
        extra_give: &[
            ("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1),
            ("3dose2attack", SUPER_ATTACK_3_ID, 1),
            ("3dose2strength", SUPER_STRENGTH_3_ID, 1),
        ],
        wear: Some((
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        )),
        loot_empty: GREEN_DRAGON_LOOT_EMPTY,
        inject: GREEN_DRAGON_POTIONS_INJECT,
        complete_quest: None,
        thieving: 0,
        agility: 0,
    };
    let attack_boost_min = preparation
        .as_ref()
        .map(|prep| prep.level + 1)
        .unwrap_or(COMBAT_ATTACK_LEVEL + 1);
    let mut scenario = if let Some(preparation) = preparation {
        prepared_combat_core_scenario(plan, preparation)
    } else {
        combat_core_scenario(plan)
    };
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat core has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [
            bank_fletcher_watch(
                "confirm no seeded Super attack(2) flask before Start",
                Proof::ItemIdAtMost {
                    id: SUPER_ATTACK_2_ID,
                    count: 0,
                },
            ),
            bank_fletcher_watch(
                "confirm no seeded Super strength(2) flask before Start",
                Proof::ItemIdAtMost {
                    id: SUPER_STRENGTH_2_ID,
                    count: 0,
                },
            ),
        ],
    );
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat core has a Start step");
    scenario.steps.splice(
        start + 1..start + 1,
        [
            bank_fletcher_watch(
                "watch the Super attack(3) dose leave the pack after Start",
                Proof::ItemIdAtMost {
                    id: SUPER_ATTACK_3_ID,
                    count: 0,
                },
            ),
            bank_fletcher_watch(
                "watch the Super attack(2) dose enter the pack after Start",
                Proof::ItemId {
                    id: SUPER_ATTACK_2_ID,
                    count: 1,
                },
            ),
            bank_fletcher_watch(
                "watch the native Super attack boost before further combat",
                Proof::Stat {
                    id: 0,
                    min: attack_boost_min,
                },
            ),
        ],
    );
    if preparation.is_some() {
        apply_combat_qualification_budget(
            &mut scenario,
            COMBAT_QUALIFICATION_DEADLINE,
            COMBAT_QUALIFICATION_WATCH_TICKS,
        );
    }
    scenario
}

pub(crate) fn green_dragon_potions_scenario() -> Scenario {
    green_dragon_potions_scenario_with_preparation("green_dragon_potions", None)
}

pub(crate) fn green_dragon_potions_prepared_scenario() -> Scenario {
    green_dragon_potions_scenario_with_preparation(
        "green_dragon_potions_prepared",
        Some(PreparedCombatPlan {
            level: REMAINING_COMBAT_PREPARED_LEVEL,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        }),
    )
}

/// FireGiant melee already inside the east dungeon room. Approach, barrel
/// escape and bank are unqualified. Waterfall Quest plus amulet 295 and
/// rope 954 use existing allowed preparation. Frozen `GearEquip` refuses a
/// carried melee fixture, so the cell wears 1331 before the hostile tele;
/// setstat 1→40 queues level-up continues that must drain before Start.
pub(crate) fn fire_giant_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
        name: "fire_giant",
        card: "FireGiant",
        tele: FIRE_GIANT_ROOM,
        radius: 10,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: FIRE_GIANT_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[
            ("glarials_amulet_waterfall_quest", GLARIALS_AMULET_ID, 1),
            ("rope", ROPE_ID, 1),
        ],
        wear: Some((
            "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport",
            COMBAT_SCIMITAR_ID,
        )),
        loot_empty: FIRE_GIANT_LOOT_EMPTY,
        inject: FIRE_GIANT_INJECT,
        complete_quest: Some(WATERFALL_QUEST_PREREQ),
        thieving: 0,
        agility: 0,
    });
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

pub(crate) fn fire_giant_prepared_scenario() -> Scenario {
    prepared_combat_core_scenario(
        CombatCorePlan {
            name: "fire_giant_prepared",
            card: "FireGiant",
            tele: FIRE_GIANT_ROOM,
            radius: 10,
            food_alias: "lobster",
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_alias: "rune_scimitar",
            weapon_id: RUNE_SCIMITAR_ID,
            extra_give: &[
                ("glarials_amulet_waterfall_quest", GLARIALS_AMULET_ID, 1),
                ("rope", ROPE_ID, 1),
            ],
            wear: Some((
                "wield and acknowledge the prepared Rune scimitar before the hostile-field teleport",
                RUNE_SCIMITAR_ID,
            )),
            loot_empty: FIRE_GIANT_LOOT_EMPTY,
            inject: FIRE_GIANT_PREPARED_INJECT,
            complete_quest: Some(WATERFALL_QUEST_PREREQ),
            thieving: 0,
            agility: 0,
        },
        PreparedCombatPlan {
            level: COMBAT_ATTACK_LEVEL,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        },
    )
}

/// ArdyFighter default Guard/strength. No fabricated cakes; the script
/// must steal cake/bread/chocolate slice after Start. PeriodicBank Off.
pub(crate) fn ardy_fighter_scenario() -> Scenario {
    combat_core_scenario(CombatCorePlan {
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
        wear: None,
        loot_empty: ARDY_FIGHTER_LOOT_EMPTY,
        inject: ARDY_FIGHTER_INJECT,
        complete_quest: None,
        thieving: 5,
        agility: 0,
    })
}

/// `banking=Auto` on AutoFighter: BankRun walks to the nearest bank from the
/// anchor, deposits everything its keep-list does not hold and restocks food.
/// Custom `loot=Bones` uses the Guard's guaranteed drop with burial disabled.
/// `bankAtLootSlots=1` ends the trip on that script-looted drop, without
/// depending on a random secondary drop or seeding deposit-class items.
const AUTO_FIGHTER_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "food",
        value: ScriptInjectValue::Str("Trout"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(AUTO_FIGHTER_BANK_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "spot",
        value: ScriptInjectValue::Str("Start position"),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("Auto"),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Bones"]),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
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
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
];

const GREEN_DRAGON_BANK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
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
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
];
const GREEN_DRAGON_BANK_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
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
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "foodReserve",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_PREPARED_FOOD as f64),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_PREPARED_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Dragon bones", "Dragonhide"]),
    },
];
/// Same prepared inventory-pressure loadout as `green_dragon_bank_prepared`,
/// but the `loot` setting is omitted so the generated Green default
/// (`DROP_DB["Green dragon"]` minus `Bass`) is the catalog that must earn
/// both guaranteed drops. Existing accepted inject stays untouched.
const GREEN_DRAGON_BANK_DEFAULT_PREPARED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Green Dragon trip food"),
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
        id: "useSpecial",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "usePotions",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "escape",
        value: ScriptInjectValue::Str("Flee to bank"),
    },
    ScriptSettingInject {
        id: "foodReserve",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_PREPARED_FOOD as f64),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(GREEN_DRAGON_BANK_PREPARED_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "buryBones",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "weapon",
        value: ScriptInjectValue::Str("Rune scimitar"),
    },
    ScriptSettingInject {
        id: "shield",
        value: ScriptInjectValue::Str("Dragonfire shield"),
    },
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

/// Shared bank-cell fixture: the same safe-tile preparation and acknowledgement
/// order as `combat_core_scenario`, plus the acknowledged bank stock the trip
/// has to draw from, the scoped weapon itself (seeded, acknowledged carried,
/// then worn — never a card stash: the card's own deposit must not be able to
/// stash it), and the bank/return watch chain after Start.
#[allow(clippy::too_many_arguments)]
fn combat_bank_scenario(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_alias: &'static str,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    thieving: i32,
    bank_alias: &'static str,
    bank_food_count: i32,
    watches: &[(&'static str, Proof)],
) -> Scenario {
    combat_bank_scenario_with_preparation(
        name,
        card,
        tele,
        radius,
        food_alias,
        food_id,
        food_count,
        weapon_alias,
        weapon_id,
        extra_give,
        loot_empty,
        inject,
        thieving,
        bank_alias,
        bank_food_count,
        watches,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn combat_bank_scenario_with_preparation(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_alias: &'static str,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    thieving: i32,
    bank_alias: &'static str,
    bank_food_count: i32,
    watches: &[(&'static str, Proof)],
    preparation: Option<PreparedCombatPlan>,
) -> Scenario {
    let xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    let combat_level = preparation
        .map(|prepared| prepared.level)
        .unwrap_or(COMBAT_ATTACK_LEVEL);
    steps.push(Step {
        name: "prepare melee stats, trip stock and bank stock on the safe tile before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &format!("setstat attack {combat_level}"));
                cheat(c, &format!("setstat strength {combat_level}"));
                cheat(c, &format!("setstat hitpoints {combat_level}"));
                if let Some(preparation) = preparation {
                    cheat(c, &format!("setstat defence {}", preparation.level));
                }
                if thieving > 0 {
                    cheat(c, &format!("setstat thieving {thieving}"));
                }
                cheat(c, "~clearinv");
                cheat(c, &format!("give {weapon_alias} 1"));
                // Zero-count food is a declared empty baseline, never a
                // `give`: the native `::give` handler clamps to at least
                // one (`Math.max(1, …)`), so `give cake 0` seeded the Cake
                // the ardy_fighter_bank baseline is supposed to lack.
                // Full-pack prepared cells wear their kit first, then stock
                // food below. Giving all armour plus 24/26 food in one debug
                // command batch would exceed the native 28-slot pack.
                if food_count > 0 && preparation.is_none() {
                    cheat(c, &format!("give {food_alias} {food_count}"));
                }
                for &(alias, _, count) in extra_give {
                    cheat(c, &format!("give {alias} {count}"));
                }
                if let Some(preparation) = preparation {
                    for &(alias, _, count) in preparation.extra_give {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                if bank_food_count > 0 {
                    cheat(c, &format!("givebank {bank_alias} {bank_food_count}"));
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: 0,
                min: combat_level,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "acknowledge prepared Attack profile",
            Proof::Stat {
                id: 0,
                min: combat_level,
            },
        ),
        (
            "acknowledge prepared Strength profile",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: combat_level,
            },
        ),
        (
            "acknowledge prepared Hitpoints profile",
            Proof::Stat {
                id: 3,
                min: combat_level,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    if let Some(preparation) = preparation {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Defence profile",
            Proof::Stat {
                id: DEFENCE_STAT,
                min: preparation.level,
            },
        ));
    }
    if thieving > 0 {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: thieving,
            },
        ));
    }
    if food_count > 0 && preparation.is_none() {
        steps.push(bank_fletcher_watch(
            "acknowledge the trip's carried food before Start",
            Proof::ItemId {
                id: food_id,
                count: food_count,
            },
        ));
    }
    for &(_, id, count) in extra_give {
        steps.push(bank_fletcher_watch(
            "acknowledge prepared extra gear before Start",
            Proof::ItemId { id, count },
        ));
    }
    if let Some(preparation) = preparation {
        for &(_, id, count) in preparation.extra_give {
            steps.push(bank_fletcher_watch(
                "acknowledge prepared tier-40 armour before Start",
                Proof::ItemId { id, count },
            ));
        }
    }
    for &id in loot_empty {
        steps.push(bank_fletcher_watch(
            "confirm no seeded kill loot in pack before Start",
            Proof::ItemIdAtMost { id, count: 0 },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge the weapon carried onto the safe tile",
        Proof::ItemId {
            id: weapon_id,
            count: 1,
        },
    ));
    steps.push(wear_combat_item_step(
        "wield and acknowledge the weapon before the hostile-field teleport",
        weapon_id,
    ));
    if let Some(preparation) = preparation {
        for &(label, id) in preparation.wear {
            steps.push(wear_combat_item_step(label, id));
        }
        if food_count > 0 {
            steps.push(Step {
                name: "stock prepared trip food after wearing the combat kit",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &format!("give {food_alias} {food_count}"));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::ItemId {
                        id: food_id,
                        count: food_count,
                    },
                    budget_ticks: 200,
                },
            });
        }
    }
    steps.push(Step {
        name: "teleport into the hostile field only after preparation is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(tele.level, tele.x, tele.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: tele.x,
                z: tele.z,
                level: tele.level,
                radius,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    // Bank-first cells deliberately start with banking (no food and/or seeded
    // deposit cargo). Their ordered watches require combat after the return;
    // waiting for incidental auto-retaliation XP here gates the wrong phase.
    const COMBAT_BANK_SKIPS_PREFIGHT_XP: &[&str] = &[
        "chaos_druid_bank",
        "moss_giant_bank_start",
        "green_dragon_tele_prepared",
    ];
    if !COMBAT_BANK_SKIPS_PREFIGHT_XP.contains(&name) {
        steps.push(bank_fletcher_watch(
            "watch Strength XP from the selected melee style after Start",
            xp,
        ));
    }
    for (step_name, arm) in watches {
        steps.push(bank_fletcher_watch(step_name, *arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(card),
            script_settings_inject: Some(inject),
            fixture_loadouts: combat_fixture_loadouts(card),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn remaining_prepared_combat_bank_scenario(
    name: &'static str,
    card: &'static str,
    tele: WorldTile,
    radius: i32,
    food_alias: &'static str,
    food_id: i32,
    food_count: i32,
    weapon_alias: &'static str,
    weapon_id: i32,
    extra_give: &'static [(&'static str, i32, i32)],
    loot_empty: &'static [i32],
    inject: &'static [ScriptSettingInject],
    bank_alias: &'static str,
    bank_food_count: i32,
    prepared_level: i32,
    deadline: Duration,
    watch_ticks: u32,
    watches: &[(&'static str, Proof)],
) -> Scenario {
    let mut scenario = combat_bank_scenario_with_preparation(
        name,
        card,
        tele,
        radius,
        food_alias,
        food_id,
        food_count,
        weapon_alias,
        weapon_id,
        extra_give,
        loot_empty,
        inject,
        0,
        bank_alias,
        bank_food_count,
        watches,
        Some(PreparedCombatPlan {
            level: prepared_level,
            extra_give: TIER40_RUNE_ARMOUR_GIVE,
            wear: TIER40_RUNE_ARMOUR_WEAR,
        }),
    );
    apply_combat_qualification_budget(&mut scenario, deadline, watch_ticks);
    scenario
}

/// Custom Bones loot with burial disabled exercises AutoFighter's BankRun
/// after a guaranteed Guard drop. No Bones are seeded: the script must loot
/// them, deposit at East Ardougne, restock ten Trout, close, return and fight.
pub(crate) fn auto_fighter_bank_scenario() -> Scenario {
    combat_bank_scenario(
        "auto_fighter_bank",
        "AutoFighter",
        ARDOUGNE_GUARD,
        8,
        "trout",
        TROUT_ID,
        AUTO_FIGHTER_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        &[BONES_ID],
        AUTO_FIGHTER_BANK_INJECT,
        0,
        "trout",
        20,
        &[
            (
                "watch a Guard drop enter a fresh Ardougne East bank",
                Proof::BankItemIdAny {
                    ids: &[BONES_ID],
                    count: 1,
                },
            ),
            (
                "watch the BankRun restock Trout to its declared ten",
                Proof::ItemId {
                    id: TROUT_ID,
                    count: AUTO_FIGHTER_BANK_RESTOCK,
                },
            ),
            ("watch the BankRun close the booth", Proof::BankClosed),
            (
                "watch return to the Guard anchor after banking",
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
    )
}

/// MossGiant's own trip end (no food left) reached after a looted Big bones,
/// so the Ardougne West booth deposit and lobster restock are the source's
/// transitions, not a seeded stock move.
pub(crate) fn moss_giant_bank_scenario() -> Scenario {
    combat_bank_scenario(
        "moss_giant_bank",
        "MossGiant",
        MOSS_GIANT_SAFESPOT,
        10,
        "lobster",
        LOBSTER_ID,
        MOSS_GIANT_BANK_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        MOSS_GIANT_LOOT_EMPTY,
        MOSS_GIANT_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch the trip's Big bones enter a fresh Ardougne West bank",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch the restock of Lobster to the card's declared line",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: MOSS_GIANT_BANK_RESTOCK,
                },
            ),
            ("watch MossGiant close its bank", Proof::BankClosed),
            (
                "watch return to the moss-giant safespot after banking",
                Proof::ArrivedNear {
                    x: MOSS_GIANT_SAFESPOT.x,
                    z: MOSS_GIANT_SAFESPOT.z,
                    level: MOSS_GIANT_SAFESPOT.level,
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
    )
}

/// MossGiant startup banking: zero trip food and one seeded Big bones (declared
/// cargo, not a looted kill) so BankRun runs before Fight. Qualifies deposit,
/// lobster restock, close, safespot return, and fresh Strength XP after return.
/// The fight-first `moss_giant_bank` cell remains unchanged for its own receipt.
pub(crate) fn moss_giant_bank_start_scenario() -> Scenario {
    combat_bank_scenario(
        "moss_giant_bank_start",
        "MossGiant",
        MOSS_GIANT_SAFESPOT,
        10,
        "lobster",
        LOBSTER_ID,
        0,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[("big_bones", BIG_BONES_ID, 1)],
        &[],
        MOSS_GIANT_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch seeded Big bones enter a fresh Ardougne West bank at startup",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch the startup restock of Lobster to the card's declared line",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: MOSS_GIANT_BANK_RESTOCK,
                },
            ),
            (
                "watch MossGiant close its bank after startup banking",
                Proof::BankClosed,
            ),
            (
                "watch return to the moss-giant safespot after startup banking",
                Proof::ArrivedNear {
                    x: MOSS_GIANT_SAFESPOT.x,
                    z: MOSS_GIANT_SAFESPOT.z,
                    level: MOSS_GIANT_SAFESPOT.level,
                    radius: 6,
                },
            ),
            (
                "watch fresh Strength XP after the startup bank return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    )
}

/// HillGiant's always-on trip end (`lootSlots=1`): one looted Giant drop ends
/// the trip, Varrock West banks it and withdraws trout back, then the pit
/// fight resumes.
pub(crate) fn hill_giant_bank_scenario() -> Scenario {
    combat_bank_scenario(
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
                "watch the trip's Big bones enter a fresh Varrock West bank",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
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
    )
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
            (
                "watch prepared HillGiant close its bank",
                Proof::BankClosed,
            ),
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

/// Deposit-only HillGiant qualification: same pit prep, inject and combat-first
/// Strength XP as `hill_giant_bank`, then a script-looted Big bones in pack,
/// then a fresh Varrock West deposit under the ordinary 150-dirty bank watch.
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
        HILL_GIANT_BANK_INJECT,
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

/// ChaosDruidKiller's own `prepare-trip` end: the pack carries less than
/// `foodWithdraw`, so the card climbs out, deposits the pack at the Edgeville
/// booth, withdraws its twelve lobster, closes and returns through the trapdoor
/// to the field, where the kills, loot and further work have to follow.
pub(crate) fn chaos_druid_bank_scenario() -> Scenario {
    combat_bank_scenario(
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
    )
}

/// ArdyFighter's `bankStrategy=Loot count` trip: after a Guard drop lands in
/// the pack the PeriodicBank walks to the East Ardougne booth, deposits the
/// card's own loot list and returns to the market anchor for further work.
/// Nothing in the deposit class is prepared: `bankEveryItems=1` would
/// otherwise treat a pre-Start listed item as the trip end.
pub(crate) fn ardy_fighter_bank_scenario() -> Scenario {
    combat_bank_scenario(
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
        ARDY_FIGHTER_BANK_INJECT,
        5,
        "",
        0,
        &[
            (
                "watch a Guard drop enter a fresh East Ardougne bank",
                Proof::BankItemIdAny {
                    ids: &GUARD_DROP_IDS,
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
    )
}

pub(crate) const ROCK_CRAB_BANK_DEPOSIT: [i32; 2] = [UNCUT_SAPPHIRE_ID, CASKET_ID];
pub(crate) const GREEN_DRAGON_BANK_DEPOSIT: [i32; 2] = [DRAGON_BONES_ID, GREEN_DRAGONHIDE_ID];
pub(crate) const HILL_GIANT_BANK_DEPOSIT: [i32; 2] = [BIG_BONES_ID, LIMPWURT_ROOT_ID];

/// FireGiant's Waterfall Quest prerequisite via the native individual-quest
/// path, ahead of the stat/inventory reset — the same targeted completion
/// `combat_core_scenario` runs for its `complete_quest` cells.
fn insert_waterfall_quest(scenario: &mut Scenario) {
    let prepare = scenario
        .steps
        .iter()
        .position(|step| step.name.starts_with("prepare melee stats"))
        .expect("combat bank/core prepares stats");
    scenario
        .steps
        .splice(prepare..prepare, quest_prereq_steps(WATERFALL_QUEST_PREREQ));
}

fn wear_shield_before_hostile_teleport(scenario: &mut Scenario) {
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.insert(
        hostile_teleport,
        wear_combat_item_step(
            "wear and acknowledge Dragonfire shield before hostile-field teleport",
            DRAGONFIRE_SHIELD_ID,
        ),
    );
}

fn prepare_safe_empty_food_escape_before_hostile_teleport(scenario: &mut Scenario) {
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [
            bank_fletcher_watch(
                "confirm the prepared escape profile carries no food before Start",
                Proof::ItemIdAtMost {
                    id: LOBSTER_ID,
                    count: 0,
                },
            ),
            Step {
                name: "prepare and acknowledge 70 of 99 Hitpoints for the configured panic trigger",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(
                            c,
                            &format!(
                                "~hit {}",
                                GREEN_DRAGON_TELE_PREPARED_LEVEL - GREEN_DRAGON_TELE_PREPARED_HP
                            ),
                        );
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::StatAtMost {
                        id: 3,
                        max: GREEN_DRAGON_TELE_PREPARED_HP,
                    },
                    budget_ticks: 200,
                },
            },
            bank_fletcher_watch(
                "confirm the prepared teleport retains 70 Hitpoints before Start",
                Proof::Stat {
                    id: 3,
                    min: GREEN_DRAGON_TELE_PREPARED_HP,
                },
            ),
        ],
    );
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
    scenario
}

/// GreenDragon BankRun to Edgeville: food-gone / pack-full trip deposits
/// bones or hide, restocks lobster to `foodWithdraw` 20, and walks back
/// past the wilderness ditch.
pub(crate) fn green_dragon_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "green_dragon_bank",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_BANK_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch dragon bones or hide enter a fresh Edgeville bank",
                Proof::BankItemIdAny {
                    ids: &GREEN_DRAGON_BANK_DEPOSIT,
                    count: 1,
                },
            ),
            (
                "watch the restock of Lobster to the card's foodWithdraw 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_RESTOCK,
                },
            ),
            ("watch GreenDragon close its bank", Proof::BankClosed),
            (
                "watch return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
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
    wear_shield_before_hostile_teleport(&mut scenario);
    scenario
}

/// Prepared earned-kill full-bank witness. The stock names `Dragon bones` and
/// `Dragonhide` are selected explicitly because the compatibility drop catalog
/// omits the hide. Twenty-six Lobsters leave two slots for that guaranteed pair;
/// exact upstream-preparation stats keep the pressure cycle inside its bound.
pub(crate) fn green_dragon_bank_prepared_scenario() -> Scenario {
    let mut scenario = remaining_prepared_combat_bank_scenario(
        "green_dragon_bank_prepared",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_BANK_PREPARED_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_BANK_PREPARED_INJECT,
        "lobster",
        40,
        BANK_PRESSURE_PREPARED_LEVEL,
        GREEN_DRAGON_BANK_QUALIFICATION_DEADLINE,
        BANK_QUALIFICATION_WATCH_TICKS,
        &[
            (
                "watch earned dragon bones or hide enter a fresh Edgeville bank",
                Proof::BankItemIdAny {
                    ids: &GREEN_DRAGON_BANK_DEPOSIT,
                    count: 1,
                },
            ),
            (
                "watch the inventory-pressure trip draw Lobster to 27",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_PREPARED_RESTOCK,
                },
            ),
            (
                "watch prepared GreenDragon close its bank",
                Proof::BankClosed,
            ),
            (
                "watch prepared return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
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
    wear_shield_before_hostile_teleport(&mut scenario);
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

/// Default-loot Green full cycle. Same prepared bank loadout as
/// `green_dragon_bank_prepared` (99/Rune/26+40 Lobster, no potions/clues/
/// burial/special/teleport) but the `loot` inject is omitted so the generated
/// source default must earn both guaranteed drops. No seeded cargo counts.
pub(crate) fn green_dragon_bank_default_prepared_scenario() -> Scenario {
    let mut scenario = remaining_prepared_combat_bank_scenario(
        "green_dragon_bank_default_prepared",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_BANK_PREPARED_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1)],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_BANK_DEFAULT_PREPARED_INJECT,
        "lobster",
        40,
        BANK_PRESSURE_PREPARED_LEVEL,
        GREEN_DRAGON_BANK_DEFAULT_PREPARED_QUALIFICATION_DEADLINE,
        GREEN_DRAGON_BANK_DEFAULT_PREPARED_QUALIFICATION_WATCH_TICKS,
        &[
            (
                "watch earned Dragon bones in pack after the selected defeat",
                Proof::ItemId {
                    id: DRAGON_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch earned Dragonhide in pack after the selected defeat",
                Proof::ItemId {
                    id: GREEN_DRAGONHIDE_ID,
                    count: 1,
                },
            ),
            (
                "watch earned Dragonhide enter a fresh Edgeville bank",
                Proof::BankItemId {
                    id: GREEN_DRAGONHIDE_ID,
                    count: 1,
                },
            ),
            (
                "watch the inventory-pressure trip draw Lobster to 27",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_PREPARED_RESTOCK,
                },
            ),
            (
                "watch default-loot GreenDragon close its bank",
                Proof::BankClosed,
            ),
            (
                "watch prepared return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
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
    wear_shield_before_hostile_teleport(&mut scenario);
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

/// `escape=Teleport to Varrock`: Magic XP and a Varrock land, then the
/// Edgeville booth restock and a return past the ditch. A south-walk flee
/// without the teleport fails this cell.
pub(crate) fn green_dragon_tele_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "green_dragon_tele",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        GREEN_DRAGON_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[
            ("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1),
            ("lawrune", LAW_RUNE_ID, VARROCK_TELE_LAW),
            ("airrune", AIR_RUNE_ID, VARROCK_TELE_AIR),
            ("firerune", FIRE_RUNE_ID, VARROCK_TELE_FIRE),
        ],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_TELE_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch Magic XP from the Varrock teleport after Start",
                Proof::StatXpGain {
                    id: MAGIC_STAT,
                    min: 1,
                },
            ),
            (
                "watch the Varrock teleport land, not a queued button",
                Proof::ArrivedNear {
                    x: VARROCK_TELE_LAND.x,
                    z: VARROCK_TELE_LAND.z,
                    level: VARROCK_TELE_LAND.level,
                    radius: 8,
                },
            ),
            (
                "watch the restock of Lobster to the card's foodWithdraw 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_RESTOCK,
                },
            ),
            (
                "watch GreenDragon close its bank after the teleport",
                Proof::BankClosed,
            ),
            (
                "watch return to the dragon field after banking",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
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
    wear_shield_before_hostile_teleport(&mut scenario);
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.splice(
        hostile_teleport..hostile_teleport,
        [Step {
            name: "prepare and acknowledge Magic 25 for Varrock teleport before Start",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, &format!("setstat magic {VARROCK_TELE_MAGIC}"));
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat {
                    id: MAGIC_STAT,
                    min: VARROCK_TELE_MAGIC,
                },
                budget_ticks: 200,
            },
        }],
    );
    scenario
}

/// The pinned source requires both no food and HP below `panicHp` to select
/// `Escape`; no food by itself selects the direct `BankRun`. This prepared
/// fixture uses the upstream max-stat profile, configures `panicHp=98`, and
/// enters the field at a safe 70/99 HP instead of reproducing the upstream
/// regression's death-prone 1-HP seed. The supported escape must walk south,
/// cast Varrock, restock/heal at Edgeville, return to the field and produce
/// later Strength XP.
pub(crate) fn green_dragon_tele_prepared_scenario() -> Scenario {
    let mut scenario = remaining_prepared_combat_bank_scenario(
        "green_dragon_tele_prepared",
        "GreenDragon",
        GREEN_DRAGON_FIELD,
        22,
        "lobster",
        LOBSTER_ID,
        0,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[
            ("antidragonbreathshield", DRAGONFIRE_SHIELD_ID, 1),
            ("lawrune", LAW_RUNE_ID, VARROCK_TELE_LAW),
            ("airrune", AIR_RUNE_ID, VARROCK_TELE_AIR),
            ("firerune", FIRE_RUNE_ID, VARROCK_TELE_FIRE),
        ],
        GREEN_DRAGON_LOOT_EMPTY,
        GREEN_DRAGON_TELE_PREPARED_INJECT,
        "lobster",
        40,
        GREEN_DRAGON_TELE_PREPARED_LEVEL,
        GREEN_DRAGON_TELE_QUALIFICATION_DEADLINE,
        GREEN_DRAGON_TELE_QUALIFICATION_WATCH_TICKS,
        &[
            (
                "watch Magic XP from the prepared Varrock teleport after Start",
                Proof::StatXpGain {
                    id: MAGIC_STAT,
                    min: 1,
                },
            ),
            (
                "watch the prepared Varrock teleport land",
                Proof::ArrivedNear {
                    x: VARROCK_TELE_LAND.x,
                    z: VARROCK_TELE_LAND.z,
                    level: VARROCK_TELE_LAND.level,
                    radius: 8,
                },
            ),
            (
                "watch the prepared escape restock Lobster to 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: GREEN_DRAGON_BANK_RESTOCK,
                },
            ),
            (
                "watch prepared GreenDragon close its bank after teleporting",
                Proof::BankClosed,
            ),
            (
                "watch prepared return to the dragon field after teleporting",
                Proof::ArrivedNear {
                    x: GREEN_DRAGON_FIELD.x,
                    z: GREEN_DRAGON_FIELD.z,
                    level: GREEN_DRAGON_FIELD.level,
                    radius: 22,
                },
            ),
            (
                "watch fresh Strength XP after the prepared escape return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    wear_shield_before_hostile_teleport(&mut scenario);
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("combat bank has a hostile-field teleport");
    scenario.steps.insert(
        hostile_teleport,
        Step {
            name: "prepare and acknowledge Magic 25 for the prepared Varrock teleport",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, &format!("setstat magic {VARROCK_TELE_MAGIC}"));
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat {
                    id: MAGIC_STAT,
                    min: VARROCK_TELE_MAGIC,
                },
                budget_ticks: 200,
            },
        },
    );
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    prepare_safe_empty_food_escape_before_hostile_teleport(&mut scenario);
    scenario
}

/// FireGiant `EnterDungeon` from the raft. Start is not already z>=9000; the
/// frozen script has to board the raft, rope the rock and tree, open the
/// ledge door, then fight. Same frozen `GearEquip` melee refuse as the core
/// cell: wear 1331 and drain setstat level-ups before the raft teleport.
pub(crate) fn fire_giant_approach_scenario() -> Scenario {
    let mut scenario = combat_core_scenario(CombatCorePlan {
        name: "fire_giant_approach",
        card: "FireGiant",
        tele: FIRE_GIANT_RAFT,
        radius: 5,
        food_alias: "lobster",
        food_id: LOBSTER_ID,
        food_count: FIRE_GIANT_FOOD,
        weapon_alias: "adamant_scimitar",
        weapon_id: COMBAT_SCIMITAR_ID,
        extra_give: &[
            ("glarials_amulet_waterfall_quest", GLARIALS_AMULET_ID, 1),
            ("rope", ROPE_ID, 1),
        ],
        wear: Some((
            "wield and acknowledge the prepared Adamant scimitar before the hostile-field teleport",
            COMBAT_SCIMITAR_ID,
        )),
        loot_empty: FIRE_GIANT_LOOT_EMPTY,
        inject: FIRE_GIANT_INJECT,
        complete_quest: Some(WATERFALL_QUEST_PREREQ),
        thieving: 0,
        agility: 0,
    });
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat core has a Start step");
    scenario.steps.insert(
        start + 1,
        bank_fletcher_watch(
            "watch the script enter the Waterfall Dungeon after Start",
            Proof::ArrivedNear {
                x: FIRE_GIANT_ROOM.x,
                z: FIRE_GIANT_ROOM.z,
                level: FIRE_GIANT_ROOM.level,
                radius: 16,
            },
        ),
    );
    scenario
}

/// FireGiant barrel exit then Ardougne West restock and re-entry. The in-room
/// core leaves this trip unqualified.
pub(crate) fn fire_giant_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "fire_giant_bank",
        "FireGiant",
        FIRE_GIANT_ROOM,
        10,
        "lobster",
        LOBSTER_ID,
        FIRE_GIANT_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[
            ("glarials_amulet_waterfall_quest", GLARIALS_AMULET_ID, 1),
            ("rope", ROPE_ID, 1),
        ],
        FIRE_GIANT_LOOT_EMPTY,
        FIRE_GIANT_INJECT,
        0,
        "lobster",
        24,
        &[
            (
                "watch the barrel wash-up before the bank walk",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_WASH.x,
                    z: FIRE_GIANT_WASH.z,
                    level: FIRE_GIANT_WASH.level,
                    radius: 6,
                },
            ),
            (
                "watch the trip's Big bones enter a fresh Ardougne West bank",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch the Ardougne West booth the barrel trip walks to",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_BANK.x,
                    z: FIRE_GIANT_BANK.z,
                    level: FIRE_GIANT_BANK.level,
                    radius: 6,
                },
            ),
            (
                "watch the restock of Lobster to the card's foodWithdraw 20",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: FIRE_GIANT_BANK_RESTOCK,
                },
            ),
            ("watch FireGiant close its bank", Proof::BankClosed),
            (
                "watch re-entry to the fire-giant room after banking",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_ROOM.x,
                    z: FIRE_GIANT_ROOM.z,
                    level: FIRE_GIANT_ROOM.level,
                    radius: 10,
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
    insert_waterfall_quest(&mut scenario);
    scenario
}

/// Prepared earned-kill bank witness. One Lobster creates the source's earliest
/// food-empty yield after eating; the strict gate still requires Big bones to
/// have been earned, never seeded. The 25-Lobster restock plus amulet and rope
/// leaves one cargo slot free. Exact upstream-preparation stats bound the cycle.
pub(crate) fn fire_giant_bank_prepared_scenario() -> Scenario {
    let mut scenario = remaining_prepared_combat_bank_scenario(
        "fire_giant_bank_prepared",
        "FireGiant",
        FIRE_GIANT_ROOM,
        10,
        "lobster",
        LOBSTER_ID,
        FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[
            ("glarials_amulet_waterfall_quest", GLARIALS_AMULET_ID, 1),
            ("rope", ROPE_ID, 1),
        ],
        FIRE_GIANT_LOOT_EMPTY,
        FIRE_GIANT_BANK_PREPARED_INJECT,
        "lobster",
        40,
        BANK_PRESSURE_PREPARED_LEVEL,
        FIRE_GIANT_BANK_QUALIFICATION_DEADLINE,
        BANK_QUALIFICATION_WATCH_TICKS,
        &[
            (
                "watch the prepared barrel wash-up before the bank walk",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_WASH.x,
                    z: FIRE_GIANT_WASH.z,
                    level: FIRE_GIANT_WASH.level,
                    radius: 6,
                },
            ),
            (
                "watch earned Big bones enter a fresh Ardougne West bank",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch the prepared barrel trip reach Ardougne West",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_BANK.x,
                    z: FIRE_GIANT_BANK.z,
                    level: FIRE_GIANT_BANK.level,
                    radius: 6,
                },
            ),
            (
                "watch the prepared one-food trip restock Lobster to 25",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: FIRE_GIANT_BANK_PREPARED_RESTOCK,
                },
            ),
            ("watch prepared FireGiant close its bank", Proof::BankClosed),
            (
                "watch prepared re-entry to the fire-giant room after banking",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_ROOM.x,
                    z: FIRE_GIANT_ROOM.z,
                    level: FIRE_GIANT_ROOM.level,
                    radius: 10,
                },
            ),
            (
                "watch fresh Strength XP after the prepared FireGiant return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    insert_waterfall_quest(&mut scenario);
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

/// Prepared FireGiant Camelot escape: source teleport land + Magic XP, Seers
/// 25-lobster restock, Waterfall return, and fresh Strength. Earned Big bones
/// stay required; bones are never seeded.
pub(crate) fn fire_giant_camelot_prepared_scenario() -> Scenario {
    let mut scenario = remaining_prepared_combat_bank_scenario(
        "fire_giant_camelot_prepared",
        "FireGiant",
        FIRE_GIANT_ROOM,
        10,
        "lobster",
        LOBSTER_ID,
        FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD,
        "rune_scimitar",
        RUNE_SCIMITAR_ID,
        &[
            ("glarials_amulet_waterfall_quest", GLARIALS_AMULET_ID, 1),
            ("rope", ROPE_ID, 1),
            ("airrune", AIR_RUNE_ID, CAMELOT_AIR_CARRY),
            ("lawrune", LAW_RUNE_ID, CAMELOT_LAW_CARRY),
        ],
        FIRE_GIANT_LOOT_EMPTY,
        FIRE_GIANT_CAMELOT_PREPARED_INJECT,
        "lobster",
        40,
        BANK_PRESSURE_PREPARED_LEVEL,
        FIRE_GIANT_BANK_QUALIFICATION_DEADLINE,
        BANK_QUALIFICATION_WATCH_TICKS,
        &[
            (
                "watch Magic XP from the Camelot escape teleport after Start",
                Proof::StatXpGain {
                    id: MAGIC_STAT,
                    min: 1,
                },
            ),
            (
                "watch the Camelot teleport land, not a Seers endpoint alone",
                Proof::ArrivedNear {
                    x: CAMELOT_TELE_LAND.x,
                    z: CAMELOT_TELE_LAND.z,
                    level: CAMELOT_TELE_LAND.level,
                    radius: 8,
                },
            ),
            (
                "watch earned Big bones enter a fresh Seers bank",
                Proof::BankItemId {
                    id: BIG_BONES_ID,
                    count: 1,
                },
            ),
            (
                "watch the Camelot trip reach Seers bank",
                Proof::ArrivedNear {
                    x: SEERS_BANK.x,
                    z: SEERS_BANK.z,
                    level: SEERS_BANK.level,
                    radius: 6,
                },
            ),
            (
                "watch the prepared Camelot trip restock Lobster to 25",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: FIRE_GIANT_BANK_PREPARED_RESTOCK,
                },
            ),
            (
                "watch prepared Camelot FireGiant close its bank",
                Proof::BankClosed,
            ),
            (
                "watch prepared re-entry to the fire-giant room after Camelot banking",
                Proof::ArrivedNear {
                    x: FIRE_GIANT_ROOM.x,
                    z: FIRE_GIANT_ROOM.z,
                    level: FIRE_GIANT_ROOM.level,
                    radius: 10,
                },
            ),
            (
                "watch fresh Strength XP after the prepared Camelot return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    );
    insert_camelot_escape_stock(&mut scenario);
    insert_waterfall_quest(&mut scenario);
    insert_setstat_drain_before_hostile_tele(&mut scenario);
    scenario
}

fn insert_camelot_escape_stock(scenario: &mut Scenario) {
    let prepare = scenario
        .steps
        .iter()
        .position(|step| step.name.starts_with("prepare melee stats"))
        .expect("camelot prepared cell prepares stats");
    scenario.steps.insert(
        prepare + 1,
        Step {
            name: "seed spare Camelot escape runes in the bank and acknowledge Magic 45",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, &format!("setstat magic {CAMELOT_TELE_MAGIC}"));
                    cheat(c, &format!("givebank airrune {CAMELOT_BANK_AIR}"));
                    cheat(c, &format!("givebank lawrune {CAMELOT_BANK_LAW}"));
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat {
                    id: MAGIC_STAT,
                    min: CAMELOT_TELE_MAGIC,
                },
                budget_ticks: 200,
            },
        },
    );
    let hostile_teleport = scenario
        .steps
        .iter()
        .position(|step| {
            step.name == "teleport into the hostile field only after preparation is acknowledged"
        })
        .expect("camelot prepared cell has a hostile-field teleport");
    scenario.steps.insert(
        hostile_teleport,
        wear_combat_item_step(
            "wear Glarial's amulet so the 25-lobster restock stays 28 slots",
            GLARIALS_AMULET_ID,
        ),
    );
    for (name, arm) in [
        (
            "acknowledge Camelot Air rune carry before Start",
            Proof::ItemId {
                id: AIR_RUNE_ID,
                count: CAMELOT_AIR_CARRY,
            },
        ),
        (
            "acknowledge Camelot Law rune carry before Start",
            Proof::ItemId {
                id: LAW_RUNE_ID,
                count: CAMELOT_LAW_CARRY,
            },
        ),
        (
            "acknowledge empty earned Big bones before Start",
            Proof::ItemIdAtMost {
                id: BIG_BONES_ID,
                count: 0,
            },
        ),
    ] {
        scenario.steps.insert(hostile_teleport + 1, bank_fletcher_watch(name, arm));
    }
}
