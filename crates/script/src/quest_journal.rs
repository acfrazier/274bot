//! Owned native journal reads.

#[path = "quest_journal/native.rs"]
mod native;
pub use native::ROOT_289;
pub use native::{title_matches, TITLE_289};
pub use native::{JournalMachine, JournalRequest};

#[cfg(test)]
pub use native::tests::widget as test_widget;
