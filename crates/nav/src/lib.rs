//! Nav: whole-world collision bake, transport graph, Dijkstra router
//! (`find` / `find_with`), pollable `Traveller::follow`, WalkTo grid,
//! arrival detection, and the content-derived bank stand table. Pack
//! magic `274V`, version byte 10.

pub mod arrival;
pub mod bake;
pub mod bank_fetch;
pub mod bundle;
pub mod camera;
pub mod canlight;
pub mod collision;
pub mod essence;
pub mod grid;
pub mod manifest;
pub mod map;
pub mod named_banks;
pub mod pack;
pub mod paint;
pub mod router;
pub mod tile;
pub mod transport;
pub mod traveller;
pub mod walk_destinations;
pub mod world;
pub mod world_state;

pub use world_state::WorldState;

/// Verbose nav/traveller dumps (`BOT_DEBUG=1`). Cached once per process.
pub fn debug_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("BOT_DEBUG").is_ok_and(|v| v == "1"))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
