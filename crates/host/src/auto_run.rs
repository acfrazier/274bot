//! Auto-run: the host turns run on once run energy crosses the threshold.

/// Minimum run energy (0–100) at which the host sends `set_run(true)`.
pub const RUN_ENERGY_THRESHOLD: i32 = 20;

/// True iff the host should send `set_run(true)` this tick: energy at or
/// above the threshold and run not already on. Stateless per call — the slot
/// owns `run_on`, flips it true after an accepted send, and clears it when
/// energy hits 0 (cannot be running) so a later 20 crossing sends again.
pub fn auto_run_tick(energy: i32, run_on: bool) -> bool {
    !run_on && energy >= RUN_ENERGY_THRESHOLD
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
    use super::{auto_run_ready, auto_run_tick, RUN_ENERGY_THRESHOLD};

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
        assert!(!auto_run_tick(19, false));
        assert!(auto_run_tick(20, false));
        assert!(!auto_run_tick(20, true));
    }

    /// Energy already at/above threshold with run on: no send.
    #[test]
    fn already_on_never_sends() {
        assert!(!auto_run_tick(20, true));
        assert!(!auto_run_tick(RUN_ENERGY_THRESHOLD, true));
        assert!(!auto_run_tick(100, true));
    }

    /// Energy stuck below threshold with run off: no send.
    #[test]
    fn below_threshold_with_run_off_never_sends() {
        assert!(!auto_run_tick(19, false));
        assert!(!auto_run_tick(0, false));
    }

    /// 20 → 0 → 20: energy 0 cannot be running, so the slot clears `run_on`
    /// and the second crossing sends again.
    #[test]
    fn deplete_then_recover_sends_again() {
        let mut run_on = false;
        assert!(auto_run_tick(20, run_on));
        run_on = true;
        assert!(!auto_run_tick(20, run_on));
        // Slot clears the sticky flag when energy hits 0.
        run_on = false;
        assert!(!auto_run_tick(0, run_on));
        assert!(
            auto_run_tick(20, run_on),
            "second 20 crossing after energy 0 must send"
        );
    }
}
