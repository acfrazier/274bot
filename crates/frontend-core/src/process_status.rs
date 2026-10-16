//! Frontend consumption of the process-unavailability conditions (F0 item
//! 5; contract §26 §3.3).
//!
//! T1's composition publishes each condition into its
//! [`ProcessConditions`] ledger; frontend-core owns the command/status
//! mapping; the panel, the TUI and the headless driver all read this one
//! borrowed projection and never detect service health themselves or parse
//! a diagnostic. While any condition is latched every surface shows the
//! persistent restart-required status ([`ProcessStatus::lines`]): a
//! transient command banner may change, this status stays until the
//! process exits, and an affected command stays refused
//! ([`ProcessStatus::refusing`]). A refused or failed command's report names
//! the operation ([`crate::command::CommandReport::command`]) and carries the
//! §3.1 text ([`crate::command::CommandFailure::operator_text`]). Nothing
//! here logs: the composition emits the one `host_log!` record per
//! condition.

use api::identity::ProcessTime;
use api::process_condition::{
    ConditionCause, ConditionId, ProcessConditions, ProcessServiceCode, ProcessServiceCondition,
};

use crate::command::OperatorCommand;

/// The process status every frontend surface presents: a borrow of the
/// ledger, at most three lines.
#[derive(Debug, Clone, Copy)]
pub struct ProcessStatus<'a> {
    conditions: &'a ProcessConditions,
}

impl<'a> ProcessStatus<'a> {
    /// The status of `conditions`.
    pub fn new(conditions: &'a ProcessConditions) -> Self {
        Self { conditions }
    }

    /// Whether the process must be restarted: any condition is latched.
    pub fn restart_required(self) -> bool {
        self.conditions.snapshot().next().is_some()
    }

    /// The persistent status lines, one per latched condition (at most
    /// three), in code order.
    pub fn lines(self) -> impl Iterator<Item = StatusLine<'a>> {
        self.conditions
            .snapshot()
            .map(|condition| StatusLine { condition })
    }

    /// The latched condition that refuses a new `command`, if any: the
    /// surface keeps that action refused and names this line.
    /// Retirement-dependent commands (Log out, Remove, Stop, Quit) are never
    /// refused.
    pub fn refusing(self, command: OperatorCommand) -> Option<StatusLine<'a>> {
        if command.retirement_dependent() {
            return None;
        }
        command
            .capabilities()
            .iter()
            .find_map(|&capability| self.conditions.blocks(capability))
            .map(|condition| StatusLine { condition })
    }

    /// Whether an automatic repeat of `command` (an auto-login, a Start-all
    /// re-queue) may be scheduled: never while a condition refuses it
    /// (§26 §3.2: suppress until process restart).
    pub fn retry_allowed(self, command: OperatorCommand) -> bool {
        self.refusing(command).is_none()
    }
}

/// One restart-required status line.
#[derive(Debug, Clone, Copy)]
pub struct StatusLine<'a> {
    condition: &'a ProcessServiceCondition,
}

impl StatusLine<'_> {
    /// The condition identity commands and logs correlate with.
    pub fn id(self) -> ConditionId {
        self.condition.id()
    }

    /// The closed code.
    pub fn code(self) -> ProcessServiceCode {
        self.condition.code()
    }

    /// Why the service became unavailable.
    pub fn cause(self) -> ConditionCause {
        self.condition.cause()
    }

    /// When it was detected, since the process origin.
    pub fn detected_at(self) -> ProcessTime {
        self.condition.detected_at()
    }

    /// The exact §3.1 operator text.
    pub fn operator_text(self) -> &'static str {
        self.condition.operator_text()
    }
}

#[cfg(test)]
#[path = "process_status_tests.rs"]
mod tests;
