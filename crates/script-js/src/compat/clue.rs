//! Compatibility clue runtime used by the JS-backed Sherlock card.

/// Shared radius for clue destination walks and posted loc approach facts.
pub const ARRIVE_RADIUS: i32 = 1;

#[path = "clue/isolate.rs"]
mod isolate;
pub(crate) use isolate::on_stop;
pub use isolate::{
    dispatch, next_native, on_hold, on_pause, on_reset, on_resume, verb_req, Clue, Delegation,
    Outcome, BANK_APPROACH_FAILED,
};
