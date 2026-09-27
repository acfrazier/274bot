use super::*;
/// Selected 289 `obj.pack`: silver_ore=442, silver_bar=2355.
pub(crate) const SILVER_ORE_ID: i32 = 442;
pub(crate) const SILVER_BAR_ID: i32 = 2355;
pub(crate) const SUPERHEAT_MAGIC: i32 = 43;
pub(crate) const BRONZE_SMITHING: i32 = 1;
/// SuperheaterLogic Silver recipe level.
pub(crate) const SILVER_SMITHING: i32 = 20;
pub(crate) const STEEL_SMITHING: i32 = 30;
/// Selected 289 `obj.pack`: mithril_ore=447, mithril_bar=2359.
pub(crate) const MITHRIL_ORE_ID: i32 = 447;
pub(crate) const SUPERHEATER_NATURES_SEED: i32 = 200;
pub(crate) const SUPERHEATER_ORE_SEED: i32 = 100;
pub(crate) const SUPERHEATER_COAL_SEED: i32 = 200;
/// SuperheaterLogic `NATURES_MIN` / one-slot nature stack + 27 ore slots.
pub(crate) const SUPERHEATER_NATURES_MIN: i32 = 28;
pub(crate) const SUPERHEATER_SINGLE_ORE_TRIP: i32 = 27;
const SUPERHEATER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Bronze"),
}];

const SUPERHEATER_STEEL_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Steel"),
}];

const SUPERHEATER_FIRE_BATTLESTAFF_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Bronze"),
}];

const SUPERHEATER_SILVER_LOW_NATURES_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Silver"),
    },
    ScriptSettingInject {
        id: "natures",
        value: ScriptInjectValue::Num(28.0),
    },
];

const SUPERHEATER_MITHRIL_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Mithril"),
}];
#[derive(Clone, Copy)]
enum SuperheaterStaff {
    Fire,
    FireBattlestaff,
}

#[derive(Clone, Copy)]
enum SuperheaterRecipe {
    Bronze,
    Steel,
    /// Single-ore 27-slot trip + minimum natures (28).
    Silver,
    /// Mithril ore + 4 Coal per bar (5 + 20 ores per trip).
    Mithril,
}

pub(crate) fn superheater_scenario() -> Scenario {
    superheater_variant(
        "superheater",
        SUPERHEATER_INJECT,
        SuperheaterRecipe::Bronze,
        SuperheaterStaff::Fire,
    )
}

pub(crate) fn superheater_steel_scenario() -> Scenario {
    superheater_variant(
        "superheater_steel",
        SUPERHEATER_STEEL_INJECT,
        SuperheaterRecipe::Steel,
        SuperheaterStaff::Fire,
    )
}

pub(crate) fn superheater_fire_battlestaff_scenario() -> Scenario {
    superheater_variant(
        "superheater_fire_battlestaff",
        SUPERHEATER_FIRE_BATTLESTAFF_INJECT,
        SuperheaterRecipe::Bronze,
        SuperheaterStaff::FireBattlestaff,
    )
}

pub(crate) fn superheater_silver_low_natures_scenario() -> Scenario {
    superheater_variant(
        "superheater_silver_low_natures",
        SUPERHEATER_SILVER_LOW_NATURES_INJECT,
        SuperheaterRecipe::Silver,
        SuperheaterStaff::Fire,
    )
}

pub(crate) fn superheater_mithril_scenario() -> Scenario {
    superheater_variant(
        "superheater_mithril",
        SUPERHEATER_MITHRIL_INJECT,
        SuperheaterRecipe::Mithril,
        SuperheaterStaff::Fire,
    )
}

