//! Shared observed-state melee and ranged combat machine.
//! Style/flick/PvP slices extend this core rather than adding another planner.
//!
//! Ranged PvM uses selected weapon/ammo facts, observes the equipped combat tab
//! before selecting its mode, and defaults to rapid. Projectile launches drive
//! its attack cycle; delayed impacts do not reset it. Missing compatible ammo
//! ends preparation with `PrepFailed(Ammo)` and an active fight with
//! `Unprotected(NoAmmo)`. After an open-tactic kill at zero danger, wind-down
//! attempts at most four reachable nearby pickups of the selected ammunition.
mod arbiter;
pub mod frame;
pub mod guard;
mod machine;
pub mod policy;
pub mod prayer;
pub mod request;
pub mod schedule;
pub mod select;
pub mod tables;
pub mod threats;
pub mod style {
    pub mod melee;
    pub mod ranged;
}
pub use guard::{GuardFailure, GuardOp, GuardProtect, GuardRefusal, WalkGuard};
pub use machine::Combat;
pub use policy::{prayer_restore, prayer_sip_due, prayer_sip_floor, wanted_protect};
pub use prayer::{
    begin_clear_owned_prayers, ClearPrayers, ClearPrayersArgs, Hygiene, PrayerSweepReport,
    RaisedPrayers,
};
pub use request::*;
pub use select::{facts, ident, retaliate, taken_by_another};
pub use tables::CombatTables;
pub use threats::{HitOnset, HitOnsets, Threat, ThreatSet};
/// `%option_nodef`: zero means auto-retaliation is on.
pub const OPTION_NODEF: i32 = 172;
