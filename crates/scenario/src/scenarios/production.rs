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
pub(crate) const UNIDENTIFIED_MARENTILL_ID: i32 = 201;
const MARRENTILL_ID: i32 = 251;
pub(crate) const UNCUT_SAPPHIRE_ID: i32 = 1623;
pub(crate) const SAPPHIRE_ID: i32 = 1607;
pub(crate) const UNCUT_OPAL_ID: i32 = 1625;
pub(crate) const CHISEL_ID: i32 = 1755;
pub(crate) const CRUSHED_GEMSTONE_ID: i32 = 1633;
pub(crate) const FLETCHING_STAT: i32 = 9;
pub(crate) const CRAFTING_STAT: i32 = 12;
pub(crate) const HERBLORE_STAT: i32 = 15;


const HERB_CLEANER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "herbs",
    value: ScriptInjectValue::StrList(&[]),
}];

const HERB_CLEANER_NAMED_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "herbs",
    value: ScriptInjectValue::StrList(&["Guam leaf"]),
}];

const HERB_CLEANER_EMPTY_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "herbs",
    value: ScriptInjectValue::StrList(&["Guam leaf", "Marrentill"]),
}];

const GEM_CUTTER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "gems",
    value: ScriptInjectValue::StrList(&[]),
}];

const GEM_CUTTER_NAMED_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "gems",
    value: ScriptInjectValue::StrList(&["Sapphire"]),
}];


pub(crate) fn herb_cleaner_scenario() -> Scenario {
    herb_cleaner_variant("herb_cleaner", HERB_CLEANER_INJECT, 3, false)
}

pub(crate) fn herb_cleaner_named_scenario() -> Scenario {
    herb_cleaner_variant("herb_cleaner_named", HERB_CLEANER_NAMED_INJECT, 5, true)
}

