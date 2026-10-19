use super::*;
pub(crate) const BONES_ID: i32 = 526;
pub(crate) const NOTED_BONES_ID: i32 = 527;
pub(crate) const AUTO_FIGHTER_FOOD: i32 = 8;
const AUTO_FIGHTER_BANK_RESTOCK: i32 = 10;
const AUTO_FIGHTER_MAGE_LEVEL: i32 = 13;
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
const AUTO_FIGHTER_LOOT_EMPTY: &[i32] = &[BONES_ID, NOTED_BONES_ID];
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

pub(crate) const UNICORN_HORN_ID: i32 = 237;
pub(crate) const FUR_ID: i32 = 948;
/// The south-east Varrock wood between the catalog's Bear camp
/// (3296,3347) and Unicorn camp (3284,3352)
/// (`AccountLeveler/trainingCamps.ts`, `AccountLeveler/combat.ts`):
/// both populations wander the same open wood inside one leash.
const VARROCK_BEAR_UNICORN: WorldTile = WorldTile {
    x: 3290,
    z: 3350,
    level: 0,
};
/// Seed + Start ~25s, then per cycle two 25-HP kills with looting
/// (~45s) and a Varrock East round trip (~150 tiles, ~100s). Two
/// cycles plus resumed combat reach ~360s; the shared 420s pattern
/// (herblore eggs) covers the second trip. 1050 dirties at the combat
/// cells' ~2.5/second allowance keeps the wall deadline controlling.
const AUTO_FIGHTER_TARGETS_DEADLINE: Duration = Duration::from_secs(420);
const AUTO_FIGHTER_TARGETS_WATCH_TICKS: u32 = 1050;
/// AutoFighter's schema carries the shared `loadout` setting, and Start
/// resolves it before `onStart`; with no loadout rows the resolution
/// fails (`loadouts_store::resolve_script_loadout`). Post the Trout
/// row every other combat card posts, named explicitly in the ported
/// cells' injects. Blank settings resolve to this first row, so the
/// older AutoFighter cells start from it unchanged.
pub(crate) const AUTO_FIGHTER_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario AutoFighter food",
    carry: &[("Trout", AUTO_FIGHTER_FOOD as u32)],
}];
pub(crate) const CAKE_ID: i32 = 1891;
/// Trout the food-override cell carries onto the field; the script must
/// not count them as food when its setting names Cake.
const FOOD_OVERRIDE_PACK_TROUT: i32 = 5;
const FOOD_OVERRIDE_CAKE_WITHDRAW: i32 = 3;
const FOOD_OVERRIDE_BANK_CAKE: i32 = 20;

const AUTO_FIGHTER_TARGETS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario AutoFighter food"),
    },
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
        value: ScriptInjectValue::Str("Bear, Unicorn"),
    },
    ScriptSettingInject {
        id: "spot",
        value: ScriptInjectValue::Str("Start position"),
    },
    ScriptSettingInject {
        id: "leashRadius",
        value: ScriptInjectValue::Num(16.0),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("Auto"),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(2.0),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["Unicorn horn", "Fur"]),
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

/// Ported from rs2b0t `e2e/autofighter-targets-loot-live.ts` @2de6bb23
/// (`targetsAndHerbs`).
/// Changed: the `npcadd` arena (two spawned Chickens, two spawned
/// Rats) becomes the real south-east Varrock wood, where the Bear and
/// Unicorn populations wander the same open ground inside one leash,
/// with targets `Bear, Unicorn` (Rat drops nothing:
/// `drop tables/scripts/rat.rs2`, so a Rat kill has no loot gate).
/// Changed: the seeded ground-herb item-ID gates become banked-loot
/// kill gates on unconditional content drops — `bear.rs2` always
/// drops `fur` (and `raw_bear_meat`), `unicorn.rs2` always drops
/// `unicorn_horn` (both in `content/scripts/drop tables/scripts/`).
/// Dropped: the herb avoid-list assertion; it has no real-content
/// gate without placing the herbs it acts on.
pub(crate) fn auto_fighter_targets_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
        "auto_fighter_targets",
        "AutoFighter",
        VARROCK_BEAR_UNICORN,
        8,
        "trout",
        TROUT_ID,
        AUTO_FIGHTER_FOOD,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        &[UNICORN_HORN_ID, FUR_ID],
        AUTO_FIGHTER_TARGETS_INJECT,
        0,
        "trout",
        20,
        &[
            (
                "watch a Unicorn kill's Unicorn horn enter a fresh Varrock bank",
                Proof::BankItemId {
                    id: UNICORN_HORN_ID,
                    count: 1,
                },
            ),
            (
                "watch a Bear kill's Fur enter a fresh Varrock bank",
                Proof::BankItemId {
                    id: FUR_ID,
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
            ("watch the loot BankRun close the booth", Proof::BankClosed),
            (
                "watch return to the Varrock wood anchor after banking",
                Proof::ArrivedNear {
                    x: VARROCK_BEAR_UNICORN.x,
                    z: VARROCK_BEAR_UNICORN.z,
                    level: VARROCK_BEAR_UNICORN.level,
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
    apply_combat_qualification_budget(
        &mut scenario,
        AUTO_FIGHTER_TARGETS_DEADLINE,
        AUTO_FIGHTER_TARGETS_WATCH_TICKS,
    );
    scenario
}

const AUTO_FIGHTER_FOOD_OVERRIDE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario AutoFighter food"),
    },
    ScriptSettingInject {
        id: "food",
        value: ScriptInjectValue::Str("Cake"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(FOOD_OVERRIDE_CAKE_WITHDRAW as f64),
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
        id: "leashRadius",
        value: ScriptInjectValue::Num(14.0),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("Auto"),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::StrList(&["clue scroll"]),
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

/// Ported from rs2b0t `e2e/autofighter-targets-loot-live.ts` @2de6bb23
/// (`foodOverride`).
/// Changed: the `BankRun triggered: out of food` runner-log gate
/// becomes the withdrawal itself — no Cake is seeded in the pack, so
/// three held Cake could only come from the bank trip it names.
/// Kept: five Trout in the pack, twenty Cake in the bank, food `Cake`
/// withdraw 3 at the real Ardougne Guards, loot `clue scroll` as a
/// setting only (a tertiary drop is chance and is never gated).
pub(crate) fn auto_fighter_food_override_scenario() -> Scenario {
    combat_bank_scenario(
        "auto_fighter_food_override",
        "AutoFighter",
        ARDOUGNE_GUARD,
        8,
        "trout",
        TROUT_ID,
        FOOD_OVERRIDE_PACK_TROUT,
        "adamant_scimitar",
        COMBAT_SCIMITAR_ID,
        &[],
        &[CAKE_ID],
        AUTO_FIGHTER_FOOD_OVERRIDE_INJECT,
        0,
        "cake",
        FOOD_OVERRIDE_BANK_CAKE,
        &[
            (
                "watch the BankRun withdraw three Cake past the pack Trout",
                Proof::ItemId {
                    id: CAKE_ID,
                    count: FOOD_OVERRIDE_CAKE_WITHDRAW,
                },
            ),
            ("watch the food BankRun close the booth", Proof::BankClosed),
            (
                "watch return to the Guard anchor after the food trip",
                Proof::ArrivedNear {
                    x: ARDOUGNE_GUARD.x,
                    z: ARDOUGNE_GUARD.z,
                    level: ARDOUGNE_GUARD.level,
                    radius: 6,
                },
            ),
            (
                "watch fresh Strength XP after the food return",
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1,
                },
            ),
        ],
    )
}
