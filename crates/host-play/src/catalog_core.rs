//! Shared full-core catalog witness used by headless and headed proof paths.
//!
//! This module intentionally contains the exact compact observations, Start
//! predicates, cycle observers and qualification logic from the original
//! `catalog_boundary_live` harness. It is proof infrastructure, not gameplay.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use api::line_of_sight::{line_of_sight_v2, CollisionQuery};
use api::obj_names::ObjNames;
use api::snapshot::{ActorKind, GameSnapshot, LocView, NpcView, SceneView, WorldTile};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[path = "catalog_core_ranging.rs"]
mod ranging;
pub use ranging::{
    ranging_guild_bank_baseline_ready, ranging_guild_full_baseline_ready,
    ranging_guild_redeem_baseline_ready, ranging_guild_round_baseline_ready, RangingGuildBankCycle,
    RangingGuildFullCycle, RangingGuildRedeemCycle, RangingGuildRoundCycle, ARCHERY_TICKET_ID,
    COINS_PER_TRIP, ENTRY_FEE, RANGED_LIVE, RANGING_GUILD_FULL_MINUTES,
    RANGING_GUILD_FULL_STOP_NEEDLE, RANGING_GUILD_MERCHANT_STAND, RANGING_GUILD_SEERS_BANK,
    RANGING_GUILD_STAND, RUNE_ARROWS_PER_TRADE, SEED_KEEP_TICKETS, SEERS_BANK_RADIUS,
    TARGET_RESULT_MODAL, TICKETS_PER_TRADE, VARP_TARGET_COUNT, VARP_TARGET_HIT, VARP_TARGET_SCORE,
};
#[path = "catalog_core_hunt.rs"]
mod hunt;
pub use hunt::{
    hunt_baseline_ready, leave_lair_box, parse_hunt_receipt_line, HuntBox, HuntCell,
    HuntDeliveryCycle, HuntObservation, HuntOutcome, HuntReceipt, ScriptAct, ScriptActLedger,
    ScriptActRow, ScriptActsPublished, ACQUIRE_KEY_DEST, ACQUIRE_KEY_RADIUS,
    ACQUIRE_KEY_RECEIPT_PREFIX, ACQUIRE_KEY_V2_STOP, BANK_V2_DEST, BANK_V2_RADIUS,
    BANK_V2_RECEIPT_PREFIX, BANK_V2_STOP, CELL_V2_DOOR, CELL_V2_RADIUS, CELL_V2_RECEIPT_PREFIX,
    CELL_V2_STOP, DUSTY_KEY_ID, ENTER_LAIR_MAX_CHEB, ENTER_LAIR_MIN_CHEB,
    ENTER_LAIR_RECEIPT_PREFIX, ENTER_LAIR_V2_STOP, HOLD_SPOT_MAX_CHEB, HOLD_SPOT_MIN_CHEB,
    HOLD_SPOT_RECEIPT_PREFIX, HOLD_SPOT_V2_STOP, HUNT_ACT_ROWS, HUNT_LAIR_FIXTURE, JAILER,
    JAIL_CELL, JAIL_DOOR, JAIL_DOOR_LOC, JAIL_KEY_ID, LEAVE_LAIR_BOX_PAD, LEAVE_LAIR_MAX_CHEB,
    LEAVE_LAIR_MIN_CHEB, LEAVE_LAIR_RADIUS, LEAVE_LAIR_RECEIPT_PREFIX, LEAVE_LAIR_V2_STOP,
    RETREAT_SPOT_MAX_CHEB, RETREAT_SPOT_MIN_CHEB, RETREAT_SPOT_RECEIPT_PREFIX,
    RETREAT_SPOT_V2_STOP, VELRAK, WALK_SPOT_MAX_CHEB, WALK_SPOT_MIN_CHEB, WALK_SPOT_RECEIPT_PREFIX,
    WALK_SPOT_V2_STOP,
};

#[path = "catalog_ids.rs"]
mod ids;
pub use ids::*;
#[path = "catalog_case.rs"]
mod case;
pub use case::*;

#[path = "catalog_observation.rs"]
mod observation;
pub use observation::*;
use observation::empty_worn;
#[path = "catalog_inspect.rs"]
mod inspect;
pub use inspect::*;
#[path = "catalog_prayer.rs"]
mod prayer;
pub use prayer::*;
#[path = "catalog_los.rs"]
mod los;
pub use los::*;
#[path = "catalog_actor.rs"]
mod actor;
pub use actor::*;
#[path = "catalog_fight_field.rs"]
mod fight_field;
pub use fight_field::*;
#[path = "catalog_bone.rs"]
mod bone;
pub use bone::*;
#[path = "catalog_fletching.rs"]
mod fletching;
pub use fletching::*;
#[path = "catalog_alchemy.rs"]
mod alchemy;
pub use alchemy::*;
#[path = "catalog_herbs.rs"]
mod herbs;
pub use herbs::*;
#[path = "catalog_gems.rs"]
mod gems;
pub use gems::*;
#[path = "catalog_doors.rs"]
mod doors;
pub use doors::*;
#[path = "catalog_agility.rs"]
mod agility;
pub use agility::*;
#[path = "catalog_flax.rs"]
mod flax;
pub use flax::*;
#[path = "catalog_combat.rs"]
mod combat;
pub use combat::*;












pub fn fire_in_varrock_east_plot(observation: &Observation) -> bool {
    observation.loc_facts.iter().any(|loc| {
        loc.level == 0
            && loc.x >= FIRE_PLOT_VARROCK_EAST_X0
            && loc.x <= FIRE_PLOT_VARROCK_EAST_X1
            && loc.z >= FIRE_PLOT_VARROCK_EAST_Z0
            && loc.z <= FIRE_PLOT_VARROCK_EAST_Z1
            && loc
                .name
                .as_deref()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("fire"))
    })
}

pub fn stall_food(observation: &Observation) -> i32 {
    observation.item_id(CAKE_ID)
        + observation.item_id(BREAD_ID)
        + observation.item_id(CHOCOLATE_SLICE_ID)
}

pub fn bank_stall_food(observation: &Observation) -> i32 {
    observation.bank_item_id(CAKE_ID)
        + observation.bank_item_id(BREAD_ID)
        + observation.bank_item_id(CHOCOLATE_SLICE_ID)
}

pub fn noted_stall_food(observation: &Observation) -> i32 {
    observation.item_id(NOTED_CAKE_ID)
        + observation.item_id(NOTED_BREAD_ID)
        + observation.item_id(NOTED_CHOCOLATE_SLICE_ID)
        + observation.bank_item_id(NOTED_CAKE_ID)
        + observation.bank_item_id(NOTED_BREAD_ID)
        + observation.bank_item_id(NOTED_CHOCOLATE_SLICE_ID)
}

pub fn ardy_thiever_baseline_ready(baseline: &Observation, thieving: i32) -> bool {
    near(baseline.tile, ARDY_THIEVER_STAND, 8)
        && baseline.level("thieving") >= thieving
        && baseline.item_id(COINS_ID) == 0
        && baseline.item_id(CAKE_ID) == 0
}

/// Baker's stall Flee cell prep plus combat stats and a weapon so FightBack can kill.
pub fn ardy_cakes_fight_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, ARDY_CAKES_STAND, 6)
        && baseline.level("thieving") >= 5
        && baseline.level("attack") >= COMBAT_ATTACK_LEVEL
        && baseline.level("strength") >= COMBAT_ATTACK_LEVEL
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && baseline.item_id(KNIFE_ID) == ARDY_CAKES_BALLAST_KNIVES
        && held_id(baseline, ADAMANT_SCIMITAR_ID) >= 1
        && stall_food(baseline) == 0
        && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
        && noted_stall_food(baseline) == 0
}

/// Guard pickpocket Fight cell: same empty-pack thieving prep plus combat kit.
pub fn ardy_thiever_fight_baseline_ready(baseline: &Observation) -> bool {
    ardy_thiever_baseline_ready(baseline, 40)
        && baseline.level("attack") >= COMBAT_ATTACK_LEVEL
        && baseline.level("strength") >= COMBAT_ATTACK_LEVEL
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && held_id(baseline, ADAMANT_SCIMITAR_ID) >= 1
}


pub fn gnome_wrong_bows(observation: &Observation) -> bool {
    observation.item_id(MAGIC_SHORTBOW_ID) > 0
        || observation.bank_item_id(MAGIC_SHORTBOW_ID) > 0
        || observation.item_id(MAGIC_LONGBOW_ID) > 0
        || observation.bank_item_id(MAGIC_LONGBOW_ID) > 0
}

pub fn gnome_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_MAGIC_LOGS_ID) > 0
        || observation.bank_item_id(NOTED_MAGIC_LOGS_ID) > 0
        || observation.item_id(NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
        || observation.bank_item_id(NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
        || observation.item_id(NOTED_UNSTRUNG_MAGIC_LONGBOW_ID) > 0
        || observation.bank_item_id(NOTED_UNSTRUNG_MAGIC_LONGBOW_ID) > 0
}

pub fn gnome_resource_tools(observation: &Observation) -> bool {
    held_id(observation, RUNE_AXE_ID) == 1 && observation.item_id(KNIFE_ID) == GNOME_BALLAST_KNIVES
}

pub fn gnome_chop_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, GNOME_SOUTH_BANK_MAGIC_STAND, 1)
        && baseline.magic_tree_ready
        && baseline.level("woodcutting") >= 75
        && gnome_resource_tools(baseline)
        && baseline.item_id(MAGIC_LOGS_ID) == 0
        && baseline.item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) == 0
        && baseline.item_id(UNSTRUNG_MAGIC_LONGBOW_ID) == 0
        && !gnome_wrong_bows(baseline)
        && !gnome_noted(baseline)
}

pub fn gnome_fletch_baseline_ready(
    baseline: &Observation,
    fletching: i32,
    max: Option<i32>,
) -> bool {
    gnome_chop_baseline_ready(baseline)
        && baseline.level("fletching") >= fletching
        && max
            .map(|max| baseline.level("fletching") <= max)
            .unwrap_or(true)
}

pub fn coal_trucks_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, COAL_MINE, 8)
        && baseline.level("mining") >= 60
        && baseline.effective_level("mining") >= 60
        && baseline.item_id(RUNE_PICKAXE_ID) == 1
        && baseline.item_id(KNIFE_ID) == COAL_BALLAST_KNIVES
        && baseline.item_ids.values().copied().sum::<i32>() == 27
        && baseline.item_id(COAL_ID) == 0
        && baseline.item_id(NOTED_COAL_ID) == 0
        && baseline.bank_item_id(COAL_ID) == 0
}

pub fn cook_wrong_or_burnt(observation: &Observation, wrong: i32) -> bool {
    observation.item_id(wrong) > 0
        || observation.bank_item_id(wrong) > 0
        || observation.item_id(BURNT_FISH_1_ID) > 0
        || observation.bank_item_id(BURNT_FISH_1_ID) > 0
        || observation.item_id(BURNT_FISH_2_ID) > 0
        || observation.bank_item_id(BURNT_FISH_2_ID) > 0
        || observation.item_id(BURNT_LOBSTER_ID) > 0
        || observation.bank_item_id(BURNT_LOBSTER_ID) > 0
}

pub fn cook_noted(observation: &Observation, noted_raw: i32, noted_product: i32) -> bool {
    observation.item_id(noted_raw) > 0
        || observation.bank_item_id(noted_raw) > 0
        || observation.item_id(noted_product) > 0
        || observation.bank_item_id(noted_product) > 0
}

pub fn cook_bot_baseline_ready(
    baseline: &Observation,
    raw: i32,
    product: i32,
    wrong: i32,
    noted_raw: i32,
    noted_product: i32,
) -> bool {
    near(baseline.tile, CATHERBY_BANK, 6)
        && baseline.level("cooking") >= COOKING_FIXTURE_LEVEL
        && baseline.item_id(raw) == 0
        && baseline.item_id(product) == 0
        && !cook_wrong_or_burnt(baseline, wrong)
        && !cook_noted(baseline, noted_raw, noted_product)
}

pub fn smelter_noted(
    observation: &Observation,
    noted_primary: i32,
    noted_secondary: i32,
    noted_product: i32,
) -> bool {
    observation.item_id(noted_primary) > 0
        || observation.bank_item_id(noted_primary) > 0
        || observation.item_id(noted_secondary) > 0
        || observation.bank_item_id(noted_secondary) > 0
        || observation.item_id(noted_product) > 0
        || observation.bank_item_id(noted_product) > 0
}

pub fn smelter_bot_baseline_ready(
    baseline: &Observation,
    primary: i32,
    secondary: i32,
    product: i32,
    wrong: i32,
    smithing: i32,
) -> bool {
    near(baseline.tile, AL_KHARID_BANK, 6)
        && baseline.level("smithing") >= smithing
        && baseline.item_id(primary) == 0
        && baseline.item_id(secondary) == 0
        && baseline.item_id(product) == 0
        && baseline.item_id(wrong) == 0
        && baseline.item_id(IRON_BAR_ID) == 0
        && !smelter_noted(baseline, primary + 1, secondary + 1, product + 1)
}

pub fn flax_spinner_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_FLAX_ID) > 0
        || observation.bank_item_id(NOTED_FLAX_ID) > 0
        || observation.item_id(NOTED_BOW_STRING_ID) > 0
        || observation.bank_item_id(NOTED_BOW_STRING_ID) > 0
}

pub fn flax_spinner_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_SPINNER_BANK, 8)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
        && !flax_spinner_noted(baseline)
}



pub fn superheater_baseline_ready(
    baseline: &Observation,
    bar: i32,
    primary: i32,
    secondary: i32,
    staff: i32,
    smithing: i32,
) -> bool {
    near(baseline.tile, (3185, 3440, 0), 6)
        && baseline.level("magic") >= 43
        && baseline.level("smithing") >= smithing
        && baseline.item_id(bar) == 0
        && baseline.item_id(primary) == 0
        && baseline.item_id(secondary) == 0
        && baseline.item_id(NATURE_RUNE_ID) == 0
        && baseline.item_id(staff) == 0
        && baseline.item_id(IRON_BAR_ID) == 0
        && baseline.equipment_id(staff) == 0
}

pub fn in_temple(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| tile.1 > TEMPLE_Z)
}

pub fn overworld(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| tile.1 <= TEMPLE_Z)
}

pub fn runecraft_baseline_ready(
    baseline: &Observation,
    bank: (i32, i32, i32),
    rune: i32,
    wrong_rune: i32,
    talisman: i32,
    rc_level: i32,
) -> bool {
    near(baseline.tile, bank, 6)
        && baseline.level("runecraft") >= rc_level
        && baseline.item_id(RUNE_ESSENCE_ID) == 0
        && baseline.item_id(NOTED_ESSENCE_ID) == 0
        && baseline.item_id(rune) == 0
        && baseline.item_id(wrong_rune) == 0
        && baseline.item_id(talisman) == 0
}

pub fn validate_case_baseline(case: CoreCase, baseline: &Observation) -> Result<(), String> {
    validate_case_baseline_with_preparation(case, baseline, None)
}

