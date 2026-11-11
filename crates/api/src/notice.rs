//! Length limits of the one-line notices the front ends show.

/// Longest summary a front end shows for a notice, in characters. The cause
/// and the count fit before the cut.
pub const SUMMARY_CHARS: usize = 100;

/// Longest notice line a front end shows, in characters: a summary, the
/// ` (see log)` pointer and a margin. At four bytes a character this is 448
/// bytes, which one packed log line holds ([`crate::hostlog::PACK_BYTES`]).
pub const LINE_CHARS: usize = 112;
