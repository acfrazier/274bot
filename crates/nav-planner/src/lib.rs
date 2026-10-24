#![deny(clippy::disallowed_types)]
#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_macros)]

//! Navigation collision, transport graph, Dijkstra routing, arrival detection,
//! and immutable map views. `nav-bake` owns release generation, and
//! `nav-executor` drives routes.
//! Pack magic `274V`, version byte 17.

pub mod arrival;
pub mod bank_fetch;
pub mod camera;
pub mod canlight;
pub mod collision;
pub mod contract;
pub mod essence;
pub mod executor_adapter;
pub mod grid;
pub mod manifest;
pub mod map;
pub mod movement;
pub mod named_banks;
pub mod pack;
pub mod paint;
pub mod producer;
pub mod quest_gates;
pub mod router;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod tile;
pub mod transport;
pub mod walk_destinations;
pub mod world;
pub mod world_state;
pub mod zones;

pub use world_state::{WorldState, WorldStateGrowthBound};

/// Verbose navigation dumps (`BOT_DEBUG=1` or host `--debug`).
pub fn debug_enabled() -> bool {
    api::hostlog::debug_enabled()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
