//! The operator's saved Load order, persisted in the preferences document
//! as `profile_order`: an object keyed by each profile's vault identity
//! (its uid, as text) with a sort key per profile. It orders the fleet
//! members, Load all and every fleet view; Name and Status are view sorts
//! that never change it.
//!
//! The stored form is one small member per profile (`"<uid>": <key>`), so
//! the preferences service writes a large order as one row per changed
//! profile (a preference value is at most 4,096 bytes) and a move changes
//! only the moved profiles' keys. Keys leave gaps; when two neighbours
//! have none left the whole order is renumbered. A name is never stored:
//! a rename keeps the profile's identity, so its place follows the vault
//! whether the rename commits or fails. A profile new to the vault is
//! appended; a deleted one's entry stays stored (the service never removes
//! a member) and is ignored. Missing, the order is seeded in vault
//! (alphabetical) order without a write.

use serde_json::Value;

use crate::selection::ProfileIdentity;

/// The preferences key of the saved Load order.
pub const PROFILE_ORDER_KEY: &str = "profile_order";

/// Where a moved profile lands relative to its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Placement {
    /// Just before the target.
    Before(ProfileIdentity),
    /// Just after the target.
    After(ProfileIdentity),
}

/// One reorder of the saved Load order ([`crate::OperatorCommand::ReorderProfiles`],
/// staged with [`crate::OperatorSession::stage_reorder`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReorderOp {
    /// Move one profile before or after another (a drag, or a move key
    /// with the neighbouring shown row as the target). Every other profile,
    /// shown or hidden by a filter, keeps its relative order.
    Move {
        /// The profile that moves.
        moving: ProfileIdentity,
        /// Where it lands.
        to: Placement,
    },
    /// Move the marked profiles to the top, in their current relative
    /// order.
    MarkedToTop,
}

/// The gap between neighbouring sort keys of a seeded or renumbered order.
const KEY_GAP: i64 = 1 << 16;

/// One stored entry: a profile identity (vault uid) and its sort key.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    uid: i32,
    key: i64,
}

/// The saved Load order, reconciled with the vault.
/// Not `Clone`: front ends read it ([`Self::names`]); the session copies it
/// only on an operator change or a write ([`Self::snapshot`],
/// [`Self::assign_from`]), never per frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProfileOrder {
    /// The vault's profiles in order: name, uid and sort key (parallel).
    names: Vec<String>,
    live: Vec<Entry>,
    /// Stored entries of profiles the vault no longer has (kept so a
    /// write never needs a removal).
    stale: Vec<Entry>,
}

impl ProfileOrder {
    /// A copy, taken before a reorder or for a queued write.
    pub(crate) fn snapshot(&self) -> Self {
        Self {
            names: self.names.clone(),
            live: self.live.clone(),
            stale: self.stale.clone(),
        }
    }

    /// Become a copy of `other`, reusing this order's buffers.
    pub(crate) fn assign_from(&mut self, other: &Self) {
        self.names.clone_from(&other.names);
        self.live.clone_from(&other.live);
        self.stale.clone_from(&other.stale);
    }

    /// The order a stored preference value holds: its `"<uid>": <key>`
    /// members (anything else is ignored), ordered by key. Names come from
    /// the vault at [`Self::reconcile`].
    pub(crate) fn from_value(value: Option<&Value>) -> Self {
        let mut order = Self::default();
        if let Some(Value::Object(members)) = value {
            for (uid, key) in members {
                let (Ok(uid), Some(key)) = (uid.parse::<i32>(), key.as_i64()) else {
                    continue;
                };
                order.stale.push(Entry { uid, key });
            }
        }
        order.stale.sort_by_key(|entry| (entry.key, entry.uid));
        order
    }

    /// The stored preference value: every entry, live or stale.
    pub(crate) fn to_value(&self) -> Value {
        let mut members = serde_json::Map::with_capacity(self.live.len() + self.stale.len());
        for entry in self.live.iter().chain(&self.stale) {
            members.insert(entry.uid.to_string(), Value::from(entry.key));
        }
        Value::Object(members)
    }

    /// The names, in order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Whether `name` is in the order.
    pub fn contains(&self, name: &str) -> bool {
        self.names.iter().any(|candidate| candidate == name)
    }

