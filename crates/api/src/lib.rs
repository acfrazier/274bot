//! Kernel API: snapshot families, queries, interact/settle, ClientProt.

pub use obj_names::{ItemDefView, LocDefView, LocDefs, ObjNames};
pub use random::{DetectedRandom, RandomClaim, RandomKind};
pub use snapshot::{LocalTile, WorldTile};

pub mod cake_stall;
pub mod content;
pub mod game_data;
pub mod gather_tools;
pub mod interact;
pub mod named_banks;
pub mod obj_names;
pub mod prot;
pub mod query;
pub mod random;
pub mod settle;
pub mod snapshot;
