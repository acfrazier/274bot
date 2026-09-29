//! Shared marked-bot selection and the small bulk command adapter.
//!
//! A mark is keyed by [`ProfileIdentity`], never by the profile's display
//! name.  Usernames are editable presentation data; the vault UID is the
//! account/profile identity that survives a rename.  Deleting a profile
//! removes its identity from the selection, while renaming leaves it marked.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use crate::scripts::Scripts;
use crate::session::OperatorSession;

/// Stable identity used by front-end selection state.
///
/// Vault profile UIDs are signed because the host handshake stores them as an
/// `i32`; the bit-preserving conversion keeps negative fixture UIDs valid too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileIdentity(u64);

impl ProfileIdentity {
    /// Construct an identity from the stable UID stored in a vault profile.
    pub const fn uid(uid: i32) -> Self {
        Self(uid as i64 as u64)
    }

    /// A deterministic fallback for UI-only fixtures that have no vault row.
    /// Production rows should always use [`Self::uid`].
    pub fn synthetic(name: &str) -> Self {
        // FNV-1a with a high-bit domain marker keeps synthetic IDs disjoint
        // from ordinary positive account UIDs used by the host.
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in name.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        Self(hash | (1_u64 << 63))
    }

    /// The raw identity value, useful only for diagnostics and widget IDs.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl From<i32> for ProfileIdentity {
    fn from(uid: i32) -> Self {
        Self::uid(uid)
    }
}

/// The shared checkbox state used by panel, TUI and later fleet actions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkedSelection {
    marks: BTreeSet<ProfileIdentity>,
}

impl MarkedSelection {
    pub fn len(&self) -> usize {
        self.marks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    pub fn contains(&self, identity: ProfileIdentity) -> bool {
        self.marks.contains(&identity)
    }

    /// Toggle one identity and return its new marked state.
    pub fn toggle(&mut self, identity: ProfileIdentity) -> bool {
        if !self.marks.insert(identity) {
            self.marks.remove(&identity);
            false
        } else {
            true
        }
    }

    pub fn set(&mut self, identity: ProfileIdentity, marked: bool) {
        if marked {
            self.marks.insert(identity);
        } else {
            self.marks.remove(&identity);
        }
    }

    pub fn clear(&mut self) {
        self.marks.clear();
    }

    /// Mark every identity in the iterator. Existing marks are retained.
    pub fn mark_all<I>(&mut self, identities: I)
    where
        I: IntoIterator<Item = ProfileIdentity>,
    {
        self.marks.extend(identities);
    }

    /// Remove a deleted profile's mark. Returns whether a mark was removed.
    pub fn remove(&mut self, identity: ProfileIdentity) -> bool {
        self.marks.remove(&identity)
    }

    /// Rename does not change a profile identity. This explicit operation is
    /// useful to adapters that have to migrate a profile whose UID changed
    /// while preserving the logical profile row.
    pub fn rename(&mut self, old: ProfileIdentity, new: ProfileIdentity) {
        if old != new && self.marks.remove(&old) {
            self.marks.insert(new);
        }
    }

    /// Drop marks that no longer have a visible profile row.
    pub fn retain<I>(&mut self, identities: I)
    where
        I: IntoIterator<Item = ProfileIdentity>,
    {
        let identities: BTreeSet<_> = identities.into_iter().collect();
        self.marks.retain(|identity| identities.contains(identity));
    }

    pub fn iter(&self) -> impl Iterator<Item = ProfileIdentity> + '_ {
        self.marks.iter().copied()
    }
}

/// Which command a marked fleet action issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkAction {
    Start,
    Stop,
}

impl BulkAction {
    fn label(self) -> &'static str {
        match self {
            Self::Start => "Start selected",
            Self::Stop => "Stop selected",
        }
    }
}

/// One marked row that could not accept a bulk command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkSkip {
    pub profile: String,
    pub reason: String,
}

/// One report for a marked command. Every requested row is either counted as
/// accepted or appears once in `skipped`; no per-row banner replaces this
/// summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkReport {
    pub action: BulkAction,
    pub requested: usize,
    pub affected: usize,
    pub skipped: Vec<BulkSkip>,
}

impl BulkReport {
    pub fn summary(&self) -> String {
        let verb = match self.action {
            BulkAction::Start => "started",
            BulkAction::Stop => "stopped",
        };
        let mut text = format!(
            "{}: {} {}, skipped {}",
            self.action.label(),
            verb,
            self.affected,
            self.skipped.len()
        );
        if !self.skipped.is_empty() {
            text.push_str(": ");
            for (index, skip) in self.skipped.iter().enumerate() {
                if index > 0 {
                    text.push_str(", ");
                }
                text.push_str(&skip.profile);
                text.push_str(": ");
                text.push_str(&skip.reason);
            }
        }
        text
    }
}

impl fmt::Display for BulkReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

