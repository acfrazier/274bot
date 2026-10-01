//! The report of a command run on marked bots (group walk, Assign, log in
//! and out, ...). Every marked row appears exactly once, with one outcome:
//! done, skipped (it could not take the command) or failed (it took the
//! command and the command was refused). Failed rows come first, then
//! skipped, then done, so a one-line banner cut at the terminal width keeps
//! the failures.

use std::fmt::{self, Write as _};

/// Names listed per outcome kind in the summary; the counts cover the rest.
const LISTED: usize = 6;

/// What a command came to for one marked bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkOutcome {
    /// The command took effect (the report's `done` verb).
    Done,
    /// The bot was not eligible; nothing was sent to it.
    Skipped(String),
    /// The bot was eligible but the command was refused.
    Failed(String),
}

impl BulkOutcome {
    fn rank(&self) -> u8 {
        match self {
            Self::Failed(_) => 0,
            Self::Skipped(_) => 1,
            Self::Done => 2,
        }
    }

    pub fn skipped(reason: impl Into<String>) -> Self {
        Self::Skipped(reason.into())
    }

    pub fn failed(reason: impl Into<String>) -> Self {
        Self::Failed(reason.into())
    }
}

/// One marked row's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkRow {
    pub profile: String,
    pub outcome: BulkOutcome,
    /// Host detail carries zone labels or the legacy-grid note through row
    /// inspection and summary rendering without changing the short outcome.
    pub detail: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
        self.rows.iter().filter_map(|row| match row.outcome {
            BulkOutcome::Done => Some(row.profile.as_str()),
            _ => None,
        })
    }

    pub fn done_count(&self) -> usize {
        self.count(|o| matches!(o, BulkOutcome::Done))
    }

    pub fn skipped_count(&self) -> usize {
        self.count(|o| matches!(o, BulkOutcome::Skipped(_)))
    }

    pub fn failed_count(&self) -> usize {
        self.count(|o| matches!(o, BulkOutcome::Failed(_)))
    }

    fn count(&self, pick: impl Fn(&BulkOutcome) -> bool) -> usize {
        self.rows.iter().filter(|row| pick(&row.outcome)).count()
    }

    /// `Label: verb D, skipped K[, failed F]`, then the failure reasons and
    /// the skip reasons plus any per-row host detail. Successful detailed rows
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
        let (done, skipped, failed) =
            (self.done_count(), self.skipped_count(), self.failed_count());
        let mut text = format!(
            "{}: {} {done}, skipped {skipped}",
            self.label, self.done_verb
        );
        if failed > 0 {
            let _ = write!(text, ", failed {failed}");
        }
        let mut sep = ": ";
        for row in self
            .rows
            .iter()
            .filter(|row| matches!(&row.outcome, BulkOutcome::Failed(_)))
            .take(LISTED)
        {
            let BulkOutcome::Failed(reason) = &row.outcome else {
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
            .filter(|row| matches!(&row.outcome, BulkOutcome::Skipped(_)))
            .take(LISTED)
        {
            let BulkOutcome::Skipped(reason) = &row.outcome else {
                continue;
            };
            let _ = write!(text, "{sep}");
            write_row_reason(&mut text, row, reason);
            sep = ", ";
        }
        let mut detail_sep = if failed > 0 || skipped > 0 {
            "; "
        } else {
            ": "
        };
        for row in self
            .rows
            .iter()
            .filter(|row| matches!(&row.outcome, BulkOutcome::Done) && row.detail.is_some())
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
}
