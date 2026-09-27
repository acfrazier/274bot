//! Script-run overlay values for the host's auto-run policy.
//!
//! The isolate sends replacements over the FlatBuffer interact wire. The host
//! owns the current value, clears it with the script runtime identity, and
//! preserves it across connection sessions within that run.

/// Effective JS auto-run energy floor. `NotANumber` never passes the host's
/// energy comparison, matching JavaScript `energy >= NaN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum RunEnergyMin {
    Floor(i32),
    NotANumber,
}

/// Script-written policy fields. `None` fields use host defaults; an absent
/// override removes the complete overlay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
pub struct RunPolicyOverride {
    pub run_auto: Option<bool>,
    pub energy_min: Option<RunEnergyMin>,
}
