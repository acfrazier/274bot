//! Bulk parameter sync ("Apply to all"): copy one profile's overrides bag
//! for a card to every other wall member assigned the same card. Prepare
//! freezes the scope (members, the bag snapshot) for confirmation; Apply
//! writes each same-card member's profile through the profile writer and,
//! once each write is durable, posts the merged bag to that member's run
//! of the card captured at Apply (identity and generation fenced).
//! Members on another card are skipped and counted. Persistence and live
//! delivery are reported separately.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};

use super::{LiveDelivery, LiveSettings, Scripts, SettingsResult, SettingsWrite};
use crate::operations::{ActionKind, OperationId, Outcome};
use crate::session::{ArmMirror, OperatorSession};

const OTHER_CARD: &str = "assigned another card";
const UNASSIGNED: &str = "no assignment";

/// A prepared sync, frozen for confirmation.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncScope {
    pub source: String,
    /// Canonical card identity the bag belongs to.
    pub card: String,
    pub card_name: String,
    /// Wall members assigned the same card when prepared.
    pub targets: Vec<String>,
    /// Other wall members, with the reason each is skipped.
    pub skipped: Vec<(String, String)>,
    /// The source's overrides bag copied to every target.
    pub overrides: Map<String, Value>,
    card_source: script::ScriptSource,
    lookup: String,
    prompt: String,
}

impl SyncScope {
    /// The confirmation line: card, source and frozen scope.
    pub fn prompt(&self) -> &str {
        &self.prompt
    }
}

/// Result of one applied sync. Counts fill in as member writes settle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub op: OperationId,
    pub source: String,
    pub card: String,
    pub card_name: String,
    pub skipped: Vec<(String, String)>,
    pub saved: usize,
    /// A newer edit of the member owns its value.
    pub superseded: usize,
    pub failed: Vec<(String, String)>,
    pub delivered: usize,
    pub unchanged: usize,
    pub stale: usize,
    pub not_running: usize,
    /// Member writes not yet settled: (write operation, member).
    pending: Vec<(OperationId, String)>,
    text: String,
}

impl SyncReport {
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn is_settled(&self) -> bool {
        self.pending.is_empty()
    }

    /// One line for the operator: persistence and live delivery counted
    /// separately. Rebuilt only when a member settles.
    pub fn summary(&self) -> &str {
        &self.text
    }

    fn refresh(&mut self) {
        let mut text = format!("Apply to all {}: saved {}", self.card_name, self.saved);
        let counts = [
            (self.failed.len(), "failed"),
            (self.superseded, "superseded"),
            (self.pending.len(), "saving"),
        ];
        for (n, label) in counts {
            if n > 0 {
                text.push_str(&format!(", {label} {n}"));
            }
        }
        if !self.skipped.is_empty() {
            let other = self
                .skipped
                .iter()
                .filter(|(_, reason)| reason == OTHER_CARD)
                .count();
            let unassigned = self.skipped.len() - other;
            text.push_str(&format!(", skipped {} (", self.skipped.len()));
            if other > 0 {
                text.push_str(&format!("{other} other card"));
            }
            if unassigned > 0 {
                if other > 0 {
                    text.push_str(", ");
                }
                text.push_str(&format!("{unassigned} unassigned"));
            }
            text.push(')');
        }
        let live = [
            (self.delivered, "delivered"),
            (self.unchanged, "unchanged"),
            (self.stale, "stale (restarted)"),
            (self.not_running, "not running"),
        ];
        let mut first = true;
        for (n, label) in live {
            if n > 0 {
                text.push_str(if first { "; live: " } else { ", " });
                text.push_str(&format!("{label} {n}"));
                first = false;
            }
        }
        for (member, error) in self.failed.iter().take(4) {
            text.push_str(&format!("; {member}: {error} (not pushed)"));
        }
        self.text = text;
    }
}

#[derive(Default)]
pub(super) struct SyncState {
    prepared: Option<SyncScope>,
    /// The newest applied sync, shown to the operator.
    last: Option<SyncReport>,
    /// Older syncs with member writes still in flight. Each settles its own
    /// operation and is dropped then; bounded by the writes in flight.
    earlier: Vec<SyncReport>,
    /// The last report settled and its final summary is not shown yet.
    announce: bool,
}

impl SyncState {
    /// A parameter edit on the source changes the bag a prepared sync
    /// would copy: drop it so a stale snapshot is never applied.
    pub(super) fn source_edited(&mut self, profile: &str, card: &str) {
        if self
            .prepared
            .as_ref()
            .is_some_and(|scope| scope.source == profile && scope.card == card)
        {
            self.prepared = None;
        }
    }

    /// Fold one settled write into the report it belongs to. Returns the
    /// sync operation and the member outcome to record there.
    pub(super) fn record(&mut self, write: &SettingsWrite) -> Option<(OperationId, Outcome)> {
        let owns = |report: &SyncReport| {
            report
                .pending
                .iter()
                .position(|(op, member)| *op == write.op && *member == write.profile)
        };
        if let Some((slot, index)) = self
            .earlier
            .iter()
            .enumerate()
            .find_map(|(slot, report)| owns(report).map(|index| (slot, index)))
        {
            let report = &mut self.earlier[slot];
            report.pending.swap_remove(index);
            let settled = (report.op, fold(report, write));
            if report.pending.is_empty() {
                self.earlier.swap_remove(slot);
            }
            return Some(settled);
        }
        let report = self.last.as_mut()?;
        let index = owns(report)?;
        report.pending.swap_remove(index);
        let outcome = fold(report, write);
        report.refresh();
        if report.pending.is_empty() {
            self.announce = true;
        }
        Some((report.op, outcome))
    }

