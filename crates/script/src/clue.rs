//! Native clue recovery identity, available without an isolate.

/// Shared radius for clue destination walks and posted loc approach facts.
#[cfg(feature = "load")]
pub const ARRIVE_RADIUS: i32 = 1;

/// Slot-retained clue debt. M-297 installs stripped gear and the abandon latch
/// during the atomic host/Load clue cutover; this has no action tokens.
#[derive(Default)]
pub struct ClueRecovery {
    _private: (),
}

#[cfg(feature = "load")]
#[path = "clue/isolate.rs"]
mod isolate;
#[cfg(feature = "load")]
pub use isolate::{
    dispatch, next_native, on_hold, on_pause, on_reset, on_resume, verb_req, Clue, Delegation,
    Outcome, BANK_APPROACH_FAILED,
};
#[cfg(feature = "load")]
pub(crate) use isolate::on_stop;
