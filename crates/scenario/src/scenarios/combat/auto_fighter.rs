use super::*;
pub(crate) const BONES_ID: i32 = 526;
pub(crate) const NOTED_BONES_ID: i32 = 527;
pub(crate) const AUTO_FIGHTER_FOOD: i32 = 8;
const AUTO_FIGHTER_BANK_RESTOCK: i32 = 10;
// Engine content: drop tables/configs/lootdrop.constant:1 gives 200 ticks.
// Two extra PLAYER_INFO observations cover the ordered ground-remove update.
const AUTO_FIGHTER_FLOOR_CLEAR_TICKS: u32 = 200 + 2;
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
            fixture_loadouts: combat_fixture_loadouts("AutoFighter"),
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
/// The `auto_fighter_bank` cell's last watch, fresh Strength XP after the
/// bank return, in **observed game ticks** (one per `PLAYER_INFO`; see
/// [`Wait`]). The ordinary post-Start watch is
/// [`SCRIPT_GOLD_WATCH_TICKS`] (150) and stays the default everywhere else.
///
/// `NIGHTLY-QUAL-1025/report.md` cell 4 measured why this one watch is
/// wider. On two of three qualification runs the bot banked once,
/// returned, took new Bones and banked a second time before it earned
/// fresh Strength XP: the second Bones take to the qualifying attack took
/// 115 and 111 host ticks, leaving only 35 to 39 of the ordinary 150-tick
/// watch as slack. A trip slightly slower than the measured 115 would
/// spend the whole watch on that extra bank cycle and fail the cell.
///
/// 320 is the ordinary 150-tick watch plus 170 ticks: it holds one full
/// measured 111-115-tick extra bank cycle with 55 to 59 ticks to spare
/// beyond the ordinary watch, and it matches the bank watches peer cells
/// carry (the seeded Ardy Fighter's 320-tick style watch,
/// `ardy_fighter.rs:19`). Only this cell's last step uses it.
const AUTO_FIGHTER_BANK_FRESH_XP_WATCH_TICKS: u32 = 320;
/// Custom Bones loot with burial disabled exercises AutoFighter's BankRun
/// after a guaranteed Guard drop. No Bones are seeded: the script must loot
/// them, deposit at East Ardougne, restock ten Trout, close, return and fight.
pub(crate) fn auto_fighter_bank_scenario() -> Scenario {
    let mut scenario = combat_bank_scenario(
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
    );
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("combat bank fixture has Start");
    scenario.steps.insert(
        start,
        Step {
            name: "wait for the Guard field's public Bones to clear before Start",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::GroundItemIdsAbsentNear {
                    ids: &[BONES_ID],
                    x: ARDOUGNE_GUARD.x,
                    z: ARDOUGNE_GUARD.z,
                    level: ARDOUGNE_GUARD.level,
                    // AutoFighter LootDrops: default leash 8 + pickup margin 4.
                    radius: 12,
                },
                budget_ticks: AUTO_FIGHTER_FLOOR_CLEAR_TICKS,
            },
        },
    );
    // The extra bank cycle measured at NIGHTLY-QUAL-1025/report.md cell 4
    // lands inside this last watch: the bot re-banks after taking new Bones
    // following the return, so only this step gets the wider budget.
    scenario
        .steps
        .iter_mut()
        .find(|step| step.name == "watch fresh Strength XP after the bank return")
        .expect("auto_fighter_bank has its fresh Strength XP watch")
        .wait
        .budget_ticks = AUTO_FIGHTER_BANK_FRESH_XP_WATCH_TICKS;
    // Preserve the existing post-Start allowance. Only the content-derived
    // pre-Start guard adds time, at Engine A's ordinary 600 ms game tick.
    scenario.settings.deadline +=
        Duration::from_millis(u64::from(AUTO_FIGHTER_FLOOR_CLEAR_TICKS) * 600);
    // The wider last watch replaces the 150-tick ordinary watch the shared
    // 180 s deadline was sized with, so the whole-cell cap grows by exactly
    // the added 170 ticks (102 s), from 301.2 s to 403.2 s. The 320-tick
    // watch alone runs 192 s, longer than the 180 s shared cap. In the
    // slowest measured run (report cell 4, run 3) the return witness holds
    // before the second Bones take at host tick 183, so the watch starts at
    // tick 183 at the latest: 183 + 320 = 503 observed ticks would pass the
    // old 502-tick cap.
    scenario.settings.deadline += Duration::from_millis(
        u64::from(AUTO_FIGHTER_BANK_FRESH_XP_WATCH_TICKS - SCRIPT_GOLD_WATCH_TICKS) * 600,
    );
    scenario
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
/// Whole-cell wall deadline. Measured live on Engine A at the 289 engine's
/// 600 ms game tick: six passes at 189.676 s (283 game ticks), 206.892 s
/// (313), 188.062 s (279), 188.250 s (281), 580.326 s (901) and 420.234 s
/// (655). The largest is 580.326 s; the deadline is that plus 50% headroom,
/// rounded up to a whole second (870.49 s -> 871 s). The per-step
/// 1050-game-tick budgets are unchanged.
pub(crate) const AUTO_FIGHTER_TARGETS_DEADLINE: Duration = Duration::from_secs(871);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// From `NIGHTLY-QUAL-1025/report.md` cell 4: the second Bones take to
    /// the qualifying attack took 115 and 111 host ticks in runs 2 and 3.
    const MEASURED_EXTRA_BANK_CYCLE_TICKS: u32 = 115;

    fn post_start_steps(scenario: &Scenario) -> &[Step] {
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("cell starts its card");
        &scenario.steps[start + 1..]
    }

    /// The nightly failure mode: a second bank cycle spends the last watch.
    /// The bank cell must outlast the ordinary watch plus one measured extra
    /// cycle, and the widening must stay on that last step.
    #[test]
    fn bank_cell_last_watch_covers_the_measured_extra_bank_cycle() {
        let bank = auto_fighter_bank_scenario();
        let last = bank.steps.last().expect("the bank cell has steps");
        assert!(
            matches!(
                last.wait.arm,
                Proof::FreshStatXpGain {
                    id: STRENGTH_STAT,
                    min: 1
                }
            ),
            "the widened step is the fresh Strength XP watch"
        );
        assert_eq!(last.wait.budget_ticks, 320);
        assert!(
            last.wait.budget_ticks > SCRIPT_GOLD_WATCH_TICKS + MEASURED_EXTRA_BANK_CYCLE_TICKS,
            "the bank watch must outlast the ordinary watch plus one extra bank cycle"
        );
        let bank_watches = post_start_steps(&bank);
        assert!(
            bank_watches[..bank_watches.len() - 1]
                .iter()
                .all(|step| step.wait.budget_ticks == SCRIPT_GOLD_WATCH_TICKS),
            "the widened budget stays on the last watch only"
        );

        // Peer cells keep the budget their own builder gives every post-Start
        // watch: the targets cell's cell-wide 1050, the food cell's ordinary
        // post-Start watch. Neither inherits the bank cell's override.
        let targets = auto_fighter_targets_scenario();
        let targets_last = targets.steps.last().expect("the targets cell has steps");
        assert!(
            post_start_steps(&targets)
                .iter()
                .all(|step| step.wait.budget_ticks == targets_last.wait.budget_ticks),
            "the targets cell keeps its cell-wide watch"
        );
        assert_eq!(targets_last.wait.budget_ticks, 1050);

        let food = auto_fighter_food_override_scenario();
        let food_last = food.steps.last().expect("the food cell has steps");
        assert!(
            post_start_steps(&food)
                .iter()
                .all(|step| step.wait.budget_ticks == SCRIPT_GOLD_WATCH_TICKS),
            "the food cell keeps the ordinary post-Start watch"
        );
        assert_eq!(food_last.wait.budget_ticks, 150);
    }

    /// The whole-cell cap must hold the wider watch in full. In report cell 4,
    /// run 3, the return witness holds before the second Bones take at host
    /// tick 183, so 183 is an upper bound on the widened watch's start.
    /// 183 + 320 = 503 ticks is 301.8 s, past the old 301.2 s cap.
    #[test]
    fn bank_cell_deadline_holds_a_full_last_watch() {
        const SLOWEST_MEASURED_WATCH_START_BOUND_TICK: u32 = 183;
        let bank = auto_fighter_bank_scenario();
        let needed = Duration::from_millis(
            u64::from(
                SLOWEST_MEASURED_WATCH_START_BOUND_TICK + AUTO_FIGHTER_BANK_FRESH_XP_WATCH_TICKS,
            ) * 600,
        );
        assert!(
            bank.settings.deadline >= needed,
            "the cap must hold a full last watch after the slowest measured prefix"
        );
        let old_cap = SCRIPT_GOLD_DEADLINE
            + Duration::from_millis(u64::from(AUTO_FIGHTER_FLOOR_CLEAR_TICKS) * 600);
        let added = Duration::from_millis(
            u64::from(AUTO_FIGHTER_BANK_FRESH_XP_WATCH_TICKS - SCRIPT_GOLD_WATCH_TICKS) * 600,
        );
        assert_eq!(
            bank.settings.deadline,
            old_cap + added,
            "the cap grows by exactly the added watch ticks"
        );
    }
}
