//! Shared observed-state combat machine. S3a admits melee Open/Hold only.
//! Style/flick/PvP slices extend this core rather than adding another planner.
mod arbiter;
pub mod frame;
mod machine;
pub mod prayer;
pub mod request;
pub mod schedule;
pub mod select;
pub mod tables;
pub mod threats;
pub mod style {
    pub mod melee;
}
pub use machine::Combat;
pub use prayer::{ClearPrayers, PrayerSweepReport};
pub use request::*;
pub use select::{facts, ident, retaliate, taken_by_another};
pub use tables::CombatTables;
pub use threats::{HitOnset, HitOnsets, Threat, ThreatSet};
/// `%option_nodef`: zero means auto-retaliation is on.
pub const OPTION_NODEF: i32 = 172;