pub fn validate_case_baseline_with_preparation(
    case: CoreCase,
    baseline: &Observation,
    start_preparation: Option<&HerbCleanerStartPreparationReceipt>,
) -> Result<(), String> {
    let ready = match case {
        CoreCase::BoneBurier => {
            near(baseline.tile, (3220, 3212, 0), 8) && baseline.item("Bones") >= 5
        }
        CoreCase::ChickenKiller => near(baseline.tile, (3235, 3295, 0), 8),
        CoreCase::ChickenKillerBank => {
            near(baseline.tile, FALADOR_CHICKENS, 8)
                && baseline.level("attack") >= 30
                && baseline.level("strength") >= 30
                && baseline.item_id(FEATHER_ID) == 0
        }
        CoreCase::Thiever => {
            near(baseline.tile, (2661, 3306, 0), 10)
                && baseline.item("Lobster") >= 10
                && baseline.level("thieving") >= 50
                && baseline.level("hitpoints") >= 50
        }
        CoreCase::Alcher
        | CoreCase::AlcherCustom
        | CoreCase::AlcherOrdered
        | CoreCase::AlcherLargeBatch => {
            near(baseline.tile, (3185, 3440, 0), 6) && baseline.level("magic") >= 55
        }
        CoreCase::AlcherLow => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.level("magic") >= LOW_ALCH_LEVEL
                && baseline.item_id(RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(CERT_RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(STAFF_OF_FIRE_ID) == 0
                && baseline.item_id(FIRE_BATTLESTAFF_ID) == 0
                && baseline.equipment_id(STAFF_OF_FIRE_ID) == 0
                && baseline.equipment_id(FIRE_BATTLESTAFF_ID) == 0
        }
        CoreCase::AlcherFireBattlestaff => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.level("magic") >= HIGH_ALCH_LEVEL
                && baseline.level("attack") >= FIRE_BATTLESTAFF_WIELD_ATTACK
                && baseline.item_id(RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(CERT_RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(STAFF_OF_FIRE_ID) == 0
                && baseline.item_id(FIRE_BATTLESTAFF_ID) == 0
                && baseline.equipment_id(STAFF_OF_FIRE_ID) == 0
                && baseline.equipment_id(FIRE_BATTLESTAFF_ID) == 0
        }
        CoreCase::AlcherSwarmDrain => {
            near(baseline.tile, VARROCK_WEST_BANK, 6)
                && baseline.level("magic") >= HIGH_ALCH_LEVEL
                && baseline.item_id(RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(CERT_RUNE_CHAINBODY_ID) == 0
                && baseline.item_id(YEW_LONGBOW_ID) == 0
                && baseline.item_id(CERT_YEW_LONGBOW_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(STAFF_OF_FIRE_ID) == 0
                && baseline.item_id(FIRE_BATTLESTAFF_ID) == 0
                && baseline.equipment_id(STAFF_OF_FIRE_ID) == 0
                && baseline.equipment_id(FIRE_BATTLESTAFF_ID) == 0
        }
        CoreCase::AlcherDefaults => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.level("magic") >= 55
                && baseline.item_id(YEW_LONGBOW_ID) == 0
                && baseline.item_id(CERT_YEW_LONGBOW_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item("Rune chainbody") == 0
        }
        CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.level("magic") >= 55
                && baseline.item_id(ADAMANT_SCIMITAR_ID) == 0
                && baseline.item_id(CERT_ADAMANT_SCIMITAR_ID) == 0
                && baseline.item_id(COINS_ID) == 0
                && baseline.item_id(NATURE_RUNE_ID) == 0
                && baseline.item("Coins") == 0
                && baseline.item("Rune chainbody") == 0
        }
        CoreCase::BankFletcher => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item("Knife") >= 1
                && baseline.item("Willow logs") >= 27
                && baseline.level("fletching") >= 35
        }
        CoreCase::BankFletcherShafts => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && (held_id(baseline, KNIFE_ID) == 1 || baseline.item("Knife") == 1)
                && baseline.item_id(LOGS_ID) == 27
                && baseline.item_id(ARROW_SHAFT_ID) == 0
                && baseline.level("fletching") >= 1
        }
        CoreCase::BankFletcherHeadless => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(FEATHER_ID) == 30
                && baseline.item_id(ARROW_SHAFT_ID) == 30
                && baseline.item_id(HEADLESS_ARROW_ID) == 0
                && baseline.level("fletching") >= 1
        }
        CoreCase::BankFletcherString => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(60) == 2
                && baseline.item_id(1777) == 2
                && baseline.item_id(849) == 0
                && baseline.level("fletching") >= 35
        }
        CoreCase::BankFletcherCutString => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item("Knife") == 1
                && baseline.item_id(1519) == 2
                && baseline.item_id(60) == 0
                && baseline.item_id(849) == 0
                && baseline.item_id(1777) == 0
                && baseline.level("fletching") >= 35
        }
        CoreCase::DartFletcher => {
            near(baseline.tile, (3220, 3212, 0), 8)
                && baseline.item_id(BRONZE_DART_TIP_ID) == 100
                && baseline.item_id(FEATHER_ID) == 100
                && baseline.item_id(BRONZE_DART_ID) == 0
                && baseline.item_id(IRON_DART_ID) == 0
                && baseline.level("fletching") >= 1
        }
        CoreCase::DartFletcherIron => {
            near(baseline.tile, (3220, 3212, 0), 8)
                && baseline.item_id(IRON_DART_TIP_ID) == 100
                && baseline.item_id(FEATHER_ID) == 100
                && baseline.item_id(IRON_DART_ID) == 0
                && baseline.item_id(BRONZE_DART_ID) == 0
                && baseline.level("fletching") >= 22
        }
        CoreCase::HerbCleaner => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(UNIDENTIFIED_GUAM_ID) == 0
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.level("herblore") >= 3
        }
        CoreCase::HerbCleanerNamed => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(UNIDENTIFIED_GUAM_ID) == 0
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.item_id(UNIDENTIFIED_MARENTILL_ID) == 0
                && baseline.level("herblore") >= 5
        }
        CoreCase::HerbCleanerEmptyBank => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(UNIDENTIFIED_GUAM_ID) == 0
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.item_id(UNIDENTIFIED_MARENTILL_ID) == 0
                && baseline.item_id(MARRENTILL_ID) == 0
                && baseline.level("herblore") >= 20
                && match start_preparation {
                    Some(receipt) => {
                        !baseline.bank_open
                            && !baseline.bank_loaded
                            && baseline.bank.is_empty()
                            && baseline.bank_ids.is_empty()
                            && baseline
                                .player
                                .as_deref()
                                .is_some_and(|player| player.eq_ignore_ascii_case(&receipt.player))
                            && receipt.guam_count == 20
                            && receipt.marrentill_count == 0
                            && baseline.bank_generation > receipt.bank_generation
                            && baseline.tick >= receipt.tick
                    }
                    None => {
                        baseline.bank_open
                            && baseline.bank_loaded
                            && baseline.bank_item_id(UNIDENTIFIED_GUAM_ID) == 20
                            && baseline.bank_item_id(UNIDENTIFIED_MARENTILL_ID) == 0
                    }
                }
        }
        CoreCase::GemCutter => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(CHISEL_ID) == 0
                && baseline.item_id(UNCUT_SAPPHIRE_ID) == 0
                && baseline.item_id(SAPPHIRE_ID) == 0
                && baseline.item_id(CRUSHED_GEMSTONE_ID) == 0
                && baseline.level("crafting") >= 20
        }
        CoreCase::GemCutterNamed => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(CHISEL_ID) == 0
                && baseline.item_id(UNCUT_SAPPHIRE_ID) == 0
                && baseline.item_id(SAPPHIRE_ID) == 0
                && baseline.item_id(UNCUT_OPAL_ID) == 0
                && baseline.item_id(CRUSHED_GEMSTONE_ID) == 0
                && baseline.level("crafting") >= 20
        }
        CoreCase::DoorOpener => {
            near(baseline.tile, LUMBRIDGE_DOOR_STAND, 1)
                && loc_at(baseline, WOODEN_DOOR_CLOSED_ID, LUMBRIDGE_DOOR, 1)
                    .is_some_and(|loc| loc.open)
        }
        CoreCase::DoorOpenerGate => {
            near(baseline.tile, LUMBRIDGE_GATE_STAND, 1)
                && loc_at(baseline, WOODEN_GATE_CLOSED_ID, LUMBRIDGE_GATE, 1)
                    .is_some_and(|loc| loc.open)
        }
        CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius => near(baseline.tile, GNOME_START, 2),
        CoreCase::WildyAgility => {
            near(baseline.tile, WILDY_START, 2)
                && baseline.level("agility") >= 52
                && baseline.level("hitpoints") >= 40
                && baseline.effective_level("hitpoints") >= 40
                && baseline.item_id(LOBSTER_ID) >= 5
        }
        CoreCase::BrimhavenAgility => {
            near(baseline.tile, BRIMHAVEN_START, 2)
                && baseline.level("agility") >= 52
                && baseline.effective_level("agility") >= 52
                && baseline.item_id(COINS_ID) >= 200
                && baseline.item_id(LOBSTER_ID) >= 10
                && baseline.item_id(BRIMHAVEN_TICKET_ID) == 0
                && baseline.varp(BRIMHAVEN_ARENA_VARP) & 0b10 == 0
        }
        CoreCase::FlaxPicker => {
            near(baseline.tile, FLAX_FIELD, 6) && baseline.item_id(FLAX_ID) == 0
        }
        CoreCase::Superheater => superheater_baseline_ready(
            baseline,
            BRONZE_BAR_ID,
            COPPER_ORE_ID,
            TIN_ORE_ID,
            STAFF_OF_FIRE_ID,
            1,
        ),
        CoreCase::SuperheaterSteel => superheater_baseline_ready(
            baseline,
            STEEL_BAR_ID,
            IRON_ORE_ID,
            COAL_ID,
            STAFF_OF_FIRE_ID,
            30,
        ),
        CoreCase::SuperheaterFireBattlestaff => {
            superheater_baseline_ready(
                baseline,
                BRONZE_BAR_ID,
                COPPER_ORE_ID,
                TIN_ORE_ID,
                FIRE_BATTLESTAFF_ID,
                1,
            ) && baseline.item_id(STAFF_OF_FIRE_ID) == 0
                && baseline.equipment_id(STAFF_OF_FIRE_ID) == 0
                && baseline.level("attack") >= 30
        }
        CoreCase::SuperheaterSilverLowNatures => superheater_baseline_ready(
            baseline,
            SILVER_BAR_ID,
            SILVER_ORE_ID,
            SILVER_ORE_ID,
            STAFF_OF_FIRE_ID,
            20,
        ),
        CoreCase::VialFiller => {
            near(baseline.tile, FALADOR_WEST_BANK, 6)
                && baseline.item_id(EMPTY_VIAL_ID) == 0
                && baseline.item_id(VIAL_OF_WATER_ID) == 0
        }
        CoreCase::VialFillerEast => {
            near(baseline.tile, FALADOR_EAST_BANK, 6)
                && baseline.item_id(EMPTY_VIAL_ID) == 0
                && baseline.item_id(VIAL_OF_WATER_ID) == 0
        }
        CoreCase::PotionMaker => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.item_id(VIAL_OF_WATER_ID) == 0
                && baseline.item_id(EYE_OF_NEWT_ID) == 0
                && baseline.item_id(GUAM_UNF_ID) == 0
                && baseline.item_id(ATTACK_POTION_3_ID) == 0
                && baseline.level("herblore") >= 3
        }
        CoreCase::PotionMakerNamed => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item_id(RANARR_WEED_ID) == 0
                && baseline.item_id(VIAL_OF_WATER_ID) == 0
                && baseline.item_id(SNAPE_GRASS_ID) == 0
                && baseline.item_id(RANARR_UNF_ID) == 0
                && baseline.item_id(PRAYER_POTION_3_ID) == 0
                && baseline.item_id(GUAM_LEAF_ID) == 0
                && baseline.level("herblore") >= 38
        }
        CoreCase::TannerBot | CoreCase::TannerBotHard => {
            near(baseline.tile, AL_KHARID_BANK, 6)
                && baseline.item_id(COW_HIDE_ID) == 0
                && baseline.item_id(SOFT_LEATHER_ID) == 0
                && baseline.item_id(HARD_LEATHER_ID) == 0
                && baseline.item_id(COINS_ID) == 0
        }
        CoreCase::RuneCrafter => runecraft_baseline_ready(
            baseline,
            FALADOR_EAST_BANK,
            AIR_RUNE_ID,
            EARTH_RUNE_ID,
            AIR_TALISMAN_ID,
            1,
        ),
        CoreCase::RuneCrafterEarth => runecraft_baseline_ready(
            baseline,
            VARROCK_EAST_BANK,
            EARTH_RUNE_ID,
            AIR_RUNE_ID,
            EARTH_TALISMAN_ID,
            9,
        ),
        CoreCase::MuleCrafter => runecraft_baseline_ready(
            baseline,
            FALADOR_EAST_BANK,
            AIR_RUNE_ID,
            EARTH_RUNE_ID,
            AIR_TALISMAN_ID,
            1,
        ),
        CoreCase::ArdyCakes => {
            near(baseline.tile, ARDY_CAKES_STAND, 6)
                && baseline.level("thieving") >= 5
                && baseline.item_id(KNIFE_ID) == ARDY_CAKES_BALLAST_KNIVES
                && baseline.item_ids.values().copied().sum::<i32>() == ARDY_CAKES_BALLAST_KNIVES
                && stall_food(baseline) == 0
                && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
                && noted_stall_food(baseline) == 0
        }
        CoreCase::ArdyCakesFight => ardy_cakes_fight_baseline_ready(baseline),
        CoreCase::ArdyThiever => ardy_thiever_baseline_ready(baseline, 40),
        CoreCase::ArdyThieverFight => ardy_thiever_fight_baseline_ready(baseline),
        CoreCase::ArdyThieverKnight => ardy_thiever_baseline_ready(baseline, 55),
        CoreCase::GnomeChop => gnome_chop_baseline_ready(baseline),
        CoreCase::GnomeFletchShort => gnome_fletch_baseline_ready(baseline, 80, Some(84)),
        CoreCase::GnomeFletchLong => gnome_fletch_baseline_ready(baseline, 85, None),
        CoreCase::CoalTrucks => coal_trucks_baseline_ready(baseline) && baseline.combat_level >= 55,
        CoreCase::CookBot => cook_bot_baseline_ready(
            baseline,
            RAW_SALMON_ID,
            SALMON_ID,
            LOBSTER_ID,
            NOTED_RAW_SALMON_ID,
            NOTED_SALMON_ID,
        ),
        CoreCase::CookBotLobster => cook_bot_baseline_ready(
            baseline,
            RAW_LOBSTER_ID,
            LOBSTER_ID,
            SALMON_ID,
            NOTED_RAW_LOBSTER_ID,
            NOTED_LOBSTER_ID,
        ),
        CoreCase::SmelterBot => smelter_bot_baseline_ready(
            baseline,
            COPPER_ORE_ID,
            TIN_ORE_ID,
            BRONZE_BAR_ID,
            STEEL_BAR_ID,
            1,
        ),
        CoreCase::SmelterBotSteel => smelter_bot_baseline_ready(
            baseline,
            IRON_ORE_ID,
            COAL_ID,
            STEEL_BAR_ID,
            BRONZE_BAR_ID,
            30,
        ),
        CoreCase::FlaxSpinner => flax_spinner_baseline_ready(baseline),
        CoreCase::FlaxAio => flax_aio_baseline_ready(baseline),
        CoreCase::FlaxAioPick => flax_aio_pick_baseline_ready(baseline),
        CoreCase::FlaxAioSpin => flax_aio_spin_baseline_ready(baseline),
        CoreCase::HerbloreSecondaries => herblore_eggs_baseline_ready(baseline),
        CoreCase::HerbloreSecondariesNewt => herblore_newt_baseline_ready(baseline),
        // Bank-only dart Start is empty pack/worn. Do not route this through
        // combat_baseline_ready(Ranged), which requires worn weapon+projectile.
        CoreCase::MossGiantDart => moss_giant_dart_baseline_ready(baseline),
        CoreCase::ChaosDruid
        | CoreCase::ChaosDruidTower
        | CoreCase::ChaosDruidYanille
        | CoreCase::MossGiant
        | CoreCase::MossGiantPrepared
        | CoreCase::HillGiant
        | CoreCase::AutoFighter
        | CoreCase::AutoFighterMage
        | CoreCase::AutoFighterRange
        | CoreCase::RockCrab
        | CoreCase::RockCrabRange
        | CoreCase::GreenDragon
        | CoreCase::GreenDragonPrepared
        | CoreCase::GreenDragonMagePrepared
        | CoreCase::GreenDragonSpecial
        | CoreCase::GreenDragonSpecialPrepared
        | CoreCase::GreenDragonPotions
        | CoreCase::GreenDragonPotionsPrepared
        | CoreCase::FireGiant
        | CoreCase::FireGiantPrepared
        | CoreCase::ArdyFighter
        | CoreCase::AutoFighterBank
        | CoreCase::MossGiantBank
        | CoreCase::HillGiantBank
        | CoreCase::HillGiantBankPrepared
        | CoreCase::ChaosDruidBank
        | CoreCase::ArdyFighterBank
        | CoreCase::RockCrabBank
        | CoreCase::GreenDragonBank
        | CoreCase::GreenDragonBankPrepared
        | CoreCase::GreenDragonBankDefaultPrepared
        | CoreCase::GreenDragonTele
        | CoreCase::GreenDragonTelePrepared
        | CoreCase::FireGiantApproach
        | CoreCase::FireGiantBank
        | CoreCase::FireGiantBankPrepared
        | CoreCase::FireGiantCamelotPrepared => combat_spec(case).is_some_and(|spec| {
            combat_baseline_ready(baseline, spec)
                && match case {
                    CoreCase::ChaosDruidTower => baseline.level("thieving") >= 46,
                    CoreCase::ChaosDruidYanille => baseline.level("agility") >= 40,
                    // Bank cells start from a wielded melee weapon: every one of
                    // these cards deposits (or is told to deposit) the pack, so a
                    // carried weapon would be stashed instead of used. Each
                    // card's loot class is refused by `combat_baseline_ready` itself
                    // (`combat_loot_count == 0`): the cell's deposit class must
                    // not be seeded before Start.
                    CoreCase::AutoFighterBank => baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1,
                    CoreCase::MossGiantBank => {
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.item_id(LOBSTER_ID) == MOSS_GIANT_BANK_FOOD
                    }
                    CoreCase::HillGiantBank => {
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.item_id(TROUT_ID) == HILL_GIANT_FOOD
                    }
                    CoreCase::ChaosDruidBank => {
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.item_id(LOBSTER_ID) == CHAOS_DRUID_BANK_FOOD
                    }
                    CoreCase::ArdyFighterBank => {
                        // Guard-drop emptiness is `combat_baseline_ready`'s
                        // `CombatLoot::GuardDrop` count == 0.
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.item_id(CAKE_ID) == 0
                            && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
                    }
                    CoreCase::RockCrabBank => {
                        baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                            && baseline.dormant_rocks_seen
                    }
                    // The melee RockCrab core fights with the scoped weapon:
                    // the frozen card's own GearEquip refuses the carried
                    // fixture, so the cell has to arrive already wearing 1331
                    // (the fixture's own pre-Start wear is the native proof).
                    CoreCase::RockCrab => baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1,
                    CoreCase::GreenDragonPrepared
                    | CoreCase::GreenDragonSpecialPrepared
                    | CoreCase::GreenDragonPotionsPrepared
                    | CoreCase::GreenDragonBankPrepared
                    | CoreCase::GreenDragonBankDefaultPrepared
                    | CoreCase::GreenDragonTelePrepared
                    | CoreCase::FireGiantPrepared
                    | CoreCase::FireGiantBankPrepared
                    | CoreCase::FireGiantCamelotPrepared
                    | CoreCase::MossGiantPrepared
                    | CoreCase::HillGiantBankPrepared
                    | CoreCase::GreenDragonMagePrepared => {
                        prepared_combat_baseline_ready(case, baseline)
                    }
                    CoreCase::GreenDragonBank => baseline.equipment_id(RUNE_SCIMITAR_ID) == 1,
                    CoreCase::GreenDragonTele => {
                        baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
                            && baseline.level("magic") >= VARROCK_TELE_MAGIC
                            && baseline.item_id(LAW_RUNE_ID) >= 1
                            && baseline.item_id(AIR_RUNE_ID) >= 3
                            && baseline.item_id(FIRE_RUNE_ID) >= 1
                    }
                    CoreCase::FireGiantBank => baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1,
                    _ => true,
                }
        }),
        CoreCase::AioTeleport => {
            near(baseline.tile, LUMBRIDGE_BANK, 8)
                && baseline.level("magic") >= VARROCK_TELE_MAGIC
                && baseline.equipment_id(STAFF_OF_AIR_ID) == 1
                && baseline.item_id(LAW_RUNE_ID) >= AIO_LAW_PACK
                && baseline.item_id(FIRE_RUNE_ID) >= 1
                && !near(baseline.tile, VARROCK_TELE_LAND, 8)
        }
        CoreCase::AioTeleportFalador => {
            near(baseline.tile, LUMBRIDGE_BANK, 8)
                && baseline.level("magic") >= FALADOR_TELE_MAGIC
                && baseline.equipment_id(STAFF_OF_WATER_ID) == 1
                && baseline.item_id(LAW_RUNE_ID) >= AIO_LAW_PACK
                && baseline.item_id(AIR_RUNE_ID) >= 3
                && !near(baseline.tile, FALADOR_TELE_LAND, 8)
        }
        CoreCase::AioTeleportNoStaff => {
            near(baseline.tile, LUMBRIDGE_BANK, 8)
                && baseline.level("magic") >= VARROCK_TELE_MAGIC
                && baseline.equipment_id(STAFF_OF_AIR_ID) == 0
                && baseline.equipment_id(STAFF_OF_WATER_ID) == 0
                && baseline.item_id(LAW_RUNE_ID) >= AIO_LAW_PACK
                && baseline.item_id(AIR_RUNE_ID) >= 3
                && baseline.item_id(FIRE_RUNE_ID) >= 1
                && !near(baseline.tile, VARROCK_TELE_LAND, 8)
        }
        CoreCase::ShopBuyout => {
            near(baseline.tile, AEMAD_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(EMPTY_VIAL_ID) == 0
        }
        CoreCase::ShopBuyoutAubury => {
            near(baseline.tile, AUBURY_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(AIR_RUNE_ID) == 0
        }
        CoreCase::ShopBuyoutLowe => {
            near(baseline.tile, LOWE_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(BRONZE_ARROW_ID) == 0
        }
        CoreCase::ShopBuyoutHickton => {
            near(baseline.tile, HICKTON_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(BRONZE_ARROW_ID) == 0
        }
        CoreCase::ShopBuyoutHarry => {
            near(baseline.tile, HARRY_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(FISHING_BAIT_ID) == 0
        }
        CoreCase::ShopBuyoutBetty => {
            near(baseline.tile, BETTY_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(FIRE_RUNE_ID) == 0
        }
        CoreCase::ShopBuyoutGerrant => {
            near(baseline.tile, GERRANT_STAND, 6)
                && empty_pack(baseline)
                && baseline.item_id(FEATHER_ID) == 0
        }
        CoreCase::SmithingBot => {
            near(baseline.tile, VARROCK_WEST_BANK, 8)
                && baseline.level("smithing") >= 1
                && empty_pack(baseline)
                && baseline.item_id(BRONZE_DAGGER_ID) == 0
                && baseline.item_id(BRONZE_BAR_ID) == 0
        }
        CoreCase::SmithingBotPlatebody => {
            near(baseline.tile, VARROCK_WEST_BANK, 8)
                && baseline.level("smithing") >= BRONZE_PLATEBODY_SMITHING
                && empty_pack(baseline)
                && baseline.item_id(BRONZE_PLATEBODY_ID) == 0
                && baseline.item_id(BRONZE_DAGGER_ID) == 0
                && baseline.item_id(BRONZE_BAR_ID) == 0
        }
        CoreCase::LeatherCrafter => {
            near(baseline.tile, AL_KHARID_BANK, 8)
                && baseline.level("crafting") >= 1
                && empty_pack(baseline)
                && baseline.item_id(LEATHER_GLOVES_ID) == 0
                && baseline.item_id(SOFT_LEATHER_ID) == 0
        }
        CoreCase::LeatherCrafterHardBody => {
            near(baseline.tile, AL_KHARID_BANK, 8)
                && baseline.level("crafting") >= HARD_LEATHER_CRAFTING
                && empty_pack(baseline)
                && baseline.item_id(HARDLEATHER_BODY_ID) == 0
                && baseline.item_id(HARD_LEATHER_ID) == 0
                && baseline.item_id(LEATHER_GLOVES_ID) == 0
        }
        CoreCase::Firemaker => {
            near(baseline.tile, VARROCK_EAST_BANK, 8)
                && baseline.level("firemaking") >= 1
                && empty_pack(baseline)
                && baseline.item_id(LOGS_ID) == 0
                && !fire_in_varrock_east_plot(baseline)
        }
        CoreCase::FiremakerOak => {
            near(baseline.tile, VARROCK_EAST_BANK, 8)
                && baseline.level("firemaking") >= OAK_FIREMAKING
                && empty_pack(baseline)
                && baseline.item_id(OAK_LOGS_ID) == 0
                && baseline.item_id(LOGS_ID) == 0
                && !fire_in_varrock_east_plot(baseline)
        }
        CoreCase::ClimbingBoots => {
            near(baseline.tile, TENZING_HUT_DOOR, 12)
                && baseline.item_id(CLIMBING_BOOTS_ID) == 0
                && baseline.bank_item_id(CLIMBING_BOOTS_ID) == 0
                && baseline.item_id(COINS_ID) == CLIMBING_BOOTS_WALK_PACK_COINS
                && baseline.item_id(LAW_RUNE_ID) == 0
                && baseline.item_id(AIR_RUNE_ID) == 0
                && baseline.item_id(WATER_RUNE_ID) == 0
        }
        CoreCase::ClimbingBootsTeleport => {
            near(baseline.tile, TENZING_HUT_DOOR, 12)
                && baseline.item_id(CLIMBING_BOOTS_ID) == 0
                && baseline.bank_item_id(CLIMBING_BOOTS_ID) == 0
                && baseline.item_id(COINS_ID) == CLIMBING_BOOTS_TELE_PACK_COINS
                && baseline.item_id(LAW_RUNE_ID) == CLIMBING_BOOTS_RUNE_STOCK_MIN
                && baseline.item_id(AIR_RUNE_ID) == 3
                && baseline.item_id(WATER_RUNE_ID) == 1
                && baseline.level("magic") >= FALADOR_TELE_MAGIC
        }
        CoreCase::RangingGuildRound => ranging_guild_round_baseline_ready(baseline),
        CoreCase::RangingGuildRedeem => ranging_guild_redeem_baseline_ready(baseline),
        CoreCase::RangingGuildBank => ranging_guild_bank_baseline_ready(baseline),
        CoreCase::RangingGuildFull => ranging_guild_full_baseline_ready(baseline),
        CoreCase::BrimhavenMossInspectV1 => brimhaven_moss_inspect_v1_baseline_ready(baseline),
        CoreCase::RouteInspectBrimhavenV2 => route_inspect_brimhaven_v2_baseline_ready(baseline),
        CoreCase::PrayerV2 | CoreCase::PrayerV1 => prayer_delivery_baseline_ready(baseline),
        CoreCase::LineOfSightV2 => line_of_sight_baseline_ready(baseline),
        CoreCase::ActorObservationV2 => actor_observation_baseline_ready(baseline),
        CoreCase::FightFieldV2 => fight_field_baseline_ready(baseline),
        CoreCase::HoldSpotV2
        | CoreCase::RetreatSpotV2
        | CoreCase::WalkSpotV2
        | CoreCase::EnterLairV2
        | CoreCase::LeaveLairV2
        | CoreCase::AcquireKeyV2
        | CoreCase::CellV2
        | CoreCase::BankV2 => case
            .hunt_cell()
            .is_some_and(|cell| hunt_baseline_ready(cell, baseline)),
    };
    if ready {
        return Ok(());
    }
    let requirement = match case {
        CoreCase::BoneBurier => "Lumbridge mainland and five Bones",
        CoreCase::ChickenKiller => "Lumbridge chicken pen",
        CoreCase::ChickenKillerBank => {
            "Falador south chickens (3029,3294,0), Attack/Strength 30 and no Feather 314"
        }
        CoreCase::Thiever => "Ardougne guard stand, ten Lobsters, and prepared stats",
        CoreCase::Alcher | CoreCase::AlcherCustom
        | CoreCase::AlcherOrdered
        | CoreCase::AlcherLargeBatch => "Varrock West bank and Magic 55",
        CoreCase::AlcherLow => {
            "Varrock West bank, Magic 21+, and no seeded chainbody note, coins, Nature rune, or staff"
        }
        CoreCase::AlcherFireBattlestaff => {
            "Varrock West bank, Magic 55+ and Attack 30+, no staff worn or seeded, and no seeded \
             chainbody note, coins, or Nature rune"
        }
        CoreCase::AlcherSwarmDrain => {
            "Varrock West bank, Magic 55+, and no seeded chainbody/yew-longbow note, coins, Nature rune, or staff"
        }
        CoreCase::AlcherDefaults => {
            "Varrock West bank, Magic 55, and no seeded yew-longbow note, coins, Nature rune, or chainbody"
        }
        CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName => {
            "Varrock West bank, Magic 55, and no seeded coins/custom-target outcome"
        }
        CoreCase::BankFletcher => {
            "Varrock West bank, Knife, twenty-seven Willow logs, and Fletching 35"
        }
        CoreCase::BankFletcherShafts => {
            "Varrock West bank, Knife, exact id 1511x27, no id 52, and Fletching 1"
        }
        CoreCase::BankFletcherHeadless => {
            "Varrock West bank, exact ids 314x30 and 52x30, no id 53, and Fletching 1"
        }
        CoreCase::BankFletcherString => {
            "Varrock West bank, exact ids 60x2 and 1777x2, no id 849, and Fletching 35"
        }
        CoreCase::BankFletcherCutString => {
            "Varrock West bank, Knife, exact id 1519x2, no bow/string ids, and Fletching 35"
        }
        CoreCase::DartFletcher => {
            "Lumbridge courtyard, exact ids 819x100 and 314x100, no darts, and Fletching 1"
        }
        CoreCase::DartFletcherIron => {
            "Lumbridge courtyard, exact ids 820x100 and 314x100, no darts, and Fletching 22"
        }
        CoreCase::HerbCleaner => "Varrock West bank, empty pack of 199/249, and Herblore 3",
        CoreCase::HerbCleanerNamed => {
            "Varrock West bank, empty pack of 199/201/249, and Herblore 5"
        }
        CoreCase::HerbCleanerEmptyBank => {
            "Varrock West bank, Herblore 20, empty pack of 199/201/249/251, and this run's loaded bank id 199x20/id 201x0 acknowledged before a closed-bank Start"
        }
        CoreCase::GemCutter => {
            "Varrock West bank, empty pack of 1755/1623/1607/1633, and Crafting 20"
        }
        CoreCase::GemCutterNamed => {
            "Varrock West bank, empty pack of 1755/1623/1625/1607/1633, and Crafting 20"
        }
        CoreCase::DoorOpener => {
            "adjacent stand (3208,3212,0) and shut wooden door 1530 offering Open"
        }
        CoreCase::DoorOpenerGate => {
            "adjacent stand (3213,3260,0) and shut wooden gate 1551 offering Open"
        }
        CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius => {
            "Gnome Stronghold start (2474,3436,0)"
        }
        CoreCase::WildyAgility => {
            "south ridge stand (2998,3916,0), Agility 52, base/effective Hitpoints 40, and five Lobsters 379"
        }
        CoreCase::BrimhavenAgility => {
            "surface entrance (2809,3194,0), Agility 52, at least 200 coins, ten Lobsters 379, no ticket 2996, and unpaid varp 309"
        }
        CoreCase::FlaxPicker => "Seers flax field (2741,3444,0) with empty pack of 1779",
        CoreCase::Superheater => {
            "Varrock West bank, Magic 43, Smithing 1, empty pack of 436/438/2349/561/1387"
        }
        CoreCase::SuperheaterSteel => {
            "Varrock West bank, Magic 43, Smithing 30, empty pack of 440/453/2353/561/1387"
        }
        CoreCase::SuperheaterFireBattlestaff => {
            "Varrock West bank, Magic 43, Attack 30, empty pack of 1393 and no 1387"
        }
        CoreCase::SuperheaterSilverLowNatures => {
            "Varrock West bank, Magic 43, Smithing 20, empty pack of 442/2355/561/1387"
        }
        CoreCase::VialFiller => "Falador West bank (2946,3369,0) and empty pack of 229/227",
        CoreCase::VialFillerEast => "Falador East bank (3013,3355,0) and empty pack of 229/227",
        CoreCase::PotionMaker => "Varrock West bank, Herblore 3, empty pack of 249/227/221/91/121",
        CoreCase::PotionMakerNamed => {
            "Varrock West bank, Herblore 38, empty pack of 257/227/231/99/139/249"
        }
        CoreCase::TannerBot | CoreCase::TannerBotHard => {
            "Al-Kharid bank (3269,3167,0) and empty pack of 1739/1741/1743/995"
        }
        CoreCase::RuneCrafter => {
            "Falador East bank (3013,3355,0), Runecraft 1, and empty pack of 1436/1437/556/557"
        }
        CoreCase::RuneCrafterEarth => {
            "Varrock East bank (3253,3420,0), Runecraft 9, and empty pack of 1436/1437/557/556"
        }
        CoreCase::MuleCrafter => {
            "Falador East bank (3013,3355,0), Runecraft 1, blank partner, and empty pack of 1436/1437/556/557"
        }
        CoreCase::ArdyCakes => {
            "Baker's stall stand (2668,3312,0), Thieving 5, exactly 22 Knives 946, and no 1891/2309/1901/1897"
        }
        CoreCase::ArdyCakesFight => {
            "Baker's stall stand (2668,3312,0), Thieving 5, Attack/Strength/Hitpoints 40, scimitar 1331, 22 Knives 946, and no stall food"
        }
        CoreCase::ArdyThiever => {
            "Ardougne Guard stand (2661,3306,0), Thieving 40, and empty pack of 995/1891"
        }
        CoreCase::ArdyThieverFight => {
            "Ardougne Guard stand (2661,3306,0), Thieving 40, Attack/Strength/Hitpoints 40, scimitar 1331, and empty pack of 995/1891"
        }
        CoreCase::ArdyThieverKnight => {
            "Ardougne Knight stand (2661,3306,0), Thieving 55, and empty pack of 995/1891"
        }
        CoreCase::GnomeChop => {
            "south-bank Magic tree stand (2433,3409,0), exact Chop-down tree 1306 at (2432,3410,0), Woodcutting 75, one Rune axe 1359, exactly 26 Knives 946, and no seeded 1513/72/70"
        }
        CoreCase::GnomeFletchShort => {
            "south-bank Magic tree, Woodcutting 75, Fletching 80-84, one Rune axe 1359, exactly 26 Knives 946, and no seeded 1513/72"
        }
        CoreCase::GnomeFletchLong => {
            "south-bank Magic tree, Woodcutting 75, Fletching 85, one Rune axe 1359, exactly 26 Knives 946, and no seeded 1513/70"
        }
        CoreCase::CoalTrucks => {
            "coal mine (2582,3481,0), native combat level >=55, base/effective Mining 60, Rune pickaxe 1275, exactly 26 nonproduct Knives 946 (one free slot), and zero 453/454"
        }
        CoreCase::CookBot => {
            "Catherby bank (2809,3441,0), Cooking 80, empty pack of 331/329"
        }
        CoreCase::CookBotLobster => {
            "Catherby bank (2809,3441,0), Cooking 80, empty pack of 377/379"
        }
        CoreCase::SmelterBot => {
            "Al-Kharid bank (3269,3167,0), Smithing 1, empty pack of 436/438/2349"
        }
        CoreCase::SmelterBotSteel => {
            "Al-Kharid bank (3269,3167,0), Smithing 30, empty pack of 440/453/2353"
        }
        CoreCase::FlaxSpinner => {
            "Seers flax bank (2722,3493,0), Crafting 1, empty pack of 1779/1777"
        }
        CoreCase::FlaxAio => {
            "Seers flax field (2741,3444,0), Crafting 1, empty pack of 1779/1777"
        }
        CoreCase::FlaxAioPick => {
            "Seers flax field (2741,3444,0) with empty pack of 1779/1777"
        }
        CoreCase::FlaxAioSpin => {
            "FlaxAIO Seers bank (2725,3493,0), Crafting 1, empty pack of 1779/1777"
        }
        CoreCase::HerbloreSecondaries => {
            "Edgeville dungeon eggs (3120,9952,0) with empty pack of 223/221"
        }
        CoreCase::HerbloreSecondariesNewt => {
            "Betty shop (3012,3259,0) with empty pack of 221/223"
        }
        CoreCase::ChaosDruid => {
            "Edgeville dungeon (3110,9936,0), Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, empty herb/law/nature"
        }
        CoreCase::ChaosDruidTower => {
            "Chaos Druid Tower (2562,3356,0) r4, Thieving 46, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, empty herb/law/nature"
        }
        CoreCase::ChaosDruidYanille => {
            "Yanille Dungeon warrior room (2580,9501,0) r8, Agility 40, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, empty herb/law/nature"
        }
        CoreCase::MossGiant => {
            "Moss safespot (2553,3406,0), Attack/Strength/Hitpoints 40, lobster 10, scimitar 1331, empty big bones 532"
        }
        CoreCase::MossGiantPrepared => {
            "Moss safespot (2553,3406,0), exact Attack/Strength/Defence/Hitpoints 70, exact lobster 10, worn rune scimitar 1333 and 1113/1079/1163, empty big bones 532"
        }
        CoreCase::MossGiantDart => {
            "Ardougne North bank (2615,3332,0) r2 with the booth still open, empty pack and worn, Ranged 50 / Defence 40 / Hitpoints 40, bank bronze dart 806x80 and lobster 15, no pack/worn/bank rune arrow 892"
        }
        CoreCase::HillGiant => {
            "Giant pit (3110,9832,0), Attack/Strength/Hitpoints 40, trout 8, scimitar 1331, empty 532/225"
        }
        CoreCase::AutoFighter => {
            "Ardougne Guard (2661,3306,0), Attack/Strength/Hitpoints 40, trout 8, scimitar 1331, banking None"
        }
        CoreCase::AutoFighterMage => {
            "Ardougne Guard (2661,3306,0), Magic 13, Hitpoints 40, trout 8, worn Staff of fire 1387, exact Mind rune 558x150 and Air rune 556x300, banking None"
        }
        CoreCase::RockCrab => {
            "safe stand (2712,3707,0), dormant Rocks observed in the supported field, Attack/Strength/Hitpoints 40, lobster 8, scimitar 1331, bank Off"
        }
        CoreCase::AutoFighterRange => {
            "Ardougne Guard (2661,3306,0), Hitpoints 40, Ranged 40, Maple shortbow 853 worn, Bronze arrow 882 x200 worn, Trout 8, bank None"
        }
        CoreCase::GreenDragon => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, lobster 20, rune scimitar 1333, worn shield 1540, empty 536/1753"
        }
        CoreCase::GreenDragonPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Defence/Hitpoints 40, exact lobster 20, carried rune scimitar 1333, worn 1113/1079/1163/1540, empty 536/1753"
        }
        CoreCase::GreenDragonMagePrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Magic/Defence/Hitpoints 70, exact lobster 12, worn Staff of fire 1387 and shield 1540, exact Mind rune 558x150 and Air rune 556x300, no rune armour or scimitar"
        }
        CoreCase::GreenDragonSpecial => {
            "Wilderness field (3096,3814,0) z>=3520, Attack 60, Hitpoints 40, worn dragon dagger 1215 and shield 1540, unarmed spec bar, lobster 12"
        }
        CoreCase::GreenDragonSpecialPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 70, exact lobster 12, worn dragon dagger 1215, shield 1540 and 1113/1079/1163, unarmed spec bar with at least 250 energy"
        }
        CoreCase::GreenDragonPotions => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, worn shield 1540, super attack(3) 145 and super strength(3) 157 with no two-dose flask and no live boost, lobster 12"
        }
        CoreCase::GreenDragonPotionsPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 70, exact lobster 12, carried rune scimitar 1333, worn shield 1540 and 1113/1079/1163, fresh 3-dose attack/strength flasks with no two-dose result or live boost"
        }
        CoreCase::RockCrabRange => {
            "safe stand (2712,3707,0), dormant Rocks observed in the supported field, Hitpoints and Ranged 40, Maple shortbow 853 worn, Bronze arrow 882 x200 worn, lobster 8, bank Off"
        }
        CoreCase::FireGiant => {
            "Fire giant room (2575,9893,0) z>=9000, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, amulet 295, rope 954, empty 532"
        }
        CoreCase::FireGiantPrepared => {
            "Fire giant room (2575,9893,0) z>=9000, Attack/Strength/Defence/Hitpoints 40, exact lobster 12, worn rune scimitar 1333 and 1113/1079/1163, amulet 295, rope 954, empty 532"
        }
        CoreCase::ArdyFighter => {
            "Ardougne Guard (2661,3306,0), Attack/Strength/Hitpoints 40, Thieving 5, scimitar 1331, empty cake/bread/slice, bank Off"
        }
        CoreCase::AutoFighterBank => {
            "Ardougne Guard (2661,3306,0) r8, Attack/Strength/Hitpoints 40, trout 8, worn scimitar 1331, banking Auto, loot Bones, buryBones off, empty 526"
        }
        CoreCase::MossGiantBank => {
            "Moss safespot (2553,3406,0) r10, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 2 (below its restock line), empty 532/225"
        }
        CoreCase::HillGiantBank => {
            "Giant pit (3110,9832,0) r16, Attack/Strength/Hitpoints 40, worn scimitar 1331, trout 8, brass key 983, lootSlots 1, empty 532/225"
        }
        CoreCase::HillGiantBankPrepared => {
            "Giant pit (3110,9832,0) r16, exact Attack/Strength/Defence/Hitpoints 70, exact trout 8, worn adamant scimitar 1331 and 1113/1079/1163, brass key 983, lootSlots 1, empty 532/225"
        }
        CoreCase::ChaosDruidBank => {
            "Edgeville dungeon (3110,9936,0) r14, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 8 (under foodWithdraw 12)"
        }
        CoreCase::ArdyFighterBank => {
            "Ardougne Guard (2661,3306,0) r12, Attack/Strength/Hitpoints 40, Thieving 5, worn scimitar 1331, empty cake pack, empty Guard-drop class 440/886/1446/565/562/561, bankStrategy Loot count"
        }
        CoreCase::RockCrabBank => {
            "safe stand (2712,3707,0), dormant Rocks, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 8, empty 1623/405, bankStrategy Loot count"
        }
        CoreCase::GreenDragonBank => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, worn rune scimitar 1333 and shield 1540, lobster 12, empty 536/1753, escape Flee to bank"
        }
        CoreCase::GreenDragonBankPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 99, exact lobster 26, worn rune scimitar 1333, shield 1540 and 1113/1079/1163, empty 536/1753, stock-selected inventory-pressure Flee to bank"
        }
        CoreCase::GreenDragonBankDefaultPrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 99, exact lobster 26, worn rune scimitar 1333, shield 1540 and 1113/1079/1163, empty 536/1753, source-default loot inventory-pressure Flee to bank"
        }
        CoreCase::GreenDragonTele => {
            "Wilderness field (3096,3814,0) z>=3520, Attack/Strength/Hitpoints 40, Magic 25, worn rune scimitar 1333 and shield 1540, lobster 12, Law/Air/Fire runes, escape Teleport to Varrock"
        }
        CoreCase::GreenDragonTelePrepared => {
            "Wilderness field (3096,3814,0) z>=3520, exact Attack/Strength/Defence/Hitpoints 99 at exactly 70 current HP, zero lobster, Magic 25, worn rune scimitar 1333, shield 1540 and 1113/1079/1163, Law/Air/Fire runes, panicHp 98, escape Teleport to Varrock, no death recovery"
        }
        CoreCase::FireGiantApproach => {
            "raft stand (2510,3493,0) z<9000, Attack/Strength/Hitpoints 40, lobster 12, scimitar 1331, amulet 295, rope 954, Waterfall Quest"
        }
        CoreCase::FireGiantBank => {
            "Fire giant room (2575,9893,0) z>=9000, Attack/Strength/Hitpoints 40, worn scimitar 1331, lobster 12, amulet 295, rope 954, empty 532, escapeTele Barrel"
        }
        CoreCase::FireGiantBankPrepared => {
            "Fire giant room (2575,9893,0) z>=9000, exact Attack/Strength/Defence/Hitpoints 99, exact initial lobster 1, worn rune scimitar 1333 and 1113/1079/1163, amulet 295, rope 954, empty earned Big bones 532, stock-selected one-food escapeTele Barrel and exact restock 25"
        }
        CoreCase::FireGiantCamelotPrepared => {
            "Fire giant room (2575,9893,0) z>=9000, exact Attack/Strength/Defence/Hitpoints 99, Magic 45, exact initial lobster 1, worn rune scimitar 1333 and 1113/1079/1163, worn amulet 295, rope 954, Air 556x15 and Law 563x3, empty earned Big bones 532, escapeTele Camelot land 2757,3478 then Seers 24-lobster restock"
        }
        CoreCase::AioTeleport => {
            "Lumbridge bank (3092,3245,0) r8, Magic 25, worn staff of air 1381, pack law 563x2 and fire 554, not already at Varrock land"
        }
        CoreCase::AioTeleportFalador => {
            "Lumbridge bank (3092,3245,0) r8, Magic 37, worn staff of water 1383, pack law 563x2 and air 556, not already at Falador land"
        }
        CoreCase::AioTeleportNoStaff => {
            "Lumbridge bank (3092,3245,0) r8, Magic 25, pack law/air/fire, no covering air/water staff, not already at Varrock land"
        }
        CoreCase::ShopBuyout => {
            "Aemad stand (2613,3294,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutAubury => {
            "Aubury stand (3253,3401,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutLowe => {
            "Lowe stand (3231,3421,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutHickton => {
            "Hickton stand (2821,3442,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutHarry => {
            "Harry stand (2833,3443,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutBetty => {
            "Betty stand (3012,3258,0) r6 and empty pack of coins/stock"
        }
        CoreCase::ShopBuyoutGerrant => {
            "Gerrant stand (3013,3224,0) r6 and empty pack of coins/stock"
        }
        CoreCase::SmithingBot => {
            "Varrock West bank (3185,3440,0), Smithing 1, empty pack of 2347/2349/1205"
        }
        CoreCase::SmithingBotPlatebody => {
            "Varrock West bank (3185,3440,0), Smithing 18, empty pack of 2347/2349/1117/1205"
        }
        CoreCase::LeatherCrafter => {
            "Al-Kharid bank (3269,3167,0), Crafting 1, empty pack of 1733/1734/1741/1059"
        }
        CoreCase::LeatherCrafterHardBody => {
            "Al-Kharid bank (3269,3167,0), Crafting 28, empty pack of 1733/1734/1743/1131/1059"
        }
        CoreCase::Firemaker => {
            "Varrock East bank (3253,3420,0), Firemaking 1, empty pack of 590/1511, no Fire loc in posted plot"
        }
        CoreCase::FiremakerOak => {
            "Varrock East bank (3253,3420,0), Firemaking 15, empty pack of 590/1521/1511, no Fire loc in posted plot"
        }
        CoreCase::ClimbingBoots => {
            "Tenzing hut door (2823,3555,0) r12, zero boots 3105 carried and banked, exact carried 336 coins (28 pair), no runes, Death Plateau complete"
        }
        CoreCase::ClimbingBootsTeleport => {
            "Tenzing hut door (2823,3555,0) r12, zero boots 3105 carried and banked, exact carried 300 coins (25 pair), Law 563x1/Air 556x3/Water 555x1, Magic 37, Death Plateau complete"
        }
        CoreCase::RangingGuildRound => {
            "range stand (2672,3419,0) r2, Ranged 70, Magic shortbow 861, 400 coins, no ticket 1464, no rune arrow 892, unpaid targetcount 156"
        }
        CoreCase::RangingGuildRedeem => {
            "merchant stand (2659,3430,0) r3, Ranged 70, Magic shortbow 861, seeded ticket 1464x2000, no rune arrow 892"
        }
        CoreCase::RangingGuildBank => {
            "Seers bank (2725,3491,0) r6, Ranged 70, seeded KEEP ticket 1464x1999, seeded rune arrow 892x50, no pack coins 995, no pack/worn Magic shortbow 861, unpaid targetcount 156"
        }
        CoreCase::RangingGuildFull => {
            "Seers bank (2725,3491,0) r6, Ranged 70, seeded KEEP ticket 1464x1999, no rune arrow 892 packed or banked, no pack coins 995, no pack/worn Magic shortbow 861, unpaid targetcount 156"
        }
        CoreCase::BrimhavenMossInspectV1 => {
            "Ardougne SE bank (2655,3283,0) r6, empty pack, Agility 30, no carried lobster/coins"
        }
        CoreCase::RouteInspectBrimhavenV2 => {
            "Captain Barnaby pier (2683,3272,0) r4, ingame && scene_state==2"
        }
        CoreCase::PrayerV2 | CoreCase::PrayerV1 => {
            "ingame && scene_state==2, prayer base>=43, positive points, varps 83..97 present and 0"
        }
        CoreCase::LineOfSightV2 => {
            "ingame && scene_state==2 && SceneView.available, here in published collision bounds"
        }
        CoreCase::ActorObservationV2 => {
            "ingame && scene_state==2 && SceneView.available, here on the published plane"
        }
        CoreCase::FightFieldV2 => {
            "ingame && scene_state==2 && SceneView.available, here on the published plane"
        }
        CoreCase::HoldSpotV2 | CoreCase::RetreatSpotV2 | CoreCase::WalkSpotV2 => {
            "ingame && scene_state==2 && SceneView.available, host here on the published plane"
        }
        CoreCase::EnterLairV2 => {
            "ingame && scene_state==2 && SceneView.available, host here not the KBD tile"
        }
        CoreCase::LeaveLairV2 => {
            "ingame && scene_state==2 && SceneView.available, host here not the KBD, Edgeville or Varrock tile"
        }
        CoreCase::AcquireKeyV2 => {
            "ingame && scene_state==2 && SceneView.available, host here outside the jail cell and the projected lair box, Chebyshev > 1 from the corridor (2931,9690,0), no Jail key held"
        }
        CoreCase::CellV2 => {
            "ingame && scene_state==2 && SceneView.available, host here outside the jail cell and the projected lair box, Chebyshev > 1 from the jail door (2931,9690,0), no Dusty key held"
        }
        CoreCase::BankV2 => {
            "ingame && scene_state==2 && SceneView.available, host here Chebyshev > 3 from (2946,3369,0) and outside the projected lair box"
        }
    };
    Err(format!(
        "{} Start baseline lacks required preparation ({requirement}): {baseline:?}",
        case.scenario_name()
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct CoreWitness {
    pub case: CoreCase,
    pub baseline: Observation,
    pub start_preparation: Option<HerbCleanerStartPreparationReceipt>,
    pub latest: Observation,
    pub max_items: BTreeMap<String, i32>,
    pub max_xp: BTreeMap<String, i32>,
    pub saw_bury_chat: bool,
    pub post_start_observations: u64,
    pub bone_bank_cycle: BoneBankCycle,
    pub bank_fletcher_cycle: BankFletcherCycle,
    pub bank_fletcher_option_cycle: BankFletcherOptionCycle,
    pub bank_fletcher_string_cycle: BankFletcherStringCycle,
    pub bank_fletcher_cut_string_cycle: BankFletcherCutStringCycle,
    pub alcher_defaults_cycle: AlcherGeneratedCustomCycle,
    pub alcher_generated_custom_cycle: AlcherGeneratedCustomCycle,
    pub alcher_spell_cycle: AlcherGeneratedCustomCycle,
    pub alcher_swarm_cycle: AlcherSwarmDrainCycle,
    pub dart_fletcher_cycle: DartFletcherCycle,
    pub herb_cleaner_cycle: HerbCleanerCycle,
    pub herb_cleaner_empty_cycle: HerbCleanerEmptyCycle,
    pub gem_cutter_cycle: GemCutterCycle,
    pub door_opener_cycle: DoorOpenerCycle,
    pub gnome_course_cycle: GnomeCourseCycle,
    pub wildy_agility_cycle: WildyAgilityCycle,
    pub brimhaven_agility_cycle: BrimhavenAgilityCycle,
    pub flax_picker_cycle: FlaxPickerCycle,
    pub superheater_cycle: SuperheaterCycle,
    pub chicken_killer_bank_cycle: ChickenKillerBankCycle,
    pub vial_filler_cycle: VialFillerCycle,
    pub potion_maker_cycle: PotionMakerCycle,
    pub tanner_bot_cycle: TannerBotCycle,
    pub rune_crafter_cycle: RuneCrafterCycle,
    pub ardy_cakes_cycle: ArdyCakesCycle,
    pub ardy_cakes_fight_cycle: ArdyCakesFightCycle,
    pub ardy_thiever_cycle: ArdyThieverCycle,
    pub ardy_thiever_fight_cycle: ArdyThieverFightCycle,
    pub gnome_chop_cycle: GnomeChopCycle,
    pub gnome_fletch_cycle: GnomeFletchCycle,
    pub coal_trucks_cycle: CoalTrucksCycle,
    pub station_production_cycle: StationProductionCycle,
    pub flax_aio_cycle: FlaxAioCycle,
    pub flax_aio_pick_cycle: FlaxAioPickCycle,
    pub herblore_eggs_cycle: HerbloreEggsCycle,
    pub herblore_newt_cycle: HerbloreNewtCycle,
    pub combat_core_cycle: CombatCoreCycle,
    pub combat_dart_branch_cycle: CombatDartBranchCycle,
    pub combat_bank_cycle: CombatBankCycle,
    pub combat_approach_cycle: CombatApproachCycle,
    pub aio_teleport_cycle: AioTeleportCycle,
    pub shop_buyout_cycle: ShopBuyoutCycle,
    pub smithing_bot_cycle: SmithingBotCycle,
    pub leather_crafter_cycle: LeatherCrafterCycle,
    pub firemaker_cycle: FiremakerCycle,
    pub climbing_boots_cycle: ClimbingBootsCycle,
    pub ranging_guild_round_cycle: RangingGuildRoundCycle,
    pub ranging_guild_redeem_cycle: RangingGuildRedeemCycle,
    pub ranging_guild_bank_cycle: RangingGuildBankCycle,
    pub ranging_guild_full_cycle: RangingGuildFullCycle,
    pub brimhaven_moss_inspect_cycle: BrimhavenMossInspectCycle,
    pub route_inspect_brimhaven_v2_cycle: RouteInspectBrimhavenV2Cycle,
    pub prayer_delivery_cycle: PrayerDeliveryCycle,
    pub line_of_sight_cycle: LineOfSightDeliveryCycle,
    pub actor_observation_cycle: ActorObservationDeliveryCycle,
    pub fight_field_cycle: FightFieldDeliveryCycle,
    pub hunt_cycle: HuntDeliveryCycle,
    pub ordered_first_exhausted: bool,
}


/// Superheat a trip, deposit bars except natures, restock ores, then smelt again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SuperheaterCycle {
    pub first_bars: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub produced_after_withdrawal: bool,
    pub wrong_product: bool,
    pub wrong_staff: bool,
}

pub struct SuperheaterSpec {
    pub bar: i32,
    pub primary: i32,
    pub secondary: i32,
    pub staff: i32,
    pub steel: bool,
}

impl SuperheaterCycle {
    pub fn observe(&mut self, spec: SuperheaterSpec, baseline: &Observation, now: &Observation) {
        let SuperheaterSpec {
            bar,
            primary,
            secondary,
            staff,
            steel,
        } = spec;
        let wrong_bars = [IRON_BAR_ID, BRONZE_BAR_ID, STEEL_BAR_ID, SILVER_BAR_ID]
            .into_iter()
            .filter(|id| *id != bar)
            .any(|id| now.item_id(id) > 0 || now.bank_item_id(id) > 0);
        self.wrong_product |= wrong_bars;
        if now.equipment_id(STAFF_OF_FIRE_ID) > 0 && staff != STAFF_OF_FIRE_ID {
            self.wrong_staff = true;
        }
        if now.equipment_id(FIRE_BATTLESTAFF_ID) > 0 && staff != FIRE_BATTLESTAFF_ID {
            self.wrong_staff = true;
        }
        self.first_bars |= now.item_id(bar) >= 1
            && now.item_id(NATURE_RUNE_ID) >= 1
            && now.item_id(staff) == 0
            && now.equipment_id(staff) >= 1
            && now.skill_xp("magic") > baseline.skill_xp("magic")
            && now.skill_xp("smithing") > baseline.skill_xp("smithing")
            && baseline.item_id(bar) == 0
            && !wrong_bars;
        if self.first_bars
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(bar) == 0
            && now.bank_item_id(bar) >= 1
            && now.item_id(NATURE_RUNE_ID) >= 1
            && now.item_id(primary) == 0
            && now.item_id(secondary) == 0
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(primary) >= 1
                && now.item_id(secondary) >= 1
                && now.bank_item_id(primary) < deposited.bank_item_id(primary)
                && now.bank_item_id(secondary) < deposited.bank_item_id(secondary)
                && now.item_id(NATURE_RUNE_ID) >= 1
                && if steel {
                    now.item_id(secondary) == 2 * now.item_id(primary)
                } else {
                    now.item_id(primary) == now.item_id(secondary)
                }
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.produced_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(bar) >= 1
                && now.item_id(primary) < withdrawn.item_id(primary)
                && now.item_id(secondary) < withdrawn.item_id(secondary)
                && now.item_id(NATURE_RUNE_ID) < withdrawn.item_id(NATURE_RUNE_ID)
                && now.skill_xp("magic") > withdrawn.skill_xp("magic")
                && now.skill_xp("smithing") > withdrawn.skill_xp("smithing")
                && now.equipment_id(staff) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.produced_after_withdrawal && !self.wrong_product && !self.wrong_staff
    }
}


/// Fill at the fountain, deposit produced water vials, restock empties, fill again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VialFillerCycle {
    pub filled: Option<Observation>,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl VialFillerCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.filled.is_none()
            && now.item_id(VIAL_OF_WATER_ID) >= 1
            && baseline.item_id(VIAL_OF_WATER_ID) == 0
            && near(now.tile, FALADOR_FOUNTAIN, 4)
        {
            self.filled = Some(now.clone());
        }
        if self.filled.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(VIAL_OF_WATER_ID) == 0
            && now.bank_item_id(VIAL_OF_WATER_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(EMPTY_VIAL_ID) >= 1
                && now.bank_item_id(EMPTY_VIAL_ID) < deposited.bank_item_id(EMPTY_VIAL_ID)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.withdrawn.is_some()
                && !now.bank_open
                && !now.bank_loaded
                // Closing the modal advances the bank session generation.
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FALADOR_FOUNTAIN, 4);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(VIAL_OF_WATER_ID) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
    }
}

