//! The report of a command run on marked bots (group walk, Assign, log in
//! and out, ...). Every marked row appears exactly once, with one outcome:
//! done, skipped (it could not take the command) or failed (it took the
//! command and the command was refused). Failed rows come first, then
//! skipped, then done, so a one-line banner cut at the terminal width keeps
//! the failures.

use std::fmt::{self, Write as _};

/// Names listed per outcome kind in the summary; the counts cover the rest.
const LISTED: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BulkOutcomeKind {
    Done,
    Skipped,
    Pending,
    Failed,
}

/// What a command came to for one marked bot.
#[derive(Debug, PartialEq, Eq)]
pub struct BulkOutcome {
    kind: BulkOutcomeKind,
    reason: Option<String>,
}

#[allow(non_upper_case_globals)]
impl BulkOutcome {
    /// The command took effect (the report's `done` verb).
    pub const Done: Self = Self {
        kind: BulkOutcomeKind::Done,
        reason: None,
    };

    /// The command is retained for completion after an asynchronous
    /// prerequisite.
    pub const Pending: Self = Self {
        kind: BulkOutcomeKind::Pending,
        reason: None,
    };

    fn rank(&self) -> u8 {
        match self.kind {
            BulkOutcomeKind::Failed => 0,
            BulkOutcomeKind::Skipped => 1,
            BulkOutcomeKind::Pending => 2,
            BulkOutcomeKind::Done => 3,
        }
    }

    pub fn skipped(reason: impl Into<String>) -> Self {
        Self {
            kind: BulkOutcomeKind::Skipped,
            reason: Some(reason.into()),
        }
    }

    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            kind: BulkOutcomeKind::Failed,
            reason: Some(reason.into()),
        }
    }

    /// Whether the command took effect.
    pub fn is_done(&self) -> bool {
        self.kind == BulkOutcomeKind::Done
    }

    /// Whether the bot was not eligible.
    pub fn is_skipped(&self) -> bool {
        self.kind == BulkOutcomeKind::Skipped
    }

    /// Whether the command is retained for an asynchronous prerequisite.
    pub fn is_pending(&self) -> bool {
        self.kind == BulkOutcomeKind::Pending
    }

    /// Whether the command was refused.
    pub fn is_failed(&self) -> bool {
        self.kind == BulkOutcomeKind::Failed
    }

    /// Skip or failure text, when this outcome carries one.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

/// One marked row's outcome.
#[derive(Debug, PartialEq, Eq)]
pub struct BulkRow {
    profile: String,
    outcome: BulkOutcome,
    /// Host detail carries zone labels or the legacy-grid note through row
    /// inspection and summary rendering without changing the short outcome.
    detail: Option<String>,
}

impl BulkRow {
    pub fn new(profile: impl Into<String>, outcome: BulkOutcome) -> Self {
        Self {
            profile: profile.into(),
            outcome,
            detail: None,
        }
    }
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// The marked profile this row is for.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// What the command came to for this profile.
    pub fn outcome(&self) -> &BulkOutcome {
        &self.outcome
    }

    /// Host detail, when the short outcome is not enough.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

fn write_row_reason(text: &mut String, row: &BulkRow, reason: &str) {
    let _ = write!(text, "{}: ", row.profile);
    if let Some(detail) = row.detail.as_deref() {
        let detail_includes_reason = detail == reason
            || detail
                .strip_prefix(reason)
                .is_some_and(|suffix| suffix.starts_with(':'));
        if detail_includes_reason {
            text.push_str(detail);
        } else {
            let _ = write!(text, "{reason} — {detail}");
        }
    } else {
        text.push_str(reason);
    }
}

/// The report of one command on the marked rows.
#[derive(Debug, PartialEq, Eq)]
pub struct BulkReport {
    label: &'static str,
    done_verb: &'static str,
    rows: Vec<BulkRow>,
    /// Set when the whole command was refused before any bot was asked
    /// (for a walk: no destination, stale map binding, no navigation).
    refusal: Option<String>,
}

impl BulkReport {
    /// `label` heads the summary (`Walk marked`); `done_verb` names the
    /// bots the command took effect on (`walking`).
    pub fn new(
        label: &'static str,
        done_verb: &'static str,
        mut rows: Vec<BulkRow>,
        refusal: Option<String>,
    ) -> Self {
        rows.sort_by(|a, b| {
            a.outcome
                .rank()
                .cmp(&b.outcome.rank())
                .then_with(|| a.profile.cmp(&b.profile))
        });
        Self {
            label,
            done_verb,
            rows,
            refusal,
        }
    }

    /// A report for a click with nothing marked.
    pub fn none_marked(label: &'static str, done_verb: &'static str) -> Self {
        Self::new(label, done_verb, Vec::new(), None)
    }

    pub fn rows(&self) -> &[BulkRow] {
        &self.rows
    }

    /// The reason the whole command was refused, when it was.
    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// Names of the bots the command took effect on, in report order.
    pub fn done(&self) -> impl Iterator<Item = &str> + '_ {
        self.rows
            .iter()
            .filter_map(|row| row.outcome.is_done().then_some(row.profile.as_str()))
    }