    /// `name`'s place (0 first).
    pub fn position(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|candidate| candidate == name)
    }

    /// Make the order hold exactly the vault's profiles (`vault`: uid and
    /// name, in vault order): a stored profile keeps its place under its
    /// current name, a profile the order lacks is appended (seeded in
    /// vault order when nothing is stored), and an entry the vault lacks
    /// is kept stored but not ordered. Returns whether an entry was added
    /// (a write is needed); a rename alone needs none.
    pub(crate) fn reconcile<'a>(&mut self, vault: impl Iterator<Item = (i32, &'a str)>) -> bool {
        let mut entries: Vec<Entry> = self.live.drain(..).chain(self.stale.drain(..)).collect();
        entries.sort_by_key(|entry| (entry.key, entry.uid));
        let mut profiles: Vec<(i32, &str)> = vault.collect();
        let mut live: Vec<(Entry, String)> = Vec::with_capacity(profiles.len());
        let mut stale = Vec::new();
        for entry in entries {
            match profiles.iter().position(|(uid, _)| *uid == entry.uid) {
                Some(at) => {
                    let (_, name) = profiles.remove(at);
                    live.push((entry, name.to_string()));
                }
                None => stale.push(entry),
            }
        }
        let added = !profiles.is_empty();
        let mut next = live.last().map_or(0, |(entry, _)| entry.key);
        for (uid, name) in profiles {
            next += KEY_GAP;
            live.push((Entry { uid, key: next }, name.to_string()));
        }
        self.names.clear();
        for (entry, name) in live {
            self.live.push(entry);
            self.names.push(name);
        }
        self.stale = stale;
        added
    }

    /// Give every live entry an evenly spaced key (no gap left).
    fn renumber(&mut self) {
        for (index, entry) in self.live.iter_mut().enumerate() {
            entry.key = (index as i64 + 1) * KEY_GAP;
        }
    }

    /// The key for a profile placed at `at` (between `at - 1` and the
    /// entry now at `at`), renumbering first when there is no gap.
    fn key_between(&mut self, at: usize) -> i64 {
        let gap_at = |live: &[Entry], at: usize| -> Option<i64> {
            let low = at.checked_sub(1).map(|i| live[i].key);
            let high = live.get(at).map(|entry| entry.key);
            match (low, high) {
                (None, None) => Some(KEY_GAP),
                (Some(low), None) => Some(low + KEY_GAP),
                (None, Some(high)) => Some(high - KEY_GAP),
                (Some(low), Some(high)) if high - low >= 2 => Some(low + (high - low) / 2),
                _ => None,
            }
        };
        if let Some(key) = gap_at(&self.live, at) {
            return key;
        }
        self.renumber();
        gap_at(&self.live, at).unwrap_or(KEY_GAP)
    }

    /// Move `moving` before or after `target` (names). Returns whether the
    /// order changed. A move onto itself, or an unknown name, changes
    /// nothing. Only the moved profile's key changes (unless the order had
    /// to be renumbered).
    pub(crate) fn move_to(&mut self, moving: &str, target: &str, after: bool) -> bool {
        if moving == target {
            return false;
        }
        let (Some(from), Some(_)) = (self.position(moving), self.position(target)) else {
            return false;
        };
        let name = self.names.remove(from);
        let mut entry = self.live.remove(from);
        let at = self
            .position(target)
            .map_or(self.names.len(), |at| at + usize::from(after));
        if at == from {
            self.names.insert(from, name);
            self.live.insert(from, entry);
            return false;
        }
        entry.key = self.key_between(at);
        self.names.insert(at, name);
        self.live.insert(at, entry);
        true
    }

    /// Move the names `marked` accepts to the top, keeping their relative
    /// order and the others' relative order. Returns whether it changed.
    /// Only the moved profiles' keys change.
    pub(crate) fn move_to_top(&mut self, marked: impl Fn(&str) -> bool) -> bool {
        let first_unmarked = self.names.iter().position(|name| !marked(name));
        let Some(first_unmarked) = first_unmarked else {
            return false;
        };
        let moved: Vec<usize> = (first_unmarked..self.names.len())
            .filter(|&index| marked(&self.names[index]))
            .collect();
        if moved.is_empty() {
            return false;
        }
        let mut names = Vec::with_capacity(self.names.len());
        let mut live = Vec::with_capacity(self.live.len());
        let mut rest_names = Vec::new();
        let mut rest_live = Vec::new();
        for (name, entry) in self.names.drain(..).zip(self.live.drain(..)) {
            if marked(&name) {
                names.push(name);
                live.push(entry);
            } else {
                rest_names.push(name);
                rest_live.push(entry);
            }
        }
        let top = names.len();
        names.extend(rest_names);
        live.extend(rest_live);
        self.names = names;
        self.live = live;
        // The leading marked entries already above every unmarked one keep
        // their keys; the moved ones go just above the first unmarked.
        let first_rest = self.live.get(top).map_or(0, |entry| entry.key);
        let unmoved = first_unmarked;
        let count = (top - unmoved) as i64;
        let low = unmoved.checked_sub(1).map(|index| self.live[index].key);
        let fits = match low {
            Some(low) => first_rest - low > count,
            None => true,
        };
        if !fits {
            self.renumber();
            return true;
        }
        let step = match low {
            Some(low) => (first_rest - low) / (count + 1),
            None => KEY_GAP,
        };
        for (offset, index) in (unmoved..top).enumerate() {
            let key = match low {
                Some(low) => low + step * (offset as i64 + 1),
                None => first_rest - step * (count - offset as i64),
            };
            self.live[index].key = key;
        }
        true
    }

    /// The audit text of a change from `before`: each profile whose place
    /// changed, `name old→new` with 1-based places.
    pub(crate) fn moved_from(&self, before: &Self) -> String {
        use std::fmt::Write as _;
        let mut text = String::new();
        for (index, name) in self.names.iter().enumerate() {
            let old = before.position(name);
            if old != Some(index) {
                if !text.is_empty() {
                    text.push_str(", ");
                }
                match old {
                    Some(old) => {
                        let _ = write!(text, "{name} {}→{}", old + 1, index + 1);
                    }
                    None => {
                        let _ = write!(text, "{name} new→{}", index + 1);
                    }
                }
            }
        }
        text
    }

    #[cfg(any(test, feature = "test-support"))]
    /// Fixture seam: an order of `names` (uids 1, 2, … in that order).
    pub fn of_names_for_test(names: &[&str]) -> Self {
        let mut order = Self::default();
        order.reconcile(
            names
                .iter()
                .enumerate()
                .map(|(index, name)| (index as i32 + 1, *name)),
        );
        order
    }

    /// The stored keys in live order (tests).
    #[cfg(test)]
    fn keys(&self) -> Vec<i64> {
        self.live.iter().map(|entry| entry.key).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::ProfileOrder;

    fn order(names: &[&str]) -> ProfileOrder {
        ProfileOrder::of_names_for_test(names)
    }

    fn names(order: &ProfileOrder) -> Vec<&str> {
        order.names().iter().map(String::as_str).collect()
    }

    fn vault<'a>(rows: &'a [(i32, &'a str)]) -> impl Iterator<Item = (i32, &'a str)> + 'a {
        rows.iter().copied()
    }

    #[test]
    fn an_empty_order_is_seeded_in_vault_order_and_new_profiles_append() {
        let mut saved = ProfileOrder::default();
        assert!(saved.reconcile(vault(&[(1, "alice"), (2, "bob"), (3, "carol")])));
        assert_eq!(names(&saved), ["alice", "bob", "carol"]);
        let mut saved = order(&["alice", "bob", "carol"]);
        assert!(saved.move_to("carol", "alice", false));
        assert!(saved.reconcile(vault(&[
            (1, "alice"),
            (2, "bob"),
            (3, "carol"),
            (4, "dave")
        ])));
        assert_eq!(
            names(&saved),
            ["carol", "alice", "bob", "dave"],
            "append, keep places"
        );
    }

    #[test]
    fn a_deleted_profile_leaves_the_order_but_stays_stored() {
        let mut saved = order(&["alice", "bob", "carol"]);
        assert!(!saved.reconcile(vault(&[(1, "alice"), (3, "carol")])));
        assert_eq!(names(&saved), ["alice", "carol"]);
        let stored = saved.to_value();
        assert_eq!(
            stored.as_object().map(|o| o.len()),
            Some(3),
            "never removed"
        );
        let restored = {
            let mut restored = ProfileOrder::from_value(Some(&stored));
            restored.reconcile(vault(&[(1, "alice"), (3, "carol")]));
            restored
        };
        assert_eq!(names(&restored), ["alice", "carol"]);
    }

    #[test]
    fn a_rename_keeps_its_place_without_a_write() {
        let mut saved = order(&["alice", "bob", "carol"]);
        assert!(saved.move_to("carol", "alice", false));
        assert!(!saved.reconcile(vault(&[(1, "alicia"), (2, "bob"), (3, "carol")])));
        assert_eq!(names(&saved), ["carol", "alicia", "bob"]);
    }

    #[test]
    fn a_move_lands_before_or_after_its_target_and_keeps_the_rest() {
        let mut saved = order(&["a", "b", "c", "d", "e"]);
        assert!(saved.move_to("e", "b", false));
        assert_eq!(names(&saved), ["a", "e", "b", "c", "d"]);
        assert!(saved.move_to("a", "c", true));
        assert_eq!(names(&saved), ["e", "b", "c", "a", "d"]);
        assert!(!saved.move_to("a", "a", true), "onto itself");
        assert!(!saved.move_to("a", "c", true), "already there");
        assert!(!saved.move_to("zz", "c", true), "unknown");
        let keys = saved.keys();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");
    }

    /// A move changes only the moved profile's key (one stored member).
    #[test]
    fn a_move_changes_one_stored_member() {
        let before = order(&["a", "b", "c", "d"]);
        let mut after = before.snapshot();
        assert!(after.move_to("d", "a", false));
        let (old, new) = (before.to_value(), after.to_value());
        let changed = new
            .as_object()
            .unwrap()
            .iter()
            .filter(|(uid, key)| old.get(uid.as_str()) != Some(key))
            .count();
        assert_eq!(changed, 1);
    }

    /// Repeated moves into the same gap renumber once the gap is spent and
    /// keep the order right.
    #[test]
    fn a_spent_gap_renumbers() {
        let mut saved = order(&["a", "b", "c"]);
        for _ in 0..40 {
            assert!(saved.move_to("c", "b", false));
            assert!(saved.move_to("b", "c", false));
        }
        assert_eq!(names(&saved), ["a", "b", "c"]);
        let keys = saved.keys();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");
    }

    /// A filtered drag: the hidden rows between keep their places.
    #[test]
    fn a_filtered_move_keeps_hidden_profiles_in_relative_order() {
        let mut saved = order(&["a", "hidden1", "b", "hidden2", "c"]);
        assert!(saved.move_to("c", "a", false));
        assert_eq!(names(&saved), ["c", "a", "hidden1", "b", "hidden2"]);
    }

    #[test]
    fn marked_to_top_keeps_both_groups_in_relative_order() {
        let mut saved = order(&["a", "b", "c", "d", "e"]);
        assert!(saved.move_to_top(|n| n == "d" || n == "b"));
        assert_eq!(names(&saved), ["b", "d", "a", "c", "e"]);
        assert!(!saved.move_to_top(|n| n == "b" || n == "d"));
        let keys = saved.keys();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");
        assert!(saved.move_to_top(|n| n == "a" || n == "e"));
        assert_eq!(names(&saved), ["a", "e", "b", "d", "c"]);
        let keys = saved.keys();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");
    }

    #[test]
    fn the_audit_text_names_each_moved_profile() {
        let before = order(&["a", "b", "c"]);
        let mut after = before.snapshot();
        after.move_to("c", "a", false);
        assert_eq!(after.moved_from(&before), "c 3→1, a 1→2, b 2→3");
    }

    #[test]
    fn the_stored_value_round_trips_and_ignores_junk() {
        let mut saved = order(&["a", "b", "c"]);
        saved.move_to("c", "a", false);
        let mut restored = ProfileOrder::from_value(Some(&saved.to_value()));
        restored.reconcile(vault(&[(1, "a"), (2, "b"), (3, "c")]));
        assert_eq!(names(&restored), ["c", "a", "b"]);
        let junk = serde_json::json!({"x": 1, "2": "no", "1": 5});
        let mut junk = ProfileOrder::from_value(Some(&junk));
        junk.reconcile(vault(&[(1, "a"), (2, "b")]));
        assert_eq!(names(&junk), ["a", "b"]);
        let mut array = ProfileOrder::from_value(Some(&serde_json::json!(["b", "a"])));
        array.reconcile(vault(&[(1, "a"), (2, "b")]));
        assert_eq!(names(&array), ["a", "b"], "an array is not an order");
    }
}
