//! Script-session overlay for the host's auto-run policy.
//!
//! The cell is shared by one script slot and one host slot. Scripts replace
//! the whole optional snapshot; absent fields use the host's true/20 defaults.
//! One atomic word keeps last-write-wins snapshots coherent without allocating
//! or locking on the frame loop.

use std::sync::atomic::{AtomicU64, Ordering};

const RUN_AUTO_PRESENT: u64 = 1;
const RUN_AUTO_VALUE: u64 = 1 << 1;
const ENERGY_MIN_PRESENT: u64 = 1 << 2;
const ENERGY_MIN_NAN: u64 = 1 << 3;
const ENERGY_MIN_SHIFT: u32 = 32;

/// Effective JS auto-run energy floor. `NotANumber` never passes the host's
/// energy comparison, matching JavaScript `energy >= NaN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnergyMin {
    Floor(i32),
    NotANumber,
}

/// Script-written policy fields. `None` fields use host defaults; clearing the
/// cell removes the complete overlay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunPolicyOverride {
    pub run_auto: Option<bool>,
    pub energy_min: Option<RunEnergyMin>,
}

/// Coherent, lock-free storage for one script session's policy overlay.
#[derive(Debug, Default)]
pub struct RunPolicyOverrideCell {
    bits: AtomicU64,
}

impl RunPolicyOverrideCell {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the whole overlay. `None` returns to host defaults.
    pub fn set(&self, policy: Option<RunPolicyOverride>) {
        self.bits.store(pack(policy), Ordering::Release);
    }

    pub fn clear(&self) {
        self.set(None);
    }

    pub fn get(&self) -> Option<RunPolicyOverride> {
        unpack(self.bits.load(Ordering::Acquire))
    }
}

fn pack(policy: Option<RunPolicyOverride>) -> u64 {
    let Some(policy) = policy else {
        return 0;
    };
    let mut bits = 0;
    if let Some(run_auto) = policy.run_auto {
        bits |= RUN_AUTO_PRESENT;
        if run_auto {
            bits |= RUN_AUTO_VALUE;
        }
    }
    match policy.energy_min {
        Some(RunEnergyMin::Floor(energy_min)) => {
            bits |= ENERGY_MIN_PRESENT | u64::from(energy_min as u32) << ENERGY_MIN_SHIFT;
        }
        Some(RunEnergyMin::NotANumber) => bits |= ENERGY_MIN_PRESENT | ENERGY_MIN_NAN,
        None => {}
    }
    bits
}

fn unpack(bits: u64) -> Option<RunPolicyOverride> {
    if bits == 0 {
        return None;
    }
    let energy_min = if bits & ENERGY_MIN_PRESENT == 0 {
        None
    } else if bits & ENERGY_MIN_NAN != 0 {
        Some(RunEnergyMin::NotANumber)
    } else {
        Some(RunEnergyMin::Floor(
            (bits >> ENERGY_MIN_SHIFT) as u32 as i32,
        ))
    };
    Some(RunPolicyOverride {
        run_auto: (bits & RUN_AUTO_PRESENT != 0).then_some(bits & RUN_AUTO_VALUE != 0),
        energy_min,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_is_a_whole_snapshot_and_clear_is_unset() {
        let cell = RunPolicyOverrideCell::new();
        assert_eq!(cell.get(), None);

        cell.set(Some(RunPolicyOverride {
            run_auto: Some(false),
            energy_min: Some(RunEnergyMin::Floor(-7)),
        }));
        assert_eq!(
            cell.get(),
            Some(RunPolicyOverride {
                run_auto: Some(false),
                energy_min: Some(RunEnergyMin::Floor(-7)),
            })
        );

        cell.set(Some(RunPolicyOverride {
            run_auto: None,
            energy_min: Some(RunEnergyMin::Floor(101)),
        }));
        assert_eq!(
            cell.get(),
            Some(RunPolicyOverride {
                run_auto: None,
                energy_min: Some(RunEnergyMin::Floor(101)),
            })
        );
        cell.set(Some(RunPolicyOverride {
            run_auto: Some(true),
            energy_min: Some(RunEnergyMin::NotANumber),
        }));
        assert_eq!(
            cell.get(),
            Some(RunPolicyOverride {
                run_auto: Some(true),
                energy_min: Some(RunEnergyMin::NotANumber),
            })
        );

        cell.clear();
        assert_eq!(cell.get(), None);
    }
}
