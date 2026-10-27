//! Native clue recovery identity, available without an isolate.

/// Slot-retained clue debt. M-297 installs stripped gear and the abandon latch
/// during the atomic host/Load clue cutover; this has no action tokens.
#[derive(Default)]
pub struct ClueRecovery {
    _private: (),
}
