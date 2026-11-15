//! Pure condition derivation and survival arithmetic.
pub mod hazard;

pub use hazard::{HazardStat, StepHazard};

pub mod condition;
pub mod cure;
pub(crate) mod eat;
pub mod math;
pub mod step;

pub use step::{Decision, Owner, SurvivalReport, SurvivalStep};

pub use cure::CurePoison;
