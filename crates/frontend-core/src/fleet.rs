//! Fleet membership: which profiles the operator loaded (in load order) and
//! which of them an explicit logout parked on the title screen. Renderer
//! state (chooser, grid, warnings) stays with each front end.

use std::collections::HashSet;

/// Ordered membership plus the logout latch. The latch is the set of names
/// an explicit logout placed on the title screen, so the next Login (not a
/// spawn-time auto-login) brings them back.
#[derive(Debug, Default, Clone)]
pub struct Fleet {
    members: Vec<String>,
    latch: HashSet<String>,
}

impl Fleet {
    pub fn members(&self) -> &[String] {
        &self.members
    }

    pub fn contains(&self, name: &str) -> bool {
        self.members.iter().any(|m| m == name)
    }

    /// Whether an explicit logout latched `name`.
    pub fn latched(&self, name: &str) -> bool {
        self.latch.contains(name)
    }

    /// Push each running name that is not already a member, preserving the
    /// `running` order.
    pub fn seed_running(&mut self, running: &[String]) {
        for name in running {
            if !self.contains(name) {
                self.members.push(name.clone());
            }
        }
    }

    /// Add a member. Returns false when it is already a member.
    pub fn add(&mut self, name: &str) -> bool {
        if self.contains(name) {
            return false;
        }
        self.members.push(name.to_string());
        true
    }

    /// Drop a member; the session stops its slot separately.
    pub fn remove(&mut self, name: &str) {
        self.members.retain(|n| n != name);
    }

    /// What the selection should become after `gone` is removed: unchanged
    /// unless `gone` was selected, then the previous member, else the next
    /// member, else none.
    pub fn focus_neighbour(&self, gone: &str, selected: Option<&str>) -> Option<String> {
        if selected != Some(gone) {
            return selected.map(String::from);
        }
        let idx = self.members.iter().position(|m| m == gone)?;
        if idx > 0 {
            Some(self.members[idx - 1].clone())
        } else {
            self.members.get(idx + 1).cloned()
        }
    }

    /// Record an intentional logout so auto-login is blocked until Login
    /// clears the latch.
    pub fn latch_logout(&mut self, name: &str) {
        self.latch.insert(name.to_string());
    }

    pub fn clear_latch(&mut self, name: &str) {
        self.latch.remove(name);
    }

    /// Auto-login only when asked to and the name is not latched by an
    /// explicit logout.
    pub fn should_auto_login(&self, name: &str, auto_login: bool) -> bool {
        auto_login && !self.latch.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::Fleet;

    #[test]
    fn remove_drops_only_that_member() {
        let mut fleet = Fleet::default();
        fleet.add("a");
        fleet.add("b");
        fleet.remove("a");
        assert_eq!(fleet.members(), ["b".to_string()]);
    }

    #[test]
    fn latch_blocks_auto_login_until_cleared() {
        let mut fleet = Fleet::default();
        assert!(fleet.should_auto_login("a", true));
        fleet.latch_logout("a");
        assert!(!fleet.should_auto_login("a", true));
        fleet.clear_latch("a");
        assert!(fleet.should_auto_login("a", true));
        assert!(!fleet.should_auto_login("a", false));
    }

    #[test]
    fn add_skips_existing_members() {
        let mut fleet = Fleet::default();
        assert!(fleet.add("a"));
        assert!(!fleet.add("a"));
        assert_eq!(fleet.members(), ["a".to_string()]);
    }

    #[test]
    fn seed_running_preserves_order_and_dedupes() {
        let mut fleet = Fleet::default();
        fleet.seed_running(&["b".into(), "a".into(), "b".into()]);
        assert_eq!(fleet.members(), ["b".to_string(), "a".to_string()]);
    }

    #[test]
    fn focus_neighbour_picks_prev_then_next_when_selected_is_gone() {
        let mut fleet = Fleet::default();
        fleet.add("a");
        fleet.add("b");
        fleet.add("c");
        assert_eq!(fleet.focus_neighbour("b", Some("b")), Some("a".into()));
        assert_eq!(fleet.focus_neighbour("a", Some("a")), Some("b".into()));
        assert_eq!(fleet.focus_neighbour("c", Some("c")), Some("b".into()));
    }

    #[test]
    fn focus_neighbour_is_none_for_last_single_member() {
        let mut fleet = Fleet::default();
        fleet.add("only");
        assert_eq!(fleet.focus_neighbour("only", Some("only")), None);
    }

    #[test]
    fn focus_neighbour_keeps_selection_when_other_member_removed() {
        let mut fleet = Fleet::default();
        fleet.add("a");
        fleet.add("b");
        assert_eq!(fleet.focus_neighbour("a", Some("b")), Some("b".into()));
    }
}
