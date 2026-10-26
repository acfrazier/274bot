//! The operator's saved Load order, persisted in the preferences document
//! as `profile_order`: an object keyed by each profile's vault identity
//! (its uid, as text) with a sort key per profile. It orders the fleet
//! members, Load all and every fleet view; Name and Status are view sorts
//! that never change it.
//!
//! The stored form is one small member per profile (`"<uid>": <key>`): a
//! preference value is at most 4,096 bytes, so the preferences service
//! holds a large order one row per profile, and one save replaces every
//! changed row in one file replacement (all of them land or none does). A
//! move changes only the moved profiles' keys. Keys leave gaps; when two
//! neighbours have none left, or a key would leave the kept range
//! (`±2^52`), the whole order is renumbered, and a stored key outside that
//! range renumbers the order when it is read. A name is never stored: a
//! rename keeps the profile's identity, so its place follows the vault
//! whether the rename commits or fails. A profile new to the vault is
//! appended. A deleted profile's entry is kept, unordered, while its
//! deletion is not durable (a failed deletion puts it back in its place),
//! and removed by the next save once it is. A new profile's uid is above
//! every uid the order has held, so it never takes a deleted profile's
//! place. Missing, the order is seeded in vault (alphabetical) order
//! without a write.

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

/// The largest sort key magnitude kept. A stored key outside
/// `-KEY_LIMIT..=KEY_LIMIT` renumbers the order when it is read, and a key
/// a change would put past it renumbers instead, so no key arithmetic can
/// overflow (and every key is an integer a JSON reader holds exactly).
const KEY_LIMIT: i64 = 1 << 52;

fn in_range(key: i64) -> bool {
    (-KEY_LIMIT..=KEY_LIMIT).contains(&key)
}

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
    /// Stored entries of profiles the vault lacks while their deletion is
    /// not durable, so a failed deletion gets its place back.
    held: Vec<Entry>,
    /// The highest uid this order has held, never lowered: a new
    /// profile's uid is above it.
    max_uid: i32,
    /// The stored keys were renumbered when read: the next reconcile asks
    /// for a write.
    renumbered: bool,
}

/// Storage changes need a save. Projection changes only invalidate views.
#[derive(Debug, Default)]
pub(crate) struct Reconciled {
    pub(crate) stored_changed: bool,
    pub(crate) sequence_changed: bool,
}

impl ProfileOrder {
    /// A copy for a test to mutate beside the original.
    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> Self {
        Self {
            names: self.names.clone(),
            live: self.live.clone(),
            held: self.held.clone(),
            max_uid: self.max_uid,
            renumbered: self.renumbered,
        }
    }

    /// Become a copy of `other`, reusing this order's buffers (the
    /// highest uid seen is kept).
    pub(crate) fn assign_from(&mut self, other: &Self) {
        self.names.clone_from(&other.names);
        self.live.clone_from(&other.live);
        self.held.clone_from(&other.held);
        self.max_uid = self.max_uid.max(other.max_uid);
        self.renumbered = other.renumbered;
    }

    /// Become `loaded` (a stored value just read), keeping the highest
    /// uid seen.
    pub(crate) fn replace(&mut self, loaded: Self) {
        let max_uid = self.max_uid.max(loaded.max_uid);
        *self = loaded;
        self.max_uid = max_uid;
    }

    /// The order a stored preference value holds: its `"<uid>": <key>`
    /// members (anything else is ignored), ordered by key; renumbered when
    /// a key is out of range. Names come from the vault at
    /// [`Self::reconcile`].
    pub(crate) fn from_value(value: Option<&Value>) -> Self {
        let mut order = Self::default();
        if let Some(Value::Object(members)) = value {
            for (uid, key) in members {
                let (Ok(uid), Some(key)) = (uid.parse::<i32>(), key.as_i64()) else {
                    continue;
                };
                order.held.push(Entry { uid, key });
            }
        }
        order.held.sort_by_key(|entry| (entry.key, entry.uid));
        order.max_uid = order.held.iter().map(|entry| entry.uid).max().unwrap_or(0);
        if !order.held.iter().all(|entry| in_range(entry.key)) {
            order.renumber();
            order.renumbered = true;
        }
        order
    }