/// Staged herb+water to unfinished, secondary to finished, deposit, restock, further product.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PotionMakerCycle {
    pub unfinished: Option<Observation>,
    pub finished: Option<Observation>,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub further: bool,
    pub wrong_product: bool,
    pub filter_violated: bool,
}

pub struct PotionMakerSpec {
    pub herb: i32,
    pub unf: i32,
    pub secondary: i32,
    pub finished: i32,
    pub wrong_unf: i32,
    pub wrong_finished: i32,
    pub named: bool,
}

impl PotionMakerCycle {
    pub fn observe(&mut self, spec: PotionMakerSpec, baseline: &Observation, now: &Observation) {
        let PotionMakerSpec {
            herb,
            unf,
            secondary,
            finished,
            wrong_unf,
            wrong_finished,
            named,
        } = spec;
        self.wrong_product |= now.item_id(wrong_unf) > 0
            || now.item_id(wrong_finished) > 0
            || now.bank_item_id(wrong_unf) > 0
            || now.bank_item_id(wrong_finished) > 0;
        if named
            && (now.item_id(GUAM_LEAF_ID) > 0
                || (now.bank_open
                    && now.bank_loaded
                    && now.bank_generation > baseline.bank_generation
                    && now.bank_item_id(GUAM_LEAF_ID) < 14))
        {
            self.filter_violated = true;
        }
        if self.unfinished.is_none()
            && now.item_id(unf) >= 1
            && now.item_id(herb) < 14
            && now.item_id(VIAL_OF_WATER_ID) < 14
            && now.item_id(finished) == 0
            && baseline.item_id(unf) == 0
            && baseline.item_id(finished) == 0
        {
            self.unfinished = Some(now.clone());
        }
        if let Some(unfinished) = &self.unfinished {
            if self.finished.is_none()
                && now.item_id(finished) >= 1
                && now.item_id(unf) < unfinished.item_id(unf)
                && now.item_id(secondary) < 14
                && now.skill_xp("herblore") > baseline.skill_xp("herblore")
                && !now.bank_open
            {
                self.finished = Some(now.clone());
            }
        }
        if self.finished.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(finished) == 0
            && now.bank_item_id(finished) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(herb) >= 1
                && now.item_id(VIAL_OF_WATER_ID) >= 1
                && now.bank_item_id(herb) < deposited.bank_item_id(herb)
                && now.bank_item_id(VIAL_OF_WATER_ID) < deposited.bank_item_id(VIAL_OF_WATER_ID)
                && (!named || now.bank_item_id(GUAM_LEAF_ID) == 14)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.further |= now.item_id(unf) >= 1
                && now.item_id(herb) < withdrawn.item_id(herb)
                && now.item_id(VIAL_OF_WATER_ID) < withdrawn.item_id(VIAL_OF_WATER_ID);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.finished.is_some() && !self.wrong_product && !self.filter_violated
    }
}

