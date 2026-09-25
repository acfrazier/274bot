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
#[path = "catalog_superheat.rs"]
mod superheat;
pub use superheat::*;
#[path = "catalog_potions.rs"]
mod potions;
pub use potions::*;
#[path = "catalog_tanning.rs"]
mod tanning;
pub use tanning::*;
#[path = "catalog_runecraft.rs"]
mod runecraft;
pub use runecraft::*;
#[path = "catalog_thieving.rs"]
mod thieving;
pub use thieving::*;
#[path = "catalog_gnome.rs"]
mod gnome;
pub use gnome::*;
#[path = "catalog_coal.rs"]
mod coal;
pub use coal::*;
#[path = "catalog_station.rs"]
mod station;
pub use station::*;
#[path = "catalog_teleport.rs"]
mod teleport;
pub use teleport::*;
#[path = "catalog_shop.rs"]
mod shop;
pub use shop::*;
#[path = "catalog_boots.rs"]
mod boots;
pub use boots::*;
#[path = "catalog_smithing.rs"]
mod smithing;
pub use smithing::*;
#[path = "catalog_leather.rs"]
mod leather;
pub use leather::*;
#[path = "catalog_firemaker.rs"]
mod firemaker;
pub use firemaker::*;
#[path = "catalog_baseline.rs"]
mod baseline;
pub use baseline::*;
#[path = "catalog_witness.rs"]
mod witness;
pub use witness::*;
#[path = "catalog_watch.rs"]
mod watch;
pub use watch::*;















































#[cfg(test)]
#[path = "catalog_actor_tests.rs"]
mod actor_observation_selection_tests;

#[cfg(test)]
#[path = "catalog_fight_field_tests.rs"]
mod fight_field_selection_tests;
