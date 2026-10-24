//! Shared marked-bot selection used by the panel, TUI and frontend commands.
//!
//! A mark is keyed by [`ProfileIdentity`], never by the profile's display
//! name.  Usernames are editable presentation data; the vault UID is the
//! account/profile identity that survives a rename.  Deleting a profile
//! removes its identity from the selection, while renaming leaves it marked.

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::path::Path;

use crate::operations::OperationId;
use crate::scripts::ScriptChoice;
use crate::scripts::Scripts;
use crate::session::OperatorSession;

/// Shown next to a marked-bots control when nothing is marked, so a
/// first-time operator can find the Fleet marks.
pub const EMPTY_MARKS_HINT: &str = "Marks are made in the Fleet window.";

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

/// The identity of member `index`: the caller's parallel vault identity
/// when it has one, else the deterministic fixture identity of its name.
/// `None` only when `index` has no name.
pub fn profile_id_at(
    ids: &[ProfileIdentity],
    names: &[String],
    index: usize,
) -> Option<ProfileIdentity> {
    ids.get(index).copied().or_else(|| {
        names
            .get(index)
            .map(|name| ProfileIdentity::synthetic(name))
    })
}

/// The identity of the member called `name` (see [`profile_id_at`]); a
/// name with no member gets its fixture identity, which marks nothing.
pub fn profile_id_for_name(
    names: &[String],
    ids: &[ProfileIdentity],
    name: &str,
) -> ProfileIdentity {
    names
        .iter()
        .position(|candidate| candidate == name)
        .and_then(|index| profile_id_at(ids, names, index))
        .unwrap_or_else(|| ProfileIdentity::synthetic(name))
}

/// The shared checkbox state used by panel, TUI and later fleet actions.
#[derive(Debug, Default, PartialEq, Eq)]
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

    #[cfg(test)]
    /// Toggle one identity and return its new marked state.
    pub(crate) fn toggle(&mut self, identity: ProfileIdentity) -> bool {
        if !self.marks.insert(identity) {
            self.marks.remove(&identity);
            false
        } else {
            true
        }
    }

    pub(crate) fn set(&mut self, identity: ProfileIdentity, marked: bool) {
        if marked {
            self.marks.insert(identity);
        } else {
            self.marks.remove(&identity);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.marks.clear();
    }

    /// Mark every identity in the iterator. Existing marks are retained.
    pub(crate) fn mark_all<I>(&mut self, identities: I)
    where
        I: IntoIterator<Item = ProfileIdentity>,
    {
        self.marks.extend(identities);
    }

    /// Remove a deleted profile's mark. Returns whether a mark was removed.
    pub(crate) fn remove(&mut self, identity: ProfileIdentity) -> bool {
        self.marks.remove(&identity)
    }

    #[cfg(test)]
    /// Rename does not change a profile identity. This explicit operation is
    /// useful to adapters that have to migrate a profile whose UID changed
    /// while preserving the logical profile row.
    pub(crate) fn rename(&mut self, old: ProfileIdentity, new: ProfileIdentity) {
        if old != new && self.marks.remove(&old) {
            self.marks.insert(new);
        }
    }

    /// Drop marks absent from `sorted`, a caller-kept slice of universe
    /// identities in ascending order (duplicates are harmless): the fleet
    /// model's prune when its universe is rebuilt. Membership is a binary
    /// search over the borrowed slice, so nothing is collected.
    pub(crate) fn retain_sorted(&mut self, sorted: &[ProfileIdentity]) {
        self.marks
            .retain(|identity| sorted.binary_search(identity).is_ok());
    }

    pub fn iter(&self) -> impl Iterator<Item = ProfileIdentity> + '_ {
        self.marks.iter().copied()
    }
}

/// One marked row that could not accept a bulk command.
#[derive(Debug, PartialEq, Eq)]
pub struct BulkSkip {
    profile: String,
    reason: String,
}

impl BulkSkip {
    pub(crate) fn new(profile: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            profile: profile.into(),
            reason: reason.into(),
        }
    }

    /// The marked profile that could not take the command.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// Why it was skipped.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// The report of Stop on marked rows. Every marked row is counted once:
/// stopped, cancelled (it was still waiting for its Start) or skipped.
#[derive(Debug, PartialEq, Eq)]
pub struct StopReport {
    stopped: usize,
    cancelled: usize,
    skipped: Vec<BulkSkip>,
    operation: Option<OperationId>,
}

impl StopReport {
    /// How many marked rows were stopped.
    pub fn stopped(&self) -> usize {
        self.stopped
    }

    /// How many waiting Starts were cancelled.
    pub fn cancelled(&self) -> usize {
        self.cancelled
    }

    /// Rows that could not take Stop.
    pub fn skipped(&self) -> &[BulkSkip] {
        &self.skipped
    }

    /// The core stop operation, when at least one marked script was stopping.
    pub fn operation(&self) -> Option<OperationId> {
        self.operation
    }
}

impl StopReport {
    pub fn summary(&self) -> String {
        let mut text = format!("Stop selected: stopped {}", self.stopped);
        if self.cancelled > 0 {
            let _ = write!(text, ", cancelled {}", self.cancelled);
        }
        let _ = write!(text, ", skipped {}", self.skipped.len());
        let mut sep = ": ";
        for skip in &self.skipped {
            let _ = write!(text, "{sep}{}: {}", skip.profile, skip.reason);
            sep = ", ";
        }
        text
    }
}

impl fmt::Display for StopReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

