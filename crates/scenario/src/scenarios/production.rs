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
mod coal_trucks;
pub(crate) use coal_trucks::*;
mod cooking;
pub(crate) use cooking::*;
mod smelting;
pub(crate) use smelting::*;
mod herblore_secondaries;
pub(crate) use herblore_secondaries::*;
mod climbing_boots;
pub(crate) use climbing_boots::*;
mod smithing;
pub(crate) use smithing::*;
mod leather;
pub(crate) use leather::*;
mod firemaking;
pub(crate) use firemaking::*;

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

pub(super) const CATHERBY_BANK: WorldTile = WorldTile {
    x: 2809,
    z: 3441,
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

/// HerbloreSecondaries default Red spiders' eggs. Banked lobster via native
/// note seed + deposit (stock289 has no `givebank`). Fixture loadout pins
/// `scriptFood` to Lobster (no food setting; blank loadout uses the operator
/// first carry). Carry `FOOD_DEFAULT_COUNT` at the field so loot starts
/// instead of an empty-pack Edgeville restock. Ground Take 223, deposit,
/// empty product pack, close, return, further Take. Eggs are not given.

pub(super) const STAFF_OF_AIR_ID: i32 = 1381;
pub(super) const STAFF_OF_WATER_ID: i32 = 1383;
