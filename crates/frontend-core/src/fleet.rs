//! Fleet membership: which profiles the operator loaded (in load order) and
//! which of them an explicit logout parked on the title screen. Renderer
//! state (chooser, grid, warnings) stays with each front end.
//!
//! The fleet view model lives here too: the one filter match, row order
//! and visible-row computation both front ends render. A surface supplies
//! its row universe (the panel: every vault profile; the TUI: the loaded
//! members), its staged filter text and its [`FleetOrder`]; the shown
//! indices are filled into a caller-kept buffer, so a frame allocates
//! nothing once it has warmed.

use std::collections::HashSet;

use crate::command::FleetOrder;
use crate::views::{FleetRow, Phase};

/// Ordered membership plus the logout latch. The latch is the set of names
/// an explicit logout placed on the title screen, so the next Login (not a
/// spawn-time auto-login) brings them back.
#[derive(Debug, Default)]
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

/// ASCII case-insensitive substring test without allocating.
pub fn contains_ci(hay: &str, needle: &str) -> bool {
    let (hay, needle) = (hay.as_bytes(), needle.as_bytes());
    needle.is_empty()
        || hay
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// The row's status label; a member not projected yet is offline.
pub fn state_label(row: Option<&FleetRow>) -> &str {
    row.map_or(Phase::Offline.label(), |row| row.brief())
}

/// Whether one filter term matches: a name substring, `wN` / `world:N`,
/// or a status substring: the phase (`ready`, `queued`, `login error`, …)
/// or the row's label (`running`, `idle`, …). A member the core has not
/// projected yet counts as offline.
fn term_matches(name: &str, row: Option<&FleetRow>, term: &str) -> bool {
    let phase = row.map_or(Phase::Offline, |row| row.phase());
    if contains_ci(name, term)
        || contains_ci(phase.label(), term)
        || contains_ci(state_label(row), term)
    {
        return true;
    }
    let bytes = term.as_bytes();
    let number = if bytes.len() > 6 && bytes[..6].eq_ignore_ascii_case(b"world:") {
        &term[6..]
    } else if bytes.len() > 1 && bytes[0].eq_ignore_ascii_case(&b'w') {
        &term[1..]
    } else {
        return false;
    };
    number
        .parse::<u16>()
        .is_ok_and(|world| row.and_then(|row| row.world()) == Some(world))
}

/// The one fleet filter match: every whitespace-separated term of the
/// staged filter text must match (see `term_matches`); an empty filter
/// matches everything.
pub fn fleet_row_matches(name: &str, row: Option<&FleetRow>, filter: &str) -> bool {
    filter
        .split_whitespace()
        .all(|term| term_matches(name, row, term))
}

/// Member `index`'s row in the core's fleet projection. Rows follow load
/// order, so the common case is one comparison, not a scan; `None` for a
/// member the core has not projected yet.
pub fn member_row<'a>(
    names: &[String],
    rows: &'a [FleetRow],
    index: usize,
) -> Option<&'a FleetRow> {
    let name = names.get(index)?;
    match rows.get(index) {
        Some(row) if row.name() == *name => Some(row),
        _ => rows.iter().find(|row| row.name() == *name),
    }
}

fn order_indices<'a>(
    out: &mut [usize],
    order: FleetOrder,
    name_of: impl Fn(usize) -> &'a str,
    phase_of: impl Fn(usize) -> Phase,
) {
    match order {
        FleetOrder::LoadOrder => {}
        FleetOrder::Name => out.sort_by(|&a, &b| name_of(a).cmp(name_of(b))),
        FleetOrder::Status => out.sort_by(|&a, &b| {
            phase_of(a)
                .label()
                .cmp(phase_of(b).label())
                .then_with(|| name_of(a).cmp(name_of(b)))
        }),
    }
}

/// Fill `out` with the indices of `names` (their projected `rows`) that
/// pass `filter`, in `order`. The buffer is cleared and reused.
pub fn fill_visible_indices(
    names: &[String],
    rows: &[FleetRow],
    filter: &str,
    order: FleetOrder,
    out: &mut Vec<usize>,
) {
    out.clear();
    out.extend(names.iter().enumerate().filter_map(|(index, name)| {
        fleet_row_matches(name, member_row(names, rows, index), filter).then_some(index)
    }));
    order_indices(
        out,
        order,
        |index| names[index].as_str(),
        |index| member_row(names, rows, index).map_or(Phase::Offline, |row| row.phase()),
    );
}

