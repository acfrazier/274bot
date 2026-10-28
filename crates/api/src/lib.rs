//! Kernel API: snapshot families, queries, interact/settle, ClientProt.

pub use obj_names::{ItemDefView, LocDefView, LocDefs, ObjNames};
pub use random::{DetectedRandom, RandomClaim, RandomKind};
pub use snapshot::{LocalTile, WorldTile};
/// Maximum further bank returns accepted by the ChickenKiller bank check.
pub const CHICKEN_BANK_MAX_RESTARTS: u8 = 3;

pub mod bank_memory;
pub mod cake_stall;
pub mod clue_facts;
pub mod clue_logic;
pub mod clue_pack;
pub mod clue_puzzle;
pub mod condition;
pub mod content;
pub mod cook_locations;
pub mod debug_commands;
pub mod ent;
mod family_asset;
pub mod game_data;
pub mod gather_methods;
pub mod gather_tools;
pub mod ground_take;
pub mod hostlog;
pub mod identity;
pub mod interact;
pub mod lifecycle;
pub mod line_of_sight;
pub mod loadout;
pub mod local_state;
pub mod named_banks;
pub mod native_input;
pub mod obj_names;
pub mod prayer;
pub mod process_condition;
pub mod prot;
pub mod query;
pub mod quest_facts;
pub mod quest_progress;
pub mod random;
pub mod run_policy;
pub mod script_contract;
pub mod selected;
pub mod settle;
pub mod shop_facts;
pub mod snapshot;
pub mod stock;
pub mod work;