    /// The stored preference value: every live entry, and every entry
    /// whose deletion is not durable yet.
    pub(crate) fn to_value(&self) -> Value {
        let mut members = serde_json::Map::with_capacity(self.live.len() + self.held.len());
        for entry in self.live.iter().chain(&self.held) {
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

    /// Whether an entry the vault lacks is held (its deletion is not
    /// durable yet).
    pub(crate) fn holds_deleted(&self) -> bool {
        !self.held.is_empty()
    }

    /// The highest uid this order has held.
    pub(crate) fn max_uid(&self) -> i32 {
        self.max_uid
    }

    /// Make the order hold exactly the vault's profiles (`vault`: uid and
    /// name, in vault order): a stored profile keeps its place under its
    /// current name, and a profile the order lacks is appended (seeded in
    /// vault order when nothing is stored). An entry the vault lacks stays
    /// stored, unordered, while `pending(uid)` says its deletion is not
    /// durable, and is dropped otherwise. Report storage changes separately
    /// from the displayed sequence: a rename needs a view refresh, not a save.
    pub(crate) fn reconcile<'a>(
        &mut self,
        vault: impl Iterator<Item = (i32, &'a str)>,
        pending: impl Fn(i32) -> bool,
    ) -> Reconciled {
        let mut entries: Vec<Entry> = self.live.drain(..).chain(self.held.drain(..)).collect();
        entries.sort_by_key(|entry| (entry.key, entry.uid));
        let mut profiles: Vec<(i32, &str)> = vault.collect();
        self.max_uid = profiles
            .iter()
            .map(|(uid, _)| *uid)
            .chain(entries.iter().map(|entry| entry.uid))
            .fold(self.max_uid, i32::max);
        let mut live: Vec<(Entry, String)> = Vec::with_capacity(profiles.len());
        let mut held = Vec::new();
        let mut dropped = false;
        for entry in entries {
            match profiles.iter().position(|(uid, _)| *uid == entry.uid) {
                Some(at) => {
                    let (_, name) = profiles.remove(at);
                    live.push((entry, name.to_string()));
                }
                None if pending(entry.uid) => held.push(entry),
                None => dropped = true,
            }
        }
        let added = !profiles.is_empty();
        let mut next = live.last().map_or(0, |(entry, _)| entry.key);
        let mut spent = false;
        for (uid, name) in profiles {
            match next.checked_add(KEY_GAP).filter(|key| in_range(*key)) {
                Some(key) => next = key,
                // Renumbered below; the key is a placeholder until then.
                None => spent = true,
            }
            live.push((Entry { uid, key: next }, name.to_string()));
        }
        let sequence_changed = self
            .names
            .iter()
            .map(String::as_str)
            .ne(live.iter().map(|(_, name)| name.as_str()));
        self.names.clear();
        for (entry, name) in live {
            self.live.push(entry);
            self.names.push(name);
        }
        self.held = held;
        if spent {
            self.renumber();
        }
        let renumbered = std::mem::take(&mut self.renumbered);
        Reconciled {
            stored_changed: added || dropped || renumbered,
            sequence_changed,
        }
    }

    /// Give every entry an evenly spaced key (no gap left): the live
    /// entries in their order, and each held entry where its key placed it
    /// among them, so a failed deletion still gets its place back. The
    /// keys stay far inside [`KEY_LIMIT`]: one gap per entry, and the
    /// entries are bounded by the preferences file.
    fn renumber(&mut self) {
        self.held.sort_by_key(|entry| (entry.key, entry.uid));
        let mut held = std::mem::take(&mut self.held).into_iter().peekable();
        let mut placed = Vec::with_capacity(held.len());
        let mut key = 0;
        for entry in &mut self.live {
            while let Some(mut deleted) = held.next_if(|deleted| deleted.key < entry.key) {
                key += KEY_GAP;
                deleted.key = key;
                placed.push(deleted);
            }
            key += KEY_GAP;
            entry.key = key;
        }
        for mut deleted in held {
            key += KEY_GAP;
            deleted.key = key;
            placed.push(deleted);
        }
        self.held = placed;
    }

    /// The key for a profile placed at `at` (between `at - 1` and the
    /// entry now at `at`), renumbering first when there is no gap or the
    /// key would leave the kept range.
    fn key_between(&mut self, at: usize) -> i64 {
        let gap_at = |live: &[Entry], at: usize| -> Option<i64> {
            let low = at.checked_sub(1).map(|i| live[i].key);
            let high = live.get(at).map(|entry| entry.key);
            let key = match (low, high) {
                (None, None) => Some(KEY_GAP),
                (Some(low), None) => low.checked_add(KEY_GAP),
                (None, Some(high)) => high.checked_sub(KEY_GAP),
                (Some(low), Some(high)) => high
                    .checked_sub(low)
                    .filter(|gap| *gap >= 2)
                    .map(|gap| low + gap / 2),
            };
            key.filter(|key| in_range(*key))
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
    /// Only the moved profiles' keys change (unless the order had to be
    /// renumbered).
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
        // their keys; the moved ones go just above the first unmarked (an
        // unmarked profile exists, so `top` is in range).
        let low = first_unmarked
            .checked_sub(1)
            .map(|index| self.live[index].key);
        match top_keys(low, self.live[top].key, top - first_unmarked) {
            Some(keys) => {
                for (entry, key) in self.live[first_unmarked..top].iter_mut().zip(keys) {
                    entry.key = key;
                }
            }
            None => self.renumber(),
        }
        true
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
            |_| false,
        );
        order
    }

    /// The stored keys in live order (tests).
    #[cfg(test)]
    fn keys(&self) -> Vec<i64> {
        self.live.iter().map(|entry| entry.key).collect()
    }
}

/// The keys of `count` profiles moved between the key `low` (none: the
/// top) and `high`, evenly spaced; `None` when they do not fit there in
/// the kept range (the order is renumbered instead).
fn top_keys(low: Option<i64>, high: i64, count: usize) -> Option<Vec<i64>> {
    let count = i64::try_from(count).ok()?;
    let step = match low {
        Some(low) => high
            .checked_sub(low)?
            .checked_div(count.checked_add(1)?)
            .filter(|step| *step > 0)?,
        None => KEY_GAP,
    };
    (1..=count)
        .map(|n| {
            let key = match low {
                Some(low) => low.checked_add(step.checked_mul(n)?)?,
                None => high.checked_sub(step.checked_mul(count - n + 1)?)?,
            };
            Some(key).filter(|key| in_range(*key))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ProfileOrder, KEY_LIMIT};

    fn order(names: &[&str]) -> ProfileOrder {
        ProfileOrder::of_names_for_test(names)
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn order_projection_keeps_its_process_only_size() {
        assert_eq!(std::mem::size_of::<ProfileOrder>(), 80);
        assert_eq!(std::mem::size_of::<super::Reconciled>(), 2);
    }

    fn names(order: &ProfileOrder) -> Vec<&str> {
        order.names().iter().map(String::as_str).collect()
    }

    /// Reconcile with `rows`, every deletion durable.
    fn sync(order: &mut ProfileOrder, rows: &[(i32, &str)]) -> bool {
        order
            .reconcile(rows.iter().copied(), |_| false)
            .stored_changed
    }

    fn stored_len(order: &ProfileOrder) -> usize {
        order.to_value().as_object().map_or(0, |o| o.len())
    }

    /// Keys strictly increase and stay in the kept range.
    fn keys_ok(order: &ProfileOrder) {
        let keys = order.keys();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");
        assert!(keys.iter().all(|k| k.abs() <= KEY_LIMIT), "{keys:?}");
    }

    #[test]
    fn an_empty_order_is_seeded_in_vault_order_and_new_profiles_append() {
        let mut saved = ProfileOrder::default();
        assert!(sync(&mut saved, &[(1, "alice"), (2, "bob"), (3, "carol")]));
        assert_eq!(names(&saved), ["alice", "bob", "carol"]);
        let mut saved = order(&["alice", "bob", "carol"]);
        assert!(saved.move_to("carol", "alice", false));
        assert!(sync(
            &mut saved,
            &[(1, "alice"), (2, "bob"), (3, "carol"), (4, "dave")]
        ));
        assert_eq!(
            names(&saved),
            ["carol", "alice", "bob", "dave"],
            "append, keep places"
        );
    }

    /// Review R3: a deletion that is not durable keeps the entry stored
    /// and its place for a failed deletion; once durable the entry is
    /// dropped (a write), and a stored value read later drops it too.
    #[test]
    fn a_deleted_profile_is_held_while_pending_and_dropped_once_durable() {
        let mut saved = order(&["alice", "bob", "carol"]);
        assert!(
            !saved
                .reconcile([(1, "alice"), (3, "carol")].into_iter(), |uid| uid == 2)
                .stored_changed
        );
        assert_eq!(names(&saved), ["alice", "carol"]);
        assert!(saved.holds_deleted());
        assert_eq!(stored_len(&saved), 3, "pending: still stored");
        // The deletion failed: bob is back in his place.
        assert!(
            !saved
                .reconcile([(1, "alice"), (2, "bob"), (3, "carol")].into_iter(), |_| {
                    false
                })
                .stored_changed
        );
        assert_eq!(names(&saved), ["alice", "bob", "carol"]);
        // Deleted again, now durable: dropped, and a write is needed.
        assert!(
            !saved
                .reconcile([(1, "alice"), (3, "carol")].into_iter(), |uid| uid == 2)
                .stored_changed
        );
        assert!(sync(&mut saved, &[(1, "alice"), (3, "carol")]));
        assert!(!saved.holds_deleted());
        assert_eq!(stored_len(&saved), 2);
        // A stored value holding a deleted profile drops it when read.
        let stale = serde_json::json!({"1": 10, "2": 20, "3": 30});
        let mut read = ProfileOrder::from_value(Some(&stale));
        assert!(sync(&mut read, &[(1, "alice"), (3, "carol")]), "a write");
        assert_eq!(stored_len(&read), 2);
    }

    /// Review R3: the highest uid the order has held is never lowered, so
    /// a fresh uid is above a deleted profile's.
    #[test]
    fn the_highest_uid_held_survives_its_deletion() {
        let mut saved = order(&["alice", "bob", "carol"]);
        assert_eq!(saved.max_uid(), 3);
        assert!(sync(&mut saved, &[(1, "alice"), (2, "bob")]));
        assert_eq!(saved.max_uid(), 3, "carol's uid stays taken");
        let mut copy = ProfileOrder::default();
        copy.assign_from(&saved);
        assert_eq!(copy.max_uid(), 3);
        let mut reread = order(&["zed"]);
        reread.replace(ProfileOrder::from_value(Some(&serde_json::json!({"1": 1}))));
        assert_eq!(reread.max_uid(), 1, "the stored value's");
        saved.replace(ProfileOrder::from_value(None));
        assert_eq!(saved.max_uid(), 3, "a reread never lowers it");
    }

    /// Review R3: a renumber while a deletion is pending keeps the held
    /// entry between its neighbours.
    #[test]
    fn a_renumber_keeps_a_pending_deletions_place() {
        let mut saved = order(&["a", "b", "c", "d"]);
        saved.reconcile([(1, "a"), (2, "b"), (4, "d")].into_iter(), |uid| uid == 3);
        saved.renumber();
        keys_ok(&saved);
        assert!(!sync(&mut saved, &[(1, "a"), (2, "b"), (3, "c"), (4, "d")]));
        assert_eq!(names(&saved), ["a", "b", "c", "d"]);
        keys_ok(&saved);
    }

    #[test]
    fn a_rename_keeps_its_place_without_a_write() {
        let mut saved = order(&["alice", "bob", "carol"]);
        assert!(saved.move_to("carol", "alice", false));
        assert!(!sync(
            &mut saved,
            &[(1, "alicia"), (2, "bob"), (3, "carol")]
        ));
        assert_eq!(names(&saved), ["carol", "alicia", "bob"]);
    }

    /// Review R5: stored keys at `i64::MIN` and `i64::MAX` are renumbered
    /// when read (their order kept, a write asked), and appending, moving
    /// and marked moves then work in range.
    #[test]
    fn extreme_stored_keys_renumber_and_every_change_stays_in_range() {
        let rows = [(1, "a"), (2, "b"), (3, "c"), (4, "d")];
        let stored = serde_json::json!({"1": i64::MAX, "2": i64::MIN, "3": 0});
        let read = || {
            let mut read = ProfileOrder::from_value(Some(&stored));
            assert!(sync(&mut read, &rows), "appended and renumbered: a write");
            read
        };
        let mut saved = read();
        assert_eq!(names(&saved), ["b", "c", "a", "d"], "MIN, 0, MAX, new");
        keys_ok(&saved);
        assert!(!sync(&mut saved, &rows), "settled");
        let mut moved = read();
        assert!(moved.move_to("a", "b", false));
        assert_eq!(names(&moved), ["a", "b", "c", "d"]);
        keys_ok(&moved);
        let mut marked = read();
        assert!(marked.move_to_top(|n| n == "d" || n == "a"));
        assert_eq!(names(&marked), ["a", "d", "b", "c"]);
        keys_ok(&marked);
        // Unparseable keys are ignored, not renumbered into the order.
        let mut junk = ProfileOrder::from_value(Some(&serde_json::json!({"1": 1.5e300})));
        sync(&mut junk, &rows[..1]);
        keys_ok(&junk);
    }

    /// Review R5: keys at the edge of the kept range: an append, a move
    /// and a marked move that would leave it renumber instead.
    #[test]
    fn extreme_neighbouring_keys_renumber_instead_of_leaving_the_range() {
        let high = serde_json::json!({"1": KEY_LIMIT - 1, "2": KEY_LIMIT});
        let low = serde_json::json!({"1": -KEY_LIMIT, "2": -KEY_LIMIT + 1});
        let rows = [(1, "a"), (2, "b"), (3, "c")];
        // Append past the top.
        let mut saved = ProfileOrder::from_value(Some(&high));
        assert!(sync(&mut saved, &rows));
        assert_eq!(names(&saved), ["a", "b", "c"]);
        keys_ok(&saved);
        // Move between two neighbours with no gap, and past the top.
        let mut saved = ProfileOrder::from_value(Some(&high));
        sync(&mut saved, &rows[..2]);
        assert!(saved.move_to("a", "b", true));
        assert_eq!(names(&saved), ["b", "a"]);
        keys_ok(&saved);
        // Move above the bottom of the range.
        let mut saved = ProfileOrder::from_value(Some(&low));
        sync(&mut saved, &rows[..2]);
        assert!(saved.move_to("b", "a", false));
        assert_eq!(names(&saved), ["b", "a"]);
        keys_ok(&saved);
        // A marked move to the top below the bottom of the range, and one
        // into a spent gap.
        let mut saved = ProfileOrder::from_value(Some(&low));
        sync(&mut saved, &rows[..2]);
        assert!(saved.move_to_top(|n| n == "b"));
        assert_eq!(names(&saved), ["b", "a"]);
        keys_ok(&saved);
        let spent = serde_json::json!({"1": 0, "2": 1, "3": 2});
        let mut saved = ProfileOrder::from_value(Some(&spent));
        sync(&mut saved, &rows);
        assert!(saved.move_to_top(|n| n == "a" || n == "c"));
        assert_eq!(names(&saved), ["a", "c", "b"]);
        keys_ok(&saved);
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
    fn the_stored_value_round_trips_and_ignores_junk() {
        let mut saved = order(&["a", "b", "c"]);
        saved.move_to("c", "a", false);
        let mut restored = ProfileOrder::from_value(Some(&saved.to_value()));
        sync(&mut restored, &[(1, "a"), (2, "b"), (3, "c")]);
        assert_eq!(names(&restored), ["c", "a", "b"]);
        let junk = serde_json::json!({"x": 1, "2": "no", "1": 5});
        let mut junk = ProfileOrder::from_value(Some(&junk));
        sync(&mut junk, &[(1, "a"), (2, "b")]);
        assert_eq!(names(&junk), ["a", "b"]);
        let mut array = ProfileOrder::from_value(Some(&serde_json::json!(["b", "a"])));
        sync(&mut array, &[(1, "a"), (2, "b")]);
        assert_eq!(names(&array), ["a", "b"], "an array is not an order");
    }
}