/// Fill `out` with the indices of `rows` that pass `filter`, in `order`,
/// for a surface whose universe is already row-shaped. The buffer is
/// cleared and reused.
pub fn fill_visible_row_indices(
    rows: &[FleetRow],
    filter: &str,
    order: FleetOrder,
    out: &mut Vec<usize>,
) {
    out.clear();
    out.extend(rows.iter().enumerate().filter_map(|(index, row)| {
        fleet_row_matches(row.name(), Some(row), filter).then_some(index)
    }));
    order_indices(
        out,
        order,
        |index| rows[index].name(),
        |index| rows[index].phase(),
    );
}

#[cfg(test)]
mod tests {
    use super::{
        fill_visible_indices, fill_visible_row_indices, fleet_row_matches, member_row, Fleet,
    };
    use crate::command::FleetOrder;
    use crate::views::{FleetRow, Phase, QueuePlace};

    #[test]
    fn remove_drops_only_that_member() {
        let mut fleet = Fleet::default();
        fleet.add("a");
        fleet.add("b");
        fleet.remove("a");
        assert_eq!(fleet.members(), ["b".to_string()]);
    }

    fn ready(name: &str, world: u16) -> FleetRow {
        FleetRow::fixture(name, Phase::Ready, Some(world), None)
    }

    #[test]
    fn filter_terms_match_name_world_phase_and_brief() {
        let rows = [
            ready("alice", 2),
            FleetRow::fixture("bob", Phase::Queued, Some(1), Some(QueuePlace::new(2, 5))),
            ready("carol", 1),
        ];
        let matched = |filter: &str| {
            rows.iter()
                .filter(|row| fleet_row_matches(row.name(), Some(row), filter))
                .map(|row| row.name())
                .collect::<Vec<_>>()
        };
        assert_eq!(matched("AL"), ["alice"], "name, case-insensitive");
        assert_eq!(matched("w2"), ["alice"], "world term");
        assert_eq!(matched("world:1"), ["bob", "carol"]);
        assert_eq!(matched("queued"), ["bob"], "phase term");
        assert_eq!(matched("idle"), ["alice", "carol"], "brief term");
        assert_eq!(matched("ready w1"), ["carol"], "every term must match");
        assert_eq!(matched(""), ["alice", "bob", "carol"]);
    }

    #[test]
    fn filter_treats_an_unprojected_member_as_offline() {
        assert!(fleet_row_matches("dave", None, "offline"));
        assert!(fleet_row_matches("dave", None, "dav off"));
        assert!(!fleet_row_matches("dave", None, "ready"));
        assert!(!fleet_row_matches("dave", None, "w301"), "no row, no world");
    }

    #[test]
    fn visible_rows_follow_the_order() {
        let rows = [
            ready("zoe", 301),
            ready("amy", 301),
            FleetRow::fixture("bob", Phase::Offline, None, None),
        ];
        let mut shown = Vec::new();
        let names = |shown: &[usize]| shown.iter().map(|&i| rows[i].name()).collect::<Vec<_>>();
        fill_visible_row_indices(&rows, "", FleetOrder::LoadOrder, &mut shown);
        assert_eq!(names(&shown), ["zoe", "amy", "bob"], "supplied order kept");
        fill_visible_row_indices(&rows, "", FleetOrder::Name, &mut shown);
        assert_eq!(names(&shown), ["amy", "bob", "zoe"]);
        fill_visible_row_indices(&rows, "", FleetOrder::Status, &mut shown);
        assert_eq!(
            names(&shown),
            ["bob", "amy", "zoe"],
            "offline < ready, then name"
        );
        fill_visible_row_indices(&rows, "ready", FleetOrder::Name, &mut shown);
        assert_eq!(names(&shown), ["amy", "zoe"], "filter composes with order");
    }

    #[test]
    fn visible_indices_over_names_reuse_the_buffer() {
        let members = ["alice".to_string(), "bob".to_string(), "carol".to_string()];
        let rows = [ready("alice", 2), ready("carol", 1)];
        let mut shown = vec![99usize];
        fill_visible_indices(&members, &rows, "", FleetOrder::LoadOrder, &mut shown);
        assert_eq!(shown, [0, 1, 2], "unprojected bob still shows");
        assert_eq!(
            member_row(&members, &rows, 2).map(|r| r.name()),
            Some("carol")
        );
        assert_eq!(member_row(&members, &rows, 1), None);
        fill_visible_indices(
            &members,
            &rows,
            "offline",
            FleetOrder::LoadOrder,
            &mut shown,
        );
        assert_eq!(shown, [1], "buffer cleared between fills");
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
