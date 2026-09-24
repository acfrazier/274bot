use super::*;
pub(crate) const MOSS_GIANT_FOOD: i32 = 10;
/// Bank-cell preparation: MossGiant only banks once the pack's food is gone, so
/// the bank cell carries a shortfall instead of a full pack; ChaosDruidKiller's
/// own `tripPrepared` needs `foodWithdraw` (12) in the field, so 8 forces its
/// declared `prepare-trip` end.
const MOSS_GIANT_BANK_FOOD: i32 = 2;
/// Restock lines the cards themselves withdraw to (MossGiant's declared
/// `foodWithdraw` default 20, AutoFighter's 10, HillGiant's 12).
const MOSS_GIANT_BANK_RESTOCK: i32 = 20;
const RUNE_ARROW_ID: i32 = 892;
const MOSS_GIANT_DART_SUPPLY: i32 = 80;
const MOSS_GIANT_DART_BANK_FOOD: i32 = 15;
const MOSS_GIANT_DART_RANGED: i32 = 50;
const MOSS_GIANT_DART_FOOD_WITHDRAW: i32 = 10;
const MOSS_GIANT_DART_FIELD_RADIUS: i32 = 12;
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
pub(super) const MOSS_GIANT_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Moss Giant food",
    carry: &[("Lobster", MOSS_GIANT_FOOD as u32)],
}];
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
const MOSS_GIANT_LOOT_EMPTY: &[i32] = &[BIG_BONES_ID, NOTED_BIG_BONES_ID];
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
                cheat(c, &format!("givebank bronze_dart {MOSS_GIANT_DART_SUPPLY}"));
                cheat(c, &format!("givebank lobster {MOSS_GIANT_DART_BANK_FOOD}"));
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
            Proof::EquipmentId { id: BRONZE_DART_ID },
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
        (
            "watch Ranged XP from worn Bronze darts after Start",
            ranged_xp,
        ),
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
