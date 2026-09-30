//! Owned native journal reads; Load retains its existing isolate adapter.

#[path = "quest_journal/native.rs"]
mod native;
pub use native::{JournalMachine, JournalRequest};

#[cfg(test)]
pub(crate) use native::tests::widget as test_widget;

#[cfg(feature = "load")]
#[path = "quest_journal/isolate.rs"]
mod isolate;
#[cfg(feature = "load")]
pub(crate) use isolate::{
    bind_compat_runtime, dispatch, on_hold, on_pause, on_reset, on_resume, on_stop,
    CompatJournalLease, QuestJournal,
};