/// Start the selected card (or each profile's saved assignment when `card` is
/// `None`) on marked profiles. The existing per-profile `Scripts` API remains
/// the command path; this function only freezes the marked scope and folds a
/// single report.
pub fn start_marked<Io>(
    selection: &MarkedSelection,
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    card: Option<&script::ScriptSel>,
    catalog_root: Option<&Path>,
) -> BulkReport {
    let mut report = BulkReport {
        action: BulkAction::Start,
        requested: selection.len(),
        affected: 0,
        skipped: Vec::new(),
    };
    let batch = scripts.open_bulk("Start selected");
    let profiles = profile_rows(core);
    for identity in selection.iter() {
        let Some((name, _)) = profiles.iter().find(|(_, id)| *id == identity) else {
            report.skipped.push(BulkSkip {
                profile: format!("profile#{}", identity.raw()),
                reason: "profile unavailable".into(),
            });
            scripts.bulk_skip(
                batch,
                &format!("profile#{}", identity.raw()),
                "profile unavailable",
            );
            continue;
        };
        if scripts.start_queue_place(name).is_some() {
            continue;
        }
        if core.play().is_none() {
            report.skipped.push(skip(name, "no play"));
            scripts.bulk_fail(batch, name, "no play");
            continue;
        }
        if script_active(core, name) {
            report.skipped.push(skip(name, "script already active"));
            scripts.bulk_skip(batch, name, "already active");
            continue;
        }
        let result = match card {
            Some(card) => scripts.queue_start(
                core,
                name,
                card.clone(),
                crate::scripts::StartKind::Start,
                None,
                batch,
            ),
            None => match Scripts::assignment(core, name)
                .and_then(|a| crate::scripts::sel_from_assignment(&a))
            {
                Some(sel) => scripts.queue_start(
                    core,
                    name,
                    sel.clone(),
                    crate::scripts::StartKind::Start,
                    Some(sel),
                    batch,
                ),
                None => Err("no assignment".to_string()),
            },
        };
        match result {
            Ok(()) => report.affected += 1,
            Err(reason) => {
                scripts.bulk_fail(batch, name, reason.as_str());
                report.skipped.push(skip(name, reason));
            }
        }
    }
    scripts.admit_starts(core, catalog_root);
    scripts.publish_bulk();
    report
}

/// Stop the selected profiles through [`OperatorSession::stop_scripts`]. A
/// row already stopping is not sent a duplicate command; a repeated command
/// therefore reports an ineligible row rather than enqueueing another stop.
pub fn stop_marked<Io>(
    selection: &MarkedSelection,
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
) -> BulkReport {
    let mut report = BulkReport {
        action: BulkAction::Stop,
        requested: selection.len(),
        affected: 0,
        skipped: Vec::new(),
    };
    let profiles = profile_rows(core);
    let mut candidates = Vec::new();
    for identity in selection.iter() {
        let Some((name, _)) = profiles.iter().find(|(_, id)| *id == identity) else {
            report.skipped.push(BulkSkip {
                profile: format!("profile#{}", identity.raw()),
                reason: "profile unavailable".into(),
            });
            continue;
        };
        let Some(play) = core.play() else {
            report.skipped.push(skip(name, "no play"));
            continue;
        };
        match play.script_state(name) {
            script::RunState::Starting | script::RunState::Running | script::RunState::Paused => {
                candidates.push(name.clone())
            }
            script::RunState::Stopping => report.skipped.push(skip(name, "already stopping")),
            script::RunState::Idle | script::RunState::Error => {
                if scripts.cancel_queued(name) {
                    report.affected += 1;
                } else {
                    report.skipped.push(skip(name, "no script"));
                }
            }
        }
    }
    if !candidates.is_empty() {
        let (_, stopped) = core.stop_scripts(&candidates);
        report.affected += stopped;
    }
    scripts.publish_start_places(core);
    report
}

fn profile_rows<Io>(core: &OperatorSession<Io>) -> Vec<(String, ProfileIdentity)> {
    core.vault()
        .map(|vault| {
            vault
                .profiles()
                .map(|profile| (profile.username.clone(), ProfileIdentity::uid(profile.uid)))
                .collect()
        })
        .unwrap_or_default()
}

fn script_active<Io>(core: &OperatorSession<Io>, name: &str) -> bool {
    matches!(
        core.play()
            .map_or(script::RunState::Idle, |play| play.script_state(name)),
        script::RunState::Starting
            | script::RunState::Running
            | script::RunState::Paused
            | script::RunState::Stopping
    )
}

fn skip(profile: &str, reason: impl Into<String>) -> BulkSkip {
    BulkSkip {
        profile: profile.to_string(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{MarkedSelection, ProfileIdentity};

    #[test]
    fn marks_are_identity_based_and_survive_filter_and_sort_order() {
        let alice = ProfileIdentity::uid(11);
        let bob = ProfileIdentity::uid(22);
        let carol = ProfileIdentity::uid(33);
        let mut marked = MarkedSelection::default();
        marked.toggle(alice);
        marked.toggle(carol);

        // A UI may filter and sort its rows without touching the model.
        let filtered_sorted = [carol, alice, bob];
        assert_eq!(
            filtered_sorted
                .iter()
                .filter(|id| marked.contains(**id))
                .count(),
            2
        );
        assert!(marked.contains(alice));
        assert!(marked.contains(carol));
        assert!(!marked.contains(bob));
    }

    #[test]
    fn delete_removes_a_mark_and_rename_keeps_it() {
        let old = ProfileIdentity::uid(7);
        let replacement = ProfileIdentity::uid(8);
        let mut marked = MarkedSelection::default();
        marked.toggle(old);
        marked.rename(old, replacement);
        assert!(marked.contains(replacement));
        marked.remove(replacement);
        assert!(marked.is_empty());
    }

    #[test]
    fn toggling_is_idempotent_for_duplicate_commands() {
        let id = ProfileIdentity::uid(4);
        let mut marked = MarkedSelection::default();
        assert!(marked.toggle(id));
        assert!(!marked.toggle(id));
        assert_eq!(marked.len(), 0);
        marked.set(id, true);
        marked.set(id, true);
        assert_eq!(marked.len(), 1);
    }
}