/// Tan at the Tanner widget, deposit produced leather, restock hides, tan again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TannerBotCycle {
    pub widget: Option<Observation>,
    pub tanned: Option<Observation>,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub shop: bool,
}

pub struct TannerBotSpec {
    pub product: i32,
    pub wrong_product: i32,
    pub tan_all: i32,
}

impl TannerBotCycle {
    pub fn observe(&mut self, spec: TannerBotSpec, baseline: &Observation, now: &Observation) {
        let TannerBotSpec {
            product,
            wrong_product,
            tan_all,
        } = spec;
        self.wrong_product |= now.item_id(wrong_product) > 0 || now.bank_item_id(wrong_product) > 0;
        self.shop |= now.main_modal == SHOPMAIN;
        if self.widget.is_none()
            && now.main_modal == TANNER_IF
            && now.has_widget(tan_all)
            && now.item_id(COW_HIDE_ID) >= 1
            && now.item_id(product) == 0
            && now.item_id(COINS_ID) >= 1
            && near(now.tile, TANNER_STAND, 4)
            && now.main_modal != SHOPMAIN
        {
            self.widget = Some(now.clone());
        }
        if let Some(widget) = &self.widget {
            if self.tanned.is_none()
                && near(now.tile, TANNER_STAND, 4)
                && now.item_id(product) >= 1
                && now.item_id(COW_HIDE_ID) == 0
                && now.item_id(COINS_ID) < widget.item_id(COINS_ID)
                && now.item_id(wrong_product) == 0
                && now.main_modal != SHOPMAIN
                && !near(now.tile, DOMMIK_STAND, 4)
            {
                self.tanned = Some(now.clone());
            }
        }
        if self.tanned.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(COW_HIDE_ID) >= 1
                && now.bank_item_id(COW_HIDE_ID) < deposited.bank_item_id(COW_HIDE_ID)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.withdrawn.is_some()
                && !now.bank_open
                && !now.bank_loaded
                // Closing the modal advances the bank session generation.
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, TANNER_STAND, 4);
        }
        if self.returned {
            self.further |= !now.bank_open
                && now.item_id(product) >= 1
                && now.item_id(COW_HIDE_ID) == 0
                && now.item_id(wrong_product) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.shop && self.tanned.is_some()
    }
}

/// Withdraw unnoted essence, enter the selected altar, convert to the selected
/// rune with Runecraft XP, portal out, deposit, restock, and craft again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RuneCrafterCycle {
    pub withdrawn: Option<Observation>,
    pub entered: Option<Observation>,
    pub crafted: Option<Observation>,
    pub exited: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

pub struct RuneCrafterSpec {
    pub rune: i32,
    pub wrong_rune: i32,
    pub ruins: (i32, i32, i32),
    pub bank: (i32, i32, i32),
}

impl RuneCrafterCycle {
    pub fn observe(&mut self, spec: RuneCrafterSpec, baseline: &Observation, now: &Observation) {
        let RuneCrafterSpec {
            rune,
            wrong_rune,
            ruins,
            bank,
        } = spec;
        self.wrong_product |= now.item_id(wrong_rune) > 0 || now.bank_item_id(wrong_rune) > 0;
        self.noted |= now.item_id(NOTED_ESSENCE_ID) > 0;
        if self.withdrawn.is_none()
            && overworld(now.tile)
            && near(now.tile, bank, 8)
            && now.item_id(RUNE_ESSENCE_ID) >= 1
            && now.item_id(rune) == 0
            && now.item_id(NOTED_ESSENCE_ID) == 0
        {
            self.withdrawn = Some(now.clone());
        }
        if self.withdrawn.is_some()
            && self.entered.is_none()
            && in_temple(now.tile)
            && now.item_id(RUNE_ESSENCE_ID) >= 1
            && now.item_id(rune) == 0
        {
            self.entered = Some(now.clone());
        }
        if self.entered.is_some()
            && self.crafted.is_none()
            && in_temple(now.tile)
            && now.item_id(rune) >= 1
            && now.item_id(RUNE_ESSENCE_ID) == 0
            && now.skill_xp("runecraft") > baseline.skill_xp("runecraft")
            && now.item_id(wrong_rune) == 0
            && now.item_id(NOTED_ESSENCE_ID) == 0
        {
            self.crafted = Some(now.clone());
        }
        if self.crafted.is_some()
            && self.exited.is_none()
            && overworld(now.tile)
            && near(now.tile, ruins, 8)
            && now.item_id(rune) >= 1
        {
            self.exited = Some(now.clone());
        }
        if self.exited.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(rune) == 0
            && now.bank_item_id(rune) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(RUNE_ESSENCE_ID) >= 1
                && now.bank_item_id(RUNE_ESSENCE_ID) < deposited.bank_item_id(RUNE_ESSENCE_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && (near(now.tile, ruins, 8) || in_temple(now.tile));
        }
        if let Some(crafted) = &self.crafted {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(rune) >= 1
                && now.item_id(RUNE_ESSENCE_ID) == 0
                && now.item_id(wrong_rune) == 0
                && now.skill_xp("runecraft") > crafted.skill_xp("runecraft");
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.entered.is_some()
            && self.crafted.is_some()
            && self.exited.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Start with exact 22-Knife ballast, steal six cake/bread/chocolate slices,
/// deposit both product and ballast in a fresh bank, return to STAND, steal
/// again. Chocolate cake 1897 and noted stall food fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyCakesCycle {
    pub stolen: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl ArdyCakesCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(CHOCOLATE_CAKE_ID) > 0 || now.bank_item_id(CHOCOLATE_CAKE_ID) > 0;
        self.noted |= noted_stall_food(now) > 0;
        if self.stolen.is_none()
            && stall_food(now) >= 1
            && stall_food(baseline) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
            && now.item_id(CHOCOLATE_CAKE_ID) == 0
            && noted_stall_food(now) == 0
        {
            self.stolen = Some(now.clone());
        }
        if self.stolen.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && stall_food(now) == 0
            && bank_stall_food(now) >= 1
            && now.item_id(KNIFE_ID) == 0
            && now.bank_item_id(KNIFE_ID)
                >= baseline.bank_item_id(KNIFE_ID) + ARDY_CAKES_BALLAST_KNIVES
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, ARDY_CAKES_STAND, 6);
        }
        if self.returned {
            self.further |= !now.bank_open && stall_food(now) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.stolen.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// `guardResponse=Fight`: stall steal then FightBack kill. Landing on the Flee
/// kite tile fails — that is the other branch.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyCakesFightCycle {
    pub stolen: Option<Observation>,
    pub killed: Option<Observation>,
    pub fled: bool,
    pub wrong_product: bool,
    pub noted: bool,
    pub engaged_guard: Option<usize>,
    pub style_xp: bool,
}

impl ArdyCakesFightCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(CHOCOLATE_CAKE_ID) > 0 || now.bank_item_id(CHOCOLATE_CAKE_ID) > 0;
        self.noted |= noted_stall_food(now) > 0;
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength")
            || now.skill_xp("attack") > baseline.skill_xp("attack");
        if self.stolen.is_none()
            && stall_food(now) >= 1
            && stall_food(baseline) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
            && now.item_id(CHOCOLATE_CAKE_ID) == 0
            && noted_stall_food(now) == 0
        {
            self.stolen = Some(now.clone());
        }
        if self.stolen.is_some() && self.killed.is_none() {
            self.fled |= near(now.tile, ARDY_FLEE_TILE, 2);
        }
        for npc in &now.npc_facts {
            let Some(name) = npc.name.as_deref().map(str::trim) else {
                continue;
            };
            if name != "Guard" {
                continue;
            }
            let selected = npc.targeting_local
                || (now.local_target_npc == Some(npc.index) && now.local_in_combat);
            if selected {
                self.engaged_guard = Some(npc.index);
            }
            if self.stolen.is_some()
                && self.killed.is_none()
                && self.engaged_guard == Some(npc.index)
                && npc.total_health > 0
                && npc.health == 0
                && self.style_xp
            {
                self.killed = Some(now.clone());
            }
        }
        // Despawn after engagement also counts once style XP landed.
        if self.stolen.is_some()
            && self.killed.is_none()
            && self.style_xp
            && self
                .engaged_guard
                .is_some_and(|index| !now.npc_facts.iter().any(|npc| npc.index == index))
        {
            self.killed = Some(now.clone());
        }
    }

    pub fn qualified(&self) -> bool {
        self.stolen.is_some()
            && self.killed.is_some()
            && self.style_xp
            && !self.fled
            && !self.wrong_product
            && !self.noted
    }
}

/// Pickpocket coins with Thieving XP, deposit coins at loot-count 1, return
/// to the market stand, pickpocket again. Stall food without coins cannot
/// qualify. Fight stays pending.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyThieverCycle {
    pub pickpocketed: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl ArdyThieverCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.pickpocketed.is_none()
            && now.item_id(COINS_ID) >= 1
            && baseline.item_id(COINS_ID) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
        {
            self.pickpocketed = Some(now.clone());
        }
        if self.pickpocketed.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(COINS_ID) == 0
            && now.bank_item_id(COINS_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, ARDY_THIEVER_STAND, 6);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(COINS_ID) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.pickpocketed.is_some() && self.deposited.is_some()
    }
}

/// `guardResponse=Fight` on ArdyThiever: a FightBack Guard kill plus the Flee
/// bank-cycle shape. The kill and the Flee-kite check count from Start, not
/// from the first coins: on 289 only a caught stall steal draws a Guard (a
/// failed pickpocket stuns but never starts combat), and ArdyThiever steals
/// from the stall only while its food is at `restockAtFood`, so the catch
/// comes from the opening restock. The deposit still needs both the coins
/// and the kill. The Flee kite tile fails this branch.
///
/// Coins count only as a pickpocket: a coin gain with a Thieving XP rise in
/// the same or the previous observation (the pickpocket script adds the
/// coins and the XP in one server tick). A dropped Guard's coins picked up by
/// `LootDrops` carry no Thieving XP, and neither does the stall steal that
/// raised it earlier.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyThieverFightCycle {
    pub pickpocketed: Option<Observation>,
    pub killed: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub fled: bool,
    pub engaged_guard: Option<usize>,
    pub style_xp: bool,
    /// Coins and Thieving XP at the previous observation, and whether the
    /// XP rose on it.
    #[serde(skip)]
    last: Option<(i32, i32, bool)>,
}

