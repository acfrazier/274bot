//! Native journal contract, available without an isolate. M-297 owns admission
//! and the atomic host/Load transport cutover; no native journal is installed yet.

use crate::native::{unavailable, ActionContext, ActionError, NativeMachine};
use api::quest_progress::JournalRead;
use std::task::Poll;

/// Prepared native read arguments; no tick-time family decode.
pub struct JournalRequest {
    pub quest: api::selected::FactKey,
    pub facts: std::sync::Arc<api::quest_facts::QuestCatalog>,
}

/// Cannot be constructed until the host journal owner is installed.
pub struct JournalMachine {
    unavailable: std::convert::Infallible,
}

impl NativeMachine for JournalMachine {
    type Args = JournalRequest;
    type Output = JournalRead;

    fn begin(_args: Self::Args, _cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Err(unavailable())
    }

    fn poll(&mut self, _cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        Poll::Ready(Err(unavailable()))
    }

    fn cancel(&mut self) {
        match self.unavailable {}
    }
}

#[cfg(feature = "load")]
#[path = "quest_journal/isolate.rs"]
mod isolate;
#[cfg(feature = "load")]
pub(crate) use isolate::{dispatch, on_hold, on_pause, on_reset, on_resume, QuestJournal};
