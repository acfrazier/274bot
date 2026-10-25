//! Owned native journal reads; Load retains its existing isolate adapter.

#[path = "quest_journal/native.rs"]
mod native;
pub use native::ROOT_289;
pub use native::{title_matches, TITLE_289};
pub use native::{JournalMachine, JournalRequest};

#[cfg(test)]
pub use native::tests::widget as test_widget;

#[cfg(feature = "load")]
#[path = "quest_journal/isolate.rs"]
mod isolate;
#[cfg(feature = "load")]
pub use isolate::{
    bind_compat_runtime, dispatch, on_hold, on_pause, on_reset, on_resume, on_stop,
    CompatJournalLease, QuestJournal,
};