/// Empty pack at Varrock West. Banked staff, natures and recipe ores.
/// Script withdraws/equips the staff, casts Superheat Item on the primary
/// ore, deposits bars except natures, restocks and smelts again.
fn superheater_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    recipe: SuperheaterRecipe,
    staff: SuperheaterStaff,
) -> Scenario {
    let first_magic = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let first_smithing = Proof::StatXpGain {
        id: SMITHING_STAT,
        min: 1,
    };
    // Bronze/steel keep cumulative min-2 XP after restock. Silver uses a fresh
    // baseline so first-trip XP cannot satisfy resumed work alone.
    let (further_magic, further_smithing, terminal_proof) =
        if matches!(recipe, SuperheaterRecipe::Silver) {
            let fresh_magic = Proof::FreshStatXpGain {
                id: MAGIC_STAT,
                min: 1,
            };
            let fresh_smithing = Proof::FreshStatXpGain {
                id: SMITHING_STAT,
                min: 1,
            };
            (fresh_magic, fresh_smithing, fresh_smithing)
        } else {
            let further_magic = Proof::StatXpGain {
                id: MAGIC_STAT,
                min: 2,
            };
            let further_smithing = Proof::StatXpGain {
                id: SMITHING_STAT,
                min: 2,
            };
            (further_magic, further_smithing, further_smithing)
        };
    // secondary_id is None for single-ore Silver (27 ore slots, no pair ore).
    let (bar_id, primary_id, secondary_id, smithing, staff_id, staff_alias) = match (recipe, staff)
    {
        (SuperheaterRecipe::Bronze, SuperheaterStaff::Fire) => (
            BRONZE_BAR_ID,
            COPPER_ORE_ID,
            Some(TIN_ORE_ID),
            BRONZE_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Steel, SuperheaterStaff::Fire) => (
            STEEL_BAR_ID,
            IRON_ORE_ID,
            Some(COAL_ID),
            STEEL_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Bronze, SuperheaterStaff::FireBattlestaff) => (
            BRONZE_BAR_ID,
            COPPER_ORE_ID,
            Some(TIN_ORE_ID),
            BRONZE_SMITHING,
            FIRE_BATTLESTAFF_ID,
            "fire_battlestaff",
        ),
        (SuperheaterRecipe::Silver, SuperheaterStaff::Fire) => (
            SILVER_BAR_ID,
            SILVER_ORE_ID,
            None,
            SILVER_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Mithril, SuperheaterStaff::Fire) => (
            MITHRIL_BAR_ID,
            MITHRIL_ORE_ID,
            Some(COAL_ID),
            MITHRIL_SMITHING,
            STAFF_OF_FIRE_ID,
            "staff_of_fire",
        ),
        (SuperheaterRecipe::Steel, SuperheaterStaff::FireBattlestaff)
        | (SuperheaterRecipe::Silver, SuperheaterStaff::FireBattlestaff)
        | (SuperheaterRecipe::Mithril, SuperheaterStaff::FireBattlestaff) => {
            unreachable!("recipe split is independent of the staff split")
        }
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic, Smithing, bank stock and tele to Varrock West before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat magic {SUPERHEAT_MAGIC}"));
                cheat(c, &format!("setstat smithing {smithing}"));
                if matches!(staff, SuperheaterStaff::FireBattlestaff) {
                    // Both selected content revisions require Attack 30 to
                    // wield a Fire battlestaff (Magic 43 is seeded above).
                    cheat(c, "setstat attack 30");
                }
                cheat(c, &format!("givebank {staff_alias} 1"));
                cheat(
                    c,
                    &format!("givebank naturerune {SUPERHEATER_NATURES_SEED}"),
                );
                match recipe {
                    SuperheaterRecipe::Bronze => {
                        cheat(c, &format!("givebank copper_ore {SUPERHEATER_ORE_SEED}"));
                        cheat(c, &format!("givebank tin_ore {SUPERHEATER_ORE_SEED}"));
                    }
                    SuperheaterRecipe::Steel => {
                        cheat(c, &format!("givebank iron_ore {SUPERHEATER_ORE_SEED}"));
                        cheat(c, &format!("givebank coal {SUPERHEATER_COAL_SEED}"));
                    }
                    SuperheaterRecipe::Silver => {
                        cheat(c, &format!("givebank silver_ore {SUPERHEATER_ORE_SEED}"));
                    }
                    SuperheaterRecipe::Mithril => {
                        cheat(c, &format!("givebank mithril_ore {SUPERHEATER_ORE_SEED}"));
                        cheat(c, &format!("givebank coal {SUPERHEATER_COAL_SEED}"));
                    }
                }
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    let mut before_start = vec![
        (
            "confirm Magic 43 before Start",
            Proof::Stat {
                id: MAGIC_STAT,
                min: SUPERHEAT_MAGIC,
            },
        ),
        (
            "confirm Smithing before Start",
            Proof::Stat {
                id: SMITHING_STAT,
                min: smithing,
            },
        ),
        (
            "confirm no seeded staff in pack before Start",
            Proof::ItemIdAtMost {
                id: staff_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded natures in pack before Start",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded primary ore in pack before Start",
            Proof::ItemIdAtMost {
                id: primary_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded bars in pack before Start",
            Proof::ItemIdAtMost {
                id: bar_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded iron bars in pack before Start",
            Proof::ItemIdAtMost {
                id: IRON_BAR_ID,
                count: 0,
            },
        ),
    ];
    if let Some(secondary_id) = secondary_id {
        before_start.push((
            "confirm no seeded secondary ore in pack before Start",
            Proof::ItemIdAtMost {
                id: secondary_id,
                count: 0,
            },
        ));
    }
    if matches!(staff, SuperheaterStaff::FireBattlestaff) {
        before_start.push((
            "confirm Attack 30 for Fire battlestaff before Start",
            Proof::Stat { id: 0, min: 30 },
        ));
        before_start.push((
            "confirm default Staff of fire is absent from pack",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ));
    }
    match recipe {
        SuperheaterRecipe::Bronze => {
            before_start.push((
                "confirm no seeded steel bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: STEEL_BAR_ID,
                    count: 0,
                },
            ));
        }
        SuperheaterRecipe::Steel => {
            before_start.push((
                "confirm no seeded bronze bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: BRONZE_BAR_ID,
                    count: 0,
                },
            ));
        }
        SuperheaterRecipe::Silver => {
            before_start.push((
                "confirm no seeded bronze bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: BRONZE_BAR_ID,
                    count: 0,
                },
            ));
            before_start.push((
                "confirm no seeded steel bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: STEEL_BAR_ID,
                    count: 0,
                },
            ));
        }
        SuperheaterRecipe::Mithril => {
            before_start.push((
                "confirm no seeded bronze bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: BRONZE_BAR_ID,
                    count: 0,
                },
            ));
            before_start.push((
                "confirm no seeded steel bars in pack before Start",
                Proof::ItemIdAtMost {
                    id: STEEL_BAR_ID,
                    count: 0,
                },
            ));
        }
    }
    for (step_name, arm) in before_start {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact fire staff seed bank",
        Proof::BankItemId {
            id: staff_id,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact nature-rune seed bank",
        Proof::BankItemId {
            id: NATURE_RUNE_ID,
            count: SUPERHEATER_NATURES_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact primary ore seed bank",
        Proof::BankItemId {
            id: primary_id,
            count: SUPERHEATER_ORE_SEED,
        },
    ));
    if let Some(secondary_id) = secondary_id {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact secondary ore seed bank",
            Proof::BankItemId {
                id: secondary_id,
                count: match recipe {
                    SuperheaterRecipe::Bronze => SUPERHEATER_ORE_SEED,
                    SuperheaterRecipe::Steel | SuperheaterRecipe::Mithril => SUPERHEATER_COAL_SEED,
                    SuperheaterRecipe::Silver => unreachable!("silver has no secondary ore"),
                },
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bars in bank",
        Proof::BankItemIdAtMost {
            id: bar_id,
            count: 0,
        },
    ));
    if matches!(staff, SuperheaterStaff::FireBattlestaff) {
        steps.push(bank_fletcher_watch(
            "acknowledge Staff of fire is absent from the alternative-staff bank",
            Proof::BankItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    // Nature ceiling is ordered after first cast XP/bar so empty-pack 0 cannot
    // satisfy the at-most check before the first withdrawal.
    let nature_after_cast = if matches!(recipe, SuperheaterRecipe::Silver) {
        SUPERHEATER_NATURES_MIN - 1
    } else {
        49
    };
    let mut watch = vec![
        ("watch Magic XP from Superheat Item", first_magic),
        ("watch Smithing XP from the produced bar", first_smithing),
        (
            "watch the exact bar id after Start",
            Proof::ItemId {
                id: bar_id,
                count: 1,
            },
        ),
        (
            "watch at least one nature rune consumed",
            Proof::ItemIdAtMost {
                id: NATURE_RUNE_ID,
                count: nature_after_cast,
            },
        ),
        (
            "watch script-created bars enter a fresh bank",
            Proof::BankItemId {
                id: bar_id,
                count: 1,
            },
        ),
        (
            "watch natures kept in pack across deposit",
            Proof::ItemId {
                id: NATURE_RUNE_ID,
                count: 1,
            },
        ),
    ];
    if matches!(recipe, SuperheaterRecipe::Silver) {
        watch.push((
            "watch a restock of the full single-ore trip",
            Proof::ItemId {
                id: primary_id,
                count: SUPERHEATER_SINGLE_ORE_TRIP,
            },
        ));
        watch.push((
            "watch natures topped to the minimum after restock",
            Proof::ItemId {
                id: NATURE_RUNE_ID,
                count: SUPERHEATER_NATURES_MIN,
            },
        ));
    } else {
        watch.push((
            "watch a restock of the exact primary ore",
            Proof::ItemId {
                id: primary_id,
                count: 1,
            },
        ));
        if let Some(secondary_id) = secondary_id {
            watch.push((
                "watch a restock of the exact secondary ore",
                Proof::ItemId {
                    id: secondary_id,
                    count: match recipe {
                        SuperheaterRecipe::Bronze => 1,
                        SuperheaterRecipe::Steel => 2,
                        SuperheaterRecipe::Mithril => 4,
                        SuperheaterRecipe::Silver => unreachable!("silver has no secondary ore"),
                    },
                },
            ));
        }
    }
    if matches!(staff, SuperheaterStaff::FireBattlestaff) {
        watch.push((
            "watch Staff of fire never enter the pack",
            Proof::ItemIdAtMost {
                id: STAFF_OF_FIRE_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        (
            "watch no iron bar from a partial recipe",
            Proof::ItemIdAtMost {
                id: IRON_BAR_ID,
                count: 0,
            },
        ),
        (
            "watch the script close its superheat bank",
            Proof::BankClosed,
        ),
        (
            "watch another exact bar after restock",
            Proof::ItemId {
                id: bar_id,
                count: 1,
            },
        ),
        ("watch Magic XP beyond the first trip", further_magic),
        ("watch Smithing XP beyond the first trip", further_smithing),
    ]);
    for (step_name, arm) in watch {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: terminal_proof,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("Superheater"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