impl ArdyThieverFightCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength")
            || now.skill_xp("attack") > baseline.skill_xp("attack");
        let (coins, thieving) = (now.item_id(COINS_ID), now.skill_xp("thieving"));
        let (last_coins, last_thieving, xp_rose_before) = self.last.unwrap_or((
            baseline.item_id(COINS_ID),
            baseline.skill_xp("thieving"),
            false,
        ));
        let xp_rose = thieving > last_thieving;
        let picked = coins > last_coins && (xp_rose || xp_rose_before);
        self.last = Some((coins, thieving, xp_rose));
        if self.pickpocketed.is_none() && picked && baseline.item_id(COINS_ID) == 0 {
            self.pickpocketed = Some(now.clone());
        }
        if self.killed.is_none() {
            self.fled |= near(now.tile, ARDY_FLEE_TILE, 2);
        }
        for npc in &now.npc_facts {
            let Some(name) = npc.name.as_deref().map(str::trim) else {
                continue;
            };
            if name != "Guard" {
                continue;
            }
            let selected = npc.targeting_local
                || (now.local_target_npc == Some(npc.index) && now.local_in_combat);
            if selected {
                self.engaged_guard = Some(npc.index);
            }
            if self.killed.is_none()
                && self.engaged_guard == Some(npc.index)
                && npc.total_health > 0
                && npc.health == 0
                && self.style_xp
            {
                self.killed = Some(now.clone());
            }
        }
        if self.killed.is_none()
            && self.style_xp
            && self
                .engaged_guard
                .is_some_and(|index| !now.npc_facts.iter().any(|npc| npc.index == index))
        {
            self.killed = Some(now.clone());
        }
        if self.pickpocketed.is_some()
            && self.killed.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(COINS_ID) == 0
            && now.bank_item_id(COINS_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, ARDY_THIEVER_STAND, 6);
        }
        if self.returned {
            self.further |= !now.bank_open && picked;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.pickpocketed.is_some()
            && self.killed.is_some()
            && self.deposited.is_some()
            && self.style_xp
            && !self.fled
    }
}

/// Chop magic logs 1513 with Woodcutting XP, deposit upstairs, return to
/// ground, chop again. Strung bows and noted logs fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GnomeChopCycle {
    pub chopped: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl GnomeChopCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |= gnome_wrong_bows(now)
            || now.item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
            || now.bank_item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
            || now.item_id(UNSTRUNG_MAGIC_LONGBOW_ID) > 0
            || now.bank_item_id(UNSTRUNG_MAGIC_LONGBOW_ID) > 0;
        self.noted |= gnome_noted(now);
        if self.chopped.is_none()
            && now.item_id(MAGIC_LOGS_ID) >= 1
            && baseline.item_id(MAGIC_LOGS_ID) == 0
            && now.skill_xp("woodcutting") > baseline.skill_xp("woodcutting")
            && gnome_resource_tools(now)
            && !gnome_wrong_bows(now)
            && !gnome_noted(now)
        {
            self.chopped = Some(now.clone());
        }
        if self.chopped.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(MAGIC_LOGS_ID) == 0
            && now.bank_item_id(MAGIC_LOGS_ID) >= 1
            && near(now.tile, GNOME_BANK_STAND, 8)
            && gnome_resource_tools(now)
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, GNOME_BANK_STAIR_SOUTH, 30)
                && gnome_resource_tools(now);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(MAGIC_LOGS_ID) >= 1 && gnome_resource_tools(now);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.chopped.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Chop logs, consume them into exact unstrung 72 or 70 with Fletching XP,
/// deposit upstairs, return, chop again. The other unstrung id and strung
/// 861/859 fail. Missing knife cannot qualify.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GnomeFletchCycle {
    pub chopped: Option<Observation>,
    pub fletched: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl GnomeFletchCycle {
    pub fn observe(&mut self, product: i32, other: i32, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            gnome_wrong_bows(now) || now.item_id(other) > 0 || now.bank_item_id(other) > 0;
        self.noted |= gnome_noted(now);
        if self.chopped.is_none()
            && now.item_id(MAGIC_LOGS_ID) >= 1
            && baseline.item_id(MAGIC_LOGS_ID) == 0
            && now.skill_xp("woodcutting") > baseline.skill_xp("woodcutting")
            && now.item_id(product) == 0
            && gnome_resource_tools(now)
            && !gnome_wrong_bows(now)
            && !gnome_noted(now)
        {
            self.chopped = Some(now.clone());
        }
        if self.chopped.is_some()
            && self.fletched.is_none()
            && now.item_id(product) >= 1
            && now.item_id(MAGIC_LOGS_ID)
                < self
                    .chopped
                    .as_ref()
                    .map(|row| row.item_id(MAGIC_LOGS_ID))
                    .unwrap_or(0)
            && now.skill_xp("fletching") > baseline.skill_xp("fletching")
            && now.item_id(other) == 0
            && gnome_resource_tools(now)
            && !gnome_wrong_bows(now)
            && !gnome_noted(now)
        {
            self.fletched = Some(now.clone());
        }
        if self.fletched.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
            && near(now.tile, GNOME_BANK_STAND, 8)
            && gnome_resource_tools(now)
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, GNOME_BANK_STAIR_SOUTH, 30)
                && gnome_resource_tools(now);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(MAGIC_LOGS_ID) >= 1 && gnome_resource_tools(now);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.chopped.is_some()
            && self.fletched.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Mine coal 453 with Mining XP, deposit to the mine truck (pack empty at
/// the truck, bank closed, bank coal unchanged), then mine again. A Seers
/// bank deposit is not the truck proof.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CoalTrucksCycle {
    pub mined: Option<Observation>,
    pub trucked: Option<Observation>,
    pub further: bool,
    pub noted: bool,
    pub banked: bool,
    pub fixture_changed: bool,
}

impl CoalTrucksCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.noted |= now.item_id(NOTED_COAL_ID) > 0 || now.bank_item_id(NOTED_COAL_ID) > 0;
        self.banked |= now.bank_item_id(COAL_ID) > baseline.bank_item_id(COAL_ID);
        self.fixture_changed |=
            now.item_id(RUNE_PICKAXE_ID) != 1 || now.item_id(KNIFE_ID) != COAL_BALLAST_KNIVES;
        if self.mined.is_none()
            && now.item_id(COAL_ID) >= 1
            && baseline.item_id(COAL_ID) == 0
            && now.skill_xp("mining") > baseline.skill_xp("mining")
            && now.item_id(NOTED_COAL_ID) == 0
        {
            self.mined = Some(now.clone());
        }
        if self.mined.is_some()
            && self.trucked.is_none()
            && now.item_id(COAL_ID) == 0
            && !now.bank_open
            && now.bank_item_id(COAL_ID) == baseline.bank_item_id(COAL_ID)
            && near(now.tile, COAL_MINE_TRUCK_STAND, 4)
            && !near(now.tile, SEERS_BANK, 8)
        {
            self.trucked = Some(now.clone());
        }
        if self.trucked.is_some() {
            self.further |= !now.bank_open
                && now.item_id(COAL_ID) >= 1
                && now.skill_xp("mining") > baseline.skill_xp("mining");
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.mined.is_some()
            && self.trucked.is_some()
            && !self.noted
            && !self.banked
            && !self.fixture_changed
    }
}

/// Produce exact unnoted output with relevant XP after Start, deposit into a
/// fresh loaded bank, restock the raw input, close, return to the station,
/// and produce again. Seeded/name-only/noted/wrong products fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StationProductionCycle {
    pub withdrawn: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

pub struct StationProductionSpec {
    pub product: i32,
    pub input: i32,
    pub extra_input: Option<i32>,
    pub wrong: i32,
    pub extra_wrong: Option<i32>,
    pub skill: &'static str,
    pub station: (i32, i32, i32),
    pub station_radius: i32,
    pub noted: bool,
}

impl StationProductionCycle {
    pub fn observe(
        &mut self,
        spec: StationProductionSpec,
        baseline: &Observation,
        now: &Observation,
    ) {
        let StationProductionSpec {
            product,
            input,
            extra_input,
            wrong,
            extra_wrong,
            skill,
            station,
            station_radius,
            noted,
        } = spec;
        self.wrong_product |= now.item_id(wrong) > 0
            || now.bank_item_id(wrong) > 0
            || extra_wrong.is_some_and(|id| now.item_id(id) > 0 || now.bank_item_id(id) > 0);
        self.noted |= noted;
        if self.withdrawn.is_none()
            && now.item_id(input) >= 1
            && now.item_id(product) == 0
            && baseline.item_id(input) == 0
            && extra_input.is_none_or(|id| now.item_id(id) >= 1)
            && now.item_id(wrong) == 0
            && extra_wrong.is_none_or(|id| now.item_id(id) == 0)
            && !noted
        {
            self.withdrawn = Some(now.clone());
        }
        if let Some(withdrawn) = &self.withdrawn {
            if self.produced.is_none()
                && now.item_id(product) >= 1
                && now.item_id(input) < withdrawn.item_id(input)
                && extra_input.is_none_or(|id| now.item_id(id) < withdrawn.item_id(id))
                && now.skill_xp(skill) > baseline.skill_xp(skill)
                && near(now.tile, station, station_radius)
                && now.item_id(wrong) == 0
                && extra_wrong.is_none_or(|id| now.item_id(id) == 0)
                && !noted
            {
                self.produced = Some(now.clone());
            }
        }
        if self.produced.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(input) >= 1
                && now.bank_item_id(input) < deposited.bank_item_id(input)
                && extra_input.is_none_or(|id| now.item_id(id) >= 1)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, station, station_radius);
        }
        if let Some(produced) = &self.produced {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(product) >= 1
                && now.item_id(wrong) == 0
                && extra_wrong.is_none_or(|id| now.item_id(id) == 0)
                && now.skill_xp(skill) > produced.skill_xp(skill);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.withdrawn.is_some()
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

pub fn station_production_spec(
    case: CoreCase,
    observation: &Observation,
) -> Option<StationProductionSpec> {
    match case {
        CoreCase::CookBot => Some(StationProductionSpec {
            product: SALMON_ID,
            input: RAW_SALMON_ID,
            extra_input: None,
            wrong: LOBSTER_ID,
            extra_wrong: None,
            skill: "cooking",
            station: CATHERBY_RANGE_STAND,
            station_radius: 8,
            noted: cook_noted(observation, NOTED_RAW_SALMON_ID, NOTED_SALMON_ID)
                || cook_wrong_or_burnt(observation, LOBSTER_ID),
        }),
        CoreCase::CookBotLobster => Some(StationProductionSpec {
            product: LOBSTER_ID,
            input: RAW_LOBSTER_ID,
            extra_input: None,
            wrong: SALMON_ID,
            extra_wrong: None,
            skill: "cooking",
            station: CATHERBY_RANGE_STAND,
            station_radius: 8,
            noted: cook_noted(observation, NOTED_RAW_LOBSTER_ID, NOTED_LOBSTER_ID)
                || cook_wrong_or_burnt(observation, SALMON_ID),
        }),
        CoreCase::SmelterBot => Some(StationProductionSpec {
            product: BRONZE_BAR_ID,
            input: COPPER_ORE_ID,
            extra_input: Some(TIN_ORE_ID),
            wrong: STEEL_BAR_ID,
            extra_wrong: Some(IRON_BAR_ID),
            skill: "smithing",
            station: AL_KHARID_FURNACE,
            station_radius: 8,
            noted: smelter_noted(
                observation,
                NOTED_COPPER_ORE_ID,
                NOTED_TIN_ORE_ID,
                NOTED_BRONZE_BAR_ID,
            ),
        }),
        CoreCase::SmelterBotSteel => Some(StationProductionSpec {
            product: STEEL_BAR_ID,
            input: IRON_ORE_ID,
            extra_input: Some(COAL_ID),
            wrong: BRONZE_BAR_ID,
            extra_wrong: Some(IRON_BAR_ID),
            skill: "smithing",
            station: AL_KHARID_FURNACE,
            station_radius: 8,
            noted: smelter_noted(
                observation,
                NOTED_IRON_ORE_ID,
                NOTED_COAL_ID,
                NOTED_STEEL_BAR_ID,
            ),
        }),
        CoreCase::FlaxSpinner => Some(StationProductionSpec {
            product: BOW_STRING_ID,
            input: FLAX_ID,
            extra_input: None,
            wrong: BALL_OF_WOOL_ID,
            extra_wrong: None,
            skill: "crafting",
            station: FLAX_SPINNER_WHEEL,
            station_radius: 8,
            noted: flax_spinner_noted(observation),
        }),
        CoreCase::FlaxAioSpin => Some(StationProductionSpec {
            product: BOW_STRING_ID,
            input: FLAX_ID,
            extra_input: None,
            wrong: BALL_OF_WOOL_ID,
            extra_wrong: None,
            skill: "crafting",
            station: FLAX_SPINNER_WHEEL,
            station_radius: 8,
            noted: flax_spinner_noted(observation),
        }),
        _ => None,
    }
}

/// Magic XP and destination tile, then native bank restock and a further
/// teleport. Walking or a queued if-button without XP/law spend fails.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AioTeleportCycle {
    pub teleported: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub closed: bool,
    pub further: bool,
}

pub struct AioTeleportSpec {
    pub landing: (i32, i32, i32),
    pub restock: (i32, i32, i32),
    pub staff: Option<i32>,
    pub air_from_pack: bool,
}

pub fn aio_teleport_spec(case: CoreCase) -> Option<AioTeleportSpec> {
    match case {
        CoreCase::AioTeleport => Some(AioTeleportSpec {
            landing: VARROCK_TELE_LAND,
            restock: VARROCK_EAST_BANK,
            staff: Some(STAFF_OF_AIR_ID),
            air_from_pack: false,
        }),
        CoreCase::AioTeleportFalador => Some(AioTeleportSpec {
            landing: FALADOR_TELE_LAND,
            restock: FALADOR_WEST_BANK,
            staff: Some(STAFF_OF_WATER_ID),
            air_from_pack: true,
        }),
        CoreCase::AioTeleportNoStaff => Some(AioTeleportSpec {
            landing: VARROCK_TELE_LAND,
            restock: VARROCK_EAST_BANK,
            staff: None,
            air_from_pack: true,
        }),
        _ => None,
    }
}

