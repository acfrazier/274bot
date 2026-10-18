//! Headless operator driver (T3.3): issue [`OperatorCommand`]s against a
//! real [`OperatorSession`] with no window and no terminal.
//!
//! The driver adds no dispatch of its own. [`HeadlessDriver::submit`] calls
//! [`OperatorSession::submit`], the entry point the panel and the TUI
//! handlers call, with a [`HeadlessSurface`] (raster off, no frame mailbox,
//! no input channel). [`HeadlessDriver::poll`] is [`OperatorSession::poll`],
//! which settles every pending command by its own request id.

use api::identity::RequestId;

use crate::command::{CommandReport, OperatorCommand};
use crate::session::{OperatorSession, SubmitError, Submitted, Targets};
use crate::surface::HeadlessSurface;

/// An operator session driven without a window.
pub struct HeadlessDriver {
    session: OperatorSession<()>,
}

impl HeadlessDriver {
    /// Drive `session` (already holding its vault and `Play`, or still
    /// locked).
    pub fn new(session: OperatorSession<()>) -> Self {
        Self { session }
    }

    /// Issue `command` for `targets` through the session's command seam.
    pub fn submit(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Result<Submitted, SubmitError> {
        self.session
            .submit(command, targets, &mut HeadlessSurface::new())
    }

    /// Stage the guardian edit the next `SetGuardian` commits.
    pub fn stage_guardian(&mut self, edit: crate::session::GuardianEdit) {
        self.session.stage_guardian(edit);
    }

    /// One headless tick: [`OperatorSession::poll`].
    pub fn poll(&mut self) {
        self.session.poll();
    }

    /// The report of `request`, while the book retains it.
    pub fn report(&self, request: RequestId) -> Option<&CommandReport> {
        self.session.commands().report(request)
    }

    /// The driven session, for state reads.
    pub fn session(&self) -> &OperatorSession<()> {
        &self.session
    }

    /// Give the session back (for example to close its `Play`).
    pub fn into_session(self) -> OperatorSession<()> {
        self.session
    }
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
