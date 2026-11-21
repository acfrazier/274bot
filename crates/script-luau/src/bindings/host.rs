//! The one handle through which a binding acts. Each method is one call
//! into the run state, which owns the seat client of `script`
//! (`design-r3.md` 7.4 rule T5).

use super::convert::Refusal;
use crate::runtime::RunState;
use script::interact::InteractReq;
use script::native::snapshot_source::{NativeFactsInput, SnapshotInput};

/// The posted inputs that the slot lends to one dispatch (ADR 0023 item 2).
pub(crate) struct Frame<'a> {
    pub(crate) input: &'a SnapshotInput<'a>,
    pub(crate) native: &'a NativeFactsInput<'a>,
}

#[derive(Clone, Copy)]
pub(crate) struct Host<'a> {
    state: &'a RunState,
    frame: Option<&'a Frame<'a>>,
}

impl<'a> Host<'a> {
    pub(crate) fn new(state: &'a RunState, frame: Option<&'a Frame<'a>>) -> Self {
        Self { state, frame }
    }

    pub(crate) fn log(self, line: String) -> Result<(), Refusal> {
        self.state.log_line(line)
    }

    pub(crate) fn stop(self, reason: String) -> Result<(), Refusal> {
        self.state.request_self_stop(reason)
    }

    pub(crate) fn wait_next_tick(self) -> Result<(), Refusal> {
        self.state.await_next_tick()
    }

    pub(crate) fn setting(self, key: &str) -> Option<serde_json::Value> {
        self.state.setting(key)
    }

    pub(crate) fn walk(self, request: InteractReq) -> Result<(), Refusal> {
        self.state.await_walk(request, self.frame)
    }
}
