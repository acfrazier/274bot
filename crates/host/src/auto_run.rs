//! Auto-run: the host turns run on according to its true/20 defaults plus an
//! optional script-session overlay.

use api::run_policy::{RunEnergyMin, RunPolicyOverride};

pub const RUN_AUTO_DEFAULT: bool = true;
/// Default minimum run energy (0–100) at which the host sends `set_run(true)`.
pub const RUN_ENERGY_THRESHOLD: i32 = 20;

/// Resolved auto-run policy for one frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunPolicy {
    pub run_auto: bool,
    pub energy_min: RunEnergyMin,
}

impl Default for RunPolicy {
    fn default() -> Self {
        Self {
            run_auto: RUN_AUTO_DEFAULT,
            energy_min: RunEnergyMin::Floor(RUN_ENERGY_THRESHOLD),
        }
    }
}

/// Resolve a script snapshot over the host defaults. Each absent override
/// field falls through independently; finite floors are clamped like frozen
/// `resolveRunPolicy`, while NaN remains unmatchable.
pub fn resolve_run_policy(over: Option<RunPolicyOverride>, defaults: RunPolicy) -> RunPolicy {
    let energy_min = match over
        .and_then(|policy| policy.energy_min)
        .unwrap_or(defaults.energy_min)
    {
        RunEnergyMin::Floor(energy_min) => RunEnergyMin::Floor(energy_min.clamp(0, 100)),
        RunEnergyMin::NotANumber => RunEnergyMin::NotANumber,
    };
    RunPolicy {
        run_auto: over
            .and_then(|policy| policy.run_auto)
            .unwrap_or(defaults.run_auto),
        energy_min,
    }
}

/// True iff the host should send `set_run(true)` this tick: auto-run is
/// enabled, energy is at or above the resolved threshold, and run is not
/// already on. Stateless per call — the slot owns `run_on`, flips it true
/// after an accepted send, and clears it when energy hits 0.
pub fn auto_run_tick(energy: i32, run_on: bool, policy: RunPolicy) -> bool {
    policy.run_auto
        && !run_on
        && match policy.energy_min {
            RunEnergyMin::Floor(floor) => energy >= floor,
            RunEnergyMin::NotANumber => false,
        }
}

/// Auto-run is a bothost host feature (2004 had no always-on run). The
/// IF_BUTTON orb send is ignored on the title / before the controls
/// overlay exists, so we only arm once `ingame && scene_state == 2`.
/// Sending earlier sticks `run_on` and never retries.
pub fn auto_run_ready(ingame: bool, scene_state: i32) -> bool {
    ingame && scene_state == 2
}

#[cfg(test)]
mod tests {
    use super::{
        auto_run_ready, auto_run_tick, resolve_run_policy, RunPolicy, RUN_ENERGY_THRESHOLD,
    };
    use api::run_policy::{RunEnergyMin, RunPolicyOverride};

    #[test]
    fn ready_only_after_ingame_scene_2() {
        assert!(!auto_run_ready(false, 0));
        assert!(!auto_run_ready(false, 2));
        assert!(!auto_run_ready(true, 0));
        assert!(!auto_run_ready(true, 1));
        assert!(auto_run_ready(true, 2));
    }

    /// Energy crossing 19 → 20 with run off: the crossing tick sends; with
    /// the slot's `run_on` flipped after the send, the next tick stays quiet
    /// ("send once").
    #[test]
    fn crossing_threshold_with_run_off_sends_once() {
        assert!(!auto_run_tick(19, false, RunPolicy::default()));
        assert!(auto_run_tick(20, false, RunPolicy::default()));
        assert!(!auto_run_tick(20, true, RunPolicy::default()));
    }

    /// Energy already at/above threshold with run on: no send.
    #[test]
    fn already_on_never_sends() {
        assert!(!auto_run_tick(20, true, RunPolicy::default()));
        assert!(!auto_run_tick(
            RUN_ENERGY_THRESHOLD,
            true,
            RunPolicy::default()
        ));
        assert!(!auto_run_tick(100, true, RunPolicy::default()));
    }

    /// Energy stuck below threshold with run off: no send.
    #[test]
    fn below_threshold_with_run_off_never_sends() {
        assert!(!auto_run_tick(19, false, RunPolicy::default()));
        assert!(!auto_run_tick(0, false, RunPolicy::default()));
    }

    /// 20 → 0 → 20: energy 0 cannot be running, so the slot clears `run_on`
    /// and the second crossing sends again.
    #[test]
    fn deplete_then_recover_sends_again() {
        let mut run_on = false;
        assert!(auto_run_tick(20, run_on, RunPolicy::default()));
        run_on = true;
        assert!(!auto_run_tick(20, run_on, RunPolicy::default()));
        // Slot clears the sticky flag when energy hits 0.
        run_on = false;
        assert!(!auto_run_tick(0, run_on, RunPolicy::default()));
        assert!(
            auto_run_tick(20, run_on, RunPolicy::default()),
            "second 20 crossing after energy 0 must send"
        );
    }

    #[test]
    fn override_fields_fall_through_to_defaults_and_energy_is_clamped_last() {
        let defaults = RunPolicy {
            run_auto: false,
            energy_min: RunEnergyMin::Floor(60),
        };
        assert_eq!(resolve_run_policy(None, defaults), defaults);
        assert_eq!(
            resolve_run_policy(
                Some(RunPolicyOverride {
                    run_auto: Some(true),
                    energy_min: None,
                }),
                defaults,
            ),
            RunPolicy {
                run_auto: true,
                energy_min: RunEnergyMin::Floor(60),
            }
        );
        assert_eq!(
            resolve_run_policy(
                Some(RunPolicyOverride {
                    run_auto: None,
                    energy_min: Some(RunEnergyMin::Floor(140)),
                }),
                defaults,
            ),
            RunPolicy {
                run_auto: false,
                energy_min: RunEnergyMin::Floor(100),
            }
        );
        assert_eq!(
            resolve_run_policy(
                Some(RunPolicyOverride {
                    run_auto: Some(true),
                    energy_min: Some(RunEnergyMin::Floor(-1)),
                }),
                defaults,
            ),
            RunPolicy {
                run_auto: true,
                energy_min: RunEnergyMin::Floor(0),
            }
        );
        assert_eq!(
            defaults,
            RunPolicy {
                run_auto: false,
                energy_min: RunEnergyMin::Floor(60),
            },
            "resolving an overlay must not mutate host defaults"
        );
    }

    #[test]
    fn override_controls_auto_run_while_set() {
        let defaults = RunPolicy::default();
        let raised = resolve_run_policy(
            Some(RunPolicyOverride {
                run_auto: None,
                energy_min: Some(RunEnergyMin::Floor(80)),
            }),
            defaults,
        );
        assert!(!auto_run_tick(79, false, raised));
        assert!(auto_run_tick(80, false, raised));

        let disabled = resolve_run_policy(
            Some(RunPolicyOverride {
                run_auto: Some(false),
                energy_min: Some(RunEnergyMin::Floor(0)),
            }),
            defaults,
        );
        assert!(!auto_run_tick(100, false, disabled));
        assert!(auto_run_tick(20, false, defaults));
        let not_a_number = resolve_run_policy(
            Some(RunPolicyOverride {
                run_auto: None,
                energy_min: Some(RunEnergyMin::NotANumber),
            }),
            defaults,
        );
        assert_eq!(not_a_number.energy_min, RunEnergyMin::NotANumber);
        assert!(
            !auto_run_tick(100, false, not_a_number)
                && !auto_run_tick(i32::MAX, false, not_a_number),
            "NaN floor never enables auto-run"
        );
    }
}