impl AioTeleportCycle {
    pub fn observe(&mut self, spec: AioTeleportSpec, baseline: &Observation, now: &Observation) {
        let AioTeleportSpec {
            landing,
            restock,
            staff,
            air_from_pack,
        } = spec;
        if self.teleported.is_none()
            && now.skill_xp("magic") > baseline.skill_xp("magic")
            && now.item_id(LAW_RUNE_ID) < baseline.item_id(LAW_RUNE_ID)
            && near(now.tile, landing, 8)
            && staff.is_none_or(|id| now.equipment_id(id) >= 1)
            && (!air_from_pack || now.item_id(AIR_RUNE_ID) < baseline.item_id(AIR_RUNE_ID))
        {
            self.teleported = Some(now.clone());
        }
        if self.teleported.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && near(now.tile, restock, 8)
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(LAW_RUNE_ID) > deposited.item_id(LAW_RUNE_ID)
                && now.bank_item_id(LAW_RUNE_ID) < deposited.bank_item_id(LAW_RUNE_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.closed |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation;
        }
        if let Some(teleported) = &self.teleported {
            if let Some(restocked) = &self.restocked {
                self.further |= self.closed
                    && !now.bank_open
                    && now.skill_xp("magic") > teleported.skill_xp("magic")
                    && now.item_id(LAW_RUNE_ID) < restocked.item_id(LAW_RUNE_ID);
            }
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.teleported.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
    }
}

/// Posted shop stock down, matching inv up, coins down, then deposit except
/// coins, retain or top up funding, and a second buy. Queued if-button without
/// stock movement fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ShopFunding {
    Retained {
        carried_coins: i32,
    },
    TopUp {
        carried_before: i32,
        carried_after: i32,
        bank_before: i32,
        bank_after: i32,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ShopBuyoutCycle {
    pub opened: Option<Observation>,
    pub bought: Option<Observation>,
    pub bought_id: Option<i32>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    /// Explicitly distinguishes already-retained coins from a bank withdrawal.
    pub funding: Option<ShopFunding>,
    pub returned: bool,
    pub reopened: Option<Observation>,
    pub further: bool,
    last_tick: Option<u32>,
    /// Peak carried product after the first purchase, until deposit. Deposit
    /// must bank this earned load, not the first 10/5/1 inventory delta.
    pub earned_quantity: i32,
    /// Product count on the first loaded bank session of this trip while the
    /// pack still held that product. None if that first loaded bank already
    /// had an empty pack: fail closed rather than invent a pre-deposit count.
    pub trip_bank_product: Option<i32>,
}

pub struct ShopBuyoutSpec {
    pub stand: (i32, i32, i32),
    pub stand_radius: i32,
    pub restock: (i32, i32, i32),
}

pub fn shop_buyout_spec(case: CoreCase) -> Option<ShopBuyoutSpec> {
    match case {
        CoreCase::ShopBuyout => Some(ShopBuyoutSpec {
            stand: AEMAD_STAND,
            stand_radius: 6,
            restock: ARDOUGNE_EAST_BANK,
        }),
        CoreCase::ShopBuyoutAubury => Some(ShopBuyoutSpec {
            stand: AUBURY_STAND,
            stand_radius: 6,
            restock: VARROCK_EAST_BANK,
        }),
        CoreCase::ShopBuyoutLowe => Some(ShopBuyoutSpec {
            stand: LOWE_STAND,
            stand_radius: 6,
            restock: VARROCK_EAST_BANK,
        }),
        CoreCase::ShopBuyoutHickton => Some(ShopBuyoutSpec {
            stand: HICKTON_STAND,
            stand_radius: 6,
            restock: CATHERBY_BANK,
        }),
        CoreCase::ShopBuyoutHarry => Some(ShopBuyoutSpec {
            stand: HARRY_STAND,
            stand_radius: 6,
            restock: CATHERBY_BANK,
        }),
        CoreCase::ShopBuyoutBetty => Some(ShopBuyoutSpec {
            stand: BETTY_STAND,
            stand_radius: 6,
            restock: FALADOR_WEST_BANK,
        }),
        CoreCase::ShopBuyoutGerrant => Some(ShopBuyoutSpec {
            stand: GERRANT_STAND,
            stand_radius: 6,
            restock: DRAYNOR_BANK,
        }),
        _ => None,
    }
}

fn shop_bought_id(
    before: &Observation,
    now: &Observation,
    stand: (i32, i32, i32),
    stand_radius: i32,
) -> Option<(i32, i32)> {
    if !now.shop_open
        || now.main_modal != SHOPMAIN
        || now.shop_stock.is_empty()
        || !near(now.tile, stand, stand_radius)
    {
        return None;
    }
    if now.item_id(COINS_ID) >= before.item_id(COINS_ID) {
        return None;
    }
    let mut found = None;
    for row in &before.shop_stock {
        if now.shop_item_id(row.id) < row.count && now.item_id(row.id) > before.item_id(row.id) {
            if row.id == EMPTY_VIAL_ID {
                return Some((
                    EMPTY_VIAL_ID,
                    now.item_id(EMPTY_VIAL_ID) - before.item_id(EMPTY_VIAL_ID),
                ));
            }
            if found.is_none() {
                found = Some((row.id, now.item_id(row.id) - before.item_id(row.id)));
            }
        }
    }
    found
}

/// First-purchase replay: equal shop stock, carried product, and coins.
fn shop_same_purchase(left: &Observation, right: &Observation, id: i32) -> bool {
    left.item_id(id) == right.item_id(id)
        && left.item_id(COINS_ID) == right.item_id(COINS_ID)
        && left.shop_stock == right.shop_stock
}

impl ShopBuyoutCycle {
    pub fn observe(&mut self, spec: ShopBuyoutSpec, baseline: &Observation, now: &Observation) {
        let ShopBuyoutSpec {
            stand,
            stand_radius,
            restock,
        } = spec;
        // Inv/shop/bank packets can publish later frames in the same game tick.
        // Reject only backwards ticks; identical no-delta replays must still
        // fail the stage predicates below rather than this cadence gate.
        if now.tick < self.last_tick.unwrap_or(baseline.tick) {
            return;
        }
        self.last_tick = Some(now.tick);
        if self.opened.is_none()
            && now.shop_open
            && now.main_modal == SHOPMAIN
            && near(now.tile, stand, stand_radius)
            && !now.shop_stock.is_empty()
        {
            self.opened = Some(now.clone());
        }
        if let Some(opened) = &self.opened {
            if self.bought.is_none() {
                if let Some((id, _)) = shop_bought_id(opened, now, stand, stand_radius) {
                    self.bought_id = Some(id);
                    self.earned_quantity = now.item_id(id);
                    self.bought = Some(now.clone());
                }
            }
        }
        if let Some(id) = self.bought_id {
            if self.deposited.is_none() {
                self.earned_quantity = self.earned_quantity.max(now.item_id(id));
            }
            // First loaded bank of this trip: latch the product count only
            // while the pack still holds the earned product.
            if self.trip_bank_product.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation > baseline.bank_generation
                && near(now.tile, restock, 8)
                && now.item_id(id) > 0
            {
                self.trip_bank_product = Some(now.bank_item_id(id));
            }
            if self.deposited.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation > baseline.bank_generation
                && near(now.tile, restock, 8)
                && now.item_id(id) == 0
                && self.earned_quantity >= 1
                && self
                    .trip_bank_product
                    .is_some_and(|before| now.bank_item_id(id) >= before + self.earned_quantity)
                && now.item_id(COINS_ID) >= 1
            {
                self.deposited = Some(now.clone());
            }
        }
        // Retained funding is recordable on the deposit observation when
        // carried and bank coins did not move, so a later open-bank tick is
        // not required. A real withdrawal in the same session overwrites as
        // TopUp so retained coins are not mislabeled as withdrawn.
        if let Some(deposited) = &self.deposited {
            if now.bank_open && now.bank_loaded && now.bank_generation == deposited.bank_generation
            {
                let carried_now = now.item_id(COINS_ID);
                let carried_then = deposited.item_id(COINS_ID);
                let bank_now = now.bank_item_id(COINS_ID);
                let bank_then = deposited.bank_item_id(COINS_ID);
                if carried_now > carried_then && bank_now < bank_then {
                    self.funding = Some(ShopFunding::TopUp {
                        carried_before: carried_then,
                        carried_after: carried_now,
                        bank_before: bank_then,
                        bank_after: bank_now,
                    });
                    self.restocked = Some(now.clone());
                } else if self.funding.is_none()
                    && carried_now == carried_then
                    && bank_now == bank_then
                    && carried_now >= 1
                {
                    self.funding = Some(ShopFunding::Retained {
                        carried_coins: carried_now,
                    });
                    self.restocked = Some(now.clone());
                }
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, stand, stand_radius);
        }
        if self.returned
            && self.reopened.is_none()
            && now.shop_open
            && now.main_modal == SHOPMAIN
            && near(now.tile, stand, stand_radius)
        {
            self.reopened = Some(now.clone());
        }
        if let Some(reopened) = &self.reopened {
            let stale = match (self.bought.as_ref(), self.bought_id) {
                (Some(bought), Some(id)) => shop_same_purchase(bought, now, id),
                _ => false,
            };
            self.further |= self.funding.is_some()
                && now.tick > reopened.tick
                && !stale
                && shop_bought_id(reopened, now, stand, stand_radius).is_some();
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.opened.is_some()
            && self.bought.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && self.funding.is_some()
    }
}

/// ClimbingBoots (frozen `ClimbingBoots.ts`) at Tenzing's hut. The buy is
/// only a purchase witness once the framed Tenzing projection and the sherpa
/// dialogue are in the observed window and carried boots rose while coins
/// fell by exactly 12 a pair. Walk and teleport are separate cells: the
/// teleport cell must show a real post-purchase return cast (magic XP, Law
/// spend, landing, still carrying the earned pack) and the walk cell must
/// not cast at all. Return, deposit, reopen and the further purchase stay
/// separate stages, so a failed full cycle keeps its purchase evidence
/// instead of silently downgrading to a smoke PASS.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ClimbingBootsCycle {
    pub tenzing: Option<Observation>,
    pub cast: Option<Observation>,
    pub bought: Option<Observation>,
    /// Pairs gained at the first `bought` sample (each pair is exactly 12 coins).
    pub bought_pairs: i32,
    /// Peak carried boots after that purchase, until deposit. Deposit must
    /// bank this earned quantity, not the early 1-pair sample.
    pub earned_boots: i32,
    /// First loaded post-Start bank: `Some(true)` only when boots 3105 were 0.
    pub boot_bank_empty: Option<bool>,
    /// The sherpa sell/buy line seen in a bounded chat projection.
    pub dialogue: bool,
    pub returned: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub departed: Option<Observation>,
    pub further: bool,
}

pub struct ClimbingBootsSpec {
    /// Explicit typed fixture option, not the frozen default.
    pub use_teleport: bool,
    /// Exact `readyToBuy` trip money: 12 coins a pair for the whole pack.
    pub coins: i32,
    /// Falador landing the teleport branch must reach after a real cast.
    pub landing: (i32, i32, i32),
    /// Falador West bank stand the walk returns through.
    pub bank: (i32, i32, i32),
}

pub fn climbing_boots_spec(case: CoreCase) -> Option<ClimbingBootsSpec> {
    match case {
        // Walking cell: 28 boots * 12 coins, no runes, no Magic requirement.
        CoreCase::ClimbingBoots => Some(ClimbingBootsSpec {
            use_teleport: false,
            coins: CLIMBING_BOOTS_WALK_PACK_COINS,
            landing: FALADOR_TELE_LAND,
            bank: FALADOR_WEST_BANK,
        }),
        // Teleport cell: the `tripQty` is 28 minus the carried rune stacks
        // (Law, Air, Water), so the ready-to-buy money is 25 pairs.
        CoreCase::ClimbingBootsTeleport => Some(ClimbingBootsSpec {
            use_teleport: true,
            coins: CLIMBING_BOOTS_TELE_PACK_COINS,
            landing: FALADOR_TELE_LAND,
            bank: FALADOR_WEST_BANK,
        }),
        _ => None,
    }
}

/// The real framed Tenzing NPC near the hut inside tile. Route coordinates
/// and quest state never stand in for this projection.
fn tenzing_visible(now: &Observation) -> bool {
    now.npc_facts.iter().any(|npc| {
        npc.name.as_deref() == Some(TENZING_NAME) && near(Some(npc.tile), TENZING_INSIDE, 12)
    })
}

/// The sherpa sale line or the purchase confirmation, from the bounded chat
/// projection (`death_sherpa.rs2`: "for 12 gold" then "Tenzing has given you
/// some Climbing boots.").
fn climbing_boots_dialogue(now: &Observation) -> bool {
    now.chat
        .iter()
        .any(|(_, text)| text.contains("Climbing boots"))
}

impl ClimbingBootsCycle {
    pub fn observe(&mut self, spec: ClimbingBootsSpec, baseline: &Observation, now: &Observation) {
        let ClimbingBootsSpec {
            use_teleport,
            coins: _,
            landing,
            bank,
        } = spec;
        self.dialogue |= climbing_boots_dialogue(now);
        if self.tenzing.is_none() && tenzing_visible(now) {
            self.tenzing = Some(now.clone());
        }
        // The purchase: same player after Start, boots up, exactly 12 coins a
        // pair gone, with the framed Tenzing/dialogue evidence. The first
        // matching frame is typically one pair; earned_boots tracks the peak
        // pack after that until deposit.
        if self.tenzing.is_some() && self.dialogue {
            if self.bought.is_none() {
                let gained = now.item_id(CLIMBING_BOOTS_ID) - baseline.item_id(CLIMBING_BOOTS_ID);
                let spent = baseline.item_id(COINS_ID) - now.item_id(COINS_ID);
                if gained >= 1 && spent == gained * CLIMBING_BOOTS_PAIR_COINS {
                    self.bought_pairs = gained;
                    self.earned_boots = gained;
                    self.bought = Some(now.clone());
                }
            } else if self.deposited.is_none() {
                self.earned_boots = self.earned_boots.max(now.item_id(CLIMBING_BOOTS_ID));
            }
        }
        // A real cast, not the option: magic XP plus the whole Falador cost
        // (Law, Air and Water, no staff assumed) and the landing. Only after
        // the purchase, on a frame that still carries the earned pack, so a
        // pre-purchase Falador-shaped cast cannot satisfy the teleport cell.
        // Compare with the purchase snapshot: Start-relative deltas could
        // count an earlier cast when the earned pack later walks through here.
        // Recorded for either cell, so a walking cell that cast fails closed.
        if self.cast.is_none()
            && self.earned_boots >= 1
            && now.item_id(CLIMBING_BOOTS_ID) >= self.earned_boots
            && self.bought.as_ref().is_some_and(|bought| {
                now.skill_xp("magic") > bought.skill_xp("magic")
                    && now.item_id(LAW_RUNE_ID) < bought.item_id(LAW_RUNE_ID)
                    && now.item_id(AIR_RUNE_ID) < bought.item_id(AIR_RUNE_ID)
                    && now.item_id(WATER_RUNE_ID) < bought.item_id(WATER_RUNE_ID)
            })
            && near(now.tile, landing, 8)
        {
            self.cast = Some(now.clone());
        }
        // Return is genuine position after the purchase (the first arrival can
        // still be inside the baseline bank generation), never a generation
        // counter that would demand a second bank trip. Teleport return must
        // follow the post-purchase cast, still carrying the earned pack.
        if self.bought.is_some()
            && self.returned.is_none()
            && !now.bank_open
            && near(now.tile, bank, 8)
            && self.earned_boots >= 1
            && now.item_id(CLIMBING_BOOTS_ID) >= self.earned_boots
            && (!use_teleport || self.cast.is_some())
        {
            self.returned = Some(now.clone());
        }
        if now.bank_open && now.bank_loaded && self.boot_bank_empty.is_none() {
            self.boot_bank_empty = Some(now.bank_item_id(CLIMBING_BOOTS_ID) == 0);
        }
        // Deposit the earned peak against a verified empty boot bank, not the
        // first 1-pair bought sample and not leftover bank stock.
        if self.returned.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && self.boot_bank_empty == Some(true)
            && now.bank_generation > baseline.bank_generation
            && self.earned_boots >= 1
        {
            let carried_lost = self.earned_boots - now.item_id(CLIMBING_BOOTS_ID);
            let bank_gained = now.bank_item_id(CLIMBING_BOOTS_ID);
            if carried_lost >= self.earned_boots && bank_gained >= self.earned_boots {
                self.deposited = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(COINS_ID) > deposited.item_id(COINS_ID)
                && now.bank_item_id(COINS_ID) < deposited.bank_item_id(COINS_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        // Departure: a later bank session closed with the restocked pack.
        if let Some(restocked) = &self.restocked {
            if self.departed.is_none()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > restocked.bank_generation
                && now.item_id(COINS_ID) >= CLIMBING_BOOTS_PAIR_COINS
            {
                self.departed = Some(now.clone());
            }
        }
        // A further pair bought back at Tenzing's hut after the departure.
        if let Some(departed) = &self.departed {
            let gained = now.item_id(CLIMBING_BOOTS_ID) - departed.item_id(CLIMBING_BOOTS_ID);
            let spent = departed.item_id(COINS_ID) - now.item_id(COINS_ID);
            self.further |= gained >= 1
                && spent == gained * CLIMBING_BOOTS_PAIR_COINS
                && near(now.tile, TENZING_HUT_DOOR, 12);
        }
    }

    /// The honest intermediate state: a real purchase with its Tenzing and
    /// dialogue evidence but no claim of return, deposit or further work.
    pub fn purchase_smoke(&self) -> bool {
        self.tenzing.is_some() && self.dialogue && self.bought.is_some()
    }

    pub fn qualified(&self, spec: ClimbingBootsSpec) -> bool {
        let branch = if spec.use_teleport {
            self.cast.is_some()
        } else {
            // A walking cell must not silently run the teleport branch.
            self.cast.is_none()
        };
        branch
            && self.purchase_smoke()
            && self.returned.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && self.departed.is_some()
            && self.further
    }
}

/// Anvil main-panel production (not chat make), then deposit except hammer,
/// restock bars, and further smithing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SmithingBotCycle {
    pub panel: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
}

pub struct SmithingBotSpec {
    pub product: i32,
    pub wrong: i32,
    pub bars_per: i32,
}

pub fn smithing_bot_spec(case: CoreCase) -> Option<SmithingBotSpec> {
    match case {
        CoreCase::SmithingBot => Some(SmithingBotSpec {
            product: BRONZE_DAGGER_ID,
            wrong: BRONZE_PLATEBODY_ID,
            bars_per: 1,
        }),
        CoreCase::SmithingBotPlatebody => Some(SmithingBotSpec {
            product: BRONZE_PLATEBODY_ID,
            wrong: BRONZE_DAGGER_ID,
            bars_per: 5,
        }),
        _ => None,
    }
}

impl SmithingBotCycle {
    pub fn observe(&mut self, spec: SmithingBotSpec, baseline: &Observation, now: &Observation) {
        let SmithingBotSpec {
            product,
            wrong,
            bars_per,
        } = spec;
        self.wrong_product |= now.item_id(wrong) > 0 || now.bank_item_id(wrong) > 0;
        if self.panel.is_none()
            && now.has_main_make(product)
            && near(now.tile, VARROCK_ANVIL, 8)
            && now.item_id(BRONZE_BAR_ID) >= bars_per
            && now.item_id(product) == 0
            && now.item_id(HAMMER_ID) >= 1
        {
            self.panel = Some(now.clone());
        }
        if let Some(panel) = &self.panel {
            if self.produced.is_none()
                && near(now.tile, VARROCK_ANVIL, 8)
                && now.item_id(product) >= 1
                && now.item_id(BRONZE_BAR_ID) <= panel.item_id(BRONZE_BAR_ID) - bars_per
                && now.skill_xp("smithing") > baseline.skill_xp("smithing")
                && now.item_id(wrong) == 0
            {
                self.produced = Some(now.clone());
            }
        }
        if self.produced.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
            && now.item_id(HAMMER_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(BRONZE_BAR_ID) >= bars_per
                && now.bank_item_id(BRONZE_BAR_ID) < deposited.bank_item_id(BRONZE_BAR_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, VARROCK_ANVIL, 8);
        }
        if let Some(produced) = &self.produced {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(product) >= 1
                && now.skill_xp("smithing") > produced.skill_xp("smithing")
                && now.item_id(wrong) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.panel.is_some()
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
    }
}

/// Soft-leather gloves wait the selected leather interface; hard-leather
/// body is the no-modal `flow=single` burst.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LeatherCrafterCycle {
    pub interface: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub used_interface: bool,
    pub wrong_product: bool,
}

pub struct LeatherCrafterSpec {
    pub product: i32,
    pub input: i32,
    pub wrong: i32,
    pub require_interface: bool,
}

pub fn leather_crafter_spec(case: CoreCase) -> Option<LeatherCrafterSpec> {
    match case {
        CoreCase::LeatherCrafter => Some(LeatherCrafterSpec {
            product: LEATHER_GLOVES_ID,
            input: SOFT_LEATHER_ID,
            wrong: HARDLEATHER_BODY_ID,
            require_interface: true,
        }),
        CoreCase::LeatherCrafterHardBody => Some(LeatherCrafterSpec {
            product: HARDLEATHER_BODY_ID,
            input: HARD_LEATHER_ID,
            wrong: LEATHER_GLOVES_ID,
            require_interface: false,
        }),
        _ => None,
    }
}

impl LeatherCrafterCycle {
    pub fn observe(&mut self, spec: LeatherCrafterSpec, baseline: &Observation, now: &Observation) {
        let LeatherCrafterSpec {
            product,
            input,
            wrong,
            require_interface,
        } = spec;
        self.wrong_product |= now.item_id(wrong) > 0 || now.bank_item_id(wrong) > 0;
        if now.main_modal == LEATHER_IF && now.has_widget(LEATHER_GLOVES_MAKE10) {
            self.used_interface = true;
            if self.interface.is_none()
                && now.item_id(input) >= 1
                && now.item_id(product) == 0
                && now.item_id(NEEDLE_ID) >= 1
            {
                self.interface = Some(now.clone());
            }
        }
        if self.withdrawn.is_none()
            && now.item_id(input) >= 1
            && now.item_id(product) == 0
            && now.item_id(NEEDLE_ID) >= 1
            && baseline.item_id(input) == 0
        {
            self.withdrawn = Some(now.clone());
        }
        let prior = if require_interface {
            self.interface.as_ref()
        } else {
            self.withdrawn.as_ref().filter(|_| !self.used_interface)
        };
        if let Some(prior) = prior {
            if self.produced.is_none()
                && now.item_id(product) >= 1
                && now.item_id(input) < prior.item_id(input)
                && now.skill_xp("crafting") > baseline.skill_xp("crafting")
                && now.item_id(wrong) == 0
            {
                self.produced = Some(now.clone());
            }
        }
        if self.produced.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
            && now.item_id(NEEDLE_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(input) >= 1
                && now.bank_item_id(input) < deposited.bank_item_id(input)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation;
        }
        if let Some(produced) = &self.produced {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(product) >= 1
                && now.skill_xp("crafting") > produced.skill_xp("crafting")
                && now.item_id(wrong) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
    }
}

/// Firemaking XP plus a Fire loc inside the posted Varrock East AABB, then
/// deposit except tinderbox, restock logs, and another light.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FiremakerCycle {
    pub withdrawn: Option<Observation>,
    pub lit: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_log: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct FiremakerSpec {
    pub log: i32,
    pub wrong: i32,
}

pub fn firemaker_spec(case: CoreCase) -> Option<FiremakerSpec> {
    match case {
        CoreCase::Firemaker => Some(FiremakerSpec {
            log: LOGS_ID,
            wrong: OAK_LOGS_ID,
        }),
        CoreCase::FiremakerOak => Some(FiremakerSpec {
            log: OAK_LOGS_ID,
            wrong: LOGS_ID,
        }),
        _ => None,
    }
}

impl FiremakerCycle {
    pub fn observe(&mut self, spec: FiremakerSpec, baseline: &Observation, now: &Observation) {
        let FiremakerSpec { log, wrong } = spec;
        self.wrong_log |= now.item_id(wrong) > 0 || now.bank_item_id(wrong) > 0;
        if self.withdrawn.is_none()
            && now.item_id(log) >= 1
            && now.item_id(TINDERBOX_ID) >= 1
            && baseline.item_id(log) == 0
            && !fire_in_varrock_east_plot(now)
        {
            self.withdrawn = Some(now.clone());
        }
        if let Some(withdrawn) = &self.withdrawn {
            if self.lit.is_none()
                && now.skill_xp("firemaking") > baseline.skill_xp("firemaking")
                && now.item_id(log) < withdrawn.item_id(log)
                && fire_in_varrock_east_plot(now)
            {
                self.lit = Some(now.clone());
            }
        }
        if self.lit.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(log) == 0
            && now.item_id(TINDERBOX_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(log) >= 1
                && now.bank_item_id(log) < deposited.bank_item_id(log)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation;
        }
        if let (Some(_lit), Some(restocked)) = (&self.lit, &self.restocked) {
            self.further |= self.returned
                && !now.bank_open
                && now.tick > restocked.tick
                && now.item_id(log) < restocked.item_id(log)
                && now.skill_xp("firemaking") > restocked.skill_xp("firemaking")
                && fire_in_varrock_east_plot(now);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.withdrawn.is_some()
            && self.lit.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_log
    }
}

impl CoreWitness {
    pub fn new(case: CoreCase, baseline: Observation) -> Self {
        Self::new_with_start_preparation(case, baseline, None)
    }

    pub fn new_with_start_preparation(
        case: CoreCase,
        baseline: Observation,
        start_preparation: Option<HerbCleanerStartPreparationReceipt>,
    ) -> Self {
        Self {
            max_items: baseline.items.clone(),
            max_xp: baseline.xp.clone(),
            latest: baseline.clone(),
            baseline,
            start_preparation,
            case,
            saw_bury_chat: false,
            post_start_observations: 0,
            bone_bank_cycle: BoneBankCycle::default(),
            bank_fletcher_cycle: BankFletcherCycle::default(),
            bank_fletcher_option_cycle: BankFletcherOptionCycle::default(),
            bank_fletcher_string_cycle: BankFletcherStringCycle::default(),
            bank_fletcher_cut_string_cycle: BankFletcherCutStringCycle::default(),
            alcher_defaults_cycle: AlcherGeneratedCustomCycle::default(),
            alcher_generated_custom_cycle: AlcherGeneratedCustomCycle::default(),
            alcher_spell_cycle: AlcherGeneratedCustomCycle::default(),
            alcher_swarm_cycle: AlcherSwarmDrainCycle::default(),
            dart_fletcher_cycle: DartFletcherCycle::default(),
            herb_cleaner_cycle: HerbCleanerCycle::default(),
            herb_cleaner_empty_cycle: HerbCleanerEmptyCycle::default(),
            gem_cutter_cycle: GemCutterCycle::default(),
            door_opener_cycle: DoorOpenerCycle::default(),
            gnome_course_cycle: GnomeCourseCycle::default(),
            wildy_agility_cycle: WildyAgilityCycle::default(),
            brimhaven_agility_cycle: BrimhavenAgilityCycle::default(),
            flax_picker_cycle: FlaxPickerCycle::default(),
            superheater_cycle: SuperheaterCycle::default(),
            chicken_killer_bank_cycle: ChickenKillerBankCycle::default(),
            vial_filler_cycle: VialFillerCycle::default(),
            potion_maker_cycle: PotionMakerCycle::default(),
            tanner_bot_cycle: TannerBotCycle::default(),
            rune_crafter_cycle: RuneCrafterCycle::default(),
            ardy_cakes_cycle: ArdyCakesCycle::default(),
            ardy_cakes_fight_cycle: ArdyCakesFightCycle::default(),
            ardy_thiever_cycle: ArdyThieverCycle::default(),
            ardy_thiever_fight_cycle: ArdyThieverFightCycle::default(),
            gnome_chop_cycle: GnomeChopCycle::default(),
            gnome_fletch_cycle: GnomeFletchCycle::default(),
            coal_trucks_cycle: CoalTrucksCycle::default(),
            station_production_cycle: StationProductionCycle::default(),
            flax_aio_cycle: FlaxAioCycle::default(),
            flax_aio_pick_cycle: FlaxAioPickCycle::default(),
            herblore_eggs_cycle: HerbloreEggsCycle::default(),
            herblore_newt_cycle: HerbloreNewtCycle::default(),
            combat_core_cycle: CombatCoreCycle::default(),
            combat_dart_branch_cycle: CombatDartBranchCycle::default(),
            combat_bank_cycle: CombatBankCycle::default(),
            combat_approach_cycle: CombatApproachCycle::default(),
            aio_teleport_cycle: AioTeleportCycle::default(),
            shop_buyout_cycle: ShopBuyoutCycle::default(),
            smithing_bot_cycle: SmithingBotCycle::default(),
            leather_crafter_cycle: LeatherCrafterCycle::default(),
            firemaker_cycle: FiremakerCycle::default(),
            climbing_boots_cycle: ClimbingBootsCycle::default(),
            ranging_guild_round_cycle: RangingGuildRoundCycle::default(),
            ranging_guild_redeem_cycle: RangingGuildRedeemCycle::default(),
            ranging_guild_bank_cycle: RangingGuildBankCycle::default(),
            ranging_guild_full_cycle: RangingGuildFullCycle::default(),
            brimhaven_moss_inspect_cycle: BrimhavenMossInspectCycle::default(),
            route_inspect_brimhaven_v2_cycle: RouteInspectBrimhavenV2Cycle::default(),
            prayer_delivery_cycle: PrayerDeliveryCycle::default(),
            line_of_sight_cycle: LineOfSightDeliveryCycle::default(),
            actor_observation_cycle: ActorObservationDeliveryCycle::default(),
            fight_field_cycle: FightFieldDeliveryCycle::default(),
            hunt_cycle: HuntDeliveryCycle::default(),
            ordered_first_exhausted: false,
        }
    }

    pub fn observe(&mut self, observation: &Observation) {
        if !observation.ingame
            || observation.scene_state != 2
            || observation.player != self.baseline.player
        {
            return;
        }
        if matches!(self.case, CoreCase::BoneBurier) {
            self.bone_bank_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BankFletcher) {
            self.bank_fletcher_cycle
                .observe(&self.baseline, observation);
        }
        if let Some(spec) = bank_fletcher_option_spec(self.case) {
            self.bank_fletcher_option_cycle
                .observe(spec, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BankFletcherString) {
            self.bank_fletcher_string_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BankFletcherCutString) {
            self.bank_fletcher_cut_string_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(
            self.case,
            CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName
        ) {
            self.alcher_generated_custom_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::AlcherDefaults) {
            self.alcher_defaults_cycle.observe_target(
                &self.baseline,
                observation,
                YEW_LONGBOW_ID,
                CERT_YEW_LONGBOW_ID,
                YEW_LONGBOW_ALCH_COINS,
            );
        }
        if matches!(
            self.case,
            CoreCase::AlcherLow | CoreCase::AlcherFireBattlestaff
        ) {
            let expectation = match self.case {
                CoreCase::AlcherLow => ALCHER_LOW_EXPECTATION,
                _ => ALCHER_FIRE_BATTLESTAFF_EXPECTATION,
            };
            self.alcher_spell_cycle.observe_spelled(
                expectation,
                &self.baseline,
                observation,
                RUNE_CHAINBODY_ID,
                CERT_RUNE_CHAINBODY_ID,
            );
        }
        if matches!(self.case, CoreCase::AlcherSwarmDrain) {
            self.alcher_swarm_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::DartFletcher) {
            self.dart_fletcher_cycle.observe(
                BRONZE_DART_TIP_ID,
                FEATHER_ID,
                BRONZE_DART_ID,
                IRON_DART_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::DartFletcherIron) {
            self.dart_fletcher_cycle.observe(
                IRON_DART_TIP_ID,
                FEATHER_ID,
                IRON_DART_ID,
                BRONZE_DART_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(
            self.case,
            CoreCase::HerbCleaner | CoreCase::HerbCleanerNamed
        ) {
            self.herb_cleaner_cycle.observe(
                matches!(self.case, CoreCase::HerbCleanerNamed),
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::HerbCleanerEmptyBank) {
            self.herb_cleaner_empty_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::GemCutter | CoreCase::GemCutterNamed) {
            self.gem_cutter_cycle.observe(
                matches!(self.case, CoreCase::GemCutterNamed),
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::DoorOpener) {
            self.door_opener_cycle.observe(
                false,
                LUMBRIDGE_DOOR,
                WOODEN_DOOR_CLOSED_ID,
                WOODEN_DOOR_OPEN_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::DoorOpenerGate) {
            self.door_opener_cycle.observe(
                true,
                LUMBRIDGE_GATE,
                WOODEN_GATE_CLOSED_ID,
                WOODEN_GATE_OPEN_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(
            self.case,
            CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius
        ) {
            self.gnome_course_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::WildyAgility) {
            self.wildy_agility_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BrimhavenAgility) {
            self.brimhaven_agility_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FlaxPicker) {
            self.flax_picker_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FlaxAio) {
            self.flax_aio_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FlaxAioPick) {
            self.flax_aio_pick_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::HerbloreSecondaries) {
            self.herblore_eggs_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::HerbloreSecondariesNewt) {
            self.herblore_newt_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::MossGiantDart) {
            if let Some(spec) = combat_spec(self.case) {
                self.combat_dart_branch_cycle
                    .observe(spec, &self.baseline, observation);
            }
        } else if let Some(spec) = combat_spec(self.case) {
            self.combat_core_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let (Some(spec), Some(bank)) = (combat_spec(self.case), combat_bank_spec(self.case)) {
            self.combat_bank_cycle
                .observe(spec, bank, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::FireGiantApproach) {
            if let Some(spec) = combat_spec(self.case) {
                self.combat_approach_cycle
                    .observe(spec, &self.baseline, observation);
            }
        }
        if let Some(spec) = aio_teleport_spec(self.case) {
            self.aio_teleport_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = shop_buyout_spec(self.case) {
            self.shop_buyout_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = smithing_bot_spec(self.case) {
            self.smithing_bot_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = leather_crafter_spec(self.case) {
            self.leather_crafter_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = firemaker_spec(self.case) {
            self.firemaker_cycle
                .observe(spec, &self.baseline, observation);
        }
        if let Some(spec) = climbing_boots_spec(self.case) {
            self.climbing_boots_cycle
                .observe(spec, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildRound) {
            self.ranging_guild_round_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildRedeem) {
            self.ranging_guild_redeem_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildBank) {
            self.ranging_guild_bank_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RangingGuildFull) {
            self.ranging_guild_full_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::BrimhavenMossInspectV1) {
            self.brimhaven_moss_inspect_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::RouteInspectBrimhavenV2) {
            self.route_inspect_brimhaven_v2_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::PrayerV2 | CoreCase::PrayerV1) {
            self.prayer_delivery_cycle.observe(observation);
        }
        if matches!(self.case, CoreCase::LineOfSightV2) {
            self.line_of_sight_cycle.observe(observation);
        }
        if matches!(self.case, CoreCase::ActorObservationV2) {
            self.actor_observation_cycle.observe(observation);
        }
        if matches!(self.case, CoreCase::FightFieldV2) {
            self.fight_field_cycle.observe(observation);
        }
        if let Some(cell) = self.case.hunt_cell() {
            self.hunt_cycle.observe(cell, &self.baseline, observation);
        }
        if matches!(self.case, CoreCase::Superheater) {
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: BRONZE_BAR_ID,
                    primary: COPPER_ORE_ID,
                    secondary: TIN_ORE_ID,
                    staff: STAFF_OF_FIRE_ID,
                    steel: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::SuperheaterSteel) {
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: STEEL_BAR_ID,
                    primary: IRON_ORE_ID,
                    secondary: COAL_ID,
                    staff: STAFF_OF_FIRE_ID,
                    steel: true,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::SuperheaterFireBattlestaff) {
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: BRONZE_BAR_ID,
                    primary: COPPER_ORE_ID,
                    secondary: TIN_ORE_ID,
                    staff: FIRE_BATTLESTAFF_ID,
                    steel: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::SuperheaterSilverLowNatures) {
            // Single-ore: primary == secondary so the pair-ratio check is a no-op.
            self.superheater_cycle.observe(
                SuperheaterSpec {
                    bar: SILVER_BAR_ID,
                    primary: SILVER_ORE_ID,
                    secondary: SILVER_ORE_ID,
                    staff: STAFF_OF_FIRE_ID,
                    steel: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::ChickenKillerBank) {
            self.chicken_killer_bank_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::VialFiller | CoreCase::VialFillerEast) {
            self.vial_filler_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::PotionMaker) {
            self.potion_maker_cycle.observe(
                PotionMakerSpec {
                    herb: GUAM_LEAF_ID,
                    unf: GUAM_UNF_ID,
                    secondary: EYE_OF_NEWT_ID,
                    finished: ATTACK_POTION_3_ID,
                    wrong_unf: RANARR_UNF_ID,
                    wrong_finished: PRAYER_POTION_3_ID,
                    named: false,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::PotionMakerNamed) {
            self.potion_maker_cycle.observe(
                PotionMakerSpec {
                    herb: RANARR_WEED_ID,
                    unf: RANARR_UNF_ID,
                    secondary: SNAPE_GRASS_ID,
                    finished: PRAYER_POTION_3_ID,
                    wrong_unf: GUAM_UNF_ID,
                    wrong_finished: ATTACK_POTION_3_ID,
                    named: true,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::TannerBot) {
            self.tanner_bot_cycle.observe(
                TannerBotSpec {
                    product: SOFT_LEATHER_ID,
                    wrong_product: HARD_LEATHER_ID,
                    tan_all: SOFT_TAN_ALL_COM,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::TannerBotHard) {
            self.tanner_bot_cycle.observe(
                TannerBotSpec {
                    product: HARD_LEATHER_ID,
                    wrong_product: SOFT_LEATHER_ID,
                    tan_all: HARD_TAN_ALL_COM,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::RuneCrafter) {
            self.rune_crafter_cycle.observe(
                RuneCrafterSpec {
                    rune: AIR_RUNE_ID,
                    wrong_rune: EARTH_RUNE_ID,
                    ruins: RUNECRAFTER_AIR_RUINS,
                    bank: FALADOR_EAST_BANK,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::RuneCrafterEarth) {
            self.rune_crafter_cycle.observe(
                RuneCrafterSpec {
                    rune: EARTH_RUNE_ID,
                    wrong_rune: AIR_RUNE_ID,
                    ruins: RUNECRAFTER_EARTH_RUINS,
                    bank: VARROCK_EAST_BANK,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::MuleCrafter) {
            self.rune_crafter_cycle.observe(
                RuneCrafterSpec {
                    rune: AIR_RUNE_ID,
                    wrong_rune: EARTH_RUNE_ID,
                    ruins: MULECRAFTER_AIR_RUINS,
                    bank: FALADOR_EAST_BANK,
                },
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::ArdyCakes) {
            self.ardy_cakes_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::ArdyCakesFight) {
            self.ardy_cakes_fight_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(
            self.case,
            CoreCase::ArdyThiever | CoreCase::ArdyThieverKnight
        ) {
            self.ardy_thiever_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::ArdyThieverFight) {
            self.ardy_thiever_fight_cycle
                .observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::GnomeChop) {
            self.gnome_chop_cycle.observe(&self.baseline, observation);
        }
        if matches!(self.case, CoreCase::GnomeFletchShort) {
            self.gnome_fletch_cycle.observe(
                UNSTRUNG_MAGIC_SHORTBOW_ID,
                UNSTRUNG_MAGIC_LONGBOW_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::GnomeFletchLong) {
            self.gnome_fletch_cycle.observe(
                UNSTRUNG_MAGIC_LONGBOW_ID,
                UNSTRUNG_MAGIC_SHORTBOW_ID,
                &self.baseline,
                observation,
            );
        }
        if matches!(self.case, CoreCase::CoalTrucks) {
            self.coal_trucks_cycle.observe(&self.baseline, observation);
        }
        if let Some(spec) = station_production_spec(self.case, observation) {
            self.station_production_cycle
                .observe(spec, &self.baseline, observation);
        }
        let baseline_sequence = self
            .baseline
            .chat
            .iter()
            .map(|(sequence, _)| *sequence)
            .max()
            .unwrap_or(i32::MIN);
        self.saw_bury_chat |= observation.chat.iter().any(|(sequence, text)| {
            *sequence > baseline_sequence && text.to_ascii_lowercase().contains("bury the bones")
        });
        for (name, count) in &observation.items {
            let peak = self.max_items.entry(name.clone()).or_insert(*count);
            *peak = (*peak).max(*count);
        }
        for (name, xp) in &observation.xp {
            let peak = self.max_xp.entry(name.clone()).or_insert(*xp);
            *peak = (*peak).max(*xp);
        }
        self.ordered_first_exhausted |= self.max_items.get("Rune platebody").copied().unwrap_or(0)
            > 0
            && observation.item("Rune platebody") == 0
            && observation.item("Rune chainbody") > 0;
        self.latest = observation.clone();
        if let Some(receipt) = observation.script_lifecycle.clone() {
            self.observe_script_lifecycle(receipt);
        }
        self.post_start_observations += 1;
    }

    /// Observe one compact, non-consuming lifecycle value at the same native
    /// publication boundary as the latest snapshot.
    pub fn observe_script_lifecycle(&mut self, receipt: script::ScriptLifecycleReceipt) {
        match self.case {
            CoreCase::HerbCleanerEmptyBank => self
                .herb_cleaner_empty_cycle
                .observe_script_lifecycle(receipt),
            CoreCase::RangingGuildFull => self
                .ranging_guild_full_cycle
                .observe_script_lifecycle(receipt),
            CoreCase::PrayerV2 | CoreCase::PrayerV1 => {
                if let Some(expected) = self.case.prayer_stop_reason() {
                    self.prayer_delivery_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::LineOfSightV2 => {
                if let Some(expected) = self.case.los_stop_reason() {
                    self.line_of_sight_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::ActorObservationV2 => {
                if let Some(expected) = self.case.actor_stop_reason() {
                    self.actor_observation_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::FightFieldV2 => {
                if let Some(expected) = self.case.fight_field_stop_reason() {
                    self.fight_field_cycle
                        .observe_script_lifecycle(receipt, expected);
                }
            }
            CoreCase::HoldSpotV2
            | CoreCase::RetreatSpotV2
            | CoreCase::WalkSpotV2
            | CoreCase::EnterLairV2
            | CoreCase::LeaveLairV2
            | CoreCase::AcquireKeyV2
            | CoreCase::CellV2
            | CoreCase::BankV2 => {
                if let Some(cell) = self.case.hunt_cell() {
                    self.hunt_cycle
                        .observe_script_lifecycle(receipt, cell.stop_reason());
                }
            }
            _ => {}
        }
    }

    fn incomplete_delta_error(&self) -> String {
        if self.case == CoreCase::AlcherSwarmDrain {
            format!(
                "{} core post-Start delta incomplete {}",
                self.case.scenario_name(),
                self.alcher_swarm_cycle.phase_status()
            )
        } else {
            format!(
                "{} core post-Start delta incomplete",
                self.case.scenario_name()
            )
        }
    }

    fn swarm_stopped_without_recovery(&self, observation: &Observation) -> Option<String> {
        if self.case != CoreCase::AlcherSwarmDrain || self.qualified() {
            return None;
        }
        let stopped = observation
            .script_lifecycle
            .as_ref()
            .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Stopped);
        stopped.then(|| {
            format!(
                "alcher_swarm_drain stopped without ordered recovery {}",
                self.alcher_swarm_cycle.phase_status()
            )
        })
    }

    pub fn peak_item(&self, name: &str) -> i32 {
        self.max_items.get(name).copied().unwrap_or(0)
    }

    pub fn xp_gained(&self, skill: &str) -> bool {
        self.latest.skill_xp(skill) > self.baseline.skill_xp(skill)
    }

    pub fn item_increased(&self, name: &str) -> bool {
        self.peak_item(name) > self.baseline.item(name)
    }

    pub fn item_consumed_from_baseline(&self, name: &str) -> bool {
        self.latest.item(name) < self.baseline.item(name)
    }

    pub fn acquired_then_consumed(&self, name: &str) -> bool {
        self.item_increased(name) && self.latest.item(name) < self.peak_item(name)
    }

    /// Cheap predicate used by headed UI polling. Evidence is serialized only
    /// once after this becomes true.
    pub fn qualified(&self) -> bool {
        if self.post_start_observations == 0 {
            return false;
        }
        match self.case {
            CoreCase::BoneBurier => {
                self.bone_bank_cycle.buried_after_withdrawal && self.saw_bury_chat
            }
            CoreCase::ChickenKiller => {
                self.xp_gained("strength")
                    && self.acquired_then_consumed("Bones")
                    && self.xp_gained("prayer")
                    && self.saw_bury_chat
            }
            CoreCase::ChickenKillerBank => self.chicken_killer_bank_cycle.qualified(),
            CoreCase::Thiever => self.xp_gained("thieving") && self.item_increased("Coins"),
            CoreCase::Alcher
            | CoreCase::AlcherCustom
            | CoreCase::AlcherOrdered
            | CoreCase::AlcherLargeBatch => {
                self.xp_gained("magic")
                    && self.acquired_then_consumed("Nature rune")
                    && self.item_increased("Coins")
                    && match self.case {
                        CoreCase::AlcherOrdered => {
                            self.peak_item("Rune platebody") >= 1
                                && self.peak_item("Rune chainbody") >= 1
                                && self.ordered_first_exhausted
                                && self.acquired_then_consumed("Rune chainbody")
                        }
                        CoreCase::AlcherLargeBatch => {
                            self.peak_item("Rune chainbody") >= 1000
                                && self.peak_item("Nature rune") >= 1000
                                && self.acquired_then_consumed("Rune chainbody")
                        }
                        _ => self.acquired_then_consumed("Rune chainbody"),
                    }
            }
            CoreCase::AlcherCustomAlias | CoreCase::AlcherCustomName => {
                self.alcher_generated_custom_cycle.consumed
            }
            CoreCase::AlcherLow | CoreCase::AlcherFireBattlestaff => {
                self.alcher_spell_cycle.qualified_spell()
            }
            CoreCase::AlcherSwarmDrain => self.alcher_swarm_cycle.qualified(),
            CoreCase::AlcherDefaults => self.alcher_defaults_cycle.consumed,
            CoreCase::BankFletcher => {
                self.bank_fletcher_cycle.crafted_after_withdrawal
                    && self.xp_gained("fletching")
                    && self.item_consumed_from_baseline("Willow logs")
                    && self.item_increased("Willow shortbow")
            }
            CoreCase::BankFletcherShafts | CoreCase::BankFletcherHeadless => {
                self.bank_fletcher_option_cycle.qualified()
            }
            CoreCase::BankFletcherString => self.bank_fletcher_string_cycle.strung_after_withdrawal,
            CoreCase::BankFletcherCutString => {
                self.bank_fletcher_cut_string_cycle.string_pair_created
            }
            CoreCase::DartFletcher | CoreCase::DartFletcherIron => {
                self.dart_fletcher_cycle.qualified()
            }
            CoreCase::HerbCleaner | CoreCase::HerbCleanerNamed => {
                self.herb_cleaner_cycle.qualified()
            }
            CoreCase::HerbCleanerEmptyBank => self.herb_cleaner_empty_cycle.qualified(),
            CoreCase::GemCutter | CoreCase::GemCutterNamed => self.gem_cutter_cycle.qualified(),
            CoreCase::DoorOpener | CoreCase::DoorOpenerGate => self.door_opener_cycle.qualified(),
            CoreCase::GnomeCourse | CoreCase::GnomeCourseRadius => {
                self.gnome_course_cycle.qualified()
            }
            CoreCase::WildyAgility => self.wildy_agility_cycle.qualified(),
            CoreCase::BrimhavenAgility => self.brimhaven_agility_cycle.qualified(),
            CoreCase::FlaxPicker => self.flax_picker_cycle.qualified(),
            CoreCase::Superheater
            | CoreCase::SuperheaterSteel
            | CoreCase::SuperheaterFireBattlestaff
            | CoreCase::SuperheaterSilverLowNatures => self.superheater_cycle.qualified(),
            CoreCase::VialFiller | CoreCase::VialFillerEast => self.vial_filler_cycle.qualified(),
            CoreCase::PotionMaker | CoreCase::PotionMakerNamed => {
                self.potion_maker_cycle.qualified()
            }
            CoreCase::TannerBot | CoreCase::TannerBotHard => self.tanner_bot_cycle.qualified(),
            CoreCase::RuneCrafter | CoreCase::RuneCrafterEarth | CoreCase::MuleCrafter => {
                self.rune_crafter_cycle.qualified()
            }
            CoreCase::ArdyCakes => self.ardy_cakes_cycle.qualified(),
            CoreCase::ArdyCakesFight => self.ardy_cakes_fight_cycle.qualified(),
            CoreCase::ArdyThiever | CoreCase::ArdyThieverKnight => {
                self.ardy_thiever_cycle.qualified()
            }
            CoreCase::ArdyThieverFight => self.ardy_thiever_fight_cycle.qualified(),
            CoreCase::GnomeChop => self.gnome_chop_cycle.qualified(),
            CoreCase::GnomeFletchShort | CoreCase::GnomeFletchLong => {
                self.gnome_fletch_cycle.qualified()
            }
            CoreCase::CoalTrucks => self.coal_trucks_cycle.qualified(),
            CoreCase::CookBot
            | CoreCase::CookBotLobster
            | CoreCase::SmelterBot
            | CoreCase::SmelterBotSteel
            | CoreCase::FlaxSpinner
            | CoreCase::FlaxAioSpin => self.station_production_cycle.qualified(),
            CoreCase::FlaxAio => self.flax_aio_cycle.qualified(),
            CoreCase::FlaxAioPick => self.flax_aio_pick_cycle.qualified(),
            CoreCase::HerbloreSecondaries => self.herblore_eggs_cycle.qualified(),
            CoreCase::HerbloreSecondariesNewt => self.herblore_newt_cycle.qualified(),
            CoreCase::AutoFighterBank
            | CoreCase::MossGiantBank
            | CoreCase::HillGiantBank
            | CoreCase::HillGiantBankPrepared
            | CoreCase::ChaosDruidBank
            | CoreCase::ArdyFighterBank
            | CoreCase::RockCrabBank
            | CoreCase::GreenDragonBank
            | CoreCase::GreenDragonBankPrepared
            | CoreCase::GreenDragonBankDefaultPrepared
            | CoreCase::GreenDragonTele
            | CoreCase::GreenDragonTelePrepared
            | CoreCase::FireGiantBank
            | CoreCase::FireGiantBankPrepared
            | CoreCase::FireGiantCamelotPrepared => {
                match (combat_spec(self.case), combat_bank_spec(self.case)) {
                    (Some(spec), Some(bank)) => self.combat_bank_cycle.qualified(spec, bank),
                    _ => false,
                }
            }
            CoreCase::FireGiantApproach => self.combat_approach_cycle.qualified(),
            CoreCase::AioTeleport | CoreCase::AioTeleportFalador | CoreCase::AioTeleportNoStaff => {
                self.aio_teleport_cycle.qualified()
            }
            CoreCase::ShopBuyout
            | CoreCase::ShopBuyoutAubury
            | CoreCase::ShopBuyoutLowe
            | CoreCase::ShopBuyoutHickton
            | CoreCase::ShopBuyoutHarry
            | CoreCase::ShopBuyoutBetty
            | CoreCase::ShopBuyoutGerrant => self.shop_buyout_cycle.qualified(),
            CoreCase::SmithingBot | CoreCase::SmithingBotPlatebody => {
                self.smithing_bot_cycle.qualified()
            }
            CoreCase::LeatherCrafter | CoreCase::LeatherCrafterHardBody => {
                self.leather_crafter_cycle.qualified()
            }
            CoreCase::Firemaker | CoreCase::FiremakerOak => self.firemaker_cycle.qualified(),
            CoreCase::ClimbingBoots | CoreCase::ClimbingBootsTeleport => {
                match climbing_boots_spec(self.case) {
                    Some(spec) => self.climbing_boots_cycle.qualified(spec),
                    None => false,
                }
            }
            CoreCase::RangingGuildRound => self.ranging_guild_round_cycle.qualified(),
            CoreCase::RangingGuildRedeem => self.ranging_guild_redeem_cycle.qualified(),
            CoreCase::RangingGuildBank => self.ranging_guild_bank_cycle.qualified(),
            CoreCase::RangingGuildFull => self.ranging_guild_full_cycle.qualified(),
            CoreCase::BrimhavenMossInspectV1 => self.brimhaven_moss_inspect_cycle.qualified(),
            CoreCase::RouteInspectBrimhavenV2 => self.route_inspect_brimhaven_v2_cycle.qualified(),
            CoreCase::PrayerV2 | CoreCase::PrayerV1 => self.prayer_delivery_cycle.qualified(),
            CoreCase::LineOfSightV2 => self.line_of_sight_cycle.qualified(),
            CoreCase::ActorObservationV2 => self.actor_observation_cycle.qualified(),
            CoreCase::FightFieldV2 => self.fight_field_cycle.qualified(),
            CoreCase::HoldSpotV2
            | CoreCase::RetreatSpotV2
            | CoreCase::WalkSpotV2
            | CoreCase::EnterLairV2
            | CoreCase::LeaveLairV2
            | CoreCase::AcquireKeyV2
            | CoreCase::CellV2
            | CoreCase::BankV2 => self
                .case
                .hunt_cell()
                .is_some_and(|cell| self.hunt_cycle.qualified(cell)),
            CoreCase::ChaosDruid
            | CoreCase::ChaosDruidTower
            | CoreCase::ChaosDruidYanille
            | CoreCase::MossGiant
            | CoreCase::MossGiantPrepared
            | CoreCase::HillGiant
            | CoreCase::AutoFighter
            | CoreCase::AutoFighterMage
            | CoreCase::AutoFighterRange
            | CoreCase::RockCrab
            | CoreCase::RockCrabRange
            | CoreCase::GreenDragon
            | CoreCase::GreenDragonPrepared
            | CoreCase::GreenDragonSpecial
            | CoreCase::GreenDragonSpecialPrepared
            | CoreCase::GreenDragonPotions
            | CoreCase::GreenDragonPotionsPrepared
            | CoreCase::FireGiant
            | CoreCase::FireGiantPrepared
            | CoreCase::ArdyFighter => {
                combat_spec(self.case).is_some_and(|spec| self.combat_core_cycle.qualified(spec))
            }
            CoreCase::MossGiantDart => self.combat_dart_branch_cycle.qualified(),
            CoreCase::GreenDragonMagePrepared => combat_spec(self.case)
                .is_some_and(|spec| qualified_mage_branch(&self.combat_core_cycle, spec)),
        }
    }

    pub fn qualify(&self) -> Result<Value, String> {
        if self.post_start_observations == 0 {
            return Err("no post-Start observations".into());
        }
        let ok = self.qualified();
        if !ok {
            return Err(self.incomplete_delta_error());
        }
        Ok(json!({
            "case": self.case,
            "baseline": self.baseline,
            "latest": self.latest,
            "max_items": self.max_items,
            "max_xp": self.max_xp,
            "saw_bury_chat": self.saw_bury_chat,
            "post_start_observations": self.post_start_observations,
            "bone_bank_cycle": self.bone_bank_cycle,
            "bank_fletcher_cycle": self.bank_fletcher_cycle,
            "bank_fletcher_option_cycle": self.bank_fletcher_option_cycle,
            "bank_fletcher_string_cycle": self.bank_fletcher_string_cycle,
            "bank_fletcher_cut_string_cycle": self.bank_fletcher_cut_string_cycle,
            "alcher_defaults_cycle": self.alcher_defaults_cycle,
            "alcher_generated_custom_cycle": self.alcher_generated_custom_cycle,
            "alcher_spell_cycle": self.alcher_spell_cycle,
            "alcher_swarm_cycle": self.alcher_swarm_cycle,
            "dart_fletcher_cycle": self.dart_fletcher_cycle,
            "herb_cleaner_cycle": self.herb_cleaner_cycle,
            "herb_cleaner_empty_cycle": self.herb_cleaner_empty_cycle,
            "gem_cutter_cycle": self.gem_cutter_cycle,
            "door_opener_cycle": self.door_opener_cycle,
            "gnome_course_cycle": self.gnome_course_cycle,
            "wildy_agility_cycle": self.wildy_agility_cycle,
            "brimhaven_agility_cycle": self.brimhaven_agility_cycle,
            "flax_picker_cycle": self.flax_picker_cycle,
            "superheater_cycle": self.superheater_cycle,
            "chicken_killer_bank_cycle": self.chicken_killer_bank_cycle,
            "vial_filler_cycle": self.vial_filler_cycle,
            "potion_maker_cycle": self.potion_maker_cycle,
            "tanner_bot_cycle": self.tanner_bot_cycle,
            "rune_crafter_cycle": self.rune_crafter_cycle,
            "ardy_cakes_cycle": self.ardy_cakes_cycle,
            "ardy_cakes_fight_cycle": self.ardy_cakes_fight_cycle,
            "ardy_thiever_cycle": self.ardy_thiever_cycle,
            "ardy_thiever_fight_cycle": self.ardy_thiever_fight_cycle,
            "gnome_chop_cycle": self.gnome_chop_cycle,
            "gnome_fletch_cycle": self.gnome_fletch_cycle,
            "coal_trucks_cycle": self.coal_trucks_cycle,
            "station_production_cycle": self.station_production_cycle,
            "flax_aio_cycle": self.flax_aio_cycle,
            "flax_aio_pick_cycle": self.flax_aio_pick_cycle,
            "herblore_eggs_cycle": self.herblore_eggs_cycle,
            "herblore_newt_cycle": self.herblore_newt_cycle,
            "combat_core_cycle": self.combat_core_cycle,
            "combat_dart_branch_cycle": self.combat_dart_branch_cycle,
            "combat_bank_cycle": self.combat_bank_cycle,
            "combat_approach_cycle": self.combat_approach_cycle,
            "aio_teleport_cycle": self.aio_teleport_cycle,
            "shop_buyout_cycle": self.shop_buyout_cycle,
            "smithing_bot_cycle": self.smithing_bot_cycle,
            "leather_crafter_cycle": self.leather_crafter_cycle,
            "firemaker_cycle": self.firemaker_cycle,
            "climbing_boots_cycle": self.climbing_boots_cycle,
            "ranging_guild_round_cycle": self.ranging_guild_round_cycle,
            "ranging_guild_redeem_cycle": self.ranging_guild_redeem_cycle,
            "ranging_guild_bank_cycle": self.ranging_guild_bank_cycle,
            "ranging_guild_full_cycle": self.ranging_guild_full_cycle,
            "brimhaven_moss_inspect_cycle": self.brimhaven_moss_inspect_cycle,
            "route_inspect_brimhaven_v2_cycle": self.route_inspect_brimhaven_v2_cycle,
            "prayer_delivery_cycle": self.prayer_delivery_cycle,
            "line_of_sight_cycle": self.line_of_sight_cycle,
            "fight_field_cycle": self.fight_field_cycle,
            "hunt_cycle": self.hunt_cycle,
            "ordered_first_exhausted": self.ordered_first_exhausted,
        }))
    }
}

/// Opt-in bridge from the production slot snapshot publisher to the catalog
/// core witness. Disabled by default; ordinary Play/panel runs pay one mutex
/// check and never build an [`Observation`].
#[derive(Clone, Default)]
pub struct CoreWatch {
    active: Arc<AtomicBool>,
    inner: Arc<Mutex<CoreWatchState>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreWatchStatus {
    Disabled,
    Ready,
    Running,
    Qualified,
    Failed,
}

#[derive(Debug, Default)]
enum CoreWatchState {
    #[default]
    Disabled,
    Ready {
        case: CoreCase,
        account: String,
        latest: Option<Observation>,
        start_preparation: HerbCleanerStartPreparation,
    },
    Running {
        account: String,
        witness: Box<CoreWitness>,
    },
    /// Immutable bounded receipt cached at the first terminal qualification.
    Qualified {
        account: String,
        evidence: Arc<Value>,
    },
    Failed {
        case: CoreCase,
        account: String,
        error: String,
        latest: Option<Observation>,
        witness: Option<Box<CoreWitness>>,
    },
}

impl CoreWatch {
    /// Replace any previous run with one ready to collect its pre-Start baseline.
    pub fn configure(&self, case: CoreCase, account: impl Into<String>) {
        *self.inner.lock().unwrap() = CoreWatchState::Ready {
            case,
            account: account.into(),
            latest: None,
            start_preparation: HerbCleanerStartPreparation::default(),
        };
        self.active.store(true, Ordering::Release);
    }

    /// Disable and drop all bounded witness state.
    pub fn clear(&self) {
        self.active.store(false, Ordering::Release);
        *self.inner.lock().unwrap() = CoreWatchState::Disabled;
    }

    /// Mark the exact instant immediately before the catalog isolate is started.
    /// The most recent observation from the production publisher becomes the
    /// immutable Start baseline; a later snapshot may already contain script work.
    pub fn begin_start(&self, account: &str) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            CoreWatchState::Disabled => {
                *state = CoreWatchState::Disabled;
                Ok(())
            }
            CoreWatchState::Ready {
                case,
                account: expected,
                latest,
                start_preparation,
            } if expected == account => match latest {
                None => {
                    *state = CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: None,
                        start_preparation,
                    };
                    Err("catalog core has no published pre-Start observation".into())
                }
                Some(observation) => {
                    let preparation = if case == CoreCase::HerbCleanerEmptyBank {
                        start_preparation.receipt_for(&expected, &observation)
                    } else {
                        None
                    };
                    match validate_start_baseline(
                        case,
                        &expected,
                        &observation,
                        preparation.as_ref(),
                    ) {
                        Ok(()) => {
                            *state = CoreWatchState::Running {
                                account: expected,
                                witness: Box::new(CoreWitness::new_with_start_preparation(
                                    case,
                                    observation,
                                    preparation,
                                )),
                            };
                            Ok(())
                        }
                        Err(error) => {
                            *state = CoreWatchState::Failed {
                                case,
                                account: expected,
                                error: error.clone(),
                                latest: Some(observation),
                                witness: None,
                            };
                            Err(error)
                        }
                    }
                }
            },
            CoreWatchState::Ready {
                case,
                account: expected,
                latest,
                start_preparation,
            } => {
                let error = format!(
                    "catalog core Start slot {account:?} is not configured account {expected:?}"
                );
                *state = CoreWatchState::Ready {
                    case,
                    account: expected,
                    latest,
                    start_preparation,
                };
                Err(error)
            }
            other => {
                *state = other;
                Err("catalog core Start was armed more than once".into())
            }
        }
    }

    /// Preserve an isolate-start refusal as a terminal core failure.
    pub fn fail_start(&self, account: &str, error: impl Into<String>) {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        *state = match current {
            CoreWatchState::Running {
                account: expected,
                witness,
            } if expected == account => CoreWatchState::Failed {
                case: witness.case,
                account: expected,
                error: error.into(),
                latest: None,
                witness: Some(witness),
            },
            other => other,
        };
    }

    /// Observe one compact record made from the already-published slot snapshot.
    /// A session boundary after Start is terminal: relogged/stale rows cannot be
    /// combined with the original baseline to manufacture progress.
    pub fn observe(&self, account: &str, observation: Observation, session_boundary: bool) {
        let mut state = self.inner.lock().unwrap();
        Self::observe_locked(&mut state, account, observation, session_boundary);
    }

    fn observe_locked(
        state: &mut CoreWatchState,
        account: &str,
        observation: Observation,
        session_boundary: bool,
    ) {
        let current = std::mem::take(&mut *state);
        *state = match current {
            CoreWatchState::Ready {
                case,
                account: expected,
                latest: _,
                mut start_preparation,
            } if expected == account => {
                if session_boundary {
                    CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: None,
                        start_preparation: HerbCleanerStartPreparation::default(),
                    }
                } else {
                    if case == CoreCase::HerbCleanerEmptyBank {
                        start_preparation.observe(&expected, &observation);
                    }
                    CoreWatchState::Ready {
                        case,
                        account: expected,
                        latest: Some(observation),
                        start_preparation,
                    }
                }
            }
            CoreWatchState::Running {
                account: expected,
                mut witness,
            } if expected == account => {
                if session_boundary {
                    CoreWatchState::Failed {
                        case: witness.case,
                        account: expected,
                        error: "catalog core session boundary after Start".into(),
                        latest: Some(observation),
                        witness: Some(witness),
                    }
                } else {
                    witness.observe(&observation);
                    if let Some(error) = witness.swarm_stopped_without_recovery(&observation) {
                        CoreWatchState::Failed {
                            case: witness.case,
                            account: expected,
                            error,
                            latest: Some(observation),
                            witness: Some(witness),
                        }
                    } else {
                        CoreWatchState::Running {
                            account: expected,
                            witness,
                        }
                    }
                }
            }
            other => other,
        };
    }

    /// Convert only for the configured slot and only after Start has been armed.
    pub fn observe_snapshot(
        &self,
        account: &str,
        snapshot: &GameSnapshot,
        names: &ObjNames,
        session_boundary: bool,
    ) {
        self.observe_snapshot_with_lifecycle(
            account,
            snapshot,
            names,
            None,
            BoundedGuardian::default(),
            session_boundary,
            None,
            None,
            None,
        );
    }

    /// Convert the published snapshot and attach the slot's bounded native
    /// lifecycle receipt, prior-frame guardian fact and (hunt cards only)
    /// the slot's act ledger without touching the panel's pending-log
    /// consumer.
    #[allow(clippy::too_many_arguments)] // watch observe packs account/snapshot/lifecycle/paint fields
    pub fn observe_snapshot_with_lifecycle(
        &self,
        account: &str,
        snapshot: &GameSnapshot,
        names: &ObjNames,
        lifecycle: Option<script::ScriptLifecycleReceipt>,
        guardian: BoundedGuardian,
        session_boundary: bool,
        inspect: Option<RouteInspectPublished>,
        paint: Option<&script::shim::ScriptPaint>,
        acts: Option<ScriptActsPublished>,
    ) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        if matches!(
            &*state,
            CoreWatchState::Ready { account: expected, .. }
                | CoreWatchState::Running { account: expected, .. }
                if expected == account
        ) {
            // Convert while the lifecycle lock is held so a reconfiguration
            // cannot attach this snapshot to a later run of the same account.
            let copies_prayer = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_prayer_varps()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_prayer_varps(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let copies_los = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_line_of_sight()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_line_of_sight(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let copies_actor = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_actor_observation()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_actor_observation(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let copies_fight = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.copies_fight_field()
                }
                CoreWatchState::Running { witness, .. } => witness.case.copies_fight_field(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
            };
            let hunt = match &*state {
                CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                    case.hunt_cell()
                }
                CoreWatchState::Running { witness, .. } => witness.case.hunt_cell(),
                CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => None,
            };
            let mut observation = Observation::from_snapshot(snapshot, names);
            observation.script_lifecycle = lifecycle;
            observation.guardian = guardian;
            if let Some(published) = inspect {
                observation.attach_route_inspect(published);
            }
            if copies_prayer {
                observation.attach_prayer_varps(snapshot);
            }
            if copies_los {
                observation.attach_line_of_sight(snapshot, paint);
            }
            if copies_actor {
                observation.attach_actor_observation(snapshot, paint);
            }
            if copies_fight {
                observation.attach_fight_field(snapshot, paint);
            }
            if let Some(cell) = hunt {
                observation.attach_hunt(cell, snapshot, paint, acts);
            }
            Self::observe_locked(&mut state, account, observation, session_boundary);
        }
    }

    pub fn configured(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// True only for the active inspect cards. Callers copy published hops
    /// only then; other cases keep the empty default.
    pub fn copies_route_inspect(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_route_inspect()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_route_inspect(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the line-of-sight File card. Callers attach paint and
    /// compact collision identity only then.
    pub fn copies_line_of_sight(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_line_of_sight()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_line_of_sight(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the actor-observation File card. Callers attach paint and
    /// one packed NPC row only then.
    pub fn copies_actor_observation(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_actor_observation()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_actor_observation(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the fight-field File card. Callers attach paint and
    /// one packed NPC row only then.
    pub fn copies_fight_field(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.copies_fight_field()
            }
            CoreWatchState::Running { witness, .. } => witness.case.copies_fight_field(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// True only for the v2 hunt File cards. Callers attach paint and the
    /// slot's act ledger only then.
    pub fn copies_hunt(&self) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Ready { case, .. } | CoreWatchState::Failed { case, .. } => {
                case.hunt_cell().is_some()
            }
            CoreWatchState::Running { witness, .. } => witness.case.hunt_cell().is_some(),
            CoreWatchState::Disabled | CoreWatchState::Qualified { .. } => false,
        }
    }

    /// Allocation-free terminal polling for the headed UI. The full receipt
    /// remains behind [`CoreWatch::qualify`] and is built once on terminal I/O.
    pub fn status(&self) -> CoreWatchStatus {
        if !self.active.load(Ordering::Acquire) {
            return CoreWatchStatus::Disabled;
        }
        let mut state = self.inner.lock().unwrap();
        let should_cache = matches!(
            &*state,
            CoreWatchState::Running { witness, .. } if witness.qualified()
        );
        if should_cache {
            let current = std::mem::take(&mut *state);
            if let CoreWatchState::Running { account, witness } = current {
                let evidence = Arc::new(
                    witness
                        .qualify()
                        .expect("qualified predicate must produce a receipt"),
                );
                *state = CoreWatchState::Qualified { account, evidence };
            }
        }
        match &*state {
            CoreWatchState::Disabled => CoreWatchStatus::Disabled,
            CoreWatchState::Ready { .. } => CoreWatchStatus::Ready,
            CoreWatchState::Running { .. } => CoreWatchStatus::Running,
            CoreWatchState::Qualified { .. } => CoreWatchStatus::Qualified,
            CoreWatchState::Failed { .. } => CoreWatchStatus::Failed,
        }
    }

    pub fn failure(&self) -> Option<String> {
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Failed { error, .. } => Some(error.clone()),
            _ => None,
        }
    }

    /// Return full core evidence only after the unchanged witness qualifies.
    pub fn qualify(&self) -> Result<Arc<Value>, String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            CoreWatchState::Running { account, witness } => match witness.qualify() {
                Ok(value) => {
                    let evidence = Arc::new(value);
                    *state = CoreWatchState::Qualified {
                        account,
                        evidence: Arc::clone(&evidence),
                    };
                    Ok(evidence)
                }
                Err(error) => {
                    *state = CoreWatchState::Running { account, witness };
                    Err(error)
                }
            },
            CoreWatchState::Qualified { account, evidence } => {
                let receipt = Arc::clone(&evidence);
                *state = CoreWatchState::Qualified { account, evidence };
                Ok(receipt)
            }
            failed @ CoreWatchState::Failed { .. } => {
                let error = match &failed {
                    CoreWatchState::Failed { error, .. } => error.clone(),
                    _ => unreachable!(),
                };
                *state = failed;
                Err(error)
            }
            ready @ CoreWatchState::Ready { .. } => {
                *state = ready;
                Err("catalog core Start not observed".into())
            }
            CoreWatchState::Disabled => {
                *state = CoreWatchState::Disabled;
                Err("catalog core watch disabled".into())
            }
        }
    }

    /// Compact failure/timeout receipt; never serializes the world snapshot.
    pub fn evidence(&self) -> Value {
        match &*self.inner.lock().unwrap() {
            CoreWatchState::Disabled => json!({"phase": "disabled"}),
            CoreWatchState::Ready {
                case,
                account,
                latest,
                start_preparation,
            } => json!({
                "phase": "ready",
                "case": case,
                "account": account,
                "latest": latest,
                "start_preparation": start_preparation,
            }),
            CoreWatchState::Running { account, witness } => json!({
                "phase": "running",
                "account": account,
                "witness": witness,
                "qualification": witness.qualify().err(),
            }),
            CoreWatchState::Qualified { account, evidence } => json!({
                "phase": "qualified",
                "account": account,
                "receipt": evidence,
            }),
            CoreWatchState::Failed {
                case,
                account,
                error,
                latest,
                witness,
            } => json!({
                "phase": "failed",
                "case": case,
                "account": account,
                "error": error,
                "latest": latest,
                "witness": witness,
            }),
        }
    }
}

fn validate_start_baseline(
    case: CoreCase,
    account: &str,
    observation: &Observation,
    start_preparation: Option<&HerbCleanerStartPreparationReceipt>,
) -> Result<(), String> {
    if !observation.ingame || observation.scene_state != 2 {
        return Err(format!(
            "Start baseline is not attached ingame scene2: {observation:?}"
        ));
    }
    let player = observation
        .player
        .as_deref()
        .ok_or_else(|| "Start baseline has no local player".to_string())?;
    let expected = client::util::jstring::JString::to_screen_name(account);
    if !player.eq_ignore_ascii_case(&expected) {
        return Err(format!(
            "Start baseline player {player:?} is not fresh account {account:?}"
        ));
    }
    if case == CoreCase::HerbCleanerEmptyBank && start_preparation.is_none() {
        return Err(format!(
            "{} Start baseline lacks this run's loaded-bank preparation receipt before closed-bank Start: {observation:?}",
            case.scenario_name()
        ));
    }
    validate_case_baseline_with_preparation(case, observation, start_preparation)
}

#[cfg(test)]
#[path = "catalog_actor_tests.rs"]
mod actor_observation_selection_tests;

#[cfg(test)]
#[path = "catalog_fight_field_tests.rs"]
mod fight_field_selection_tests;
