//! Kernel API: snapshot families, queries, interact/settle, ClientProt.

pub use obj_names::{ItemDefView, LocDefView, LocDefs, ObjNames};
pub use snapshot::{LocalTile, WorldTile};

pub mod interact;
pub mod obj_names;
pub mod prot;
pub mod query;
pub mod settle;
pub mod snapshot;
