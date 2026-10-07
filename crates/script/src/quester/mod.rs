//! Typed Path, compiler, colour-only runner, and the Quester card.
mod bank_run;
pub mod card;
pub mod choices;
pub mod compile;
pub mod eligibility;
pub mod families;
pub mod gang;
pub mod handlers;
pub mod loadouts;
mod nav_coverage;
pub mod pair;
pub mod path;
#[cfg(any(test, feature = "test-hooks"))]
pub mod probe;
pub mod progress;
pub mod provision;
pub mod queue;
pub mod registry;
pub mod runner;
pub mod schema;
pub mod select;
pub mod watchdog;

pub use card::CARD;
pub use runner::Quester;

/// Slot-owned run memory, using the same retention lifecycle as Gatherer.
/// Server quest evidence is reread after recreation; a local cursor is not
/// retained as progress. Operator Stop discards this cell.
#[derive(Default)]
pub struct QuesterRetained {
    pub anchor: Option<api::WorldTile>,
    pub death_seq: Option<i32>,
    pub deaths: u16,
    pub completed: u16,
}
