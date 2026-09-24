use super::{combat::*, navigation::*, shop::*};
use crate::*;
mod chickens;
pub(crate) use chickens::*;
mod thiever;
pub(crate) use thiever::thiever_scenario;
pub use thiever::thiever_sustained_scenario;
mod alcher;
pub(crate) use alcher::*;
mod bank_fletcher;
pub(crate) use bank_fletcher::*;
mod dart_fletcher;
pub(crate) use dart_fletcher::*;
mod herb_cleaner;
pub(crate) use herb_cleaner::*;
mod gem_cutter;
pub(crate) use gem_cutter::*;
mod door_opener;
pub(crate) use door_opener::*;
mod agility;
pub(crate) use agility::*;
mod flax;
pub(crate) use flax::*;
mod superheater;
pub(crate) use superheater::*;
mod vial_filler;
pub(crate) use vial_filler::*;
mod potion_maker;
pub(crate) use potion_maker::*;
mod tanner;
pub(crate) use tanner::*;
mod runecrafting;
pub(crate) use runecrafting::*;
mod ardy_thieving;
pub(crate) use ardy_thieving::*;
mod gnome;
pub(crate) use gnome::*;

pub(crate) const STRENGTH_STAT: i32 = 2;

/// East Ardougne market guard tile (rs2b0t Thiever live gold).
pub(super) const ARDOUGNE_GUARD: WorldTile = WorldTile {
    x: 2661,
    z: 3306,
    level: 0,
};

/// Durable prepare / run-prepared gates for Thiever (not script progress).
pub(super) const THIEVER_FIXTURE_PREREQS: &[Proof] = &[
    Proof::ArrivedNear {
        x: 2661,
        z: 3306,
        level: 0,
        radius: 10,
    },
    Proof::Stat { id: 17, min: 50 },
    Proof::Stat { id: 3, min: 50 },
    Proof::Item {
        name: "Lobster",
        count: 10,
    },
];

/// Varrock West bank stand (Alcher / BankFletcher gold).
const VARROCK_WEST_BANK: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};

pub(crate) const NATURE_RUNE_ID: i32 = 561;
pub(crate) const COINS_ID: i32 = 995;
pub(crate) const STAFF_OF_FIRE_ID: i32 = 1387;

pub(crate) const LOGS_ID: i32 = 1511;
pub(crate) const BOW_STRING_ID: i32 = 1777;
const VARROCK_WEST_BANK_BOOTH_ID: i32 = 2213;

pub(super) fn bank_fletcher_watch(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

fn bank_fletcher_open_seed_bank(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                // Use (OP_LOC1) starts banker dialogue. Resolve Use-quickly
                // from the selected booth's published actions instead.
                let booth = WorldTile {
                    x: VARROCK_WEST_BANK.x + 1,
                    z: VARROCK_WEST_BANK.z,
                    level: VARROCK_WEST_BANK.level,
                };
                matches!(
                    Interactions::new(snapshot, c).open_booth_at(booth, VARROCK_WEST_BANK_BOOTH_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

pub(crate) fn bank_fletcher_close_seed_bank() -> Step {
    Step {
        name: "close the acknowledged seed bank before Start",
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).close_modal(),
                    SendResult::Sent { .. } | SendResult::Refused { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::BankClosed,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

pub(crate) const FEATHER_ID: i32 = 314;
pub(crate) const UNIDENTIFIED_GUAM_ID: i32 = 199;
pub(crate) const GUAM_LEAF_ID: i32 = 249;
pub(crate) const FLETCHING_STAT: i32 = 9;
pub(crate) const CRAFTING_STAT: i32 = 12;
pub(crate) const HERBLORE_STAT: i32 = 15;

/// FlaxRunner meet tile. Same stand as the shared pair witness.
pub(super) const FLAX_MEET: WorldTile = WorldTile {
    x: 2719,
    z: 3471,
    level: 0,
};
/// Duel Arena challenge-area seed. Same stand as the shared pair witness.
pub(super) const DUEL_CHALLENGE: WorldTile = WorldTile {
    x: 3368,
    z: 3274,
    level: 0,
};

pub(crate) const MAGIC_STAT: i32 = 6;
pub(crate) const SMITHING_STAT: i32 = 13;
pub(crate) const COPPER_ORE_ID: i32 = 436;
pub(crate) const TIN_ORE_ID: i32 = 438;
pub(crate) const IRON_ORE_ID: i32 = 440;
pub(crate) const COAL_ID: i32 = 453;
pub(crate) const FIRE_BATTLESTAFF_ID: i32 = 1393;
pub(crate) const BRONZE_BAR_ID: i32 = 2349;
pub(crate) const IRON_BAR_ID: i32 = 2351;
pub(crate) const STEEL_BAR_ID: i32 = 2353;
/// SuperheaterLogic Mithril recipe level; 4 Coal per bar (5 bars / 27-slot trip).
pub(crate) const MITHRIL_SMITHING: i32 = 50;
pub(crate) const MITHRIL_BAR_ID: i32 = 2359;

pub(crate) const VIAL_OF_WATER_ID: i32 = 227;

// The seed opener performs once: start adjacent so Sent means bank operation.
pub(super) const FALADOR_WEST_BANK: WorldTile = WorldTile {
    x: 2946,
    z: 3368,
    level: 0,
};
pub(super) const FALADOR_WEST_BOOTH: WorldTile = WorldTile {
    x: 2946,
    z: 3367,
    level: 0,
};
pub(crate) const FALADOR_EAST_BANK: WorldTile = WorldTile {
    x: 3013,
    z: 3355,
    level: 0,
};

pub(crate) const SOFT_LEATHER_ID: i32 = 1741;
pub(crate) const HARD_LEATHER_ID: i32 = 1743;

pub(crate) const AL_KHARID_BANK: WorldTile = WorldTile {
    x: 3269,
    z: 3167,
    level: 0,
};

pub(crate) const AIR_RUNE_ID: i32 = 556;
pub(super) const PAIR_AIR_FIRST_LOAD: i32 = 25;
pub(super) const PAIR_MULE_FIRST_LOAD: i32 = 27;

pub(super) const VARROCK_EAST_BANK: WorldTile = WorldTile {
    x: 3253,
    z: 3420,
    level: 0,
};

fn open_seed_booth(name: &'static str, booth: WorldTile, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).open_booth_at(booth, VARROCK_WEST_BANK_BOOTH_ID),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

pub(super) fn tanner_open_seed_bank(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).open_nearest_booth(),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

pub(crate) const HITPOINTS_STAT: i32 = 3;

/// Six free slots at the Baker's stall stand. Flee, clues off. Script steals
/// cake/bread/chocolate slice with Thieving XP, deposits the acquired stock
/// when the pack is full, returns to STAND and steals again. Fight stays
/// pending. Cake 1891 is the sequential identity; chocolate cake 1897 is not
/// stall food.

pub(crate) const KNIFE_ID: i32 = 946;
pub(crate) const RUNE_PICKAXE_ID: i32 = 1275;
const NOTED_COAL_ID: i32 = 454;
/// A Rune pickaxe plus 26 unstackable Knives leaves one slot for Coal.
/// CoalTrucks keeps non-coal items when it empties the pack into a truck.
pub(crate) const COAL_BALLAST_KNIVES: i32 = 26;
/// With the ordinary prayer/ranged/magic defaults, 48 in each melee/HP stat
/// yields native combat level 55 without over-leveling the fixture.
const COAL_MELEE_LEVEL: i32 = 48;

const COAL_MINE: WorldTile = WorldTile {
    x: 2582,
    z: 3481,
    level: 0,
};
pub(crate) const COAL_MINE_TRUCK_STAND: WorldTile = WorldTile {
    x: 2575,
    z: 3486,
    level: 0,
};

/// One free product slot at the south-bank Magic tree. fletchLogs off. Seed WC 75, Rune
/// axe 1359, and retained nonproduct Knife ballast. The script chops one
/// magic log 1513 with Woodcutting XP, deposits it at the upstairs gnome
/// booth, returns to ground and chops again. This qualifies the resource
/// cycle, not ordinary 28-slot throughput. Death recovery stays out.
pub(crate) fn coal_trucks_scenario() -> Scenario {
    let stand = COAL_MINE;
    let first_xp = Proof::StatXpGain {
        id: MINING_STAT,
        min: 1,
    };
    let coal = Proof::ItemId {
        id: COAL_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed mining 60, combat-safe melee stats, Rune pickaxe and 26-Knife ballast",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat mining 60");
                cheat(c, &format!("setstat attack {COAL_MELEE_LEVEL}"));
                cheat(c, &format!("setstat strength {COAL_MELEE_LEVEL}"));
                cheat(c, &format!("setstat defence {COAL_MELEE_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COAL_MELEE_LEVEL}"));
                cheat(c, "give rune_pickaxe 1");
                cheat(c, &format!("give knife {COAL_BALLAST_KNIVES}"));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat {
                id: 0,
                min: COAL_MELEE_LEVEL,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Attack 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: 0,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Strength 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Defence 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: 1,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Hitpoints 48 on the safe tile before the bat mine",
            Proof::Stat {
                id: 3,
                min: COAL_MELEE_LEVEL,
            },
        ),
        (
            "confirm Mining 60 before Start",
            Proof::Stat {
                id: MINING_STAT,
                min: 60,
            },
        ),
        (
            "confirm Rune pickaxe 1275 before Start",
            Proof::ItemId {
                id: RUNE_PICKAXE_ID,
                count: 1,
            },
        ),
        (
            "confirm 26 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: COAL_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 26 retained nonproduct Knives and one available slot before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: COAL_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded coal in pack before Start",
            Proof::ItemIdAtMost {
                id: COAL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted coal in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_COAL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(Step {
        name: "teleport into the giant-bat mine after combat readiness is acknowledged",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Mining XP from coal rocks after Start", first_xp),
        ("watch exact Coal 453 mined after Start", coal),
        (
            "watch arrival at the mine coal truck after the pack fill",
            Proof::ArrivedNear {
                x: COAL_MINE_TRUCK_STAND.x,
                z: COAL_MINE_TRUCK_STAND.z,
                level: COAL_MINE_TRUCK_STAND.level,
                radius: 4,
            },
        ),
        (
            "watch the pack empty of coal after the mine-truck deposit",
            Proof::ItemIdAtMost {
                id: COAL_ID,
                count: 0,
            },
        ),
        ("watch another exact Coal 453 after the truck deposit", coal),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "coal_trucks",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: coal,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("CoalTrucks"),
            terminal_shot: Some("coal_trucks"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) const COOKING_STAT: i32 = 7;
pub(crate) const RAW_SALMON_ID: i32 = 331;
pub(crate) const SALMON_ID: i32 = 329;
const NOTED_RAW_SALMON_ID: i32 = 332;
const NOTED_SALMON_ID: i32 = 330;
const RAW_LOBSTER_ID: i32 = 377;
pub(crate) const LOBSTER_ID: i32 = 379;
const NOTED_RAW_LOBSTER_ID: i32 = 378;
pub(crate) const NOTED_LOBSTER_ID: i32 = 380;
const BURNT_LOBSTER_ID: i32 = 381;
const BURNT_FISH_1_ID: i32 = 323;
const BURNT_FISH_2_ID: i32 = 343;
const NOTED_COPPER_ORE_ID: i32 = 437;
const NOTED_TIN_ORE_ID: i32 = 439;
const NOTED_IRON_ORE_ID: i32 = 441;
const NOTED_BRONZE_BAR_ID: i32 = 2350;
const NOTED_STEEL_BAR_ID: i32 = 2354;
pub(crate) const COOKING_FIXTURE_LEVEL: i32 = 80;
pub(crate) const COOK_RAW_SEED: i32 = 56;
pub(crate) const SMELT_ORE_SEED: i32 = 56;
pub(crate) const STEEL_COAL_SEED: i32 = 112;

pub(super) const CATHERBY_BANK: WorldTile = WorldTile {
    x: 2809,
    z: 3441,
    level: 0,
};
pub(crate) const CATHERBY_RANGE_STAND: WorldTile = WorldTile {
    x: 2817,
    z: 3443,
    level: 0,
};
pub(crate) const AL_KHARID_FURNACE: WorldTile = WorldTile {
    x: 3275,
    z: 3185,
    level: 0,
};
pub(crate) const RED_SPIDERS_EGGS_ID: i32 = 223;
const NOTED_RED_SPIDERS_EGGS_ID: i32 = 224;
const NOTED_EYE_OF_NEWT_ID: i32 = 222;
pub(crate) const HERBLORE_EGG_FOOD_SEED: i32 = 50;
/// Canonical `FOOD_DEFAULT_COUNT` / `foodWithdraw` default. Carried at Start
/// so loot `needsRestock` is false (takeFood && foodCount < 1).
pub(crate) const HERBLORE_EGG_FOOD_CARRY: i32 = 10;
pub(crate) const HERBLORE_NEWT_COIN_SEED: i32 = 5000;
const EGG_FIELD: WorldTile = WorldTile {
    x: 3120,
    z: 9952,
    level: 0,
};
/// Safe native approach stand for the selected Edgeville booth. Headed shots
/// for both selected revision packs show this south-adjacent tile as
/// walkable, immediately next to booth 2213, without changing bank APIs.
pub(crate) const EDGEVILLE_BANK_APPROACH: WorldTile = WorldTile {
    x: 3096,
    z: 3494,
    level: 0,
};
/// Selected 274/289 packs both contain this Edgeville booth as id 2213 with
/// native booth op2 (`Use-quickly`). Keep the walk stand separate from it.
pub(crate) const EDGEVILLE_BANK_BOOTH: WorldTile = WorldTile {
    x: 3096,
    z: 3493,
    level: 0,
};
pub(crate) const EDGEVILLE_BANK_BOOTH_ID: i32 = 2213;
pub(super) const BETTY_SHOP: WorldTile = WorldTile {
    x: 3012,
    z: 3259,
    level: 0,
};
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const DRAYNOR_BANK: WorldTile = WorldTile {
    x: 3093,
    z: 3243,
    level: 0,
};
/// Stock-facing walkable adjacent for `DRAYNOR_BANK_BOOTH`. Capture
/// `2026-09-18T06-11-18` booth 2213@3091,3243 is shape 10 angle 1 force 0
/// 1×1; collision at 3092,3243 is open (0). Canonical `DRAYNOR_BANK`
/// 3093,3243 is Chebyshev 2 — `open_booth_at_matching` refuses Unreachable
/// and does not walk. Keep this seed stand separate from `DRAYNOR_BANK`.
pub(crate) const DRAYNOR_BANK_APPROACH: WorldTile = WorldTile {
    x: 3092,
    z: 3243,
    level: 0,
};
/// Selected 274/289 Draynor booth with native `Use-quickly` (id 2213). Capture
/// tile matches the open booth west of `DRAYNOR_BANK`; closed 2214/2215 are
/// not operable. Keep the walk stand separate from the booth identity.
pub(crate) const DRAYNOR_BANK_BOOTH: WorldTile = WorldTile {
    x: 3091,
    z: 3243,
    level: 0,
};
pub(crate) const DRAYNOR_BANK_BOOTH_ID: i32 = 2213;

const HERBLORE_EGGS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "secondary",
        value: ScriptInjectValue::Str("Red spiders' eggs"),
    },
    // scriptFood reads the selected loadout, not a food setting. Blank
    // loadout uses the operator's first saved carry (headed LIVE: Swordfish).
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Scenario Herblore food"),
    },
    ScriptSettingInject {
        id: "foodWithdraw",
        value: ScriptInjectValue::Num(HERBLORE_EGG_FOOD_CARRY as f64),
    },
];
const HERBLORE_EGGS_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Herblore food",
    carry: &[("Lobster", HERBLORE_EGG_FOOD_CARRY as u32)],
}];
const HERBLORE_NEWT_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "secondary",
    value: ScriptInjectValue::Str("Eye of newt"),
}];

const COOK_BOT_SALMON_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "fish",
        value: ScriptInjectValue::Str("Raw salmon"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Catherby"),
    },
    ScriptSettingInject {
        id: "surface",
        value: ScriptInjectValue::Str("Range"),
    },
];

const COOK_BOT_LOBSTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "fish",
        value: ScriptInjectValue::Str("Raw lobster"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Catherby"),
    },
    ScriptSettingInject {
        id: "surface",
        value: ScriptInjectValue::Str("Range"),
    },
];

const SMELTER_BOT_BRONZE_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Bronze"),
}];

const SMELTER_BOT_STEEL_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "bar",
    value: ScriptInjectValue::Str("Steel"),
}];

