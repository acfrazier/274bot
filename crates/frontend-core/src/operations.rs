//! Operator operation results. Every lifecycle command returns an
//! [`OperationId`]; its [`OperationReport`] keeps one outcome per targeted
//! member. A rejected command is not success, and an accepted asynchronous
//! command (Login, Remove, script Start/Stop) stays `Pending` until
//! [`crate::OperatorSession::poll`] observes the host settle it.

use std::collections::VecDeque;

/// Settled reports kept for inspection. Only settled history is evicted
/// (oldest first); a report with a pending member stays until it settles.
/// Pending reports are bounded by live work: each new Login, Logout or
/// Remove of a slot cancels that slot's older pending one, and Start, Stop,
/// Pause, Resume and profile writes settle as the host or the writer reports.
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
    SaveProfile,
    DeleteProfile,
    /// Bulk parameter sync ("Apply to all"); one member per wall member.
    SyncSettings,
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
            Self::SaveProfile => "Save profile",
            Self::DeleteProfile => "Delete profile",
            Self::SyncSettings => "Apply to all",
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

/// One member outcome that changed: what the log and the fleet rows see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpChange {
    pub id: OperationId,
    pub action: ActionKind,
    pub slot: String,
    pub outcome: Outcome,
}

/// `op#41 Start completed`, `op#42 Log in failed: <reason>`: how the log
/// and the fleet rows name one member's operation outcome.
pub(crate) fn write_op(
    out: &mut impl std::fmt::Write,
    id: OperationId,
    action: ActionKind,
    outcome: &Outcome,
) -> std::fmt::Result {
    write!(out, "op#{} {} ", id.0, action.label())?;
    match outcome {
        Outcome::Pending => out.write_str("accepted"),
        Outcome::Completed => out.write_str("completed"),
        Outcome::Skipped(reason) => write!(out, "skipped: {reason}"),
        Outcome::Failed(reason) => write!(out, "failed: {reason}"),
        Outcome::Cancelled => out.write_str("cancelled"),
    }
}

/// Bounded operation history with id allocation. Every member outcome that
/// changes is also journaled until the next [`Self::append_changes`] (each
/// poll), so settlement is observed in order.
#[derive(Debug, Default)]
pub(crate) struct OperationBook {
    next: u64,
    reports: VecDeque<OperationReport>,
    changes: Vec<OpChange>,
}

impl OperationBook {
    pub(crate) fn open(&mut self, action: ActionKind) -> OperationId {
        self.next += 1;
        let id = OperationId(self.next);
        if self.reports.len() >= REPORT_CAP {
            if let Some(index) = self.reports.iter().position(OperationReport::is_settled) {
                self.reports.remove(index);
            }
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

    pub(crate) fn last(&self) -> Option<&OperationReport> {
        self.reports.back()
    }

    /// Record (or replace) one member's outcome.
    pub(crate) fn set(&mut self, id: OperationId, slot: &str, outcome: Outcome) {
        let Ok(index) = self.reports.binary_search_by_key(&id, |r| r.id) else {
            return;
        };
        let report = &mut self.reports[index];
        match report.members.iter_mut().find(|m| m.slot == slot) {
            Some(member) if member.outcome == outcome => return,
            Some(member) => member.outcome = outcome.clone(),
            None => report.members.push(MemberOutcome {
                slot: slot.to_string(),
                outcome: outcome.clone(),
            }),
        }
        self.changes.push(OpChange {
            id,
            action: report.action,
            slot: slot.to_string(),
            outcome,
        });
    }

    /// Settle every pending member of the given action kinds with `decide`.
    /// `decide` returns `None` to keep a member pending.
    pub(crate) fn settle(&mut self, mut decide: impl FnMut(ActionKind, &str) -> Option<Outcome>) {
        let Self {
            reports, changes, ..
        } = self;
        for report in reports.iter_mut() {
            let action = report.action;
            for member in report.members.iter_mut() {
                if member.outcome.is_pending() {
                    if let Some(outcome) = decide(action, &member.slot) {
                        member.outcome = outcome;
                        changes.push(OpChange {
                            id: report.id,
                            action,
                            slot: member.slot.clone(),
                            outcome: member.outcome.clone(),
                        });
                    }
                }
            }
        }
    }

    /// Cancel pending members of `action` for `slot` (a newer command
    /// superseded them).
    pub(crate) fn cancel_pending(&mut self, action: ActionKind, slot: &str) {
        self.resolve_pending(action, slot, Outcome::Cancelled);
    }

    /// Settle every pending member of `action` for `slot` with `outcome`.
    pub(crate) fn resolve_pending(&mut self, action: ActionKind, slot: &str, outcome: Outcome) {
        let Self {
            reports, changes, ..
        } = self;
        for report in reports.iter_mut().filter(|r| r.action == action) {
            for member in report.members.iter_mut() {
                if member.slot == slot && member.outcome.is_pending() {
                    member.outcome = outcome.clone();
                    changes.push(OpChange {
                        id: report.id,
                        action,
                        slot: member.slot.clone(),
                        outcome: outcome.clone(),
                    });
                }
            }
        }
    }

    /// Move the changes journaled since the last call onto the end of
    /// `out`, in order (the journal keeps its capacity).
    pub(crate) fn append_changes(&mut self, out: &mut Vec<OpChange>) {
        out.append(&mut self.changes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_history_evicts_settled_reports_and_keeps_a_pending_one_until_it_settles() {
        let mut book = OperationBook::default();
        let pending = book.open(ActionKind::Login);
        book.set(pending, "slow", Outcome::Pending);
        let first_settled = book.open(ActionKind::Load);
        book.set(first_settled, "a", Outcome::Completed);
        for _ in 0..(2 * REPORT_CAP) {
            let op = book.open(ActionKind::Load);
            book.set(op, "a", Outcome::Completed);
        }
        assert!(book.get(first_settled).is_none(), "settled history rolls");
        assert_eq!(book.reports.len(), REPORT_CAP);

        book.settle(|_, _| Some(Outcome::Completed));
        assert_eq!(
            book.get(pending).unwrap().outcome("slow"),
            Some(&Outcome::Completed),
            "a late settlement still lands on the accepted operation"
        );
    }
}
