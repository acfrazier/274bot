//! The running report of Start all and Start on marked rows.
//!
//! One tally covers every Start click while any bot it holds is still
//! waiting for a permit or still setting up; the first click after that
//! opens a fresh one. Each bot has exactly one outcome in it, so clicks that
//! overlap neither drop an earlier click's skips and failures nor count a
//! bot twice.

use std::fmt::Write as _;

/// Names listed per outcome kind; the counts cover the rest.
const LISTED: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    Queued,
    Started,
    Skipped(String),
    Failed(String),
}

#[derive(Debug)]
pub(super) struct StartTally {
    label: &'static str,
    bots: Vec<(String, Outcome)>,
}

impl StartTally {
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            bots: Vec::new(),
        }
    }

    pub fn label(&self) -> &'static str {
        self.label
    }

    /// A later click folded into this tally names it.
    pub fn relabel(&mut self, label: &'static str) {
        self.label = label;
    }

    /// Whether any bot is still waiting for its permit.
    pub fn waiting(&self) -> bool {
        self.bots
            .iter()
            .any(|(_, outcome)| *outcome == Outcome::Queued)
    }

    pub fn outcome(&self, profile: &str) -> Option<&Outcome> {
        self.bots
            .iter()
            .find(|(name, _)| name == profile)
            .map(|(_, outcome)| outcome)
    }

    /// Record `profile`'s outcome. Returns whether it changed. A bot this
    /// tally started keeps `started` when a later click finds it already
    /// active.
    pub fn set(&mut self, profile: &str, outcome: Outcome) -> bool {
        match self.bots.iter_mut().find(|(name, _)| name == profile) {
            Some((_, Outcome::Started)) if matches!(outcome, Outcome::Skipped(_)) => false,
            Some((_, current)) if *current == outcome => false,
            Some((_, current)) => {
                *current = outcome;
                true
            }
            None => {
                self.bots.push((profile.to_string(), outcome));
                true
            }
        }
    }

    /// `Start all: started S[, queued Q], skipped K[, failed F]`, then the
    /// failure reasons and the skip reasons. Failures come first so a
    /// one-line banner cut at the terminal width keeps them.
    pub fn report(&self) -> String {
        let (mut started, mut queued, mut skipped, mut failed) = (0, 0, 0, 0);
        for (_, outcome) in &self.bots {
            match outcome {
                Outcome::Queued => queued += 1,
                Outcome::Started => started += 1,
                Outcome::Skipped(_) => skipped += 1,
                Outcome::Failed(_) => failed += 1,
            }
        }
        let mut text = format!("{}: started {started}", self.label);
        if queued > 0 {
            let _ = write!(text, ", queued {queued}");
        }
        let _ = write!(text, ", skipped {skipped}");
        if failed > 0 {
            let _ = write!(text, ", failed {failed}");
        }
        let mut sep = ": ";
        for (name, reason) in self.reasons(|o| match o {
            Outcome::Failed(reason) => Some(reason),
            _ => None,
        }) {
            let _ = write!(text, "{sep}{name}: {reason}");
            sep = "; ";
        }
        let mut sep = if failed > 0 { "; skipped " } else { ": " };
        for (name, reason) in self.reasons(|o| match o {
            Outcome::Skipped(reason) => Some(reason),
            _ => None,
        }) {
            let _ = write!(text, "{sep}{name}: {reason}");
            sep = ", ";
        }
        text
    }

    fn reasons<'a>(
        &'a self,
        pick: impl Fn(&'a Outcome) -> Option<&'a String> + 'a,
    ) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        self.bots
            .iter()
            .filter_map(move |(name, outcome)| {
                pick(outcome).map(|reason| (name.as_str(), reason.as_str()))
            })
            .take(LISTED)
    }
}
