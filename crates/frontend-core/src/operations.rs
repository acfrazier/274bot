//! Operator operation results. Every lifecycle command returns an
//! [`OperationId`]; its [`OperationReport`] keeps one outcome per targeted
//! member. A rejected command is not success, and an accepted asynchronous
//! command (Login, Remove, script Start/Stop) stays `Pending` until
//! [`crate::OperatorSession::poll`] observes the host settle it.

use std::collections::VecDeque;

/// Reports kept for inspection. Older reports are dropped first; a pending
/// member of a dropped report is simply no longer tracked.
const REPORT_CAP: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Select,
    Load,
    Remove,
    Login,
    Logout,
    ScriptStart,
    ScriptPause,
    ScriptResume,
    ScriptStop,
}

impl ActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Load => "Load",
            Self::Remove => "Remove",
            Self::Login => "Log in",
            Self::Logout => "Log out",
            Self::ScriptStart => "Start",
            Self::ScriptPause => "Pause",
            Self::ScriptResume => "Resume",
            Self::ScriptStop => "Stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Accepted; the host has not settled it yet.
    Pending,
    Completed,
    Skipped(String),
    Failed(String),
    /// A later command or the slot's removal superseded it.
    Cancelled,
}

impl Outcome {
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberOutcome {
    pub slot: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationReport {
    pub id: OperationId,
    pub action: ActionKind,
    pub members: Vec<MemberOutcome>,
}

impl OperationReport {
    pub fn outcome(&self, slot: &str) -> Option<&Outcome> {
        self.members
            .iter()
            .find(|m| m.slot == slot)
            .map(|m| &m.outcome)
    }

    pub fn is_settled(&self) -> bool {
        self.members.iter().all(|m| !m.outcome.is_pending())
    }

    /// First failure text, formatted `slot: reason` for multi-member scopes.
    pub fn first_failure(&self) -> Option<String> {
        self.members.iter().find_map(|m| match &m.outcome {
            Outcome::Failed(reason) if self.members.len() == 1 => Some(reason.clone()),
            Outcome::Failed(reason) => Some(format!("{}: {reason}", m.slot)),
            _ => None,
        })
    }
}

/// Bounded operation history with id allocation.
#[derive(Debug, Default)]
pub(crate) struct OperationBook {
    next: u64,
    reports: VecDeque<OperationReport>,
}

impl OperationBook {
    pub(crate) fn open(&mut self, action: ActionKind) -> OperationId {
        self.next += 1;
        let id = OperationId(self.next);
        if self.reports.len() == REPORT_CAP {
            self.reports.pop_front();
        }
        self.reports.push_back(OperationReport {
            id,
            action,
            members: Vec::new(),
        });
        id
    }

    pub(crate) fn get(&self, id: OperationId) -> Option<&OperationReport> {
        // Ids are allocated in increasing order, so the book stays sorted.
        let index = self.reports.binary_search_by_key(&id, |r| r.id).ok()?;
        self.reports.get(index)
    }

    fn get_mut(&mut self, id: OperationId) -> Option<&mut OperationReport> {
        let index = self.reports.binary_search_by_key(&id, |r| r.id).ok()?;
        self.reports.get_mut(index)
    }

    pub(crate) fn last(&self) -> Option<&OperationReport> {
        self.reports.back()
    }

    /// Record (or replace) one member's outcome.
    pub(crate) fn set(&mut self, id: OperationId, slot: &str, outcome: Outcome) {
        let Some(report) = self.get_mut(id) else {
            return;
        };
        match report.members.iter_mut().find(|m| m.slot == slot) {
            Some(member) => member.outcome = outcome,
            None => report.members.push(MemberOutcome {
                slot: slot.to_string(),
                outcome,
            }),
        }
    }

    /// Settle every pending member of the given action kinds with `decide`.
    /// `decide` returns `None` to keep a member pending.
    pub(crate) fn settle(&mut self, mut decide: impl FnMut(ActionKind, &str) -> Option<Outcome>) {
        for report in self.reports.iter_mut() {
            let action = report.action;
            for member in report.members.iter_mut() {
                if member.outcome.is_pending() {
                    if let Some(outcome) = decide(action, &member.slot) {
                        member.outcome = outcome;
                    }
                }
            }
        }
    }

    /// Cancel pending members of `action` for `slot` (a newer command
    /// superseded them).
    pub(crate) fn cancel_pending(&mut self, action: ActionKind, slot: &str) {
        for report in self.reports.iter_mut().filter(|r| r.action == action) {
            for member in report.members.iter_mut() {
                if member.slot == slot && member.outcome.is_pending() {
                    member.outcome = Outcome::Cancelled;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_drops_the_oldest_report_past_its_cap() {
        let mut book = OperationBook::default();
        let first = book.open(ActionKind::Load);
        for _ in 0..REPORT_CAP {
            book.open(ActionKind::Login);
        }
        assert!(book.get(first).is_none());
        assert_eq!(book.last().unwrap().id, OperationId(REPORT_CAP as u64 + 1));
    }
}
