//! Wall membership model (Task 9): the multibox chooser, the logout latch,
//! and bulk login/logout state, kept outside the imgui widgets so the
//! session can drive it without a live UI.

use std::collections::HashSet;

/// Wall membership state. `members` keeps insertion order; the latch is
/// the set of names an explicit logout placed on the title screen so the
/// next `login_all` (not a spawn-time auto-login) brings them back.
#[derive(Debug, Default, Clone)]
pub struct Wall {
    pub members: Vec<String>,
    pub chooser_open: bool,
    pub opened_once: bool,
    pub grid: bool,
    pub latch: HashSet<String>,
    /// Scary "render all" confirm: wanted while the user has unchecked
    /// "only render selected" but not yet accepted (OK) or backed out.
    pub render_all_warn_open: bool,
    /// "I understand" checkbox state inside that confirm.
    pub render_all_understood: bool,
}

impl Wall {
    /// Push each running name that is not already a member, preserving the
    /// `running` order. Used when the multibox hook first sees the running
    /// slot list.
    pub fn seed_running(&mut self, running: &[String]) {
        for name in running {
            if !self.members.iter().any(|m| m == name) {
                self.members.push(name.clone());
            }
        }
    }

    /// Multibox on: seed the running slots and open the chooser on the
    /// first time only.
    pub fn on_multibox_on(&mut self, running: &[String]) {
        self.seed_running(running);
        if !self.opened_once {
            self.chooser_open = true;
            self.opened_once = true;
        }
    }

    /// Multibox off: drop the grid overlay and any open chooser or
    /// render-all warning.
    pub fn on_multibox_off(&mut self) {
        self.grid = false;
        self.chooser_open = false;
        self.render_all_warn_open = false;
    }

    /// Add a member. Returns false when it is already a member; selection
    /// is implied by the caller, so a re-click just focuses.
    pub fn load(&mut self, name: &str) -> bool {
        if self.members.iter().any(|m| m == name) {
            return false;
        }
        self.members.push(name.to_string());
        true
    }

    /// Add every name that is not yet a member, returning the newly added
    /// names in input order.
    pub fn load_all<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut added = Vec::new();
        for name in names {
            if self.load(name) {
                added.push(name.to_string());
            }
        }
        added
    }

    /// The chooser ✕ deletes a vault row only; wall members are untouched.
    pub fn chooser_delete_vault_only(&self, name: &str) {
        let _ = name;
    }

    /// The rail ✕ removes the member; the session stops the slot later.
    pub fn rail_remove(&mut self, name: &str) {
        self.members.retain(|n| n != name);
    }

    /// What focus should become after a member is removed: unchanged unless
    /// the removed name was focused, then the previous member, else the
    /// next member, else none.
    pub fn focus_neighbour(&self, gone: &str, focused: Option<&str>) -> Option<String> {
        if focused != Some(gone) {
            return focused.map(String::from);
        }
        let idx = self.members.iter().position(|m| m == gone)?;
        if idx > 0 {
            Some(self.members[idx - 1].clone())
        } else {
            self.members.get(idx + 1).cloned()
        }
    }

    /// Record an intentional logout so auto-login is blocked until the
    /// latch is cleared (`login_all`).
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
    use super::Wall;

    #[test]
    fn chooser_x_does_not_drop_member() {
        let mut w = Wall::default();
        assert!(w.load("a"));
        w.chooser_delete_vault_only("a");
        assert_eq!(w.members, vec!["a".to_string()]);
    }

    #[test]
    fn rail_x_removes_member_not_via_vault() {
        let mut w = Wall::default();
        w.load("a");
        w.load("b");
        w.rail_remove("a");
        assert_eq!(w.members, vec!["b".to_string()]);
    }

    #[test]
    fn latch_blocks_auto_login_until_login_all() {
        let mut w = Wall::default();
        assert!(w.should_auto_login("a", true));
        w.latch_logout("a");
        assert!(!w.should_auto_login("a", true));
        w.clear_latch("a");
        assert!(w.should_auto_login("a", true));
        assert!(!w.should_auto_login("a", false));
    }

    #[test]
    fn load_all_skips_existing() {
        let mut w = Wall::default();
        w.load("a");
        let added = w.load_all(["a", "b", "c"]);
        assert_eq!(added, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn first_multibox_opens_chooser_later_does_not() {
        let mut w = Wall::default();
        w.on_multibox_on(&["seed".into()]);
        assert!(w.chooser_open);
        assert_eq!(w.members, vec!["seed".to_string()]);
        w.chooser_open = false;
        w.on_multibox_on(&["seed".into()]);
        assert!(!w.chooser_open);
    }

    #[test]
    fn seed_running_preserves_order_and_dedupes() {
        let mut w = Wall::default();
        w.seed_running(&["b".into(), "a".into(), "b".into()]);
        assert_eq!(w.members, vec!["b".to_string(), "a".to_string()]);
    }

    #[test]
    fn multibox_off_clears_grid_and_chooser() {
        let mut w = Wall::default();
        w.on_multibox_on(&["a".into()]);
        w.grid = true;
        w.render_all_warn_open = true;
        w.on_multibox_off();
        assert!(!w.grid);
        assert!(!w.chooser_open);
        assert!(!w.render_all_warn_open);
        assert_eq!(w.members, vec!["a".to_string()]);
    }

    #[test]
    fn focus_neighbour_picks_prev_then_next_when_focused_is_gone() {
        let mut w = Wall::default();
        w.load("a");
        w.load("b");
        w.load("c");
        assert_eq!(w.focus_neighbour("b", Some("b")), Some("a".into()));
        assert_eq!(w.focus_neighbour("a", Some("a")), Some("b".into()));
        assert_eq!(w.focus_neighbour("c", Some("c")), Some("b".into()));
    }

    #[test]
    fn focus_neighbour_is_none_for_last_single_member() {
        let mut w = Wall::default();
        w.load("only");
        assert_eq!(w.focus_neighbour("only", Some("only")), None);
    }

    #[test]
    fn focus_neighbour_keeps_focus_when_other_member_removed() {
        let mut w = Wall::default();
        w.load("a");
        w.load("b");
        assert_eq!(w.focus_neighbour("a", Some("b")), Some("b".into()));
    }
}
