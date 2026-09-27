use super::*;
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

/// Prepared FireGiant melee core. One 111-HP kill at 40/40/40 Rune scimitar
/// is ~188 ticks of uninterrupted swings (hit 0.53, max 9, 4-tick rate) and
/// measured 250-360 ticks with the card's eats and re-issues, so the kill,
/// Big bones and the further engagement need the 300s combat wall.
pub(crate) fn fire_giant_prepared_scenario() -> Scenario {
    let mut scenario = prepared_combat_core_scenario(
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
    );
    apply_combat_qualification_budget(
        &mut scenario,
        COMBAT_QUALIFICATION_DEADLINE,
        COMBAT_QUALIFICATION_WATCH_TICKS,
    );
    scenario
}
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
/// 24-lobster restock, Waterfall return, and fresh Strength. Earned Big bones
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
                "watch the prepared Camelot trip restock Lobster to 24",
                Proof::ItemId {
                    id: LOBSTER_ID,
                    count: FIRE_GIANT_CAMELOT_PREPARED_RESTOCK,
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
            "wear Glarial's amulet so pack food plus rope and escape runes stay at 27 slots",
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
        scenario
            .steps
            .insert(hostile_teleport + 1, bank_fletcher_watch(name, arm));
    }
}
const CAMELOT_TELE_MAGIC: i32 = 45;
const CAMELOT_TELE_AIR: i32 = 5;
const CAMELOT_TELE_LAW: i32 = 1;
const CAMELOT_TELE_STOCK: i32 = 2;
const CAMELOT_AIR_CARRY: i32 = CAMELOT_TELE_AIR * (CAMELOT_TELE_STOCK + 1);
const CAMELOT_LAW_CARRY: i32 = CAMELOT_TELE_LAW * (CAMELOT_TELE_STOCK + 1);
pub(super) const FIRE_GIANT_FOOD: i32 = 12;
pub(crate) const FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD: i32 = 1;
pub(crate) const FIRE_GIANT_BANK_PREPARED_RESTOCK: i32 = 25;
/// Camelot escape restocks Air/Law after food; `foodWithdraw` 25 fills 28/28 and
/// re-triggers frozen `BankRun.isFull()`. Barrel prepared keeps the shared 25.
pub(crate) const FIRE_GIANT_CAMELOT_PREPARED_RESTOCK: i32 = 24;
/// FireGiant's one-food trigger plus barrel exit, bank leg, Waterfall re-entry
/// and fresh post-return XP is estimated at 259.4s before margin.
const FIRE_GIANT_BANK_QUALIFICATION_DEADLINE: Duration = Duration::from_secs(600);
pub(crate) const GLARIALS_AMULET_ID: i32 = 295;
pub(crate) const ROPE_ID: i32 = 954;
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
const CAMELOT_TELE_LAND: WorldTile = WorldTile {
    x: 2757,
    z: 3478,
    level: 0,
};
const FIRE_GIANT_BANK_RESTOCK: i32 = 20;
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
        value: ScriptInjectValue::Num(FIRE_GIANT_CAMELOT_PREPARED_RESTOCK as f64),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Big bones"]),
    },
];
const FIRE_GIANT_LOOT_EMPTY: &[i32] = &[BIG_BONES_ID, NOTED_BIG_BONES_ID];
/// Waterfall Quest (`quest_names_enum` 50, journal row `[waterfall]`) — the
/// FireGiant dungeon entry's requirement.
pub(crate) const WATERFALL_QUEST_PREREQ: NativeQuestPrereq = NativeQuestPrereq {
    dialog: "Waterfall Quest",
    journal: "Waterfall Quest",
};
