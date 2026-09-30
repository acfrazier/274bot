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
const NOT_MARKED: &str = "not marked";

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
    selection: script::ScriptSel,
    /// Per-target fields kept locally, published before any preparation/save.
    pub excluded: Vec<(String, Vec<String>)>,
    field: Option<String>,
    prompt: String,
}

impl SyncScope {
    /// The confirmation line: card, source and frozen scope.
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Keep only the targets `keep` accepts; the rest are skipped as not
    /// marked, and the confirmation line names the narrowed scope.
    fn restrict(&mut self, keep: impl Fn(&str) -> bool) {
        let (kept, dropped): (Vec<_>, Vec<_>) = std::mem::take(&mut self.targets)
            .into_iter()
            .partition(|target| keep(target));
        self.targets = kept;
        self.excluded
            .retain(|(target, _)| self.targets.contains(target));
        self.skipped.extend(
            dropped
                .into_iter()
                .map(|target| (target, NOT_MARKED.to_string())),
        );
        let mut prompt = format!(
            "Copy {} parameters from {} to {} marked same-card member(s); {} other member(s) skipped.",
            self.card_name,
            self.source,
            self.targets.len(),
            self.skipped.len()
        );
        for (target, fields) in &self.excluded {
            prompt.push_str(&format!(" {target}: preserve {}.", fields.join(", ")));
        }
        self.prompt = prompt;
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
    /// A newer edit of this card's parameters on the member (on a failed
    /// save, a newer write of the member) owns its value.
    pub superseded: usize,
    pub failed: Vec<(String, String)>,
    pub delivered: usize,
    pub unchanged: usize,
    pub stale: usize,
    pub not_running: usize,
    pub applied: usize,
    pub pending_boundary: usize,
    pub restart_required: usize,
    pub live_rejected: Vec<(String, String)>,
    pub excluded: Vec<(String, Vec<String>)>,
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
            (self.applied, "applied"),
            (self.pending_boundary, "pending boundary"),
            (self.restart_required, "restart required"),
            (self.live_rejected.len(), "rejected"),
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
        for (member, fields) in &self.excluded {
            text.push_str(&format!("; {member}: preserved {}", fields.join(", ")));
        }
        for (member, error) in &self.live_rejected {
            text.push_str(&format!("; {member}: saved, live rejected: {error}"));
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
                LiveDelivery::Applied => report.applied += 1,
                LiveDelivery::PendingBoundary => report.pending_boundary += 1,
                LiveDelivery::RestartRequired => report.restart_required += 1,
                LiveDelivery::Rejected(error) => {
                    report
                        .live_rejected
                        .push((write.profile.clone(), error.clone()));
                    return Outcome::Failed(format!("saved, live rejected: {error}"));
                }
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
            selection: script::ScriptSel::Loaded(
                card_source,
                super::lookup_name(card_source, card_name, card_path),
            ),
            excluded: Vec::new(),
            field: None,
            prompt,
        })
    }

    pub fn prepare_compiled_settings_sync<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        source: &str,
        id: script::CompiledId,
        field: Option<&str>,
    ) -> Result<&SyncScope, String> {
        self.sync.prepared = None;
        let descriptor = script::compiled_card(id).ok_or("compiled card unavailable")?;
        let mut excluded_fields: Vec<String> = descriptor
            .per_account_settings
            .iter()
            .map(|field| (*field).into())
            .collect();
        if !excluded_fields
            .iter()
            .any(|field| field == script::CLUE_DUEL_PARTNER)
        {
            excluded_fields.push(script::CLUE_DUEL_PARTNER.into());
        }
        if let Some(field) = field {
            if excluded_fields.iter().any(|excluded| excluded == field) {
                return Err(format!(
                    "Apply to all: {field} is per-account and cannot be copied"
                ));
            }
            if !(descriptor.schema)()
                .iter()
                .any(|setting| setting.id == field)
            {
                return Err(format!("Apply to all: unknown setting {field}"));
            }
        }
        let card = script::compiled_identity_key(id);
        let overrides = self.compiled_overrides(core, source, id)?;
        let mut targets = Vec::new();
        let mut skipped = Vec::new();
        for member in core.members().iter().filter(|member| *member != source) {
            match Self::assignment(core, member) {
                Some(assignment) if assignment.key() == card => targets.push(member.clone()),
                Some(_) => skipped.push((member.clone(), OTHER_CARD.into())),
                None => skipped.push((member.clone(), UNASSIGNED.into())),
            }
        }
        let excluded: Vec<_> = targets
            .iter()
            .map(|target| (target.clone(), excluded_fields.clone()))
            .collect();
        let mut prompt = format!("Copy {} parameters from {source} to {} same-card member(s); {} other member(s) skipped.", descriptor.name, targets.len(), skipped.len());
        for (target, fields) in &excluded {
            prompt.push_str(&format!(" {target}: preserve {}.", fields.join(", ")));
        }
        Ok(self.sync.prepared.insert(SyncScope {
            source: source.into(),
            card,
            card_name: descriptor.name.into(),
            targets,
            skipped,
            overrides,
            selection: script::ScriptSel::Compiled(id),
            excluded,
            field: field.map(str::to_owned),
            prompt,
        }))
    }

    pub fn prepared_settings_sync(&self) -> Option<&SyncScope> {
        self.sync.prepared.as_ref()
    }

    /// Narrow the prepared Apply to all to the members `keep` accepts (the
    /// marked rows): the others are skipped as not marked and the
    /// confirmation names the narrowed scope. No effect when nothing is
    /// prepared.
    pub fn restrict_prepared_settings_sync(&mut self, keep: impl Fn(&str) -> bool) {
        if let Some(scope) = self.sync.prepared.as_mut() {
            scope.restrict(keep);
        }
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
            selection,
            excluded,
            field,
            ..
        } = scope;
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
            let result = match &selection {
                script::ScriptSel::Compiled(id) => {
                    let fields = excluded
                        .iter()
                        .find(|(member, _)| member == &target)
                        .map_or(&[][..], |(_, fields)| fields.as_slice());
                    match self.compiled_overrides(core, &target, *id) {
                        Ok(current) => {
                            let values =
                                bulk_values(&overrides, &current, fields, field.as_deref());
                            self.set_compiled_overrides(core, &target, *id, values)
                        }
                        Err(error) => Err(error),
                    }
                }
                script::ScriptSel::Loaded(source, lookup) => {
                    let schema = self
                        .js
                        .get(*source, lookup)
                        .map_or(&[][..], |card| card.settings_schema.as_slice());
                    row.settings
                        .script_settings
                        .insert(card.clone(), overrides.clone());
                    let live =
                        super::live_fence(core, &target, &card).map(|(identity, generation)| {
                            LiveSettings {
                                identity,
                                generation,
                                run: None,
                                prepared: None,
                                bag: Arc::new(self.run_bag(core, &target, schema, &overrides)),
                            }
                        });
                    core.save_profile(
                        row,
                        ArmMirror::ScriptSettings {
                            card: card.clone(),
                            live,
                        },
                        "apply to all",
                    )
                }
            };
            match result {
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
            applied: 0,
            pending_boundary: 0,
            restart_required: 0,
            live_rejected: Vec::new(),
            excluded,
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

fn bulk_values(
    source: &Map<String, Value>,
    target: &Map<String, Value>,
    excluded: &[String],
    field: Option<&str>,
) -> Map<String, Value> {
    let mut values = if let Some(field) = field {
        let mut values = target.clone();
        if let Some(value) = source.get(field) {
            values.insert(field.into(), value.clone());
        } else {
            values.remove(field);
        }
        values
    } else {
        source.clone()
    };
    for key in excluded {
        if let Some(value) = target.get(key) {
            values.insert(key.clone(), value.clone());
        } else {
            values.remove(key);
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bulk_copy_preserves_absent_and_explicit_empty_pairing_fields() {
        let source =
            json!({"path": "arrav", "declared_gang": "phoenix", "partner_account": "alice"});
        let target = json!({"path": "heroes", "partner_account": "", "local": 7});
        let excluded = vec!["declared_gang".into(), "partner_account".into()];
        let all = bulk_values(
            source.as_object().unwrap(),
            target.as_object().unwrap(),
            &excluded,
            None,
        );
        assert_eq!(
            Value::Object(all),
            json!({"path": "arrav", "partner_account": ""})
        );
        let single = bulk_values(
            source.as_object().unwrap(),
            target.as_object().unwrap(),
            &excluded,
            Some("path"),
        );
        assert_eq!(
            Value::Object(single),
            json!({"path": "arrav", "partner_account": "", "local": 7})
        );
        let cleared = bulk_values(
            &Map::new(),
            target.as_object().unwrap(),
            &excluded,
            Some("path"),
        );
        assert_eq!(
            Value::Object(cleared),
            json!({"partner_account": "", "local": 7})
        );
    }
}
