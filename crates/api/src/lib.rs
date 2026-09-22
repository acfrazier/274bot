//! Kernel API: snapshot families, queries, interact/settle, ClientProt.

pub use obj_names::{ItemDefView, LocDefView, LocDefs, ObjNames};
pub use random::{DetectedRandom, RandomClaim, RandomKind};
pub use snapshot::{LocalTile, WorldTile};

pub mod cake_stall;
pub mod content;
pub mod ent;
pub mod game_data;
pub mod gather_methods;
pub mod gather_tools;
pub mod clue_facts;
pub mod clue_logic;
pub mod clue_pack;
pub mod quest_facts;
pub mod interact;
pub mod line_of_sight;
pub mod named_banks;
pub mod native_input;
pub mod obj_names;
pub mod prayer;
pub mod prot;
pub mod query;
pub mod random;
pub mod settle;
pub mod shop_facts;
pub mod snapshot;
