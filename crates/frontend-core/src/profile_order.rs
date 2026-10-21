//! The operator's saved Load order: one ordered list of profile names,
//! persisted in the preferences document (`profile_order`). It orders the
//! fleet members, Load all and every fleet view; Name and Status are view
//! sorts that never change it.
//!
//! The list is reconciled with the vault: a missing list is seeded in the
//! vault's (alphabetical) order, a profile new to the vault is appended,
//! a deleted one is dropped, a renamed one keeps its place, and a repeated
//! entry is ignored. A reorder moves profiles by identity; the session
//! resolves the names at the write.

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

/// The saved Load order, reconciled with the vault.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProfileOrder {
    names: Vec<String>,
}

impl ProfileOrder {
    /// The order a stored preference value holds: its string entries in
    /// order, a repeated one kept once. Anything else reads as empty.
    pub(crate) fn from_value(value: Option<&Value>) -> Self {
        let mut order = Self::default();
        if let Some(Value::Array(entries)) = value {
            for entry in entries {
                if let Some(name) = entry.as_str() {
                    if !order.contains(name) {
                        order.names.push(name.to_string());
                    }
                }
            }
        }
        order
    }

    /// The stored preference value.
    pub(crate) fn to_value(&self) -> Value {
        Value::Array(self.names.iter().cloned().map(Value::String).collect())
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

    /// Make the order hold exactly the profiles `exists` accepts from
    /// itself plus `vault` (the vault's names in vault order): drop a name
    /// that left the vault or repeats, append a vault name it lacks. An
    /// empty order is seeded in vault order. Returns whether it changed.
    pub(crate) fn reconcile<'a>(
        &mut self,
        exists: impl Fn(&str) -> bool,
        vault: impl Iterator<Item = &'a str>,
    ) -> bool {
        let before = self.names.len();
        let mut changed = false;
        let mut index = 0;
        while index < self.names.len() {
            let name = &self.names[index];
            if !exists(name) || self.names[..index].contains(name) {
                self.names.remove(index);
                changed = true;
            } else {
                index += 1;
            }
        }
        for name in vault {
            if !self.contains(name) {
                self.names.push(name.to_string());
                changed = true;
            }
        }
        changed || self.names.len() != before
    }

    /// Rename `old` to `new` in place. Returns whether `old` was there.
    pub(crate) fn rename(&mut self, old: &str, new: &str) -> bool {
        if old == new {
            return false;
        }
        let Some(index) = self.position(old) else {
            return false;
        };
        if let Some(existing) = self.position(new) {
            self.names.remove(existing);
            let index = if existing < index { index - 1 } else { index };
            self.names[index] = new.to_string();
        } else {
            self.names[index] = new.to_string();
        }
        true
    }

    /// Move `moving` before or after `target` (names). Returns whether the
    /// order changed. A move onto itself, or an unknown name, changes
    /// nothing.
    pub(crate) fn move_to(&mut self, moving: &str, target: &str, after: bool) -> bool {
        if moving == target {
            return false;
        }
        let (Some(from), Some(_)) = (self.position(moving), self.position(target)) else {
            return false;
        };
        let name = self.names.remove(from);
        let at = self
            .position(target)
            .map_or(self.names.len(), |at| at + usize::from(after));
        self.names.insert(at, name);
        self.names.iter().position(|n| n == moving) != Some(from)
    }

    /// Move the names `marked` accepts to the top, keeping their relative
    /// order and the others' relative order. Returns whether it changed.
    pub(crate) fn move_to_top(&mut self, marked: impl Fn(&str) -> bool) -> bool {
        let before = self.names.clone();
        let (mut top, rest): (Vec<String>, Vec<String>) = std::mem::take(&mut self.names)
            .into_iter()
            .partition(|name| marked(name));
        top.extend(rest);
        self.names = top;
        self.names != before
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
    /// Fixture seam: an order of `names`.
    pub fn of_names_for_test(names: &[&str]) -> Self {
        Self {
            names: names.iter().map(|name| (*name).to_string()).collect(),
        }
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

    #[test]
    fn an_empty_order_is_seeded_in_vault_order_and_new_profiles_append() {
        let mut saved = ProfileOrder::default();
        let vault = ["alice", "bob", "carol"];
        assert!(saved.reconcile(|n| vault.contains(&n), vault.iter().copied()));
        assert_eq!(names(&saved), vault);
        let mut saved = order(&["carol", "alice", "bob"]);
        let vault = ["alice", "bob", "carol", "dave"];
        assert!(saved.reconcile(|n| vault.contains(&n), vault.iter().copied()));
        assert_eq!(
            names(&saved),
            ["carol", "alice", "bob", "dave"],
            "append, keep places"
        );
    }

    #[test]
    fn deleted_and_repeated_entries_are_dropped() {
        let mut saved = order(&["carol", "alice", "carol", "gone", "bob"]);
        let vault = ["alice", "bob", "carol"];
        assert!(saved.reconcile(|n| vault.contains(&n), vault.iter().copied()));
        assert_eq!(names(&saved), ["carol", "alice", "bob"]);
        assert!(!saved.reconcile(|n| vault.contains(&n), vault.iter().copied()));
    }

    #[test]
    fn a_rename_keeps_its_place() {
        let mut saved = order(&["carol", "alice", "bob"]);
        assert!(saved.rename("alice", "alicia"));
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
    }

    /// A filtered drag: the hidden rows between keep their places.
    #[test]
    fn a_filtered_move_keeps_hidden_profiles_in_relative_order() {
        let mut saved = order(&["a", "hidden1", "b", "hidden2", "c"]);
        // Shown: a, b, c. Drag c before a.
        assert!(saved.move_to("c", "a", false));
        assert_eq!(names(&saved), ["c", "a", "hidden1", "b", "hidden2"]);
    }

    #[test]
    fn marked_to_top_keeps_both_groups_in_relative_order() {
        let mut saved = order(&["a", "b", "c", "d", "e"]);
        assert!(saved.move_to_top(|n| n == "d" || n == "b"));
        assert_eq!(names(&saved), ["b", "d", "a", "c", "e"]);
        assert!(!saved.move_to_top(|n| n == "b" || n == "d"));
    }

    #[test]
    fn the_audit_text_names_each_moved_profile() {
        let before = order(&["a", "b", "c"]);
        let mut after = before.clone();
        after.move_to("c", "a", false);
        assert_eq!(after.moved_from(&before), "c 3→1, a 1→2, b 2→3");
    }

    #[test]
    fn the_stored_value_round_trips_and_ignores_junk() {
        let saved = order(&["b", "a"]);
        assert_eq!(ProfileOrder::from_value(Some(&saved.to_value())), saved);
        let junk = serde_json::json!(["a", 3, "a", null, "b"]);
        assert_eq!(names(&ProfileOrder::from_value(Some(&junk))), ["a", "b"]);
        assert_eq!(
            ProfileOrder::from_value(Some(&serde_json::json!({}))),
            order(&[])
        );
    }
}
