//! Shared marked-bot selection used by the panel, TUI and frontend commands.
//!
//! A mark is keyed by [`ProfileIdentity`], never by the profile's display
//! name.  Usernames are editable presentation data; the vault UID is the
//! account/profile identity that survives a rename.  Deleting a profile
//! removes its identity from the selection, while renaming leaves it marked.

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::path::Path;

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

/// One marked row that could not accept a bulk command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkSkip {
    pub profile: String,
    pub reason: String,
}

/// The report of Stop on marked rows. Every marked row is counted once:
/// stopped, cancelled (it was still waiting for its Start) or skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopReport {
    pub stopped: usize,
    pub cancelled: usize,
    pub skipped: Vec<BulkSkip>,
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
pub fn start_marked<Io>(
    selection: &MarkedSelection,
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    card: Option<&script::ScriptSel>,
    catalog_root: Option<&Path>,
) {
    scripts.open_tally("Start selected");
    let profiles = profile_rows(core);
    for identity in selection.iter() {
        let Some((name, _)) = profiles.iter().find(|(_, id)| *id == identity) else {
            scripts.tally_skip_unavailable(
                &format!("profile#{}", identity.raw()),
                "profile unavailable",
            );
            continue;
        };
        if scripts.start_queue_place(name).is_some() {
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
        if let Err(reason) = result {
            scripts.tally_fail(name, &reason);
        }
    }
    scripts.admit_starts(core, catalog_root);
    scripts.publish_bulk();
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
        let (_, stopped) = core.stop_scripts(&candidates);
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
        assert_eq!(report.accepted, 1);
        assert_eq!(report.total(), 3);
        assert_eq!(
            report.skipped,
            vec![
                super::BulkSkip {
                    profile: "bob".into(),
                    reason: "not ready".into(),
                },
                super::BulkSkip {
                    profile: format!("profile#{}", missing.raw()),
                    reason: "profile unavailable".into(),
                },
            ]
        );
    }
}
