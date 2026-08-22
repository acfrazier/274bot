//! Auto-run: the host turns run on once run energy crosses the threshold.

/// Minimum run energy (0–100) at which the host sends `set_run(true)`.
pub const RUN_ENERGY_THRESHOLD: i32 = 20;

/// True iff the host should send `set_run(true)` this tick: energy at or
/// above the threshold and run not already on. Stateless per call — the slot
/// owns `run_on`, flips it true after an accepted send, and `false` keeps it
/// that way until the player stops running.
pub fn auto_run_tick(energy: i32, run_on: bool) -> bool {
    !run_on && energy >= RUN_ENERGY_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::{RUN_ENERGY_THRESHOLD, auto_run_tick};

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
}
