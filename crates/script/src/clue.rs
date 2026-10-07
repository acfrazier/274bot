//! Native clue recovery identity, available without an isolate.

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
pub(crate) use isolate::{
    dispatch, next_native, on_hold, on_pause, on_reset, on_resume, on_stop, verb_req, Clue,
    Delegation, Outcome, BANK_APPROACH_FAILED,
};