/// Frozen reference fixture: one sub-full pack of guam, Marrentill selected
/// but absent, then the script's own eventual empty-bank Stop.
pub(crate) fn herb_cleaner_empty_bank_scenario() -> Scenario {
    let xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Herblore 20 and exactly 20 unidentified guam before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat herblore 20");
                cheat(c, "givebank unidentified_guam 20");
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
    steps.push(drain_advancestat());
    for (step_name, arm) in [
        (
            "confirm Herblore 20 before Start",
            Proof::Stat {
                id: HERBLORE_STAT,
                min: 20,
            },
        ),
        (
            "confirm no unidentified guam in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 0,
            },
        ),
        (
            "confirm no clean guam in pack before Start",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ),
        (
            "confirm no unidentified marrentill in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 0,
            },
        ),
        (
            "confirm no clean marrentill in pack before Start",
            Proof::ItemIdAtMost {
                id: MARRENTILL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge exactly 20 unidentified guam",
        Proof::BankItemId {
            id: UNIDENTIFIED_GUAM_ID,
            count: 20,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge Marrentill is absent from the loaded seed bank",
        Proof::BankItemIdAtMost {
            id: UNIDENTIFIED_MARENTILL_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch script-created clean guam without requiring a full pack",
        Proof::ItemId {
            id: GUAM_LEAF_ID,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "watch Herblore XP from post-Start guam cleaning",
        xp,
    ));
    Scenario {
        name: "herb_cleaner_empty_bank",
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
            deadline: Duration::from_secs(420),
            start_script: Some("HerbCleaner"),
            script_settings_inject: Some(HERB_CLEANER_EMPTY_INJECT),
            terminal_shot: Some("herb_cleaner_empty_bank"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Empty pack, banked unidentified guam. Named also banks marrentill unids
/// that must stay put. Identify fills the pack, then deposit-all restocks.
fn herb_cleaner_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    level: i32,
    named: bool,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 2,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Herblore and unidentified bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat herblore {level}"));
                cheat(c, "givebank unidentified_guam 30");
                if named {
                    cheat(c, "givebank unidentified_marentill 4");
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
    for (step_name, arm) in [
        (
            "confirm Herblore before Start",
            Proof::Stat {
                id: HERBLORE_STAT,
                min: level,
            },
        ),
        (
            "confirm no seeded unidentified guam in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded clean guam before Start",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unidentified marrentill in pack before Start",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded clean marrentill before Start",
            Proof::ItemIdAtMost {
                id: MARRENTILL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact unidentified guam seed bank",
        Proof::BankItemId {
            id: UNIDENTIFIED_GUAM_ID,
            count: 30,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded clean guam in bank",
        Proof::BankItemIdAtMost {
            id: GUAM_LEAF_ID,
            count: 0,
        },
    ));
    if named {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact untouched marrentill seed bank",
            Proof::BankItemId {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 4,
            },
        ));
        steps.push(bank_fletcher_watch(
            "acknowledge no seeded clean marrentill in bank",
            Proof::BankItemIdAtMost {
                id: MARRENTILL_ID,
                count: 0,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        ("watch Herblore XP from identifying guam", first_xp),
        (
            "watch a full pack of exact clean guam before the bank cycle",
            Proof::ItemId {
                id: GUAM_LEAF_ID,
                count: 28,
            },
        ),
        (
            "watch exact unidentified guam consumed",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 0,
            },
        ),
        (
            "watch the script-created clean guam enter a fresh bank",
            Proof::BankItemId {
                id: GUAM_LEAF_ID,
                count: 28,
            },
        ),
        (
            "watch a restock of exact unidentified guam",
            Proof::ItemId {
                id: UNIDENTIFIED_GUAM_ID,
                count: 1,
            },
        ),
        (
            "watch exact unidentified guam bank stock decrease",
            Proof::BankItemIdAtMost {
                id: UNIDENTIFIED_GUAM_ID,
                count: 2,
            },
        ),
    ];
    if named {
        watch.push((
            "watch the filtered marrentill unids stay in bank",
            Proof::BankItemId {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 4,
            },
        ));
        watch.push((
            "watch no filtered marrentill enter the pack",
            Proof::ItemIdAtMost {
                id: UNIDENTIFIED_MARENTILL_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        ("watch the script close its herb bank", Proof::BankClosed),
        (
            "watch another exact clean guam after restock",
            Proof::ItemId {
                id: GUAM_LEAF_ID,
                count: 1,
            },
        ),
        ("watch Herblore XP beyond the first pack", further_xp),
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
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("HerbCleaner"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn gem_cutter_scenario() -> Scenario {
    gem_cutter_variant("gem_cutter", GEM_CUTTER_INJECT, false)
}

pub(crate) fn gem_cutter_named_scenario() -> Scenario {
    gem_cutter_variant("gem_cutter_named", GEM_CUTTER_NAMED_INJECT, true)
}

/// Empty pack, banked chisel plus uncut sapphires. Named also banks uncut
/// opal that must stay put. Deposit keeps the chisel.
fn gem_cutter_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    named: bool,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: CRAFTING_STAT,
        min: 2,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Crafting and uncut sapphire bank stock before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 20");
                cheat(c, "givebank chisel 1");
                cheat(c, "givebank uncut_sapphire 28");
                if named {
                    cheat(c, "givebank uncut_opal 4");
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
    for (step_name, arm) in [
        (
            "confirm Crafting 20 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 20,
            },
        ),
        (
            "confirm no seeded chisel in pack before Start",
            Proof::ItemIdAtMost {
                id: CHISEL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded uncut sapphire in pack before Start",
            Proof::ItemIdAtMost {
                id: UNCUT_SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded cut sapphire before Start",
            Proof::ItemIdAtMost {
                id: SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "confirm no crushed gemstone before Start",
            Proof::ItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded uncut opal in pack before Start",
            Proof::ItemIdAtMost {
                id: UNCUT_OPAL_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_open_seed_bank(
        "open and acknowledge the exact uncut sapphire seed bank",
        Proof::BankItemId {
            id: UNCUT_SAPPHIRE_ID,
            count: 28,
        },
    ));
    for (step_name, arm) in [
        (
            "acknowledge the exact chisel seed bank",
            Proof::BankItemId {
                id: CHISEL_ID,
                count: 1,
            },
        ),
        (
            "acknowledge no seeded cut sapphire in bank",
            Proof::BankItemIdAtMost {
                id: SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "acknowledge no crushed gemstone in bank",
            Proof::BankItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    if named {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact untouched uncut opal seed bank",
            Proof::BankItemId {
                id: UNCUT_OPAL_ID,
                count: 4,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        ("watch Crafting XP from cutting sapphire", first_xp),
        (
            "watch a chisel-kept pack of exact cut sapphires",
            Proof::ItemId {
                id: SAPPHIRE_ID,
                count: 27,
            },
        ),
        (
            "watch the chisel remain in pack",
            Proof::ItemId {
                id: CHISEL_ID,
                count: 1,
            },
        ),
        (
            "watch exact uncut sapphires consumed",
            Proof::ItemIdAtMost {
                id: UNCUT_SAPPHIRE_ID,
                count: 0,
            },
        ),
        (
            "watch no crushed gemstone from sapphire",
            Proof::ItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
        (
            "watch the script-created sapphires enter a fresh bank",
            Proof::BankItemId {
                id: SAPPHIRE_ID,
                count: 27,
            },
        ),
        (
            "watch a restock of exact uncut sapphire",
            Proof::ItemId {
                id: UNCUT_SAPPHIRE_ID,
                count: 1,
            },
        ),
        (
            "watch the chisel stay out of the deposit",
            Proof::ItemId {
                id: CHISEL_ID,
                count: 1,
            },
        ),
        (
            "watch exact uncut sapphire bank stock decrease",
            Proof::BankItemIdAtMost {
                id: UNCUT_SAPPHIRE_ID,
                count: 1,
            },
        ),
    ];
    if named {
        watch.push((
            "watch the filtered uncut opal stay in bank",
            Proof::BankItemId {
                id: UNCUT_OPAL_ID,
                count: 4,
            },
        ));
        watch.push((
            "watch no filtered uncut opal enter the pack",
            Proof::ItemIdAtMost {
                id: UNCUT_OPAL_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        ("watch the script close its gem bank", Proof::BankClosed),
        (
            "watch another exact cut sapphire after restock",
            Proof::ItemId {
                id: SAPPHIRE_ID,
                count: 1,
            },
        ),
        ("watch Crafting XP beyond the first pack", further_xp),
        (
            "watch crushed gemstone stay empty",
            Proof::ItemIdAtMost {
                id: CRUSHED_GEMSTONE_ID,
                count: 0,
            },
        ),
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
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("GemCutter"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) const AGILITY_STAT: i32 = 16;
pub(crate) const FLAX_ID: i32 = 1779;
pub(crate) const GATE_CLOSED_ID: i32 = 1551;
pub(crate) const GATE_OPEN_ID: i32 = 1552;

/// Packed closed wooden door 1530 on selected 274/289 m50_50, adjacent to
/// DoorOpener's default Lumbridge stand.
const LUMBRIDGE_DOOR: WorldTile = WorldTile {
    x: 3208,
    z: 3211,
    level: 0,
};
const LUMBRIDGE_DOOR_STAND: WorldTile = WorldTile {
    x: 3208,
    z: 3212,
    level: 0,
};

/// Packed closed wooden gate 1551 on selected 274/289 m50_50.
const LUMBRIDGE_GATE: WorldTile = WorldTile {
    x: 3213,
    z: 3261,
    level: 0,
};
const LUMBRIDGE_GATE_STAND: WorldTile = WorldTile {
    x: 3213,
    z: 3260,
    level: 0,
};

const GNOME_START: WorldTile = WorldTile {
    x: 2474,
    z: 3436,
    level: 0,
};
/// Log Walk-across dest is coord z-7 from selected gnome_course.rs2.
const GNOME_AFTER_LOG: WorldTile = WorldTile {
    x: 2474,
    z: 3429,
    level: 0,
};
/// Climb-down lands at packed 0_38_53_55_28.
const GNOME_GROUND_RETURN: WorldTile = WorldTile {
    x: 2487,
    z: 3420,
    level: 0,
};
const GNOME_PIPE: WorldTile = WorldTile {
    x: 2484,
    z: 3431,
    level: 0,
};

const WILDY_START: WorldTile = WorldTile {
    x: 2998,
    z: 3916,
    level: 0,
};
/// North of the selected inner Gate at (2998,3931). Radius 2 cannot include
/// the gate tile, so a ridge click without the world crossing fails closed.
const WILDY_AFTER_RIDGE: WorldTile = WorldTile {
    x: 2998,
    z: 3934,
    level: 0,
};
/// Selected m46_61 loc 2288 at (3004,3938); rs2 lands at loc z+9.
const WILDY_PIPE_DEST: WorldTile = WorldTile {
    x: 3004,
    z: 3947,
    level: 0,
};
/// Selected m46_61 loc 2283 at (3005,3952); rs2 lands five north of its stand.
const WILDY_ROPE_DEST: WorldTile = WorldTile {
    x: 3005,
    z: 3958,
    level: 0,
};
/// Selected m46_61 loc 2311 at (3001,3960); the sixth jump lands x-5.
const WILDY_STONE_DEST: WorldTile = WorldTile {
    x: 2996,
    z: 3960,
    level: 0,
};
/// Selected m46_61 loc 2297 is raw plane 1 over a LinkBelow bridge; the
/// player remains on observed scene plane 0 while the moves land x-7.
const WILDY_LOG_DEST: WorldTile = WorldTile {
    x: 2994,
    z: 3945,
    level: 0,
};
/// Centre tile of the selected three-wide rocks 2328; rs2 lands three south.
const WILDY_ROCKS_DEST: WorldTile = WorldTile {
    x: 2994,
    z: 3933,
    level: 0,
};

const BRIMHAVEN_ENTRANCE: WorldTile = WorldTile {
    x: 2809,
    z: 3194,
    level: 0,
};
/// Selected ladder 3617 Climb-Down destination, also arena platform 24.
const BRIMHAVEN_LADDER_LANDING: WorldTile = WorldTile {
    x: 2805,
    z: 9590,
    level: 3,
};
const AGILITY_TICKET_ID: i32 = 2996;
const AGILITY_ARENA_VARP: i32 = 309;

pub(super) const FLAX_FIELD: WorldTile = WorldTile {
    x: 2741,
    z: 3444,
    level: 0,
};
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

const DOOR_OPENER_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "stand",
    value: ScriptInjectValue::Str("3208,3212,0"),
}];

const DOOR_OPENER_GATE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "stand",
        value: ScriptInjectValue::Str("3213,3260,0"),
    },
    ScriptSettingInject {
        id: "obstacle",
        value: ScriptInjectValue::Str("gate"),
    },
];

const GNOME_COURSE_RADIUS_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "searchRadius",
    value: ScriptInjectValue::Num(8.0),
}];

const WILDY_AGILITY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "acquireFoodAtStart",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "minFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const BRIMHAVEN_AGILITY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "stealRestock",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtTickets",
        value: ScriptInjectValue::Num(1000.0),
    },
];

pub(crate) fn door_opener_scenario() -> Scenario {
    door_opener_variant(
        "door_opener",
        DOOR_OPENER_INJECT,
        LUMBRIDGE_DOOR,
        LUMBRIDGE_DOOR_STAND,
        CLOSED_ID,
        OPEN_ID,
    )
}

pub(crate) fn door_opener_gate_scenario() -> Scenario {
    door_opener_variant(
        "door_opener_gate",
        DOOR_OPENER_GATE_INJECT,
        LUMBRIDGE_GATE,
        LUMBRIDGE_GATE_STAND,
        GATE_CLOSED_ID,
        GATE_OPEN_ID,
    )
}

/// Walk to an adjacent stand, Close any open leaf before Start, then require
/// the selected shut loc to become the open id through a same-session world
/// change. Queued Open or script counters are not this proof.
fn door_opener_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    packed: WorldTile,
    stand: WorldTile,
    closed_id: i32,
    open_id: i32,
) -> Scenario {
    let shut = Proof::LocActionNear {
        id: closed_id,
        x: packed.x,
        z: packed.z,
        level: packed.level,
        radius: 1,
        action: "Open",
        present: true,
    };
    let opened = Proof::LocIdNear {
        id: open_id,
        x: packed.x,
        z: packed.z,
        level: packed.level,
        radius: 3,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "tele adjacent to the selected shut loc",
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
                radius: 1,
            },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "close the selected loc if it is still open",
        kind: StepKind::Perform {
            send: Box::new(move |c, snapshot| {
                if let Some(loc) = snapshot.locs().iter().find(|loc| {
                    loc.id == open_id
                        && loc.tile.level == packed.level
                        && (loc.tile.x - packed.x)
                            .abs()
                            .max((loc.tile.z - packed.z).abs())
                            <= 3
                }) {
                    op_loc(c, loc.tile.x, loc.tile.z, loc.id);
                }
                true
            }),
        },
        wait: Wait {
            arm: shut,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    });
    steps.push(start_catalog_step());
    steps.push(bank_fletcher_watch(
        "watch the selected loc become open after Start",
        opened,
    ));
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: opened,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("DoorOpener"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn gnome_course_scenario() -> Scenario {
    gnome_course_variant("gnome_course", None)
}

pub(crate) fn gnome_course_radius_scenario() -> Scenario {
    gnome_course_variant("gnome_course_radius", Some(GNOME_COURSE_RADIUS_INJECT))
}

/// Cross the south ridge, complete the five selected wilderness obstacles and
/// make real progress through the next pipe. All setup cheats happen before
/// catalog Start; post-Start steps are observation-only.
pub(crate) fn wildy_agility_scenario() -> Scenario {
    let further_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 598,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Agility 52 and five Lobsters, then tele south of the ridge",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "advancestat agility 52");
                cheat(c, "setstat hitpoints 40");
                cheat(c, "give lobster 5");
                cheat(
                    c,
                    &tele_args(WILDY_START.level, WILDY_START.x, WILDY_START.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: WILDY_START.x,
                z: WILDY_START.z,
                level: WILDY_START.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm five exact Lobsters before Start",
        Proof::ItemId {
            id: LOBSTER_ID,
            count: 5,
        },
    ));
    steps.push(drain_advancestat());
    steps.push(bank_fletcher_watch(
        "confirm Hitpoints 40 before the wilderness course",
        Proof::Stat {
            id: HITPOINTS_STAT,
            min: 40,
        },
    ));
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch ridge Agility XP",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 15,
            },
        ),
        (
            "watch the ridge world crossing north of the inner gate",
            Proof::ArrivedNear {
                x: WILDY_AFTER_RIDGE.x,
                z: WILDY_AFTER_RIDGE.z,
                level: WILDY_AFTER_RIDGE.level,
                radius: 2,
            },
        ),
        (
            "watch pipe XP after the ridge",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 27,
            },
        ),
        (
            "watch the selected pipe destination",
            Proof::ArrivedNear {
                x: WILDY_PIPE_DEST.x,
                z: WILDY_PIPE_DEST.z,
                level: WILDY_PIPE_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch ropeswing XP after the pipe",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 47,
            },
        ),
        (
            "watch the selected ropeswing destination",
            Proof::ArrivedNear {
                x: WILDY_ROPE_DEST.x,
                z: WILDY_ROPE_DEST.z,
                level: WILDY_ROPE_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch stepping-stone XP after the ropeswing",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 67,
            },
        ),
        (
            "watch the selected stepping-stone destination",
            Proof::ArrivedNear {
                x: WILDY_STONE_DEST.x,
                z: WILDY_STONE_DEST.z,
                level: WILDY_STONE_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch log XP after the stepping stones",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 87,
            },
        ),
        (
            "watch the selected log destination",
            Proof::ArrivedNear {
                x: WILDY_LOG_DEST.x,
                z: WILDY_LOG_DEST.z,
                level: WILDY_LOG_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch the five-obstacle lap XP bonus",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 586,
            },
        ),
        (
            "watch the selected rocks destination",
            Proof::ArrivedNear {
                x: WILDY_ROCKS_DEST.x,
                z: WILDY_ROCKS_DEST.z,
                level: WILDY_ROCKS_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch the next pipe destination after the full lap",
            Proof::ArrivedNear {
                x: WILDY_PIPE_DEST.x,
                z: WILDY_PIPE_DEST.z,
                level: WILDY_PIPE_DEST.level,
                radius: 3,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "wildy_agility",
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
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("WildyAgility"),
            script_settings_inject: Some(WILDY_AGILITY_INJECT),
            terminal_shot: Some("wildy_agility"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Pay and enter naturally, require obstacle XP before the script's first
/// no-ticket Tag, earn a later ticket, then require independently fresh obstacle
/// XP after that ticket.
pub(crate) fn brimhaven_agility_scenario() -> Scenario {
    let first_hop_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 1,
    };
    let subsequent_xp = Proof::FreshStatXpGain {
        id: AGILITY_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Agility 52, 1000 Coins and ten Lobsters, then tele to the arena entrance",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "advancestat agility 52");
                cheat(c, "give coins 1000");
                cheat(c, "give lobster 10");
                cheat(
                    c,
                    &tele_args(
                        BRIMHAVEN_ENTRANCE.level,
                        BRIMHAVEN_ENTRANCE.x,
                        BRIMHAVEN_ENTRANCE.z,
                    ),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: BRIMHAVEN_ENTRANCE.x,
                z: BRIMHAVEN_ENTRANCE.z,
                level: BRIMHAVEN_ENTRANCE.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(drain_advancestat());
    for (name, arm) in [
        (
            "confirm 1000 exact Coins before Start",
            Proof::ItemId {
                id: COINS_ID,
                count: 1000,
            },
        ),
        (
            "confirm ten exact Lobsters before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 10,
            },
        ),
        (
            "confirm no seeded agility-arena ticket before Start",
            Proof::ItemIdAtMost {
                id: AGILITY_TICKET_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch the 200-Coin arena fee",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 800,
            },
        ),
        (
            "watch the arena paid bit",
            Proof::Varp {
                id: AGILITY_ARENA_VARP,
                min: 2,
            },
        ),
        (
            "watch Climb-Down reach the arena ladder platform",
            Proof::ArrivedNear {
                x: BRIMHAVEN_LADDER_LANDING.x,
                z: BRIMHAVEN_LADDER_LANDING.z,
                level: BRIMHAVEN_LADDER_LANDING.level,
                radius: 2,
            },
        ),
        (
            "watch obstacle XP from a real hop before the first Tag",
            first_hop_xp,
        ),
        (
            "watch the first Tag prompt for the next pillar",
            Proof::Chat {
                needle: "tag the next",
            },
        ),
        (
            "watch the first Tag set the tagged bit",
            Proof::Varp {
                id: AGILITY_ARENA_VARP,
                min: 15,
            },
        ),
        (
            "confirm the first Tag grants no ticket",
            Proof::ItemIdAtMost {
                id: AGILITY_TICKET_ID,
                count: 0,
            },
        ),
        (
            "watch a later Tag grant the first ticket",
            Proof::ItemId {
                id: AGILITY_TICKET_ID,
                count: 1,
            },
        ),
        (
            "watch subsequent arena obstacle XP after the ticket",
            subsequent_xp,
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "brimhaven_agility",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: subsequent_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BrimhavenAgility"),
            script_settings_inject: Some(BRIMHAVEN_AGILITY_INJECT),
            terminal_shot: Some("brimhaven_agility"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Complete a natural gnome lap, then cross the log again. Selected 274/289
/// content grants 86.5 XP per lap plus 7.5 for the next log. Stored milestones
/// are on the ground; snapshot levels follow the actual client plane.
fn gnome_course_variant(
    name: &'static str,
    inject: Option<&'static [ScriptSettingInject]>,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 94,
    };
    let start = GNOME_START;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "tele to the gnome course start before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(start.level, start.x, start.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: start.x,
                z: start.z,
                level: start.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Agility XP from the first obstacle", first_xp),
        (
            "watch the log dest tile after Walk-across",
            Proof::ArrivedNear {
                x: GNOME_AFTER_LOG.x,
                z: GNOME_AFTER_LOG.z,
                level: GNOME_AFTER_LOG.level,
                radius: 3,
            },
        ),
        (
            "watch the selected climb-down ground return",
            Proof::ArrivedNear {
                x: GNOME_GROUND_RETURN.x,
                z: GNOME_GROUND_RETURN.z,
                level: GNOME_GROUND_RETURN.level,
                radius: 3,
            },
        ),
        (
            "watch the obstacle pipe after the ground nets",
            Proof::ArrivedNear {
                x: GNOME_PIPE.x,
                z: GNOME_PIPE.z,
                level: GNOME_PIPE.level,
                radius: 6,
            },
        ),
        (
            "watch further Agility XP at the start of a second lap",
            further_xp,
        ),
        (
            "watch the log dest after second-lap progress",
            Proof::ArrivedNear {
                x: GNOME_AFTER_LOG.x,
                z: GNOME_AFTER_LOG.z,
                level: GNOME_AFTER_LOG.level,
                radius: 3,
            },
        ),
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
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("GnomeCourse"),
            script_settings_inject: inject,
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn flax_picker_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "clear the pack and tele to the default Seers flax field",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm no seeded flax in pack before Start",
        Proof::ItemIdAtMost {
            id: FLAX_ID,
            count: 0,
        },
    ));
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch a full pack of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        (
            "watch script-created flax enter a fresh Seers bank",
            Proof::BankItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        ("watch the flax bank close after deposit", Proof::BankClosed),
        (
            "watch return to the flax field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 12,
            },
        ),
        (
            "watch further exact flax after return",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_picker",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: FLAX_ID,
            count: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxPicker"),
            terminal_shot: Some("flax_picker"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) const MAGIC_STAT: i32 = 6;
pub(crate) const SMITHING_STAT: i32 = 13;
pub(crate) const COPPER_ORE_ID: i32 = 436;
pub(crate) const TIN_ORE_ID: i32 = 438;
pub(crate) const IRON_ORE_ID: i32 = 440;
/// Selected 289 `obj.pack`: silver_ore=442, silver_bar=2355.
pub(crate) const SILVER_ORE_ID: i32 = 442;
pub(crate) const COAL_ID: i32 = 453;
pub(crate) const FIRE_BATTLESTAFF_ID: i32 = 1393;
pub(crate) const BRONZE_BAR_ID: i32 = 2349;
pub(crate) const IRON_BAR_ID: i32 = 2351;
pub(crate) const STEEL_BAR_ID: i32 = 2353;
pub(crate) const SILVER_BAR_ID: i32 = 2355;
pub(crate) const SUPERHEAT_MAGIC: i32 = 43;
pub(crate) const BRONZE_SMITHING: i32 = 1;
/// SuperheaterLogic Silver recipe level.
pub(crate) const SILVER_SMITHING: i32 = 20;
pub(crate) const STEEL_SMITHING: i32 = 30;
/// SuperheaterLogic Mithril recipe level; 4 Coal per bar (5 bars / 27-slot trip).
pub(crate) const MITHRIL_SMITHING: i32 = 50;
/// Selected 289 `obj.pack`: mithril_ore=447, mithril_bar=2359.
pub(crate) const MITHRIL_ORE_ID: i32 = 447;
pub(crate) const MITHRIL_BAR_ID: i32 = 2359;
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

pub(crate) const EMPTY_VIAL_ID: i32 = 229;
pub(crate) const VIAL_OF_WATER_ID: i32 = 227;
pub(crate) const EYE_OF_NEWT_ID: i32 = 221;
pub(crate) const GUAM_UNF_ID: i32 = 91;
pub(crate) const ATTACK_POTION_3_ID: i32 = 121;
pub(crate) const RANARR_WEED_ID: i32 = 257;
pub(crate) const RANARR_UNF_ID: i32 = 99;
pub(crate) const SNAPE_GRASS_ID: i32 = 231;
pub(crate) const PRAYER_POTION_3_ID: i32 = 139;
pub(crate) const POTION_BATCH_SEED: i32 = 42;
pub(crate) const VIAL_EMPTY_SEED: i32 = 56;
pub(crate) const GUAM_HERBLORE: i32 = 3;
/// Source has no herblore field. Seed high enough that Ranarr mixing is
/// not refused; this is not a generated-data requirement.
pub(crate) const RANARR_HERBLORE: i32 = 38;

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
pub(crate) const FALADOR_EAST_BOOTH: WorldTile = WorldTile {
    x: 3013,
    z: 3354,
    level: 0,
};
const FALADOR_FOUNTAIN: WorldTile = WorldTile {
    x: 2949,
    z: 3381,
    level: 0,
};

const VIAL_FILLER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bank",
        value: ScriptInjectValue::Str("Falador West"),
    },
    ScriptSettingInject {
        id: "buyVials",
        value: ScriptInjectValue::Bool(false),
    },
];

const VIAL_FILLER_EAST_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "bank",
        value: ScriptInjectValue::Str("Falador East"),
    },
    ScriptSettingInject {
        id: "buyVials",
        value: ScriptInjectValue::Bool(false),
    },
];

const POTION_MAKER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "herb",
        value: ScriptInjectValue::Str("Custom"),
    },
    ScriptSettingInject {
        id: "herbCustom",
        value: ScriptInjectValue::Str("Guam leaf"),
    },
    ScriptSettingInject {
        id: "secondary",
        value: ScriptInjectValue::Str("Custom"),
    },
    ScriptSettingInject {
        id: "secondaryCustom",
        value: ScriptInjectValue::Str("Eye of newt"),
    },
];

const POTION_MAKER_NAMED_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "herb",
        value: ScriptInjectValue::Str("Ranarr weed"),
    },
    ScriptSettingInject {
        id: "secondary",
        value: ScriptInjectValue::Str("Snape grass"),
    },
];

pub(crate) const COW_HIDE_ID: i32 = 1739;
pub(crate) const SOFT_LEATHER_ID: i32 = 1741;
pub(crate) const HARD_LEATHER_ID: i32 = 1743;
pub(crate) const TANNER_HIDE_SEED: i32 = 28;
pub(crate) const TANNER_COIN_SEED: i32 = 5000;

pub(crate) const AL_KHARID_BANK: WorldTile = WorldTile {
    x: 3269,
    z: 3167,
    level: 0,
};
const TANNER_STAND: WorldTile = WorldTile {
    x: 3277,
    z: 3191,
    level: 0,
};

const TANNER_BOT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "hideType",
        value: ScriptInjectValue::Str("Soft leather"),
    },
    ScriptSettingInject {
        id: "buyThread",
        value: ScriptInjectValue::Bool(false),
    },
];

const TANNER_BOT_HARD_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "hideType",
        value: ScriptInjectValue::Str("Hard leather"),
    },
    ScriptSettingInject {
        id: "buyThread",
        value: ScriptInjectValue::Bool(false),
    },
];

pub(crate) const RUNECRAFT_STAT: i32 = 20;
pub(crate) const RUNE_ESSENCE_ID: i32 = 1436;
pub(crate) const NOTED_ESSENCE_ID: i32 = 1437;
pub(crate) const AIR_TALISMAN_ID: i32 = 1438;
const EARTH_TALISMAN_ID: i32 = 1440;
pub(crate) const AIR_RUNE_ID: i32 = 556;
pub(crate) const EARTH_RUNE_ID: i32 = 557;
pub(crate) const RUNE_ESSENCE_SEED: i32 = 200;
pub(super) const PAIR_AIR_FIRST_LOAD: i32 = 25;
pub(super) const PAIR_MULE_FIRST_LOAD: i32 = 27;

const RUNECRAFTER_AIR_RUINS: WorldTile = WorldTile {
    x: 2988,
    z: 3294,
    level: 0,
};
const RUNECRAFTER_EARTH_RUINS: WorldTile = WorldTile {
    x: 3303,
    z: 3477,
    level: 0,
};
pub(super) const MULECRAFTER_AIR_RUINS: WorldTile = WorldTile {
    x: 2983,
    z: 3288,
    level: 0,
};
pub(super) const VARROCK_EAST_BANK: WorldTile = WorldTile {
    x: 3253,
    z: 3420,
    level: 0,
};

const RUNE_CRAFTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "rune",
        value: ScriptInjectValue::Str("Air runes"),
    },
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("Solo"),
    },
];

const RUNE_CRAFTER_EARTH_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "rune",
        value: ScriptInjectValue::Str("Earth runes"),
    },
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("Solo"),
    },
];

const MULE_CRAFTER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "rune",
        value: ScriptInjectValue::Str("Air rune"),
    },
    ScriptSettingInject {
        id: "mode",
        value: ScriptInjectValue::Str("Crafter"),
    },
    ScriptSettingInject {
        id: "partner",
        value: ScriptInjectValue::Str(""),
    },
    ScriptSettingInject {
        id: "bankFill",
        value: ScriptInjectValue::Bool(true),
    },
];

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

pub(crate) fn vial_filler_scenario() -> Scenario {
    vial_filler_variant(
        "vial_filler",
        VIAL_FILLER_INJECT,
        FALADOR_WEST_BANK,
        FALADOR_WEST_BOOTH,
    )
}

pub(crate) fn vial_filler_east_scenario() -> Scenario {
    vial_filler_variant(
        "vial_filler_east",
        VIAL_FILLER_EAST_INJECT,
        FALADOR_EAST_BANK,
        FALADOR_EAST_BOOTH,
    )
}

/// Empty pack at the selected Falador bank. Banked empty vials, no water.
/// Script withdraws, fills at the west fountain, deposits produced water
/// vials, empties the pack of water, restocks empties, returns and fills
/// again. Shop-buy stays pending.
fn vial_filler_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    bank: WorldTile,
    booth: WorldTile,
) -> Scenario {
    let fountain = Proof::ArrivedNear {
        x: FALADOR_FOUNTAIN.x,
        z: FALADOR_FOUNTAIN.z,
        level: FALADOR_FOUNTAIN.level,
        radius: 4,
    };
    let filled = Proof::ItemId {
        id: VIAL_OF_WATER_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed empty vials and tele to the selected Falador bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("givebank vial_empty {VIAL_EMPTY_SEED}"));
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
            "confirm no seeded empty vials in pack before Start",
            Proof::ItemIdAtMost {
                id: EMPTY_VIAL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded water vials in pack before Start",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(open_seed_booth(
        "open and acknowledge the exact empty-vial seed bank",
        booth,
        Proof::BankItemId {
            id: EMPTY_VIAL_ID,
            count: VIAL_EMPTY_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded water vials in bank",
        Proof::BankItemIdAtMost {
            id: VIAL_OF_WATER_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the Falador fountain after Start",
            fountain,
        ),
        (
            "watch empty vials become water vials at the fountain",
            filled,
        ),
        (
            "watch the withdrawn empty vials finish filling",
            Proof::ItemIdAtMost {
                id: EMPTY_VIAL_ID,
                count: 0,
            },
        ),
        (
            "watch script-created water vials enter a fresh bank",
            Proof::BankItemId {
                id: VIAL_OF_WATER_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of water vials after deposit",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
        (
            "watch a restock of exact empty vials",
            Proof::ItemId {
                id: EMPTY_VIAL_ID,
                count: 1,
            },
        ),
        ("watch the script close its vial bank", Proof::BankClosed),
        (
            "watch return to the Falador fountain after restock",
            fountain,
        ),
        ("watch another exact water vial after restock", filled),
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
        proof: filled,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("VialFiller"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn potion_maker_scenario() -> Scenario {
    potion_maker_variant(
        "potion_maker",
        POTION_MAKER_INJECT,
        PotionMakerRecipe {
            herb_id: GUAM_LEAF_ID,
            unf_id: GUAM_UNF_ID,
            secondary_id: EYE_OF_NEWT_ID,
            finished_id: ATTACK_POTION_3_ID,
            wrong_unf_id: RANARR_UNF_ID,
            wrong_finished_id: PRAYER_POTION_3_ID,
            herb_alias: "guam_leaf",
            secondary_alias: "eye_of_newt",
            herblore: GUAM_HERBLORE,
            named: false,
        },
    )
}

pub(crate) fn potion_maker_named_scenario() -> Scenario {
    potion_maker_variant(
        "potion_maker_named",
        POTION_MAKER_NAMED_INJECT,
        PotionMakerRecipe {
            herb_id: RANARR_WEED_ID,
            unf_id: RANARR_UNF_ID,
            secondary_id: SNAPE_GRASS_ID,
            finished_id: PRAYER_POTION_3_ID,
            wrong_unf_id: GUAM_UNF_ID,
            wrong_finished_id: ATTACK_POTION_3_ID,
            herb_alias: "ranarr_weed",
            secondary_alias: "snape_grass",
            herblore: RANARR_HERBLORE,
            named: true,
        },
    )
}

fn potion_maker_live_seed_steps() -> Vec<Step> {
    vec![
        Step {
            name: "stick tutorial skip and complete Druidic Ritual",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, "setvar tutorial 1000");
                    // Frozen quest def: druidquest is the server-only
                    // permanent varp; COMPLETE = 4. Relog paints the journal.
                    cheat(c, "setvar druidquest 4");
                    cheat(c, "getvar tutorial");
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Chat {
                    needle: "get tutorial: 1000",
                },
                budget_ticks: 200,
            },
        },
        Step {
            name: "relog so the inv tab binds and journal refreshes",
            kind: StepKind::Relog,
            wait: Wait {
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 600,
            },
        },
        bank_fletcher_watch(
            "acknowledge Druidic Ritual before Start",
            Proof::QuestDone {
                name: "Druidic Ritual",
            },
        ),
    ]
}

/// Empty pack at Varrock West. Banked herb, water and secondary. Script
/// withdraws a batch, spam-uses herb onto water, withdraws the secondary,
/// finishes, deposits, restocks and makes further product. Named selector
/// also banks leftover Guam that must stay put.
#[derive(Clone, Copy)]
struct PotionMakerRecipe {
    herb_id: i32,
    unf_id: i32,
    secondary_id: i32,
    finished_id: i32,
    wrong_unf_id: i32,
    wrong_finished_id: i32,
    herb_alias: &'static str,
    secondary_alias: &'static str,
    herblore: i32,
    named: bool,
}

fn potion_maker_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    recipe: PotionMakerRecipe,
) -> Scenario {
    let PotionMakerRecipe {
        herb_id,
        unf_id,
        secondary_id,
        finished_id,
        wrong_unf_id,
        wrong_finished_id,
        herb_alias,
        secondary_alias,
        herblore,
        named,
    } = recipe;
    let first_xp = Proof::StatXpGain {
        id: HERBLORE_STAT,
        min: 1,
    };
    let finished = Proof::ItemId {
        id: finished_id,
        count: 1,
    };
    let bank = VARROCK_WEST_BANK;
    let mut steps = potion_maker_live_seed_steps();
    steps.push(Step {
        name: "seed Herblore and potion ingredients before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat herblore {herblore}"));
                cheat(c, &format!("givebank {herb_alias} {POTION_BATCH_SEED}"));
                cheat(c, &format!("givebank vial_water {POTION_BATCH_SEED}"));
                cheat(
                    c,
                    &format!("givebank {secondary_alias} {POTION_BATCH_SEED}"),
                );
                if named {
                    cheat(c, "givebank guam_leaf 14");
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
            "confirm Herblore before Start",
            Proof::Stat {
                id: HERBLORE_STAT,
                min: herblore,
            },
        ),
        (
            "confirm no seeded herb in pack before Start",
            Proof::ItemIdAtMost {
                id: herb_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded water vials in pack before Start",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded secondary in pack before Start",
            Proof::ItemIdAtMost {
                id: secondary_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded unfinished potion before Start",
            Proof::ItemIdAtMost {
                id: unf_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded finished potion before Start",
            Proof::ItemIdAtMost {
                id: finished_id,
                count: 0,
            },
        ),
        (
            "confirm no wrong unfinished potion before Start",
            Proof::ItemIdAtMost {
                id: wrong_unf_id,
                count: 0,
            },
        ),
        (
            "confirm no wrong finished potion before Start",
            Proof::ItemIdAtMost {
                id: wrong_finished_id,
                count: 0,
            },
        ),
    ];
    if named {
        before_start.push((
            "confirm no seeded leftover Guam in pack before Start",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ));
    }
    for (step_name, arm) in before_start {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(open_seed_booth(
        "open and acknowledge the exact herb seed bank",
        WorldTile {
            x: VARROCK_WEST_BANK.x + 1,
            z: VARROCK_WEST_BANK.z,
            level: VARROCK_WEST_BANK.level,
        },
        Proof::BankItemId {
            id: herb_id,
            count: POTION_BATCH_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact water-vial seed bank",
        Proof::BankItemId {
            id: VIAL_OF_WATER_ID,
            count: POTION_BATCH_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact secondary seed bank",
        Proof::BankItemId {
            id: secondary_id,
            count: POTION_BATCH_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded unfinished potion in bank",
        Proof::BankItemIdAtMost {
            id: unf_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded finished potion in bank",
        Proof::BankItemIdAtMost {
            id: finished_id,
            count: 0,
        },
    ));
    if named {
        steps.push(bank_fletcher_watch(
            "acknowledge the exact leftover Guam seed bank",
            Proof::BankItemId {
                id: GUAM_LEAF_ID,
                count: 14,
            },
        ));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    let mut watch = vec![
        (
            "watch herb plus water become the exact unfinished potion",
            Proof::ItemId {
                id: unf_id,
                count: 1,
            },
        ),
        (
            "watch the withdrawn herb consumed into unfinished potions",
            Proof::ItemIdAtMost {
                id: herb_id,
                count: 0,
            },
        ),
        (
            "watch the withdrawn water vials consumed into unfinished potions",
            Proof::ItemIdAtMost {
                id: VIAL_OF_WATER_ID,
                count: 0,
            },
        ),
        (
            "watch the unfinished potions become the exact finished potion",
            finished,
        ),
        (
            "watch unfinished potions consumed into the finished product",
            Proof::ItemIdAtMost {
                id: unf_id,
                count: 0,
            },
        ),
        ("watch Herblore XP from finishing potions", first_xp),
        (
            "watch no wrong unfinished potion",
            Proof::ItemIdAtMost {
                id: wrong_unf_id,
                count: 0,
            },
        ),
        (
            "watch no wrong finished potion",
            Proof::ItemIdAtMost {
                id: wrong_finished_id,
                count: 0,
            },
        ),
        (
            "watch script-created finished potions enter a fresh bank",
            Proof::BankItemId {
                id: finished_id,
                count: 1,
            },
        ),
        (
            "watch a restock of exact water vials",
            Proof::ItemId {
                id: VIAL_OF_WATER_ID,
                count: 1,
            },
        ),
        (
            "watch a restock of the exact herb",
            Proof::ItemId {
                id: herb_id,
                count: 1,
            },
        ),
        (
            "watch finished potions leave the pack across deposit/restock",
            Proof::ItemIdAtMost {
                id: finished_id,
                count: 0,
            },
        ),
    ];
    if named {
        watch.push((
            "watch leftover Guam stay in bank",
            Proof::BankItemId {
                id: GUAM_LEAF_ID,
                count: 14,
            },
        ));
        watch.push((
            "watch leftover Guam never enter the pack",
            Proof::ItemIdAtMost {
                id: GUAM_LEAF_ID,
                count: 0,
            },
        ));
    }
    watch.extend([
        (
            "watch the script close its potion restock bank",
            Proof::BankClosed,
        ),
        (
            "watch another exact unfinished potion after restock",
            Proof::ItemId {
                id: unf_id,
                count: 1,
            },
        ),
        (
            "watch another exact finished potion after restock",
            finished,
        ),
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
        proof: finished,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("PotionMaker"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
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

pub(crate) fn tanner_bot_scenario() -> Scenario {
    tanner_bot_variant(
        "tanner_bot",
        TANNER_BOT_INJECT,
        SOFT_LEATHER_ID,
        HARD_LEATHER_ID,
    )
}

pub(crate) fn tanner_bot_hard_scenario() -> Scenario {
    tanner_bot_variant(
        "tanner_bot_hard",
        TANNER_BOT_HARD_INJECT,
        HARD_LEATHER_ID,
        SOFT_LEATHER_ID,
    )
}

/// Empty pack at Al-Kharid bank. Banked cowhides 1739 and coins, never leather.
/// Script withdraws, tans at the Tanner widget (not a shop), deposits produced
/// leather, empties the pack of leather, restocks hides, returns and tans again.
/// Dommik thread-buy stays pending.
fn tanner_bot_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    product_id: i32,
    wrong_product_id: i32,
) -> Scenario {
    let tanner = Proof::ArrivedNear {
        x: TANNER_STAND.x,
        z: TANNER_STAND.z,
        level: TANNER_STAND.level,
        radius: 4,
    };
    let leather = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let bank = AL_KHARID_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed cowhides, coins, and tele to Al-Kharid bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("givebank cow_hide {TANNER_HIDE_SEED}"));
                cheat(c, &format!("givebank coins {TANNER_COIN_SEED}"));
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
            "confirm no seeded cowhides in pack before Start",
            Proof::ItemIdAtMost {
                id: COW_HIDE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded soft leather in pack before Start",
            Proof::ItemIdAtMost {
                id: SOFT_LEATHER_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded hard leather in pack before Start",
            Proof::ItemIdAtMost {
                id: HARD_LEATHER_ID,
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
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact cowhide seed bank",
        Proof::BankItemId {
            id: COW_HIDE_ID,
            count: TANNER_HIDE_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact coin seed bank",
        Proof::BankItemId {
            id: COINS_ID,
            count: TANNER_COIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded leather in bank",
        Proof::BankItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded wrong leather in bank",
        Proof::BankItemIdAtMost {
            id: wrong_product_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch arrival at the Tanner after Start", tanner),
        (
            "watch cowhides become the selected leather at the Tanner",
            leather,
        ),
        (
            "watch the withdrawn cowhides finish converting",
            Proof::ItemIdAtMost {
                id: COW_HIDE_ID,
                count: 0,
            },
        ),
        (
            "watch script-created leather enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of leather after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch a restock of exact cowhides",
            Proof::ItemId {
                id: COW_HIDE_ID,
                count: 1,
            },
        ),
        ("watch the script close its tanner bank", Proof::BankClosed),
        ("watch return to the Tanner after restock", tanner),
        ("watch another exact leather after restock", leather),
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
        proof: leather,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("TannerBot"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) fn rune_crafter_scenario() -> Scenario {
    rune_craft_variant(RuneCraftSpec {
        name: "rune_crafter",
        start_script: "RuneCrafter",
        inject: RUNE_CRAFTER_INJECT,
        bank: FALADOR_EAST_BANK,
        ruins: RUNECRAFTER_AIR_RUINS,
        rc_level: 1,
        talisman_alias: "air_talisman",
        talisman_id: AIR_TALISMAN_ID,
        rune_id: AIR_RUNE_ID,
        wrong_rune_id: EARTH_RUNE_ID,
    })
}

pub(crate) fn rune_crafter_earth_scenario() -> Scenario {
    rune_craft_variant(RuneCraftSpec {
        name: "rune_crafter_earth",
        start_script: "RuneCrafter",
        inject: RUNE_CRAFTER_EARTH_INJECT,
        bank: VARROCK_EAST_BANK,
        ruins: RUNECRAFTER_EARTH_RUINS,
        rc_level: 9,
        talisman_alias: "earth_talisman",
        talisman_id: EARTH_TALISMAN_ID,
        rune_id: EARTH_RUNE_ID,
        wrong_rune_id: AIR_RUNE_ID,
    })
}

pub(crate) fn mule_crafter_scenario() -> Scenario {
    rune_craft_variant(RuneCraftSpec {
        name: "mule_crafter",
        start_script: "MuleCrafter",
        inject: MULE_CRAFTER_INJECT,
        bank: FALADOR_EAST_BANK,
        ruins: MULECRAFTER_AIR_RUINS,
        rc_level: 1,
        talisman_alias: "air_talisman",
        talisman_id: AIR_TALISMAN_ID,
        rune_id: AIR_RUNE_ID,
        wrong_rune_id: EARTH_RUNE_ID,
    })
}

fn runecraft_open_seed_bank(name: &'static str, arm: Proof) -> Step {
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

/// Empty pack at the selected bank. Banked unnoted essence 1436 and the
/// selected talisman; never crafted runes or noted 1437. Script withdraws,
/// uses the talisman on the selected Mysterious ruins, Craft-rune, banks the
/// produced runes, restocks essence and crafts again. Trade/paired modes stay
/// pending.
struct RuneCraftSpec {
    name: &'static str,
    start_script: &'static str,
    inject: &'static [ScriptSettingInject],
    bank: WorldTile,
    ruins: WorldTile,
    rc_level: i32,
    talisman_alias: &'static str,
    talisman_id: i32,
    rune_id: i32,
    wrong_rune_id: i32,
}

fn rune_craft_variant(spec: RuneCraftSpec) -> Scenario {
    let RuneCraftSpec {
        name,
        start_script,
        inject,
        bank,
        ruins,
        rc_level,
        talisman_alias,
        talisman_id,
        rune_id,
        wrong_rune_id,
    } = spec;
    let ruins_near = Proof::ArrivedNear {
        x: ruins.x,
        z: ruins.z,
        level: ruins.level,
        radius: 4,
    };
    let crafted = Proof::ItemId {
        id: rune_id,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed runecraft, banked essence, talisman, and tele before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat runecraft {rc_level}"));
                cheat(c, &format!("givebank blankrune {RUNE_ESSENCE_SEED}"));
                cheat(c, &format!("givebank {talisman_alias} 1"));
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
            "confirm runecraft level before Start",
            Proof::Stat {
                id: RUNECRAFT_STAT,
                min: rc_level,
            },
        ),
        (
            "confirm no seeded essence in pack before Start",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted essence in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded selected runes in pack before Start",
            Proof::ItemIdAtMost {
                id: rune_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded wrong runes in pack before Start",
            Proof::ItemIdAtMost {
                id: wrong_rune_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded talisman in pack before Start",
            Proof::ItemIdAtMost {
                id: talisman_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(runecraft_open_seed_bank(
        "open and acknowledge the exact unnoted essence seed bank",
        Proof::BankItemId {
            id: RUNE_ESSENCE_ID,
            count: RUNE_ESSENCE_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge the exact talisman seed bank",
        Proof::BankItemId {
            id: talisman_id,
            count: 1,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted essence in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_ESSENCE_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded selected runes in bank",
        Proof::BankItemIdAtMost {
            id: rune_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded wrong runes in bank",
        Proof::BankItemIdAtMost {
            id: wrong_rune_id,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch script withdrawal of unnoted essence after Start",
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: 1,
            },
        ),
        (
            "watch arrival at the selected mysterious ruins after Start",
            ruins_near,
        ),
        (
            "watch Runecraft XP from the selected craft",
            Proof::StatXpGain {
                id: RUNECRAFT_STAT,
                min: 1,
            },
        ),
        (
            "watch essence become the selected rune after altar entry",
            crafted,
        ),
        (
            "watch the withdrawn essence finish converting",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: 0,
            },
        ),
        ("watch portal exit back to the selected ruins", ruins_near),
        (
            "watch script-created runes enter a fresh bank",
            Proof::BankItemId {
                id: rune_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of runes after deposit",
            Proof::ItemIdAtMost {
                id: rune_id,
                count: 0,
            },
        ),
        (
            "watch a restock of unnoted essence",
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: 1,
            },
        ),
        (
            "watch the script close its runecraft bank",
            Proof::BankClosed,
        ),
        (
            "watch return to the selected ruins after restock",
            ruins_near,
        ),
        ("watch another exact selected rune after restock", crafted),
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
        proof: crafted,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some(start_script),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) const THIEVING_STAT: i32 = 17;
pub(crate) const HITPOINTS_STAT: i32 = 3;
const ARDY_CAKES_BALLAST_KNIVES: i32 = 22;
pub(crate) const CAKE_ID: i32 = 1891;
pub(crate) const BREAD_ID: i32 = 2309;
pub(crate) const CHOCOLATE_SLICE_ID: i32 = 1901;
pub(crate) const CHOCOLATE_CAKE_ID: i32 = 1897;

const ARDY_CAKES_STAND: WorldTile = WorldTile {
    x: 2668,
    z: 3312,
    level: 0,
};
pub(crate) const ARDY_BANK: WorldTile = WorldTile {
    x: 2655,
    z: 3286,
    level: 0,
};

const ARDY_CAKES_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Flee"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
];

const ARDY_CAKES_FIGHT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Fight"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
];

const ARDY_THIEVER_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "thieveTarget",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Flee"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "restockAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const ARDY_THIEVER_FIGHT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "thieveTarget",
        value: ScriptInjectValue::Str("Guard"),
    },
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Fight"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    // A full opening restock: one frozen stealCakes session (<= 90 s) at the
    // stall, where the Guard catches. See ardy_thiever_fight_scenario.
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(27.0),
    },
    ScriptSettingInject {
        id: "restockAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const ARDY_THIEVER_KNIGHT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "thieveTarget",
        value: ScriptInjectValue::Str("Knight of Ardougne"),
    },
    ScriptSettingInject {
        id: "guardResponse",
        value: ScriptInjectValue::Str("Flee"),
    },
    ScriptSettingInject {
        id: "solveClues",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtLootSlots",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "foodTarget",
        value: ScriptInjectValue::Num(1.0),
    },
    ScriptSettingInject {
        id: "restockAtFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

/// Six free slots at the Baker's stall stand. Flee, clues off. Script steals
/// cake/bread/chocolate slice with Thieving XP, deposits the acquired stock
/// when the pack is full, returns to STAND and steals again. Fight stays
/// pending. Cake 1891 is the sequential identity; chocolate cake 1897 is not
/// stall food.
pub(crate) fn ardy_cakes_scenario() -> Scenario {
    let stand = ARDY_CAKES_STAND;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let cake = Proof::ItemId {
        id: CAKE_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, hitpoints, 22-Knife ballast and Baker's stall stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat thieving 5");
                cheat(c, "setstat hitpoints 40");
                cheat(c, &format!("give knife {ARDY_CAKES_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Thieving 5 before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: 5,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: 40,
            },
        ),
        (
            "confirm 22 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 22 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bread in pack before Start",
            Proof::ItemIdAtMost {
                id: BREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate slice in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_SLICE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP from Baker's stall after Start", first_xp),
        ("watch exact Cake 1891 stolen after Start", cake),
        (
            "watch arrival at the Ardougne bank after the stall fill",
            Proof::ArrivedNear {
                x: ARDY_BANK.x,
                z: ARDY_BANK.z,
                level: ARDY_BANK.level,
                radius: 6,
            },
        ),
        (
            "watch script-stolen cake enter a fresh Ardougne bank",
            Proof::BankItemId {
                id: CAKE_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of cake after deposit",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
        (
            "watch return to the Baker's stall stand after deposit",
            Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
        ),
        ("watch the cake bank close after deposit", Proof::BankClosed),
        ("watch another exact Cake 1891 after return", cake),
    ] {
        let bank_arrival = matches!(
            arm,
            Proof::ArrivedNear { x, z, .. } if x == ARDY_BANK.x && z == ARDY_BANK.z
        );
        let mut step = bank_fletcher_watch(step_name, arm);
        // The fill plus the bank walk: see ARDY_CAKES_BANK_WATCH_TICKS.
        if bank_arrival {
            step.wait.budget_ticks = ARDY_CAKES_BANK_WATCH_TICKS;
        }
        steps.push(step);
    }
    Scenario {
        name: "ardy_cakes",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: cake,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: ARDY_CAKES_DEADLINE,
            start_script: Some("ArdyCakes"),
            script_settings_inject: Some(ARDY_CAKES_INJECT),
            terminal_shot: Some("ardy_cakes"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// `guardResponse=Fight`: stall steal then FightBack kill of the catching Guard.
/// Combat stats and a scimitar are prepared so the FightBack branch can land;
/// the Flee kite is not this cell. Catalog owns the Guard defeat witness.
pub(crate) fn ardy_cakes_fight_scenario() -> Scenario {
    let stand = ARDY_CAKES_STAND;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let cake = Proof::ItemId {
        id: CAKE_ID,
        count: 1,
    };
    let style_xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, combat stats, scimitar, 22-Knife ballast and Baker's stall stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat thieving 5");
                cheat(c, &format!("setstat attack {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat strength {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "give adamant_scimitar 1");
                cheat(c, &format!("give knife {ARDY_CAKES_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Thieving 5 before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: 5,
            },
        ),
        (
            "confirm Attack 40 before Start",
            Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Strength 40 before Start",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm 22 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 22 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: ARDY_CAKES_BALLAST_KNIVES,
            },
        ),
        (
            "confirm prepared scimitar before wielding",
            Proof::ItemId {
                id: COMBAT_SCIMITAR_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bread in pack before Start",
            Proof::ItemIdAtMost {
                id: BREAD_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate slice in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_SLICE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded chocolate cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CHOCOLATE_CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(wear_combat_item_step(
        "wield and acknowledge Adamant scimitar before Start",
        COMBAT_SCIMITAR_ID,
    ));
    steps.push(select_strength_combat_style_step());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP from Baker's stall after Start", first_xp),
        ("watch exact Cake 1891 stolen after Start", cake),
        (
            "watch Strength XP from FightBack on the catching Guard after Start",
            style_xp,
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "ardy_cakes_fight",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: style_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ArdyCakes"),
            script_settings_inject: Some(ARDY_CAKES_FIGHT_INJECT),
            terminal_shot: Some("ardy_cakes_fight"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

struct ArdyThieverSpec {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    thieving: i32,
}

pub(crate) fn ardy_thiever_scenario() -> Scenario {
    ardy_thiever_variant(ArdyThieverSpec {
        name: "ardy_thiever",
        inject: ARDY_THIEVER_INJECT,
        thieving: 40,
    })
}

pub(crate) fn ardy_thiever_knight_scenario() -> Scenario {
    ardy_thiever_variant(ArdyThieverSpec {
        name: "ardy_thiever_knight",
        inject: ARDY_THIEVER_KNIGHT_INJECT,
        thieving: 55,
    })
}

/// `guardResponse=Fight` on ArdyThiever: FightBack kills the Guard that catches
/// a stall steal, then pickpocket coins and the loot-count bank/return/further
/// cycle of the Flee cell. Combat kit is prepared before Start; catalog owns
/// the Guard defeat + no-Flee gate.
///
/// **Why the kill comes first (289 content).** A failed pickpocket never starts
/// combat: `fail_pick_pocket` (thieving.rs2) only stuns and deals 2 damage.
/// The only Guard aggression is `stealing_check_for_guard` on a stall steal
/// (stealing.rs2). ArdyThiever steals from the stall only while its food is at
/// `restockAtFood`, and after the first coins it gets there only by eating,
/// which needs HP lost to failed pickpockets (Guard success at Thieving 40 is
/// 126/256). Each success also costs a ~60-tick bank trip at loot-count 1.
/// Modelled, a catch after the coins within the 150-dirty watch is 1-9% with
/// the old inject and about 42% with the best food seed (at a 0.2 catch
/// chance per steal). So this cell watches
/// Strength XP first, and injects `foodTarget` 27 with `restockAtFood` 0: the
/// opening RestockCakes call is one full frozen stealCakes session at the
/// stall. A Monte Carlo of that session (8-tick respawn after a success, ~5
/// ticks per silent owner refusal with a swap after three) puts a Guard catch
/// within 150 ticks of Start at 0.97 on the pooled 289 rates (11 success /
/// 5 refused / 3 caught of 19 steals), 0.84 on the ArdyCakes run alone, and
/// 1.0 at the passing `ardy_cakes_fight` rates. The watch is 150 runner
/// dirties, about 143 engine ticks at the measured 1.05 dirties per tick,
/// which gives 0.967 / 0.837 / 1.0. The coins, deposit, return and
/// further-coins gates follow unchanged; the whole-run cap is
/// `ARDY_THIEVER_FIGHT_DEADLINE`.
pub(crate) fn ardy_thiever_fight_scenario() -> Scenario {
    let stand = ARDOUGNE_GUARD;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let coins = Proof::ItemId {
        id: COINS_ID,
        count: 1,
    };
    let style_xp = Proof::StatXpGain {
        id: STRENGTH_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, combat stats, scimitar, empty pack and market stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat thieving 40");
                cheat(c, &format!("setstat attack {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat strength {COMBAT_ATTACK_LEVEL}"));
                cheat(c, &format!("setstat hitpoints {COMBAT_ATTACK_LEVEL}"));
                cheat(c, "give adamant_scimitar 1");
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
    for (step_name, arm) in [
        (
            "confirm prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: 40,
            },
        ),
        (
            "confirm Attack 40 before Start",
            Proof::Stat {
                id: 0,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Strength 40 before Start",
            Proof::Stat {
                id: STRENGTH_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: COMBAT_ATTACK_LEVEL,
            },
        ),
        (
            "confirm prepared scimitar before wielding",
            Proof::ItemId {
                id: COMBAT_SCIMITAR_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(wear_combat_item_step(
        "wield and acknowledge Adamant scimitar before Start",
        COMBAT_SCIMITAR_ID,
    ));
    steps.push(select_strength_combat_style_step());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Strength XP from FightBack on the catching Guard after Start",
            style_xp,
        ),
        ("watch Thieving XP after Start", first_xp),
        ("watch exact Coins 995 pickpocketed after Start", coins),
        (
            "watch script-pickpocketed coins enter a fresh Ardougne bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of coins after deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "watch return to the market stand after deposit",
            Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
        ),
        (
            "watch the pickpocket bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Coins 995 after return", coins),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "ardy_thiever_fight",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: coins,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: ARDY_THIEVER_FIGHT_DEADLINE,
            start_script: Some("ArdyThiever"),
            script_settings_inject: Some(ARDY_THIEVER_FIGHT_INJECT),
            terminal_shot: Some("ardy_thiever_fight"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Empty pack at the Guard/Knight stand. Flee, clues off, loot-count bank
/// at 1 slot. Script restocks one stall food, pickpockets coins with
/// Thieving XP, deposits those coins, returns to the stand and pickpockets
/// again. PeriodicBank Off is not the bank proof. Fight stays pending.
///
/// **Windows under the Baker owner model** (see `ARDY_CAKES_BANK_WATCH_TICKS`:
/// 0.63 refusals per attempt, guard catch 0.12). Only one stall success is
/// needed (`foodTarget` 1), so the ordinary 150-dirty (~143 engine ticks)
/// watches hold. A 20k-run simulation gives P(first Thieving XP ≤143) = 0.962
/// for the Guard and 0.964 for the Knight (p95 132 / 128). It gives P(coins
/// ≤143 after that XP) = 0.986 / 0.989 (p95 68 / 60). That counts
/// pickpockets at 126/256 and 155/256, stun damage 2 / 3, and a stall
/// revisit when a one-bite food is eaten.
fn ardy_thiever_variant(spec: ArdyThieverSpec) -> Scenario {
    let ArdyThieverSpec {
        name,
        inject,
        thieving,
    } = spec;
    let stand = ARDOUGNE_GUARD;
    let first_xp = Proof::StatXpGain {
        id: THIEVING_STAT,
        min: 1,
    };
    let coins = Proof::ItemId {
        id: COINS_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed thieving, hitpoints, empty pack and market stand before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat thieving {thieving}"));
                cheat(c, "setstat hitpoints 40");
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
    for (step_name, arm) in [
        (
            "confirm prepared Thieving before Start",
            Proof::Stat {
                id: THIEVING_STAT,
                min: thieving,
            },
        ),
        (
            "confirm Hitpoints 40 before Start",
            Proof::Stat {
                id: HITPOINTS_STAT,
                min: 40,
            },
        ),
        (
            "confirm no seeded coins in pack before Start",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded cake in pack before Start",
            Proof::ItemIdAtMost {
                id: CAKE_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Thieving XP after Start", first_xp),
        ("watch exact Coins 995 pickpocketed after Start", coins),
        (
            "watch script-pickpocketed coins enter a fresh Ardougne bank",
            Proof::BankItemId {
                id: COINS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of coins after deposit",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "watch return to the market stand after deposit",
            Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
        ),
        (
            "watch the pickpocket bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Coins 995 after return", coins),
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
        proof: coins,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("ArdyThiever"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

pub(crate) const WOODCUTTING_STAT: i32 = 8;
pub(crate) const MINING_STAT: i32 = 14;
pub(crate) const MAGIC_LOGS_ID: i32 = 1513;
const NOTED_MAGIC_LOGS_ID: i32 = 1514;
pub(crate) const UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 72;
pub(crate) const UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 70;
pub(crate) const MAGIC_SHORTBOW_ID: i32 = 861;
const MAGIC_LONGBOW_ID: i32 = 859;
const NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 73;
const NOTED_UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 71;
pub(crate) const KNIFE_ID: i32 = 946;
pub(crate) const RUNE_AXE_ID: i32 = 1359;
const MAGIC_TREE_ID: i32 = 1306;
/// One Rune axe plus 26 unstackable Knives leaves one product slot. The
/// frozen Gnome script preserves Knife as a tool during gear prep and bank trips.
pub(crate) const GNOME_BALLAST_KNIVES: i32 = 26;
pub(crate) const RUNE_PICKAXE_ID: i32 = 1275;
const NOTED_COAL_ID: i32 = 454;
/// A Rune pickaxe plus 26 unstackable Knives leaves one slot for Coal.
/// CoalTrucks keeps non-coal items when it empties the pack into a truck.
pub(crate) const COAL_BALLAST_KNIVES: i32 = 26;
/// With the ordinary prayer/ranged/magic defaults, 48 in each melee/HP stat
/// yields native combat level 55 without over-leveling the fixture.
const COAL_MELEE_LEVEL: i32 = 48;

const GNOME_SOUTH_BANK_MAGIC_STAND: WorldTile = WorldTile {
    x: 2433,
    z: 3409,
    level: 0,
};
const GNOME_SOUTH_BANK_MAGIC_TREE: WorldTile = WorldTile {
    x: 2432,
    z: 3410,
    level: 0,
};
pub(crate) const GNOME_BANK_STAND: WorldTile = WorldTile {
    x: 2445,
    z: 3425,
    level: 1,
};
pub(crate) const GNOME_BANK_STAIR_SOUTH: WorldTile = WorldTile {
    x: 2444,
    z: 3416,
    level: 0,
};
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

const GNOME_CHOP_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "fletchLogs",
    value: ScriptInjectValue::Bool(false),
}];

const GNOME_FLETCH_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "fletchLogs",
    value: ScriptInjectValue::Bool(true),
}];

/// One free product slot at the south-bank Magic tree. fletchLogs off. Seed WC 75, Rune
/// axe 1359, and retained nonproduct Knife ballast. The script chops one
/// magic log 1513 with Woodcutting XP, deposits it at the upstairs gnome
/// booth, returns to ground and chops again. This qualifies the resource
/// cycle, not ordinary 28-slot throughput. Death recovery stays out.
pub(crate) fn gnome_chop_scenario() -> Scenario {
    let stand = GNOME_SOUTH_BANK_MAGIC_STAND;
    let first_xp = Proof::StatXpGain {
        id: WOODCUTTING_STAT,
        min: 1,
    };
    let logs = Proof::ItemId {
        id: MAGIC_LOGS_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed woodcutting, rune axe, 26-Knife ballast and the south-bank Magic tree before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat woodcutting 75");
                cheat(c, "give rune_axe 1");
                cheat(c, &format!("give knife {GNOME_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 1,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Woodcutting 75 before Start",
            Proof::Stat {
                id: WOODCUTTING_STAT,
                min: 75,
            },
        ),
        (
            "confirm Rune axe 1359 before Start",
            Proof::ItemId {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exactly one Rune axe before Start",
            Proof::ItemIdAtMost {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exact south-bank Magic tree and Chop down action before Start",
            Proof::LocActionNear {
                id: MAGIC_TREE_ID,
                x: GNOME_SOUTH_BANK_MAGIC_TREE.x,
                z: GNOME_SOUTH_BANK_MAGIC_TREE.z,
                level: GNOME_SOUTH_BANK_MAGIC_TREE.level,
                radius: 0,
                action: "Chop down",
                present: true,
            },
        ),
        (
            "confirm 26 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 26 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded magic logs in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unstrung magic shortbow in pack before Start",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_MAGIC_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unstrung magic longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_MAGIC_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted magic logs in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted unstrung magic shortbow in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted unstrung magic longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_UNSTRUNG_MAGIC_LONGBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Woodcutting XP from a Magic tree after Start",
            first_xp,
        ),
        ("watch exact Magic logs 1513 chopped after Start", logs),
        (
            "watch arrival at the upstairs gnome booth after the log fill",
            Proof::ArrivedNear {
                x: GNOME_BANK_STAND.x,
                z: GNOME_BANK_STAND.z,
                level: GNOME_BANK_STAND.level,
                radius: 8,
            },
        ),
        (
            "watch script-chopped magic logs enter a fresh gnome bank",
            Proof::BankItemId {
                id: MAGIC_LOGS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of magic logs after deposit",
            Proof::ItemIdAtMost {
                id: MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "watch ground return at the gnome bank stairs after deposit",
            Proof::ArrivedNear {
                x: GNOME_BANK_STAIR_SOUTH.x,
                z: GNOME_BANK_STAIR_SOUTH.z,
                level: GNOME_BANK_STAIR_SOUTH.level,
                radius: 30,
            },
        ),
        (
            "watch the gnome log bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Magic logs 1513 after return", logs),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "gnome_chop",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: logs,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("GnomeMagicChopper"),
            script_settings_inject: Some(GNOME_CHOP_INJECT),
            terminal_shot: Some("gnome_chop"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

struct GnomeFletchSpec {
    name: &'static str,
    fletching: i32,
    fletching_max: Option<i32>,
    product_id: i32,
}

pub(crate) fn gnome_fletch_short_scenario() -> Scenario {
    gnome_fletch_variant(GnomeFletchSpec {
        name: "gnome_fletch_short",
        fletching: 80,
        fletching_max: Some(84),
        product_id: UNSTRUNG_MAGIC_SHORTBOW_ID,
    })
}

pub(crate) fn gnome_fletch_long_scenario() -> Scenario {
    gnome_fletch_variant(GnomeFletchSpec {
        name: "gnome_fletch_long",
        fletching: 85,
        fletching_max: None,
        product_id: UNSTRUNG_MAGIC_LONGBOW_ID,
    })
}

/// fletchLogs on. Seed WC 75, Fletching 80/85, Rune axe, and 26 retained
/// Knives so one script-chopped log fills the pack. The script consumes it
/// into exact unstrung 72/70 with Fletching XP, deposits the bow upstairs,
/// returns to ground and chops again. This is cycle qualification, not
/// ordinary capacity proof. Missing Knife is a stop, not a pass; strung
/// 861/859 are not the unstrung product.
fn gnome_fletch_variant(spec: GnomeFletchSpec) -> Scenario {
    let GnomeFletchSpec {
        name,
        fletching,
        fletching_max,
        product_id,
    } = spec;
    let stand = GNOME_SOUTH_BANK_MAGIC_STAND;
    let first_wc = Proof::StatXpGain {
        id: WOODCUTTING_STAT,
        min: 1,
    };
    let first_fletch = Proof::StatXpGain {
        id: FLETCHING_STAT,
        min: 1,
    };
    let logs = Proof::ItemId {
        id: MAGIC_LOGS_ID,
        count: 1,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name:
            "seed woodcutting, fletching, rune axe, 26-Knife ballast and the south-bank Magic tree before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat woodcutting 75");
                cheat(c, &format!("setstat fletching {fletching}"));
                cheat(c, "give rune_axe 1");
                cheat(c, &format!("give knife {GNOME_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 1,
            },
            budget_ticks: 200,
        },
    });
    let mut seed_arms = vec![
        (
            "confirm Woodcutting 75 before Start",
            Proof::Stat {
                id: WOODCUTTING_STAT,
                min: 75,
            },
        ),
        (
            "confirm prepared Fletching before Start",
            Proof::Stat {
                id: FLETCHING_STAT,
                min: fletching,
            },
        ),
        (
            "confirm Rune axe 1359 before Start",
            Proof::ItemId {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exactly one Rune axe before Start",
            Proof::ItemIdAtMost {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exact south-bank Magic tree and Chop down action before Start",
            Proof::LocActionNear {
                id: MAGIC_TREE_ID,
                x: GNOME_SOUTH_BANK_MAGIC_TREE.x,
                z: GNOME_SOUTH_BANK_MAGIC_TREE.z,
                level: GNOME_SOUTH_BANK_MAGIC_TREE.level,
                radius: 0,
                action: "Chop down",
                present: true,
            },
        ),
        (
            "confirm 26 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 26 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded magic logs in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unstrung product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded strung magic shortbow in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded strung magic longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_LONGBOW_ID,
                count: 0,
            },
        ),
    ];
    if let Some(max) = fletching_max {
        seed_arms.insert(
            2,
            (
                "confirm Fletching below longbow 85 before Start",
                Proof::StatAtMost {
                    id: FLETCHING_STAT,
                    max,
                },
            ),
        );
    }
    for (step_name, arm) in seed_arms {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    // The two Magic-tree roll arms get MAGIC_TREE_CHOP_WATCH_TICKS (dirty
    // increments sized to the 17/256-per-4-ticks roll, not 150×600ms).
    // Other arms keep the ordinary gold watch.
    let chop = MAGIC_TREE_CHOP_WATCH_TICKS;
    let gold = SCRIPT_GOLD_WATCH_TICKS;
    for (step_name, arm, budget_ticks) in [
        (
            "watch Woodcutting XP from a Magic tree after Start",
            first_wc,
            chop,
        ),
        (
            "watch exact Magic logs 1513 chopped after Start",
            logs,
            gold,
        ),
        ("watch Fletching XP after Start", first_fletch, gold),
        (
            "watch exact unstrung magic bow after logs are consumed",
            product,
            gold,
        ),
        (
            "watch script-fletched bows enter a fresh gnome bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
            gold,
        ),
        (
            "watch the pack empty of unstrung bows after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
            gold,
        ),
        (
            "watch ground return at the gnome bank stairs after deposit",
            Proof::ArrivedNear {
                x: GNOME_BANK_STAIR_SOUTH.x,
                z: GNOME_BANK_STAIR_SOUTH.z,
                level: GNOME_BANK_STAIR_SOUTH.level,
                radius: 30,
            },
            gold,
        ),
        (
            "watch the gnome fletch bank close after deposit",
            Proof::BankClosed,
            gold,
        ),
        (
            "watch another exact Magic logs 1513 after return",
            logs,
            chop,
        ),
    ] {
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
        proof: logs,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: MAGIC_TREE_CHOP_DEADLINE,
            start_script: Some("GnomeMagicChopper"),
            script_settings_inject: Some(GNOME_FLETCH_INJECT),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Seed Mining 60, ordinary combat-55 melee stats, Rune pickaxe 1275, and
/// 26 retained nonproduct Knives on the safe initial tile. The one free slot
/// makes the first mined Coal fill the pack. Observe real mining XP and exact
/// coal 453, then a mine-truck deposit (pack empty of coal at the truck stand,
/// not a Seers bank), then further mining.
/// Filling truck 120 then Seers haul/bank/return cannot fit
/// SCRIPT_GOLD_DEADLINE 180s from an empty truck; no truck-content seed
/// primitive exists. Death/combat recovery is not this core.
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
const NOTED_FLAX_ID: i32 = 1780;
const NOTED_BOW_STRING_ID: i32 = 1778;
pub(crate) const BALL_OF_WOOL_ID: i32 = 1759;
pub(crate) const COOKING_FIXTURE_LEVEL: i32 = 80;
pub(crate) const COOK_RAW_SEED: i32 = 56;
pub(crate) const SMELT_ORE_SEED: i32 = 56;
pub(crate) const STEEL_COAL_SEED: i32 = 112;
pub(crate) const FLAX_SPIN_SEED: i32 = 56;

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
const FLAX_SPINNER_BANK: WorldTile = WorldTile {
    x: 2722,
    z: 3493,
    level: 0,
};
pub(crate) const FLAX_SPINNER_WHEEL: WorldTile = WorldTile {
    x: 2711,
    z: 3471,
    level: 1,
};
/// FlaxAIO BANK_STAND; the spinner booth seed is 2722,3493,0.
const FLAX_AIO_BANK: WorldTile = WorldTile {
    x: 2725,
    z: 3493,
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

const FLAX_AIO_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "picking",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "spinning",
        value: ScriptInjectValue::Bool(true),
    },
];
const FLAX_AIO_PICK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "picking",
        value: ScriptInjectValue::Bool(true),
    },
    ScriptSettingInject {
        id: "spinning",
        value: ScriptInjectValue::Bool(false),
    },
];
const FLAX_AIO_SPIN_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "picking",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "spinning",
        value: ScriptInjectValue::Bool(true),
    },
];
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
pub(crate) fn flax_spinner_scenario() -> Scenario {
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let bank = FLAX_SPINNER_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed crafting, banked flax, and tele to the Seers flax bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 10");
                cheat(c, &format!("givebank flax {FLAX_SPIN_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Crafting 10 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 10,
            },
        ),
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded ball of wool in pack before Start",
            Proof::ItemIdAtMost {
                id: BALL_OF_WOOL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted flax in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_BOW_STRING_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact flax seed bank",
        Proof::BankItemId {
            id: FLAX_ID,
            count: FLAX_SPIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bow string in bank",
        Proof::BankItemIdAtMost {
            id: BOW_STRING_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted flax in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_FLAX_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the upstairs spinning wheel after Start",
            wheel,
        ),
        (
            "watch Crafting XP from the spinning wheel after Start",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bow string 1777 after Start", product),
        (
            "watch the withdrawn flax finish converting",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "watch script-spun bow string enter a fresh Seers bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bow string after deposit",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "watch a restock of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
        ("watch the script close its spin bank", Proof::BankClosed),
        (
            "watch return to the upstairs spinning wheel after restock",
            wheel,
        ),
        ("watch another exact bow string after restock", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_spinner",
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
            start_script: Some("FlaxSpinner"),
            terminal_shot: Some("flax_spinner"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn flax_aio_empty_pack_confirms() -> Vec<(&'static str, Proof)> {
    vec![
        (
            "confirm Crafting 10 before Start",
            Proof::Stat {
                id: CRAFTING_STAT,
                min: 10,
            },
        ),
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded ball of wool in pack before Start",
            Proof::ItemIdAtMost {
                id: BALL_OF_WOOL_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted flax in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_BOW_STRING_ID,
                count: 0,
            },
        ),
    ]
}

/// Empty pack at the Seers flax field. FlaxAIO own script, both flags on.
/// Pick 1779, climb, makeX 1779→1777 with Crafting XP, deposit strings,
/// closed return to the field, further Pick. Not FlaxPicker+FlaxSpinner.
pub(crate) fn flax_aio_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let flax = Proof::ItemId {
        id: FLAX_ID,
        count: 1,
    };
    let product = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed crafting and tele to the Seers flax field before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 10");
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in flax_aio_empty_pack_confirms() {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch exact flax 1779 from the Seers field after Start",
            flax,
        ),
        (
            "watch arrival at the upstairs spinning wheel after picking",
            wheel,
        ),
        (
            "watch Crafting XP from the spinning wheel after Start",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bow string 1777 after Start", product),
        (
            "watch the picked flax finish converting",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "watch script-spun bow string enter a fresh Seers bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bow string after deposit",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        ("watch the script close its flax bank", Proof::BankClosed),
        (
            "watch return to the flax field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 12,
            },
        ),
        ("watch further exact flax after return", flax),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_aio",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: flax,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxAIO"),
            script_settings_inject: Some(FLAX_AIO_INJECT),
            terminal_shot: Some("flax_aio"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// FlaxAIO pick-only. Full flax pack, deposit 1779, closed return, further
/// Pick. Spun 1777 must not qualify.
pub(crate) fn flax_aio_pick_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "clear the pack and tele to the Seers flax field before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted flax in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_FLAX_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch a full pack of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        (
            "watch script-created flax enter a fresh Seers bank",
            Proof::BankItemId {
                id: FLAX_ID,
                count: 28,
            },
        ),
        (
            "watch the pack empty of flax after deposit",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        ("watch the flax bank close after deposit", Proof::BankClosed),
        (
            "watch return to the flax field after banking",
            Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 12,
            },
        ),
        (
            "watch further exact flax after return",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_aio_pick",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::ItemId {
            id: FLAX_ID,
            count: 1,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("FlaxAIO"),
            script_settings_inject: Some(FLAX_AIO_PICK_INJECT),
            terminal_shot: Some("flax_aio_pick"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// FlaxAIO spin-only. Banked flax at FlaxAIO's own booth stand, wheel
/// conversion+XP, string deposit, restock, closed return upstairs, further
/// spin. Wool is not this core.
pub(crate) fn flax_aio_spin_scenario() -> Scenario {
    let wheel = Proof::ArrivedNear {
        x: FLAX_SPINNER_WHEEL.x,
        z: FLAX_SPINNER_WHEEL.z,
        level: FLAX_SPINNER_WHEEL.level,
        radius: 8,
    };
    let product = Proof::ItemId {
        id: BOW_STRING_ID,
        count: 1,
    };
    let bank = FLAX_AIO_BANK;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed crafting, banked flax, and tele to FlaxAIO's Seers bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat crafting 10");
                cheat(c, &format!("givebank flax {FLAX_SPIN_SEED}"));
                cheat(c, &tele_args(bank.level, bank.x, bank.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank.x,
                z: bank.z,
                level: bank.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in flax_aio_empty_pack_confirms() {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the exact flax seed bank",
        Proof::BankItemId {
            id: FLAX_ID,
            count: FLAX_SPIN_SEED,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded bow string in bank",
        Proof::BankItemIdAtMost {
            id: BOW_STRING_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_watch(
        "acknowledge no seeded noted flax in bank",
        Proof::BankItemIdAtMost {
            id: NOTED_FLAX_ID,
            count: 0,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch arrival at the upstairs spinning wheel after Start",
            wheel,
        ),
        (
            "watch Crafting XP from the spinning wheel after Start",
            Proof::StatXpGain {
                id: CRAFTING_STAT,
                min: 1,
            },
        ),
        ("watch exact unnoted bow string 1777 after Start", product),
        (
            "watch the withdrawn flax finish converting",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "watch script-spun bow string enter a fresh Seers bank",
            Proof::BankItemId {
                id: BOW_STRING_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of bow string after deposit",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        (
            "watch a restock of exact flax 1779",
            Proof::ItemId {
                id: FLAX_ID,
                count: 1,
            },
        ),
        ("watch the script close its spin bank", Proof::BankClosed),
        (
            "watch return to the upstairs spinning wheel after restock",
            wheel,
        ),
        ("watch another exact bow string after restock", product),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "flax_aio_spin",
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
            start_script: Some("FlaxAIO"),
            script_settings_inject: Some(FLAX_AIO_SPIN_INJECT),
            terminal_shot: Some("flax_aio_spin"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

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
