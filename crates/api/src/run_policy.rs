//! Script-session overlay for the host's auto-run policy.
//!
//! The cell is shared by one script slot and one host slot. Scripts replace
//! the whole optional snapshot; the host resolves absent fields against its
//! immutable global policy. One atomic word keeps last-write-wins snapshots
//! coherent without allocating or locking on the frame loop.

use std::sync::atomic::{AtomicU64, Ordering};

const RUN_AUTO_PRESENT: u64 = 1;
const RUN_AUTO_VALUE: u64 = 1 << 1;
const ENERGY_MIN_PRESENT: u64 = 1 << 2;
const ENERGY_MIN_SHIFT: u32 = 32;

/// Script-written policy fields. `None` fields fall through to the global
/// policy; clearing the cell returns every field to global.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunPolicyOverride {
    pub run_auto: Option<bool>,
    pub energy_min: Option<i32>,
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

    /// Replace the whole overlay. `None` returns to the global policy.
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
    if let Some(energy_min) = policy.energy_min {
        bits |= ENERGY_MIN_PRESENT | u64::from(energy_min as u32) << ENERGY_MIN_SHIFT;
    }
    bits
}

fn unpack(bits: u64) -> Option<RunPolicyOverride> {
    if bits == 0 {
        return None;
    }
    Some(RunPolicyOverride {
        run_auto: (bits & RUN_AUTO_PRESENT != 0).then_some(bits & RUN_AUTO_VALUE != 0),
        energy_min: (bits & ENERGY_MIN_PRESENT != 0)
            .then_some((bits >> ENERGY_MIN_SHIFT) as u32 as i32),
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
            energy_min: Some(-7),
        }));
        assert_eq!(
            cell.get(),
            Some(RunPolicyOverride {
                run_auto: Some(false),
                energy_min: Some(-7),
            })
        );

        cell.set(Some(RunPolicyOverride {
            run_auto: None,
            energy_min: Some(101),
        }));
        assert_eq!(
            cell.get(),
            Some(RunPolicyOverride {
                run_auto: None,
                energy_min: Some(101),
            })
        );

        cell.clear();
        assert_eq!(cell.get(), None);
    }
}