/// Start the selected card (or each profile's saved assignment when `card` is
/// `None`) on marked profiles. Eligible rows wait for the same paced permit
/// as Start all; the running Start report ([`Scripts::last_bulk_report`])
/// names each row as it is started, skipped or failed.
pub(crate) fn start_marked<Io>(
    selection: &MarkedSelection,
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    card: Option<&ScriptChoice>,
    catalog_root: Option<&Path>,
) -> Vec<String> {
    let card = card.map(ScriptChoice::as_script);
    scripts.open_tally("Start selected");
    let profiles = profile_rows(core);
    let mut accepted = Vec::new();
    for identity in selection.iter() {
        let Some((name, _)) = profiles.iter().find(|(_, id)| *id == identity) else {
            scripts.tally_skip_unavailable(
                &format!("profile#{}", identity.raw()),
                "profile unavailable",
            );
            continue;
        };
        if scripts.start_queue_place(name).is_some() || scripts.has_start_preparation(name) {
            accepted.push(name.clone());
            continue;
        }
        if script_active(core, name) {
            scripts.tally_skip(name, "already active");
            continue;
        }
        let result = match card {
            Some(card) => scripts.queue_start(core, name, card.clone(), None),
            None => match Scripts::assignment(core, name)
                .and_then(|a| crate::scripts::sel_from_assignment(&a))
            {
                Some(sel) => scripts.queue_start(core, name, sel.clone(), Some(sel)),
                None => Err("no assignment".to_string()),
            },
        };
        match result {
            Ok(()) => accepted.push(name.clone()),
            Err(reason) => scripts.tally_fail(name, &reason),
        }
    }
    scripts.admit_starts(core, catalog_root);
    scripts.publish_bulk();
    accepted
}

/// Whether a Start on the marked profiles could start anything, in the
/// terms [`start_marked`] uses: with a selected card, any mark counts
/// (rows that cannot take it are named in its report); without one, at
/// least one marked profile needs a saved assignment [`start_marked`]
/// can start. Front ends use this to enable their Start control, so
/// it only borrows: no assignment or selection is cloned and no
/// profile is looked up by name.
pub fn marked_startable<Io>(
    selection: &MarkedSelection,
    core: &OperatorSession<Io>,
    card: Option<&ScriptChoice>,
) -> bool {
    let card = card.map(ScriptChoice::as_script);
    if selection.is_empty() {
        return false;
    }
    if card.is_some() {
        return true;
    }
    let Some(vault) = core.vault() else {
        return false;
    };
    vault.profiles().any(|profile| {
        selection.contains(ProfileIdentity::uid(profile.uid))
            && profile
                .settings
                .script_assignment
                .as_ref()
                .is_some_and(crate::scripts::assignment_has_sel)
    })
}

/// Stop the selected profiles through [`OperatorSession::stop_scripts`]. A
/// row already stopping is not sent a duplicate command; a repeated command
/// therefore reports an ineligible row rather than enqueueing another stop.
/// A row still waiting for its Start is cancelled, not stopped.
pub fn stop_marked<Io>(
    selection: &MarkedSelection,
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
) -> StopReport {
    let mut report = StopReport {
        stopped: 0,
        cancelled: 0,
        skipped: Vec::new(),
        operation: None,
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
                    report.cancelled += 1;
                } else {
                    report.skipped.push(skip(name, "no script"));
                }
            }
        }
    }
    if !candidates.is_empty() {
        let (operation, stopped) = core.stop_scripts(&candidates);
        report.operation = Some(operation);
        report.stopped = stopped;
    }
    scripts.publish_start_places(core);
    report
}

pub(crate) fn profile_rows<Io>(core: &OperatorSession<Io>) -> Vec<(String, ProfileIdentity)> {
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
    crate::scripts::ScriptRunState::from_script(
        core.play()
            .map_or(script::RunState::Idle, |play| play.script_state(name)),
    )
    .is_live()
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
    fn retain_sorted_drops_marks_outside_the_borrowed_universe() {
        let alice = ProfileIdentity::uid(11);
        let bob = ProfileIdentity::uid(22);
        let carol = ProfileIdentity::uid(33);
        let mut marked = MarkedSelection::default();
        marked.mark_all([alice, bob, carol]);
        marked.retain_sorted(&[alice, carol]);
        assert!(marked.contains(alice));
        assert!(!marked.contains(bob));
        assert!(marked.contains(carol));
        marked.retain_sorted(&[]);
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

    #[test]
    fn marked_command_uses_snapshot_and_reports_each_admission() {
        let alice = ProfileIdentity::uid(11);
        let bob = ProfileIdentity::uid(22);
        let missing = ProfileIdentity::uid(33);
        let mut marked = MarkedSelection::default();
        marked.mark_all([alice, bob, missing]);
        let mut admitted = Vec::new();
        let report = crate::marked::run_marked_command(
            &marked,
            vec![(alice, "alice".to_string()), (bob, "bob".to_string())],
            |profile| {
                admitted.push(profile.to_string());
                (profile == "alice")
                    .then_some(())
                    .ok_or_else(|| "not ready".to_string())
            },
        );
        assert_eq!(admitted, ["alice", "bob"]);
        assert_eq!(report.accepted(), 1);
        assert_eq!(report.total(), 3);
        assert_eq!(
            report.skipped(),
            vec![
                super::BulkSkip::new("bob", "not ready"),
                super::BulkSkip {
                    profile: format!("profile#{}", missing.raw()),
                    reason: "profile unavailable".into(),
                },
            ]
        );
    }
}