pub(crate) fn cook_bot_scenario() -> Scenario {
    cook_bot_variant(CookSpec {
        name: "cook_bot",
        inject: COOK_BOT_SALMON_INJECT,
        raw_alias: "raw_salmon",
        raw_id: RAW_SALMON_ID,
        product_id: SALMON_ID,
        wrong_product_id: LOBSTER_ID,
        noted_raw_id: NOTED_RAW_SALMON_ID,
        noted_product_id: NOTED_SALMON_ID,
    })
}

pub(crate) fn cook_bot_lobster_scenario() -> Scenario {
    cook_bot_variant(CookSpec {
        name: "cook_bot_lobster",
        inject: COOK_BOT_LOBSTER_INJECT,
        raw_alias: "raw_lobster",
        raw_id: RAW_LOBSTER_ID,
        product_id: LOBSTER_ID,
        wrong_product_id: SALMON_ID,
        noted_raw_id: NOTED_RAW_LOBSTER_ID,
        noted_product_id: NOTED_LOBSTER_ID,
    })
}

struct CookSpec {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    raw_alias: &'static str,
    raw_id: i32,
    product_id: i32,
    wrong_product_id: i32,
    noted_raw_id: i32,
    noted_product_id: i32,
}

/// Empty pack at Catherby bank. Banked raw fish, never cooked/burnt/noted.
/// Cooking 80 is a fixture seed so ordinary burn randomness does not replace
/// the product contract; the script encodes no cook level. Range only — Fire
/// stays behind native fire work. Script withdraws, cooks on the Catherby
/// Range, deposits the exact product, restocks raw, returns and cooks again.
fn cook_bot_variant(spec: CookSpec) -> Scenario {
    let CookSpec {
        name,
        inject,
        raw_alias,
        raw_id,
        product_id,
        wrong_product_id,
        noted_raw_id,
        noted_product_id,
    } = spec;
    let range = Proof::ArrivedNear {
        x: CATHERBY_RANGE_STAND.x,
        z: CATHERBY_RANGE_STAND.z,
        level: CATHERBY_RANGE_STAND.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let bank = CATHERBY_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed cooking, banked raw fish, and tele to Catherby bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat cooking {COOKING_FIXTURE_LEVEL}"));
                cheat(c, &format!("givebank {raw_alias} {COOK_RAW_SEED}"));
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
    for (step_name, arm) in [
        (
            "confirm Cooking 80 before Start",
            Proof::Stat {
                id: COOKING_STAT,
                min: COOKING_FIXTURE_LEVEL,
            },
        ),
        (
            "confirm no seeded raw fish in pack before Start",
            Proof::ItemIdAtMost {
                id: raw_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded cooked product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong cooked fish in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded burnt fish 323 in pack before Start",
            Proof::ItemIdAtMost {
                id: BURNT_FISH_1_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded burnt fish in pack before Start",
            Proof::ItemIdAtMost {
                id: BURNT_FISH_2_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded burnt lobster in pack before Start",
            Proof::ItemIdAtMost {
                id: BURNT_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted raw in pack before Start",
            Proof::ItemIdAtMost {
                id: noted_raw_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted product in pack before Start",
            Proof::ItemIdAtMost {
                id: noted_product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact raw-fish seed bank",
        Proof::BankItemId {
            id: raw_id,
            count: COOK_RAW_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded cooked product in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted raw in bank",
        Proof::BankItemIdAtMost {
            id: noted_raw_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch arrival at the Catherby Range after Start", range),
        (
            "watch Cooking XP from the Catherby Range after Start",
            Proof::StatXpGain {
                id: COOKING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted cooked fish after Start", product),
        (
            "watch the withdrawn raw finish converting",
            Proof::ItemIdAtMost {
                id: raw_id,
                count: 0,
            },
        ),
        (
            "watch script-cooked fish enter a fresh Catherby bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of cooked fish after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of exact raw fish",
            Proof::ItemId {
                id: raw_id,
                count: 1,
            },
        ),
        ("watch the script close its cook bank", Proof::BankClosed),
        ("watch return to the Catherby Range after restock", range),
        ("watch another exact cooked fish after restock", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("CookBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn smelter_bot_scenario() -> Scenario {
    smelter_bot_variant(SmelterSpec {
        name: "smelter_bot",
        inject: SMELTER_BOT_BRONZE_INJECT,
        smithing: 1,
        primary_alias: "copper_ore",
        primary_id: COPPER_ORE_ID,
        primary_seed: SMELT_ORE_SEED,
        secondary_alias: "tin_ore",
        secondary_id: TIN_ORE_ID,
        secondary_seed: SMELT_ORE_SEED,
        product_id: BRONZE_BAR_ID,
        wrong_product_id: STEEL_BAR_ID,
        noted_primary_id: NOTED_COPPER_ORE_ID,
        noted_secondary_id: NOTED_TIN_ORE_ID,
        noted_product_id: NOTED_BRONZE_BAR_ID,
    })
}

pub(crate) fn smelter_bot_steel_scenario() -> Scenario {
    smelter_bot_variant(SmelterSpec {
        name: "smelter_bot_steel",
        inject: SMELTER_BOT_STEEL_INJECT,
        smithing: 30,
        primary_alias: "iron_ore",
        primary_id: IRON_ORE_ID,
        primary_seed: SMELT_ORE_SEED,
        secondary_alias: "coal",
        secondary_id: COAL_ID,
        secondary_seed: STEEL_COAL_SEED,
        product_id: STEEL_BAR_ID,
        wrong_product_id: BRONZE_BAR_ID,
        noted_primary_id: NOTED_IRON_ORE_ID,
        noted_secondary_id: NOTED_COAL_ID,
        noted_product_id: NOTED_STEEL_BAR_ID,
    })
}

struct SmelterSpec {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    smithing: i32,
    primary_alias: &'static str,
    primary_id: i32,
    primary_seed: i32,
    secondary_alias: &'static str,
    secondary_id: i32,
    secondary_seed: i32,
    product_id: i32,
    wrong_product_id: i32,
    noted_primary_id: i32,
    noted_secondary_id: i32,
    noted_product_id: i32,
}

/// Empty pack at Al-Kharid bank. Banked ores, never bars. Script withdraws
/// the recipe, smelts at the real furnace, deposits the exact bar, restocks
/// ore, returns and smelts again. Smithing main panel is not this hop.
fn smelter_bot_variant(spec: SmelterSpec) -> Scenario {
    let SmelterSpec {
        name,
        inject,
        smithing,
        primary_alias,
        primary_id,
        primary_seed,
        secondary_alias,
        secondary_id,
        secondary_seed,
        product_id,
        wrong_product_id,
        noted_primary_id,
        noted_secondary_id,
        noted_product_id,
    } = spec;
    let furnace = Proof::ArrivedNear {
        x: AL_KHARID_FURNACE.x,
        z: AL_KHARID_FURNACE.z,
        level: AL_KHARID_FURNACE.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let bank = AL_KHARID_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed smithing, banked ores, and tele to Al-Kharid bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat smithing {smithing}"));
                cheat(c, &format!("givebank {primary_alias} {primary_seed}"));
                cheat(c, &format!("givebank {secondary_alias} {secondary_seed}"));
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
    for (step_name, arm) in [
        (
            "confirm Smithing before Start",
            Proof::Stat {
                id: SMITHING_STAT,
                min: smithing,
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
            "confirm no seeded secondary ore in pack before Start",
            Proof::ItemIdAtMost {
                id: secondary_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded bars in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong bars in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_product_id,
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
        (
            "confirm no seeded noted product in pack before Start",
            Proof::ItemIdAtMost {
                id: noted_product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact primary-ore seed bank",
        Proof::BankItemId {
            id: primary_id,
            count: primary_seed,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact secondary-ore seed bank",
        Proof::BankItemId {
            id: secondary_id,
            count: secondary_seed,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bars in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted primary in bank",
        Proof::BankItemIdAtMost {
            id: noted_primary_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted secondary in bank",
        Proof::BankItemIdAtMost {
            id: noted_secondary_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the Al-Kharid furnace after Start",
            furnace,
        ),
        (
            "watch Smithing XP from the furnace after Start",
            Proof::StatXpGain {
                id: SMITHING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bar after Start", product),
        (
            "watch the withdrawn primary ore finish converting",
            Proof::ItemIdAtMost {
                id: primary_id,
                count: 0,
            },
        ),
        (
            "watch script-smelted bars enter a fresh Al-Kharid bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bars after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of exact primary ore",
            Proof::ItemId {
                id: primary_id,
                count: 1,
            },
        ),
        ("watch the script close its smelt bank", Proof::BankClosed),
        (
            "watch return to the Al-Kharid furnace after restock",
            furnace,
        ),
        ("watch another exact bar after restock", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("SmelterBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Empty pack at the Seers flax bank. Banked flax 1779, never bow string.
/// Script withdraws, climbs to the wheel, spins Flax into 1777 with Crafting
/// XP, deposits, restocks, returns upstairs and spins again. Wool is not this
/// core.

/// Open one exact booth until the bank arm holds. Runner re-fires `Repeat`
/// every tick *before* checking the arm: re-clicking Use-quickly on an already
/// open loaded bank bumps the bank session and clears `bank_loaded` /
/// `bank_side`, so deposit then hard-fails. Skip the booth send once the
/// current session is open and loaded.
pub(super) fn herblore_open_seed_bank_at(
    name: &'static str,
    arm: Proof,
    booth: WorldTile,
    booth_id: i32,
) -> Step {
    Step {
        name,
        kind: StepKind::Repeat {
            send: Box::new(move |c, snapshot| {
                if snapshot.bank_component_id() >= 0 && snapshot.bank_loaded() {
                    return true;
                }
                match Interactions::new(snapshot, c).open_booth_at(booth, booth_id) {
                    SendResult::Sent { .. } => true,
                    SendResult::Refused {
                        reason:
                            SendReason::SceneUnavailable
                            | SendReason::OffScene
                            | SendReason::StaleTarget,
                        ..
                    } => true,
                    SendResult::Refused { reason, .. } => {
                        eprintln!(
                            "[scenario] exact booth {booth_id}@{},{} send refused: {reason:?}",
                            booth.x, booth.z
                        );
                        false
                    }
                }
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

fn herblore_open_seed_bank(name: &'static str, arm: Proof) -> Step {
    herblore_open_seed_bank_at(name, arm, EDGEVILLE_BANK_BOOTH, EDGEVILLE_BANK_BOOTH_ID)
}

fn herblore_seed_bank_readiness() -> Step {
    bank_fletcher_watch(
        "acknowledge exact Edgeville booth identity and Use-quickly action before bank send",
        Proof::LocActionNear {
            id: EDGEVILLE_BANK_BOOTH_ID,
            x: EDGEVILLE_BANK_BOOTH.x,
            z: EDGEVILLE_BANK_BOOTH.z,
            level: EDGEVILLE_BANK_BOOTH.level,
            radius: 0,
            action: "Use-quickly",
            present: true,
        },
    )
}

fn herblore_newt_seed_bank_readiness() -> Step {
    bank_fletcher_watch(
        "acknowledge exact Draynor booth identity and Use-quickly action before bank send",
        Proof::LocActionNear {
            id: DRAYNOR_BANK_BOOTH_ID,
            x: DRAYNOR_BANK_BOOTH.x,
            z: DRAYNOR_BANK_BOOTH.z,
            level: DRAYNOR_BANK_BOOTH.level,
            radius: 0,
            action: "Use-quickly",
            present: true,
        },
    )
}

/// HerbloreSecondaries default Red spiders' eggs. Banked lobster via native
/// note seed + deposit (stock289 has no `givebank`). Fixture loadout pins
/// `scriptFood` to Lobster (no food setting; blank loadout uses the operator
/// first carry). Carry `FOOD_DEFAULT_COUNT` at the field so loot starts
/// instead of an empty-pack Edgeville restock. Ground Take 223, deposit,
/// empty product pack, close, return, further Take. Eggs are not given.
pub(crate) fn herblore_secondaries_scenario() -> Scenario {
    let field = EGG_FIELD;
    let bank_approach = EDGEVILLE_BANK_APPROACH;
    let eggs = Proof::ItemId {
        id: RED_SPIDERS_EGGS_ID,
        count: 1,
    };
    // stock289: 379=lobster (nonstackable), 380=cert_lobster (stackable note).
    let lobster = NativeSeed {
        unnoted_id: LOBSTER_ID,
        debug_alias: "lobster",
        note_alias: Some("cert_lobster"),
        quantity: HERBLORE_EGG_FOOD_SEED,
        note_id: Some(NOTED_LOBSTER_ID),
    };
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed banked lobster and tele to the safe Edgeville booth approach before Start",
        bank_approach,
        vec![lobster],
        "hitpoints",
        10,
    ));
    for (step_name, arm) in [
        (
            "confirm the exact noted lobster seed in pack before deposit",
            Proof::ItemId {
                id: NOTED_LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "bound the noted lobster seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "confirm no seeded eggs in pack before deposit",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eye of newt in pack before deposit",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted eggs in pack before deposit",
            Proof::ItemIdAtMost {
                id: NOTED_RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(herblore_seed_bank_readiness());
    steps.push(herblore_open_seed_bank(
        "open and acknowledge the lobster seed bank",
        Proof::BankItemIdAtMost {
            id: LOBSTER_ID,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the noted lobster seed through the bank window",
        vec![lobster],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact lobster food seed bank",
            Proof::BankItemId {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "bound the lobster food seed bank count",
            Proof::BankItemIdAtMost {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_SEED,
            },
        ),
        (
            "confirm the noted lobster seed was removed from pack",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "confirm no noted lobster remains in bank",
            Proof::BankItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded red spiders' eggs in bank",
            Proof::BankItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded noted eggs in bank",
            Proof::BankItemIdAtMost {
                id: NOTED_RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded eye of newt in bank",
            Proof::BankItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    // Pack is empty of seed after deposit; clearinv only after bank accepts seed.
    // Carry canonical food so takeFood loot does not BankTrip before first Take.
    steps.push(Step {
        name: "tele to the Edgeville dungeon egg field with lobster food before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give lobster {HERBLORE_EGG_FOOD_CARRY}"));
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded eggs in pack before Start",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eye of newt in pack before Start",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted eggs in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted lobster in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_LOBSTER_ID,
                count: 0,
            },
        ),
        (
            "confirm lobster food carry in pack before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_CARRY,
            },
        ),
        (
            "bound the lobster food carry in pack before Start",
            Proof::ItemIdAtMost {
                id: LOBSTER_ID,
                count: HERBLORE_EGG_FOOD_CARRY,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch exact red spiders' eggs 223 from the ground after Start",
            eggs,
        ),
        (
            "watch script-taken eggs enter a fresh Edgeville bank",
            Proof::BankItemId {
                id: RED_SPIDERS_EGGS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of eggs after deposit",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        ("watch the script close its egg bank", Proof::BankClosed),
        (
            "watch return to the egg field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 14,
            },
        ),
        ("watch further exact eggs after return", eggs),
    ] {
        // Deposit arms on first ground egg while food/respawn still run.
        // Return is the reverse dungeon hop. See HERBLORE_EGG_* constant
        // docs (dirty increments, not 150×600ms). Other arms keep 150.
        let budget_ticks = if matches!(arm, Proof::BankItemId { .. }) {
            HERBLORE_EGG_DEPOSIT_WATCH_TICKS
        } else if matches!(arm, Proof::ArrivedNear { .. }) {
            HERBLORE_EGG_RETURN_WATCH_TICKS
        } else {
            SCRIPT_GOLD_WATCH_TICKS
        };
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait { arm, budget_ticks },
        });
    }
    Scenario {
        name: "herblore_secondaries",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: eggs,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: HERBLORE_EGG_DEADLINE,
            start_script: Some("HerbloreSecondaries"),
            script_settings_inject: Some(HERBLORE_EGGS_INJECT),
            fixture_loadouts: Some(HERBLORE_EGGS_FIXTURE_LOADOUTS),
            terminal_shot: Some("herblore_secondaries"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// HerbloreSecondaries Eye of newt shop branch. Banked coins via native
/// stackable seed + deposit (stock289 has no `givebank`), Betty stand.
/// Distinct from ground eggs. LIVE still waits on Shop.buy publication.
pub(crate) fn herblore_secondaries_newt_scenario() -> Scenario {
    let shop = BETTY_SHOP;
    let newt = Proof::ItemId {
        id: EYE_OF_NEWT_ID,
        count: 1,
    };
    // stock289: 995=coins (stackable base; no certificate).
    let coins = NativeSeed {
        unnoted_id: COINS_ID,
        debug_alias: "coins",
        note_alias: None,
        quantity: HERBLORE_NEWT_COIN_SEED,
        note_id: None,
    };
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed banked coins at the Draynor booth approach before Start",
        DRAYNOR_BANK_APPROACH,
        vec![coins],
        "hitpoints",
        10,
    ));
    for (step_name, arm) in [
        (
            "confirm the exact coin seed in pack before deposit",
            Proof::ItemId {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "bound the coin seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "confirm no seeded eye of newt in pack before deposit",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eggs in pack before deposit",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted newt in pack before deposit",
            Proof::ItemIdAtMost {
                id: NOTED_EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(herblore_newt_seed_bank_readiness());
    steps.push(herblore_open_seed_bank_at(
        "open and acknowledge the coin seed bank",
        Proof::BankItemIdAtMost {
            id: COINS_ID,
            count: 0,
        },
        DRAYNOR_BANK_BOOTH,
        DRAYNOR_BANK_BOOTH_ID,
    ));
    steps.extend(native_bank_deposit(
        "deposit the coin seed through the bank window",
        vec![coins],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact coin seed bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "bound the coin seed bank count",
            Proof::BankItemIdAtMost {
                id: COINS_ID,
                count: HERBLORE_NEWT_COIN_SEED,
            },
        ),
        (
            "confirm the coin seed was removed from pack",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded eye of newt in bank",
            Proof::BankItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded noted newt in bank",
            Proof::BankItemIdAtMost {
                id: NOTED_EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no seeded red spiders' eggs in bank",
            Proof::BankItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    // Pack is empty of seed after deposit; clearinv only after bank accepts seed.
    steps.push(Step {
        name: "tele to Betty's shop stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(shop.level, shop.x, shop.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: shop.x,
                z: shop.z,
                level: shop.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded eye of newt in pack before Start",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded eggs in pack before Start",
            Proof::ItemIdAtMost {
                id: RED_SPIDERS_EGGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted newt in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch exact eye of newt 221 after Start", newt),
        (
            "watch script-bought newt enter a fresh Draynor bank",
            Proof::BankItemId {
                id: EYE_OF_NEWT_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of newt after deposit",
            Proof::ItemIdAtMost {
                id: EYE_OF_NEWT_ID,
                count: 0,
            },
        ),
        ("watch the script close its newt bank", Proof::BankClosed),
        (
            "watch return to Betty after banking",
            Proof::ArrivedNear {
                x: shop.x,
                z: shop.z,
                level: shop.level,
                radius: 6,
            },
        ),
        ("watch further exact newt after return", newt),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "herblore_secondaries_newt",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: newt,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("HerbloreSecondaries"),
            script_settings_inject: Some(HERBLORE_NEWT_INJECT),
            terminal_shot: Some("herblore_secondaries_newt"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

const FIREMAKING_STAT: i32 = 11;
pub(super) const STAFF_OF_AIR_ID: i32 = 1381;
pub(super) const STAFF_OF_WATER_ID: i32 = 1383;
pub(crate) const HAMMER_ID: i32 = 2347;
pub(crate) const BRONZE_BAR_CERT_ID: i32 = 2350;
/// Selected 289 `obj.pack`: `cert_steel_bar` = 2354 (unnoted steel bar 2353).
pub(crate) const STEEL_BAR_CERT_ID: i32 = 2354;
/// Selected 289 `obj.pack`: mithril_bar=2359, cert_mithril_bar=2360.
pub(crate) const MITHRIL_BAR_CERT_ID: i32 = 2360;
const BRONZE_DAGGER_ID: i32 = 1205;
/// Mithril dagger product id (stock smithing dbrow lvl 50).
const MITHRIL_DAGGER_ID: i32 = 1209;
const BRONZE_PLATEBODY_ID: i32 = 1117;
/// Selected 289 nails (steel-only anvil product, stackable, out 2/bar).
const STEEL_NAILS_ID: i32 = 1539;
/// Nails recipe levelrequired (steel bar tier is 30; product gate is 34).
const STEEL_NAILS_SMITHING: i32 = 34;
/// Stock nails `product_amount` — stack output must prove ≥ this count.
pub(crate) const STEEL_NAILS_OUTPUT: i32 = 2;
pub(crate) const NEEDLE_ID: i32 = 1733;
pub(crate) const THREAD_ID: i32 = 1734;
pub(crate) const LEATHER_GLOVES_ID: i32 = 1059;
const LEATHER_CHAPS_ID: i32 = 1095;
const HARDLEATHER_BODY_ID: i32 = 1131;
const DRAGONHIDE_BODY_ID: i32 = 1135;
const DRAGONHIDE_CHAPS_ID: i32 = 1099;
pub(crate) const LEATHER_CERT_ID: i32 = 1742;
pub(crate) const HARD_LEATHER_CERT_ID: i32 = 1744;
/// Green dragon leather (selected289 `dragon_leather` / alias `dragon_leather`).
pub(crate) const GREEN_DRAGON_LEATHER_ID: i32 = 1745;
pub(crate) const GREEN_DRAGON_LEATHER_CERT_ID: i32 = 1746;
/// Enough banked leather for two 26-slot trips, plus four pieces.
const LEATHER_CRAFTER_TWO_TRIP_SEED: i32 = 56;
/// Banked coins for missing-thread `fundThread` (default threadPerTrip 100 ×
/// THREAD_MAX_PRICE 3 = 300 needed; 1000 leaves headroom).
pub(crate) const LEATHER_THREAD_SHOP_COIN_SEED: i32 = 1000;
/// Canonical fundThread withdrawal ceiling: threadPerTrip default 100 × max price 3.
pub(crate) const LEATHER_THREAD_SHOP_FUND: i32 = 300;
pub(crate) const LOGS_CERT_ID: i32 = 1512;
pub(crate) const OAK_LOGS_CERT_ID: i32 = 1522;
pub(crate) const OAK_LOGS_ID: i32 = 1521;
pub(crate) const TINDERBOX_ID: i32 = 590;

const SMITHING_BOT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Bronze"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Dagger"),
    },
];
const SMITHING_BOT_PLATEBODY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Bronze"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Platebody"),
    },
];
const SMITHING_BOT_NAILS_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Steel"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Nails"),
    },
];
const SMITHING_BOT_MITHRIL_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bar",
        value: ScriptInjectValue::Str("Mithril"),
    },
    ScriptSettingInject {
        id: "product",
        value: ScriptInjectValue::Str("Dagger"),
    },
];
pub(crate) const LEATHER_CRAFTER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "leatherType",
    value: ScriptInjectValue::Str("Leather"),
}];
const LEATHER_CRAFTER_HARD_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "leatherType",
    value: ScriptInjectValue::Str("Hard leather"),
}];
const LEATHER_CRAFTER_GREEN_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "leatherType",
    value: ScriptInjectValue::Str("Green dragon leather"),
}];
const FIREMAKER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "logType",
        value: ScriptInjectValue::Str("Logs"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Varrock East"),
    },
];
/// Whole-scenario wall for the Firemaking-1 Logs cell. The frozen card
/// withdraws a full pack (`inventorySize - used` = 27 beside the tinderbox)
/// and banks only once every log is burnt, so the core's restock and further
/// light follow a whole 27-fire lane. At Firemaking 1 each light is a run of
/// 4-tick attempts: 73 live fire-to-fire gaps (2eeaa5090, 1b88d89, 341f0f4e4)
/// average 9.9 ticks (SD 7.3). The cycle is ~30 ticks Start → first fire,
/// 26 gaps (~257), the bank leg (~17), then close, the walk back to a lane
/// and one light (~26): ~345 engine ticks after the observed Start tick (13),
/// ~212s at the measured ~0.62s a tick (all three cells hit 180s at tick
/// 292, having lit 24, 25 and 27 of the 27 logs). 300s is that mean plus
/// ~3.8 SD of the 27-light sum.
pub(crate) const FIREMAKER_LOGS_DEADLINE: Duration = Duration::from_secs(300);
const FIREMAKER_OAK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "logType",
        value: ScriptInjectValue::Str("Oak logs"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Varrock East"),
    },
];

/// ClimbingBoots option cells. `useTeleport` and `runeStock` are typed on
/// purpose: the frozen script's defaults are true/50, and the walking cell
/// must be the explicit false branch, the teleport cell the explicit true
/// branch with the smallest non-zero rune stock.
const CLIMBING_BOOTS_WALK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "useTeleport",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "runeStock",
        value: ScriptInjectValue::Num(1.0),
    },
];
const CLIMBING_BOOTS_TELEPORT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "useTeleport",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "runeStock",
        value: ScriptInjectValue::Num(1.0),
    },
];

pub(crate) const CLIMBING_BOOTS_ID: i32 = 3105;
const CLIMBING_BOOTS_PAIR_COINS: i32 = 12;
const CLIMBING_BOOTS_TELE_MAGIC: i32 = 37;
/// `readyToBuy` wants coins === 12 * tripQty with no unrelated inventory.
/// tripQty is 28 minus the carried rune stacks, so the walk cell carries
/// 28 pairs and the teleport cell 25 pairs beside Law 1/Air 3/Water 1.
pub(crate) const CLIMBING_BOOTS_WALK_PACK_COINS: i32 = 28 * CLIMBING_BOOTS_PAIR_COINS;
pub(crate) const CLIMBING_BOOTS_TELE_PACK_COINS: i32 = 25 * CLIMBING_BOOTS_PAIR_COINS;
/// Bank stock for the later withdrawals the full cycle claims: two further
/// trips of coins, and — teleport cell only — the exact Law 1/Air 3/Water 1
/// restock, because `runeStock=1` consumes the whole carried stack per cast.
pub(crate) const CLIMBING_BOOTS_BANK_TRIPS: i32 = 2;
const CLIMBING_BOOTS_RUNES: &[(&str, i32)] = &[("lawrune", 1), ("airrune", 3), ("waterrune", 1)];
/// The rest-of-trip arm after the 2-pair spend watch: the walk cell's
/// return to Falador West and the teleport cell's Falador landing.
///
/// **Units:** runner dirty-snapshot increments (`budget_ticks`), not engine
/// ticks or wall seconds. `readyToBuy` fixes one trip at 28 (walk) / 25
/// (teleport) pairs, and the frozen card returns only once `tripComplete`
/// (`ClimbingBoots.ts` loop: `returnToBank` runs only when the pack is
/// complete). Each pair is seven `death_sherpa.rs2` pause pages (two
/// `~chatplayer`, two `~chatnpc`, the `~objbox`, `~p_choice2`, the
/// `~p_choice5` reprompt), and frozen `driveShop` waits for each page to
/// change, then `delayTicks(1)` per continue and `delayTicks(2)` per choice:
/// at least 16 engine ticks (9.6s) a pair. Live 290253afd measured ~11s a
/// pair at ~2.4 dirty increments/s, so the ordinary 150 (~62s) covered
/// seven pairs of the 28.
/// Walk: 26 pairs (~290s) plus the ~260-tile hut → Falador West walk
/// (≤156s at walking pace) ≈ 450s ≈ 1080 dirties → **1200**.
/// Teleport: 23 pairs (~255s) plus the cast ≈ 615 dirties → **750**. In the
/// teleport cell that arm is the cast's Magic XP (see the watch list); the
/// landing follows it within a few ticks.
pub(crate) const CLIMBING_BOOTS_WALK_RETURN_WATCH_TICKS: u32 = 1200;
pub(crate) const CLIMBING_BOOTS_TELE_CAST_WATCH_TICKS: u32 = 750;
/// The further-pair arm (`has_item_id(3105)>=2` after the restock left zero
/// carried): the walk back to the hut (≤156s), the 3745 entry, the talk and
/// two pairs (~30s) ≈ 190s ≈ 460 dirties → **600**.
pub(crate) const CLIMBING_BOOTS_FURTHER_WATCH_TICKS: u32 = 600;
/// Whole-scenario wall: seed (~50s) + first pairs (~30s) + the arms above at
/// their wall estimates + bank open/deposit/restock (~25s). Walk ≈ 745s
/// with ~20% margin. Teleport measured live at bce85f2df: Start 15s,
/// 25th pair 286s, landing 292s, restock closed 306s, hut re-entered 404s,
/// further pair (last watch) 427s → 720s keeps ~65% margin. The Magic XP
/// proof shares the cast arm's baseline, so it holds at the last watch; a
/// baseline taken after the cycle instead needs a second full trip (that
/// run's 25th second-trip pair only landed at 715s).
pub(crate) const CLIMBING_BOOTS_WALK_DEADLINE: Duration = Duration::from_secs(900);
pub(crate) const CLIMBING_BOOTS_TELE_DEADLINE: Duration = Duration::from_secs(720);
/// The Water rune id for the bank-seed acknowledgement (Law 563 and Air 556
/// already have crate constants).
pub(crate) const WATER_RUNE_ID: i32 = 555;
/// Tenzing's hut: the door tile the pack stands on and the inside tile the
/// frozen script targets. Route/prep targets only, never PASS predicates.
pub(crate) const TENZING_DOOR: WorldTile = WorldTile {
    x: 2823,
    z: 3555,
    level: 0,
};
pub(crate) const TENZING_INSIDE: WorldTile = WorldTile {
    x: 2820,
    z: 3556,
    level: 0,
};
pub(crate) const TENZING_NAME: &str = "Tenzing";

pub(crate) fn climbing_boots_scenario() -> Scenario {
    climbing_boots_variant(
        "climbing_boots",
        CLIMBING_BOOTS_WALK_INJECT,
        CLIMBING_BOOTS_WALK_PACK_COINS,
        false,
    )
}

pub(crate) fn climbing_boots_teleport_scenario() -> Scenario {
    climbing_boots_variant(
        "climbing_boots_teleport",
        CLIMBING_BOOTS_TELEPORT_INJECT,
        CLIMBING_BOOTS_TELE_PACK_COINS,
        true,
    )
}

/// Clean pack at Tenzing's hut: Death Plateau completed through the authentic
/// primary `setvar death_equiproom 80` **and** retained map progress
/// `setvar death_map 8`, each with its own native `getvar` Chat receipt, a
/// relog so the journal repaints, then the exact carried trip money (plus the
/// rune stack for the teleport cell) and a bank stock for later withdrawals.
///
/// Authentic completion keeps map progress: the commander path reaches
/// `denulth_has_map` only when primary is 70 and `death_get_map >= 8`
/// (`death_scouted_area`); the completion queue then sets primary 80 and
/// leaves `death_map` unchanged. Tenzing door gates read bits 0..3 of that
/// same varp. Seeding primary alone leaves map 0 and is not a completed state.
///
/// Both `getvar` readbacks (`get death_equiproom: 80` and `get death_map: 8`),
/// the `Death Plateau` journal row and the framed `Tenzing` NPC are the
/// fixture's fail-closed prerequisite: they are exact server Chat replies to
/// the cheat path, not a client varp snapshot (default published 315 stays 0
/// and does not prove transmission). A pack that does not provide them times
/// this cell out before Start instead of seeding a shortcut. There is no
/// revision switch in this runner and none is invented here; the guard is the
/// authenticated content those steps resolve.
///
/// The pack itself carries zero boots. The bank stock is a real booth session
/// (the ordinary window, the inventory's bulk deposit op, a real close) — not
/// `givebank` and not a bank-side cheat — and it is not a purchase claim.
/// The open seed session also acknowledges `BankItemIdAtMost` boots 3105 = 0
/// before close, so leftover banked boots cannot hide behind a closed-bank
/// Start snapshot. Start is the real frozen script; nothing intervenes after
/// it, and the purchase, return, deposit and further stages are watched
/// separately. No boots, extra cash, reward XP, or unrelated progress is
/// granted in preparation.
fn climbing_boots_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    pack_coins: i32,
    use_teleport: bool,
) -> Scenario {
    let door = Proof::ArrivedNear {
        x: TENZING_DOOR.x,
        z: TENZING_DOOR.z,
        level: TENZING_DOOR.level,
        radius: 4,
    };
    let tenzing = Proof::NpcNameNear {
        name: TENZING_NAME,
        x: TENZING_INSIDE.x,
        z: TENZING_INSIDE.z,
        level: TENZING_INSIDE.level,
        radius: 12,
    };
    let bank = FALADOR_WEST_BANK;
    let returned = Proof::ArrivedNear {
        x: bank.x,
        z: bank.z,
        level: bank.level,
        radius: 8,
    };
    let mut steps: Vec<Step> = Vec::new();
    // Authentic completed Death Plateau: primary 80 + retained map progress 8
    // (bits 0..3). Separate setvar/getvar + Chat receipts so each value is
    // proven by the server reply before relog/Start — not a client snapshot.
    steps.push(Step {
        name: "complete Death Plateau primary by the authentic setvar and read it back",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setvar death_equiproom 80");
                cheat(c, "getvar death_equiproom");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Chat {
                needle: "get death_equiproom: 80",
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "retain Death Plateau map progress by the authentic setvar and read it back",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "setvar death_map 8");
                cheat(c, "getvar death_map");
                true
            }),
        },
        wait: Wait {
            arm: Proof::Chat {
                needle: "get death_map: 8",
            },
            budget_ticks: 200,
        },
    });
    steps.extend(script_live_seed_steps());
    steps.push(bank_fletcher_watch(
        "acknowledge Death Plateau complete before Start",
        Proof::QuestDone {
            name: "Death Plateau",
        },
    ));
    let booth = FALADOR_WEST_BOOTH;
    let bank_coins = CLIMBING_BOOTS_BANK_TRIPS * pack_coins;
    steps.push(Step {
        name: "seed the bank-bound stack and stand at the Falador West booth",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give coins {bank_coins}"));
                if use_teleport {
                    for (alias, count) in CLIMBING_BOOTS_RUNES {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                cheat(c, &tele_args(booth.level, booth.x, booth.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: booth.x,
                z: booth.z,
                level: booth.level,
                radius: 4,
            },
            budget_ticks: 200,
        },
    });
    // The bank seed is a real session, not a bank-side cheat: the ordinary
    // booth window, the inventory's bulk deposit op, and a real close. The
    // deposited rows are the same stackables the script later withdraws.
    steps.push(open_seed_booth(
        "open the real Falador West bank for the seed deposit",
        booth,
        Proof::BankItemId {
            id: COINS_ID,
            count: 0,
        },
    ));
    steps.push(Step {
        name: "deposit the seeded coins and runes through the bank window",
        kind: StepKind::Repeat {
            send: Box::new(move |c, snapshot| {
                // Repeat sends before checking its arm. Once the deposit has
                // landed the bank-side pack is empty; acknowledge the actual
                // fresh-bank readback instead of refusing an unnecessary send.
                if (Proof::BankItemId {
                    id: COINS_ID,
                    count: bank_coins,
                })
                .check(snapshot, None)
                {
                    return true;
                }
                let mut ix = Interactions::new(snapshot, c);
                let mut wrote = false;
                for item in snapshot.bank_side() {
                    if let Some(op) = bank_deposit_all_op(&item.actions) {
                        wrote |= matches!(
                            ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                            SendResult::Sent { .. }
                        );
                    }
                }
                wrote
            }),
        },
        wait: Wait {
            arm: Proof::BankItemId {
                id: COINS_ID,
                count: bank_coins,
            },
            budget_ticks: 200,
        },
    });
    if use_teleport {
        // `runeStock=1` spends the whole carried stack on the first cast, so
        // the seed bank itself must be acknowledged to hold the exact restock
        // the full cycle withdraws (`CLIMBING_BOOTS_RUNES`) before the session
        // closes and Start inherits it. Coins alone would let a rune-less bank
        // seed pass.
        steps.push(bank_fletcher_watch(
            "acknowledge the banked teleport restock: Law rune",
            Proof::BankItemId {
                id: LAW_RUNE_ID,
                count: 1,
            },
        ));
        steps.push(bank_fletcher_watch(
            "acknowledge the banked teleport restock: Air runes",
            Proof::BankItemId {
                id: AIR_RUNE_ID,
                count: 3,
            },
        ));
        steps.push(bank_fletcher_watch(
            "acknowledge the banked teleport restock: Water rune",
            Proof::BankItemId {
                id: WATER_RUNE_ID,
                count: 1,
            },
        ));
    }
    steps.push(bank_fletcher_watch(
        "acknowledge the seed bank holds no leftover climbing boots",
        Proof::BankItemIdAtMost {
            id: CLIMBING_BOOTS_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "seed the exact trip stack and stand at Tenzing's hut",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                if use_teleport {
                    cheat(c, &format!("setstat magic {CLIMBING_BOOTS_TELE_MAGIC}"));
                    for (alias, count) in CLIMBING_BOOTS_RUNES {
                        cheat(c, &format!("give {alias} {count}"));
                    }
                }
                cheat(c, &format!("give coins {pack_coins}"));
                cheat(
                    c,
                    &tele_args(TENZING_DOOR.level, TENZING_DOOR.x, TENZING_DOOR.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: door,
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm no seeded boots in the pack before Start",
        Proof::ItemIdAtMost {
            id: CLIMBING_BOOTS_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm the real Tenzing NPC is framed at the hut before Start",
        tenzing,
    ));
    steps.push(start_catalog_step());
    // Serial arms in cycle order: purchase, spend, return (the teleport cell
    // lands the real Falador cast first, then walks to the bank stand),
    // deposit, restock close, further pair. The cast lands only after a full
    // trip, so its arm follows the purchase arms in cycle order and carries
    // the rest-of-trip budget; a 150-dirty cast arm could never see it.
    let trip_watch = if use_teleport {
        CLIMBING_BOOTS_TELE_CAST_WATCH_TICKS
    } else {
        CLIMBING_BOOTS_WALK_RETURN_WATCH_TICKS
    };
    let mut watches: Vec<(&'static str, Proof, u32)> = vec![
        (
            "watch the real purchase gain a pair of boots",
            Proof::ItemId {
                id: CLIMBING_BOOTS_ID,
                count: 1,
            },
            SCRIPT_GOLD_WATCH_TICKS,
        ),
        (
            "watch the purchase spend 12 coins a pair",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: pack_coins - CLIMBING_BOOTS_PAIR_COINS * 2,
            },
            SCRIPT_GOLD_WATCH_TICKS,
        ),
        (
            "watch the return to the Falador West bank",
            returned,
            if use_teleport {
                SCRIPT_GOLD_WATCH_TICKS
            } else {
                trip_watch
            },
        ),
        (
            "watch Falador West bank hold the deposited boots",
            Proof::BankItemId {
                id: CLIMBING_BOOTS_ID,
                count: 1,
            },
            SCRIPT_GOLD_WATCH_TICKS,
        ),
        (
            "watch the bank close after restocking for a further trip",
            Proof::BankClosed,
            SCRIPT_GOLD_WATCH_TICKS,
        ),
        (
            "watch a further pair for the full cycle",
            Proof::ItemId {
                id: CLIMBING_BOOTS_ID,
                count: 2,
            },
            CLIMBING_BOOTS_FURTHER_WATCH_TICKS,
        ),
    ];
    if use_teleport {
        // The runner takes a `StatXpGain` baseline when the first arm of
        // that shape begins and the proof reuses it. Without an arm here
        // the baseline is taken when proving starts, after the further
        // pair, so the proof would wait for a second full trip and cast.
        // `magic_teleport` deletes the runes and grants the XP one tick
        // before `player_teleport_normal` jumps, so the XP arm (baseline
        // after the 2-pair spend, before any cast) comes first, then the
        // landing.
        watches.insert(
            2,
            (
                "watch the real Falador cast gain Magic XP",
                Proof::StatXpGain {
                    id: MAGIC_STAT,
                    min: 1,
                },
                trip_watch,
            ),
        );
        watches.insert(
            3,
            (
                "watch the real Falador cast land at the bank",
                Proof::ArrivedNear {
                    x: FALADOR_TELE_LAND.x,
                    z: FALADOR_TELE_LAND.z,
                    level: FALADOR_TELE_LAND.level,
                    radius: 8,
                },
                SCRIPT_GOLD_WATCH_TICKS,
            ),
        );
    }
    for (step_name, arm, budget_ticks) in watches {
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait { arm, budget_ticks },
        });
    }
    let proof = if use_teleport {
        Proof::StatXpGain {
            id: MAGIC_STAT,
            min: 1,
        }
    } else {
        Proof::ItemId {
            id: CLIMBING_BOOTS_ID,
            count: 2,
        }
    };
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: if use_teleport {
                CLIMBING_BOOTS_TELE_DEADLINE
            } else {
                CLIMBING_BOOTS_WALK_DEADLINE
            },
            start_script: Some("ClimbingBoots"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn smithing_bot_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot",
        SMITHING_BOT_INJECT,
        1,
        BRONZE_DAGGER_ID,
        BRONZE_PLATEBODY_ID,
        BRONZE_BAR_ID,
        BRONZE_BAR_CERT_ID,
        "bronze_bar",
        "cert_bronze_bar",
        28,
        1,
    )
}

pub(crate) fn smithing_bot_platebody_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot_platebody",
        SMITHING_BOT_PLATEBODY_INJECT,
        18,
        BRONZE_PLATEBODY_ID,
        BRONZE_DAGGER_ID,
        BRONZE_BAR_ID,
        BRONZE_BAR_CERT_ID,
        "bronze_bar",
        "cert_bronze_bar",
        30,
        1,
    )
}

/// Steel Nails: steel-only special panel slot, stackable out=2/bar, F2P.
/// 28 steel bars → first trip 27 bars (+ hammer) then remaining 1 bar restock.
pub(crate) fn smithing_bot_nails_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot_nails",
        SMITHING_BOT_NAILS_INJECT,
        STEEL_NAILS_SMITHING,
        STEEL_NAILS_ID,
        BRONZE_DAGGER_ID,
        STEEL_BAR_ID,
        STEEL_BAR_CERT_ID,
        "steel_bar",
        "cert_steel_bar",
        28,
        STEEL_NAILS_OUTPUT,
    )
}

/// Mithril Dagger: 1-bar F2P product on a higher metal tier (lvl 50 / id 1209).
pub(crate) fn smithing_bot_mithril_scenario() -> Scenario {
    smithing_bot_variant(
        "smithing_bot_mithril",
        SMITHING_BOT_MITHRIL_INJECT,
        MITHRIL_SMITHING,
        MITHRIL_DAGGER_ID,
        BRONZE_DAGGER_ID,
        MITHRIL_BAR_ID,
        MITHRIL_BAR_CERT_ID,
        "mithril_bar",
        "cert_mithril_bar",
        28,
        1,
    )
}

#[allow(clippy::too_many_arguments)] // scenario factory bundles inject/skills/products
fn smithing_bot_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    smithing: i32,
    product_id: i32,
    wrong_id: i32,
    bar_id: i32,
    bar_cert_id: i32,
    bar_alias: &'static str,
    bar_note_alias: &'static str,
    bar_quantity: i32,
    product_count: i32,
) -> Scenario {
    let anvil = Proof::ArrivedNear {
        x: VARROCK_ANVIL.x,
        z: VARROCK_ANVIL.z,
        level: VARROCK_ANVIL.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: product_count,
    };
    let xp = Proof::StatXpGain {
        id: SMITHING_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: SMITHING_STAT,
        min: 1,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Smithing, banked hammer/bars, and tele to Varrock West before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: HAMMER_ID,
                debug_alias: "hammer",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: bar_id,
                debug_alias: bar_alias,
                note_alias: Some(bar_note_alias),
                quantity: bar_quantity,
                note_id: Some(bar_cert_id),
            },
        ],
        "smithing",
        smithing,
    ));
    for (step_name, arm) in [
        (
            "confirm Smithing before Start",
            Proof::Stat {
                id: SMITHING_STAT,
                min: smithing,
            },
        ),
        (
            "confirm the exact noted bar seed in pack before deposit",
            Proof::ItemId {
                id: bar_cert_id,
                count: bar_quantity,
            },
        ),
        (
            "confirm the hammer seed in pack before deposit",
            Proof::ItemId {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "bound the noted bar seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: bar_cert_id,
                count: bar_quantity,
            },
        ),
        (
            "bound the hammer seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong product in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the bar seed bank",
        Proof::BankItemIdAtMost {
            id: bar_id,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the hammer and native note seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: HAMMER_ID,
                debug_alias: "hammer",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: bar_id,
                debug_alias: bar_alias,
                note_alias: Some(bar_note_alias),
                quantity: bar_quantity,
                note_id: Some(bar_cert_id),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the hammer seed bank",
            Proof::BankItemId {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the exact bar seed bank",
            Proof::BankItemId {
                id: bar_id,
                count: bar_quantity,
            },
        ),
        (
            "confirm the hammer seed was removed from pack",
            Proof::ItemIdAtMost {
                id: HAMMER_ID,
                count: 0,
            },
        ),
        (
            "confirm the noted bar seed was removed from pack",
            Proof::ItemIdAtMost {
                id: bar_cert_id,
                count: 0,
            },
        ),
        (
            "bound the hammer seed bank count",
            Proof::BankItemIdAtMost {
                id: HAMMER_ID,
                count: 1,
            },
        ),
        (
            "bound the bar seed bank count",
            Proof::BankItemIdAtMost {
                id: bar_id,
                count: bar_quantity,
            },
        ),
        (
            "confirm no noted bars remain in bank",
            Proof::BankItemIdAtMost {
                id: bar_cert_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch arrival at the Varrock anvil after Start", anvil),
        ("watch Smithing XP from the anvil panel after Start", xp),
        ("watch the selected smithing product after Start", product),
        (
            "watch script-smithed product enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: product_count,
            },
        ),
        (
            "watch the pack empty of product after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of bars",
            Proof::ItemId {
                id: bar_id,
                count: 1,
            },
        ),
        ("watch the smithing bank close", Proof::BankClosed),
        ("watch return to the anvil after restock", anvil),
        ("watch further Smithing XP after restock", further_xp),
    ] {
        // Deposit arms on the first product while ≤26 bars may still forge.
        // SMITHING_PRODUCT_DEPOSIT_WATCH_TICKS is runner dirty increments
        // (not engine p_delay ticks); see constant docs. Other arms keep
        // the ordinary gold watch; do not loosen the global deadline.
        let budget_ticks = if matches!(arm, Proof::BankItemId { .. }) {
            SMITHING_PRODUCT_DEPOSIT_WATCH_TICKS
        } else {
            SCRIPT_GOLD_WATCH_TICKS
        };
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait { arm, budget_ticks },
        });
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("SmithingBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn leather_crafter_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter",
        LEATHER_CRAFTER_INJECT,
        1,
        "leather",
        "cert_leather",
        SOFT_LEATHER_ID,
        LEATHER_CERT_ID,
        28,
        LEATHER_GLOVES_ID,
        HARDLEATHER_BODY_ID,
    )
}

pub(crate) fn leather_crafter_hard_body_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter_hard_body",
        LEATHER_CRAFTER_HARD_INJECT,
        28,
        "hard_leather",
        "cert_hard_leather",
        HARD_LEATHER_ID,
        HARD_LEATHER_CERT_ID,
        28,
        HARDLEATHER_BODY_ID,
        LEATHER_GLOVES_ID,
    )
}

/// Green body at Crafting 63: multi3 make-X / chat count-dialog branch.
/// 3 leather per body; needle+thread leave 26 slots so a trip may leave 2 leather.
pub(crate) fn leather_crafter_green_body_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter_green_body",
        LEATHER_CRAFTER_GREEN_INJECT,
        63,
        "dragon_leather",
        "cert_dragon_leather",
        GREEN_DRAGON_LEATHER_ID,
        GREEN_DRAGON_LEATHER_CERT_ID,
        LEATHER_CRAFTER_TWO_TRIP_SEED,
        DRAGONHIDE_BODY_ID,
        DRAGONHIDE_CHAPS_ID,
    )
}

/// Soft leather chaps at Crafting 18: selected289 uses make-10 button 8645.
pub(crate) fn leather_crafter_chaps_scenario() -> Scenario {
    leather_crafter_variant(
        "leather_crafter_chaps",
        LEATHER_CRAFTER_INJECT,
        18,
        "leather",
        "cert_leather",
        SOFT_LEATHER_ID,
        LEATHER_CERT_ID,
        LEATHER_CRAFTER_TWO_TRIP_SEED,
        LEATHER_CHAPS_ID,
        LEATHER_GLOVES_ID,
    )
}

/// Missing-thread purchase + return: level-1 Leather gloves with default
/// `threadPerTrip` 100. Bank holds needle, coins, and raw soft leather only —
/// absolutely no thread or crafted product in pack or bank before Start.
/// Script `fundThread` withdraws up to `threadPerTrip * THREAD_MAX_PRICE`
/// (100×3), walks nearest Dommik (3322,3194), `Shop.buy(Thread)`, closes,
/// returns to the remembered Al-Kharid bank stand, restocks leather, crafts.
///
/// Post-Start scope is purchase / return / resumed craft — not a second full
/// product bank cycle. `BankClosed` is the existing bank-modal gate only; there
/// is no `ShopClosed` proof, so shop-close is not asserted here.
pub(crate) fn leather_crafter_thread_shop_scenario() -> Scenario {
    let product = Proof::ItemId {
        id: LEATHER_GLOVES_ID,
        count: 1,
    };
    let xp = Proof::FreshStatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let bank = AL_KHARID_BANK;
    let returned = Proof::ArrivedNear {
        x: bank.x,
        z: bank.z,
        level: bank.level,
        radius: 8,
    };
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Crafting, banked needle/coins/leather, and tele to Al-Kharid before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: COINS_ID,
                debug_alias: "coins",
                note_alias: None,
                quantity: LEATHER_THREAD_SHOP_COIN_SEED,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: SOFT_LEATHER_ID,
                debug_alias: "leather",
                note_alias: Some("cert_leather"),
                quantity: 28,
                note_id: Some(LEATHER_CERT_ID),
            },
        ],
        "crafting",
        1,
    ));
    for (step_name, arm) in [
        (
            "confirm Crafting before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        (
            "confirm the exact noted leather seed in pack before deposit",
            Proof::ItemId {
                id: LEATHER_CERT_ID,
                count: 28,
            },
        ),
        (
            "confirm the needle seed in pack before deposit",
            Proof::ItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "confirm the exact coin seed in pack before deposit",
            Proof::ItemId {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "bound the noted leather seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: LEATHER_CERT_ID,
                count: 28,
            },
        ),
        (
            "bound the needle seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the coin seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "confirm zero thread in pack before deposit",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded gloves product in pack before Start",
            Proof::ItemIdAtMost {
                id: LEATHER_GLOVES_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong product in pack before Start",
            Proof::ItemIdAtMost {
                id: HARDLEATHER_BODY_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the leather seed bank",
        Proof::BankItemIdAtMost {
            id: SOFT_LEATHER_ID,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the native leather seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: COINS_ID,
                debug_alias: "coins",
                note_alias: None,
                quantity: LEATHER_THREAD_SHOP_COIN_SEED,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: SOFT_LEATHER_ID,
                debug_alias: "leather",
                note_alias: Some("cert_leather"),
                quantity: 28,
                note_id: Some(LEATHER_CERT_ID),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact leather seed bank",
            Proof::BankItemId {
                id: SOFT_LEATHER_ID,
                count: 28,
            },
        ),
        (
            "acknowledge the needle seed bank",
            Proof::BankItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the exact coin seed bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "confirm noted leather removal",
            Proof::ItemIdAtMost {
                id: LEATHER_CERT_ID,
                count: 0,
            },
        ),
        (
            "confirm needle removal",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 0,
            },
        ),
        (
            "confirm coin removal",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm zero thread in pack after deposit",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm zero thread in bank before Start",
            Proof::BankItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no gloves product in bank before Start",
            Proof::BankItemIdAtMost {
                id: LEATHER_GLOVES_ID,
                count: 0,
            },
        ),
        (
            "confirm no wrong product in bank before Start",
            Proof::BankItemIdAtMost {
                id: HARDLEATHER_BODY_ID,
                count: 0,
            },
        ),
        (
            "bound the leather seed bank count",
            Proof::BankItemIdAtMost {
                id: SOFT_LEATHER_ID,
                count: 28,
            },
        ),
        (
            "bound the needle seed bank count",
            Proof::BankItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the coin seed bank count",
            Proof::BankItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_COIN_SEED,
            },
        ),
        (
            "confirm no noted leather remains in bank",
            Proof::BankItemIdAtMost {
                id: LEATHER_CERT_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch thread acquired from the zero-thread baseline after Start",
            Proof::ItemId {
                id: THREAD_ID,
                count: 1,
            },
        ),
        (
            // fundThread withdraws at most threadPerTrip*THREAD_MAX_PRICE (300).
            // After Shop.buy the pack holds strictly less than that withdrawal
            // while leftover coins remain, matching climbing_boots / shop_buyout
            // ItemIdAtMost spend arms. Not a ShopClosed proof.
            "watch coin expenditure after thread purchase",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: LEATHER_THREAD_SHOP_FUND - 1,
            },
        ),
        (
            "watch return near the original Al-Kharid bank after acquisition",
            returned,
        ),
        (
            "watch the leather bank close after restock",
            Proof::BankClosed,
        ),
        ("watch fresh Crafting XP after purchase return", xp),
        ("watch exact leather gloves after resumed craft", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "leather_crafter_thread_shop",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("LeatherCrafter"),
            script_settings_inject: Some(LEATHER_CRAFTER_INJECT),
            terminal_shot: Some("leather_crafter_thread_shop"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[allow(clippy::too_many_arguments)] // scenario factory bundles inject/skills/products
fn leather_crafter_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    crafting: i32,
    leather_alias: &'static str,
    leather_note_alias: &'static str,
    leather_id: i32,
    leather_note: i32,
    leather_qty: i32,
    product_id: i32,
    wrong_id: i32,
) -> Scenario {
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let bank = AL_KHARID_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Crafting, banked needle/thread/leather, and tele to Al-Kharid before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: THREAD_ID,
                debug_alias: "thread",
                note_alias: None,
                quantity: 100,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: leather_id,
                debug_alias: leather_alias,
                note_alias: Some(leather_note_alias),
                quantity: leather_qty,
                note_id: Some(leather_note),
            },
        ],
        "crafting",
        crafting,
    ));
    for (step_name, arm) in [
        (
            "confirm Crafting before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: crafting,
            },
        ),
        (
            "confirm the exact noted leather seed in pack before deposit",
            Proof::ItemId {
                id: leather_note,
                count: leather_qty,
            },
        ),
        (
            "confirm the needle seed in pack before deposit",
            Proof::ItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "confirm the thread seed in pack before deposit",
            Proof::ItemId {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "bound the noted leather seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: leather_note,
                count: leather_qty,
            },
        ),
        (
            "bound the needle seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the thread seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "confirm no seeded product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong product in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the leather seed bank",
        Proof::BankItemIdAtMost {
            id: leather_id,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the native leather seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: NEEDLE_ID,
                debug_alias: "needle",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: THREAD_ID,
                debug_alias: "thread",
                note_alias: None,
                quantity: 100,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: leather_id,
                debug_alias: leather_alias,
                note_alias: Some(leather_note_alias),
                quantity: leather_qty,
                note_id: Some(leather_note),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact leather seed bank",
            Proof::BankItemId {
                id: leather_id,
                count: leather_qty,
            },
        ),
        (
            "acknowledge the needle seed bank",
            Proof::BankItemId {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "acknowledge the thread seed bank",
            Proof::BankItemId {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "confirm noted leather removal",
            Proof::ItemIdAtMost {
                id: leather_note,
                count: 0,
            },
        ),
        (
            "confirm needle removal",
            Proof::ItemIdAtMost {
                id: NEEDLE_ID,
                count: 0,
            },
        ),
        (
            "confirm thread removal",
            Proof::ItemIdAtMost {
                id: THREAD_ID,
                count: 0,
            },
        ),
        (
            "bound the leather seed bank count",
            Proof::BankItemIdAtMost {
                id: leather_id,
                count: leather_qty,
            },
        ),
        (
            "bound the needle seed bank count",
            Proof::BankItemIdAtMost {
                id: NEEDLE_ID,
                count: 1,
            },
        ),
        (
            "bound the thread seed bank count",
            Proof::BankItemIdAtMost {
                id: THREAD_ID,
                count: 100,
            },
        ),
        (
            "confirm no noted leather remains in bank",
            Proof::BankItemIdAtMost {
                id: leather_note,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Crafting XP after Start", xp),
        ("watch the leather product after Start", product),
        (
            "watch script-crafted product enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of product after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of leather",
            Proof::ItemId {
                id: leather_id,
                count: 1,
            },
        ),
        ("watch the leather bank close", Proof::BankClosed),
        ("watch further Crafting XP after restock", further_xp),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("LeatherCrafter"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn firemaker_scenario() -> Scenario {
    firemaker_variant(
        "firemaker",
        FIREMAKER_INJECT,
        1,
        "logs",
        LOGS_ID,
        OAK_LOGS_ID,
        FIREMAKER_LOGS_DEADLINE,
    )
}

pub(crate) fn firemaker_oak_scenario() -> Scenario {
    firemaker_variant(
        "firemaker_oak",
        FIREMAKER_OAK_INJECT,
        15,
        "oak_logs",
        OAK_LOGS_ID,
        LOGS_ID,
        SCRIPT_GOLD_DEADLINE,
    )
}

fn firemaker_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    firemaking: i32,
    log_alias: &'static str,
    log_id: i32,
    wrong_id: i32,
    deadline: Duration,
) -> Scenario {
    let xp = Proof::StatXpGain {
        id: FIREMAKING_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: FIREMAKING_STAT,
        min: 1,
    };
    let bank = VARROCK_EAST_BANK;
    let mut steps = script_live_seed_steps();
    let log_note = if log_alias == "logs" {
        LOGS_CERT_ID
    } else {
        OAK_LOGS_CERT_ID
    };
    steps.push(native_bank_seed(
        "seed Firemaking, banked tinderbox/logs, and tele to Varrock East before Start",
        bank,
        vec![
            NativeSeed {
                unnoted_id: TINDERBOX_ID,
                debug_alias: "tinderbox",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: log_id,
                debug_alias: log_alias,
                note_alias: Some(if log_alias == "logs" {
                    "cert_logs"
                } else {
                    "cert_oak_logs"
                }),
                quantity: 28,
                note_id: Some(log_note),
            },
        ],
        "firemaking",
        firemaking,
    ));
    for (step_name, arm) in [
        (
            "confirm Firemaking before Start",
            Proof::Stat {
                id: FIREMAKING_STAT,
                min: firemaking,
            },
        ),
        (
            "confirm the exact noted log seed in pack before deposit",
            Proof::ItemId {
                id: log_note,
                count: 28,
            },
        ),
        (
            "confirm the tinderbox seed in pack before deposit",
            Proof::ItemId {
                id: TINDERBOX_ID,
                count: 1,
            },
        ),
        (
            "bound the noted log seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: log_note,
                count: 28,
            },
        ),
        (
            "bound the tinderbox seed count in pack before deposit",
            Proof::ItemIdAtMost {
                id: TINDERBOX_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded wrong logs in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the log seed bank",
        Proof::BankItemIdAtMost {
            id: log_id,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the tinderbox and native note log seed through the bank window",
        vec![
            NativeSeed {
                unnoted_id: TINDERBOX_ID,
                debug_alias: "tinderbox",
                note_alias: None,
                quantity: 1,
                note_id: None,
            },
            NativeSeed {
                unnoted_id: log_id,
                debug_alias: log_alias,
                note_alias: Some(if log_alias == "logs" {
                    "cert_logs"
                } else {
                    "cert_oak_logs"
                }),
                quantity: 28,
                note_id: Some(log_note),
            },
        ],
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact log seed bank",
            Proof::BankItemId {
                id: log_id,
                count: 28,
            },
        ),
        (
            "acknowledge the tinderbox seed bank",
            Proof::BankItemId {
                id: TINDERBOX_ID,
                count: 1,
            },
        ),
        (
            "confirm noted log removal",
            Proof::ItemIdAtMost {
                id: log_note,
                count: 0,
            },
        ),
        (
            "confirm tinderbox removal",
            Proof::ItemIdAtMost {
                id: TINDERBOX_ID,
                count: 0,
            },
        ),
        (
            "bound the log seed bank count",
            Proof::BankItemIdAtMost {
                id: log_id,
                count: 28,
            },
        ),
        (
            "bound the tinderbox seed bank count",
            Proof::BankItemIdAtMost {
                id: TINDERBOX_ID,
                count: 1,
            },
        ),
        (
            "confirm no noted logs remain in bank",
            Proof::BankItemIdAtMost {
                id: log_note,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Firemaking XP after Start", xp),
        (
            "watch logs consumed after Start",
            Proof::ItemIdAtMost {
                id: log_id,
                count: 27,
            },
        ),
        (
            "watch a restock of logs",
            Proof::ItemId {
                id: log_id,
                count: 1,
            },
        ),
        ("watch the fire bank close", Proof::BankClosed),
        ("watch further Firemaking XP after restock", further_xp),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline,
            start_script: Some("Firemaker"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
