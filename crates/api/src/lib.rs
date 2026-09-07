//! Kernel API: snapshot families, queries, interact/settle, ClientProt.

pub use obj_names::{ItemDefView, LocDefView, LocDefs, ObjNames};
pub use random::{DetectedRandom, RandomClaim, RandomKind};
pub use snapshot::{LocalTile, WorldTile};

pub mod content;
pub mod interact;
pub mod obj_names;
pub mod prot;
pub mod query;
pub mod random;
pub mod settle;
pub mod snapshot;
/// Opt-in A1 rebuild-edge exact family body dedup (feature `snapshot-dedup`).
pub mod snapshot_dedup;
/// Test harness cursor over [`snapshot_dedup`] (mechanism fixtures).
pub mod snapshot_dedup_proto;