    /// Replace the shown report; one still waiting on writes keeps settling
    /// in `earlier`.
    fn push(&mut self, report: SyncReport) {
        if let Some(previous) = self.last.replace(report) {
            if !previous.is_settled() {
                self.earlier.push(previous);
            }
        }
    }

    pub(super) fn take_settled_summary(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.announce) {
            return None;
        }
        self.last.as_ref().map(|report| report.text.clone())
    }
}

/// Count one settled member write into `report`; the member's outcome.
fn fold(report: &mut SyncReport, write: &SettingsWrite) -> Outcome {
    match &write.result {
        SettingsResult::Saved(live) => {
            report.saved += 1;
            match live {
                LiveDelivery::Delivered => report.delivered += 1,
                LiveDelivery::Unchanged => report.unchanged += 1,
                LiveDelivery::Stale => report.stale += 1,
                LiveDelivery::NotRunning => report.not_running += 1,
            }
            Outcome::Completed
        }
        SettingsResult::Superseded => {
            report.superseded += 1;
            Outcome::Cancelled
        }
        SettingsResult::Failed(error) => {
            report.failed.push((write.profile.clone(), error.clone()));
            Outcome::Failed(error.clone())
        }
    }
}

impl Scripts {
    /// Freeze an Apply-to-all of `source`'s parameters for one card: the
    /// wall members assigned the same card and the overrides bag to copy.
    /// Nothing is written until [`Self::apply_settings_sync`].
    pub fn prepare_settings_sync<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        source: &str,
        card_source: script::ScriptSource,
        card_name: &str,
        card_path: &Path,
    ) -> &SyncScope {
        let card = script::card_identity_key(card_source, card_path, card_name);
        let overrides = self.profile_overrides(core, source, &card, card_name);
        let mut targets = Vec::new();
        let mut skipped = Vec::new();
        for member in core.members().iter().filter(|m| *m != source) {
            match Self::assignment(core, member) {
                Some(asg) if asg.key() == card => targets.push(member.clone()),
                Some(_) => skipped.push((member.clone(), OTHER_CARD.to_string())),
                None => skipped.push((member.clone(), UNASSIGNED.to_string())),
            }
        }
        let prompt = format!(
            "Copy {card_name} parameters from {source} to {} same-card member(s); {} other member(s) skipped.",
            targets.len(),
            skipped.len()
        );
        self.sync.prepared.insert(SyncScope {
            source: source.to_string(),
            card,
            card_name: card_name.to_string(),
            targets,
            skipped,
            overrides,
            card_source,
            lookup: super::lookup_name(card_source, card_name, card_path),
            prompt,
        })
    }

    pub fn prepared_settings_sync(&self) -> Option<&SyncScope> {
        self.sync.prepared.as_ref()
    }

    pub fn cancel_settings_sync(&mut self) {
        self.sync.prepared = None;
    }

    pub fn last_settings_sync(&self) -> Option<&SyncReport> {
        self.sync.last.as_ref()
    }

    /// Apply the prepared sync. Each target still assigned the card gets
    /// the snapshot bag in its profile; a target whose run of the card is
    /// live at this moment receives the merged bag once its write is
    /// durable, fenced to that run. Returns the sync operation.
    pub fn apply_settings_sync<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
    ) -> Result<OperationId, String> {
        let scope = self
            .sync
            .prepared
            .take()
            .ok_or_else(|| "Apply to all: nothing prepared".to_string())?;
        let SyncScope {
            source,
            card,
            card_name,
            targets,
            mut skipped,
            overrides,
            card_source,
            lookup,
            ..
        } = scope;
        let schema = self
            .js
            .get(card_source, &lookup)
            .map(|c| c.settings_schema.as_slice())
            .unwrap_or_default();
        // One merged bag for every live target.
        let bag = Arc::new(script::merge_bag(schema, &overrides, self.inject.as_ref()));
        let op = core.open_operation(ActionKind::SyncSettings);
        let mut failed = Vec::new();
        let mut pending = Vec::new();
        for target in targets {
            let row = core.vault().and_then(|v| v.get(&target)).cloned();
            let Some(mut row) = row else {
                failed.push((target, "no profile".to_string()));
                continue;
            };
            let same_card = row
                .settings
                .script_assignment
                .as_ref()
                .is_some_and(|asg| asg.key() == card);
            if !same_card {
                skipped.push((target, OTHER_CARD.to_string()));
                continue;
            }
            row.settings
                .script_settings
                .insert(card.clone(), overrides.clone());
            let live = super::live_fence(core, &target, &card).map(|(identity, generation)| {
                LiveSettings {
                    identity,
                    generation,
                    bag: Arc::clone(&bag),
                }
            });
            match core.save_profile(row, ArmMirror::ScriptSettings(live), "apply to all") {
                Ok(write) => pending.push((write, target)),
                Err(error) => failed.push((target, error)),
            }
        }
        for (member, reason) in &skipped {
            core.set_outcome(op, member, Outcome::Skipped(reason.clone()));
        }
        for (member, error) in &failed {
            core.set_outcome(op, member, Outcome::Failed(error.clone()));
        }
        for (_, member) in &pending {
            core.set_outcome(op, member, Outcome::Pending);
        }
        let mut report = SyncReport {
            op,
            source,
            card,
            card_name,
            skipped,
            saved: 0,
            superseded: 0,
            failed,
            delivered: 0,
            unchanged: 0,
            stale: 0,
            not_running: 0,
            pending,
            text: String::new(),
        };
        report.refresh();
        // Shown now; `record` announces the final summary once writes settle.
        self.sync.announce = false;
        self.show(report.text.clone());
        self.sync.push(report);
        Ok(op)
    }
}