    pub fn done_count(&self) -> usize {
        self.count(BulkOutcome::is_done)
    }

    pub fn skipped_count(&self) -> usize {
        self.count(BulkOutcome::is_skipped)
    }

    pub fn failed_count(&self) -> usize {
        self.count(BulkOutcome::is_failed)
    }

    pub fn pending_count(&self) -> usize {
        self.count(BulkOutcome::is_pending)
    }

    fn count(&self, pick: impl Fn(&BulkOutcome) -> bool) -> usize {
        self.rows.iter().filter(|row| pick(&row.outcome)).count()
    }

    /// `Label: verb D, skipped K[, pending P][, failed F]`, then each
    /// failure, skip and retained pending reason. Successful detailed rows
    /// are also named. A refused command names its one cause instead.
    pub fn summary(&self) -> String {
        if let Some(reason) = &self.refusal {
            return format!(
                "{}: {reason} ({} marked bots not sent)",
                self.label,
                self.rows.len()
            );
        }
        if self.rows.is_empty() {
            return format!("{}: no bots marked", self.label);
        }
        let (done, skipped, pending, failed) = (
            self.done_count(),
            self.skipped_count(),
            self.pending_count(),
            self.failed_count(),
        );
        let mut text = format!(
            "{}: {} {done}, skipped {skipped}",
            self.label, self.done_verb
        );
        if pending > 0 {
            let _ = write!(text, ", pending {pending}");
        }
        if failed > 0 {
            let _ = write!(text, ", failed {failed}");
        }
        let mut sep = ": ";
        for row in self
            .rows
            .iter()
            .filter(|row| row.outcome.is_failed())
            .take(LISTED)
        {
            let Some(reason) = row.outcome.reason() else {
                continue;
            };
            let _ = write!(text, "{sep}");
            write_row_reason(&mut text, row, reason);
            sep = "; ";
        }
        let mut sep = if failed > 0 { "; skipped " } else { ": " };
        for row in self
            .rows
            .iter()
            .filter(|row| row.outcome.is_skipped())
            .take(LISTED)
        {
            let Some(reason) = row.outcome.reason() else {
                continue;
            };
            let _ = write!(text, "{sep}");
            write_row_reason(&mut text, row, reason);
            sep = ", ";
        }
        if pending > 0 {
            let mut sep = if failed > 0 || skipped > 0 {
                "; "
            } else {
                ": "
            };
            for row in self
                .rows
                .iter()
                .filter(|row| row.outcome.is_pending())
                .take(LISTED)
            {
                let _ = write!(
                    text,
                    "{sep}{}: {}",
                    row.profile,
                    row.detail.as_deref().unwrap_or("pending")
                );
                sep = ", ";
            }
        }
        let mut detail_sep = if failed > 0 || skipped > 0 || pending > 0 {
            "; "
        } else {
            ": "
        };
        for row in self
            .rows
            .iter()
            .filter(|row| row.outcome.is_done() && row.detail.is_some())
            .take(LISTED)
        {
            let detail = row.detail.as_deref().unwrap_or_default();
            let _ = write!(text, "{detail_sep}{}: {detail}", row.profile);
            detail_sep = "; ";
        }
        text
    }
}

impl fmt::Display for BulkReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

#[cfg(test)]
mod tests {
    use super::{BulkOutcome, BulkReport, BulkRow};

    /// The banner names six of each kind and the counts cover the rest, so a
    /// large fleet never produces a runaway line.
    #[test]
    fn summary_lists_a_bounded_number_of_names_per_kind() {
        let rows = (0..9)
            .map(|i| BulkRow::new(format!("gw{i}"), BulkOutcome::skipped("not logged in")))
            .collect();
        let summary = BulkReport::new("Walk marked", "walking", rows, None).summary();
        assert!(summary.contains("gw5: not logged in"), "{summary}");
        assert!(!summary.contains("gw6"), "{summary}");
    }

    /// Failures head the line even when they sort after every other row.
    #[test]
    fn failures_are_listed_before_skips_and_successes() {
        let rows = vec![
            BulkRow::new("alice", BulkOutcome::Done),
            BulkRow::new("bob", BulkOutcome::skipped("running a script")),
            BulkRow::new("zed", BulkOutcome::failed("no path")),
        ];
        let report = BulkReport::new("Walk marked", "walking", rows, None);
        assert_eq!(
            report
                .rows()
                .iter()
                .map(|r| r.profile.as_str())
                .collect::<Vec<_>>(),
            ["zed", "bob", "alice"]
        );
    }

    #[test]
    fn pending_demands_are_counted_and_named_without_becoming_failures() {
        let report = BulkReport::new(
            "Assign marked",
            "assigned",
            vec![BulkRow::new("alice", BulkOutcome::Pending).with_detail("catalog scan pending")],
            None,
        );

        assert_eq!(report.pending_count(), 1);
        assert_eq!(report.failed_count(), 0);
        assert_eq!(
            report.summary(),
            "Assign marked: assigned 0, skipped 0, pending 1: alice: catalog scan pending"
        );
    }
}
