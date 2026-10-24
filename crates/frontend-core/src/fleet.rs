//! Fleet membership: which profiles the operator loaded (in load order) and
//! which of them an explicit logout parked on the title screen. Renderer
//! state (chooser, grid, warnings) stays with each front end.
//!
//! The fleet view model lives here too. [`FleetModel`] is the one profile
//! universe (every vault profile and every loaded slot, one row per
//! identity), the marks, the filter and order, the shown rows and their
//! counts. Each front end holds one and lends it to the command seam; the
//! Fleet table, the MultiBox rail, the Grid and the TUI all read the same
//! rows, order and marks from it. It rebuilds its universe only when the
//! core's rows or the vault's profiles moved, and its shown rows only when
//! the universe, the filter, the order or the marks changed, so a steady
//! frame computes and allocates nothing.

use std::collections::HashSet;

use crate::command::FleetOrder;
use crate::selection::{MarkedSelection, ProfileIdentity};
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

    /// Order the members by `rank` (the saved Load order), keeping the
    /// relative order of equal ranks; a member without a rank goes after
    /// every ranked one.
    pub(crate) fn sort_by_rank(&mut self, rank: impl Fn(&str) -> Option<usize>) {
        if self
            .members
            .windows(2)
            .all(|pair| rank_key(&rank, &pair[0]) <= rank_key(&rank, &pair[1]))
        {
            return;
        }
        self.members.sort_by_key(|name| rank_key(&rank, name));
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

/// The sort key of a member: its rank, unranked last.
fn rank_key(rank: &impl Fn(&str) -> Option<usize>, name: &str) -> usize {
    rank(name).unwrap_or(usize::MAX)
}

/// ASCII case-insensitive substring test without allocating.
pub fn contains_ci(hay: &str, needle: &str) -> bool {
    let (hay, needle) = (hay.as_bytes(), needle.as_bytes());
    needle.is_empty()
        || hay
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// The status label of a profile that is not loaded: distinct from a
/// loaded bot's `offline`.
pub const NOT_LOADED: &str = "not loaded";

/// The row's status label; a member not projected yet is offline, and a
/// profile that is not loaded is [`NOT_LOADED`].
pub fn state_label(row: Option<&FleetRow>) -> &str {
    match row {
        Some(row) if !row.loaded() => NOT_LOADED,
        Some(row) => row.brief(),
        None => Phase::Offline.label(),
    }
}

/// The label the Status order sorts by: the phase, or [`NOT_LOADED`].
fn status_key(row: Option<&FleetRow>) -> &str {
    match row {
        Some(row) if !row.loaded() => NOT_LOADED,
        Some(row) => row.phase().label(),
        None => Phase::Offline.label(),
    }
}

/// Whether one filter term matches: a name substring, `wN` / `world:N`,
/// or a status substring: the phase (`ready`, `queued`, `login error`, …)
/// or the row's label (`running`, `idle`, …). A member the core has not
/// projected yet counts as offline; a profile that is not loaded matches
/// only its [`NOT_LOADED`] status.
fn term_matches(name: &str, row: Option<&FleetRow>, term: &str) -> bool {
    if contains_ci(name, term)
        || contains_ci(status_key(row), term)
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

fn order_indices<'a>(
    out: &mut [usize],
    order: FleetOrder,
    name_of: impl Fn(usize) -> &'a str,
    status_of: impl Fn(usize) -> &'a str,
) {
    match order {
        FleetOrder::LoadOrder => {}
        FleetOrder::Name => out.sort_by(|&a, &b| name_of(a).cmp(name_of(b))),
        FleetOrder::Status => out.sort_by(|&a, &b| {
            status_of(a)
                .cmp(status_of(b))
                .then_with(|| name_of(a).cmp(name_of(b)))
        }),
    }
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
        |index| status_key(Some(&rows[index])),
    );
}

#[cfg(test)]
mod tests {
    use super::{fill_visible_row_indices, fleet_row_matches, Fleet};
    use crate::command::FleetOrder;
    use crate::views::{FleetRow, Phase, QueuePlace};

    /// A profile that is not loaded reads `not loaded` and matches only
    /// that status; a loaded bot that is offline still reads `offline`.
    #[test]
    fn an_unloaded_profile_is_not_loaded_not_offline() {
        let loaded = FleetRow::fixture("alice", Phase::Offline, None, None);
        let unloaded = FleetRow::for_profile("bob");
        assert_eq!(super::state_label(Some(&loaded)), "offline");
        assert_eq!(super::state_label(Some(&unloaded)), super::NOT_LOADED);
        assert!(fleet_row_matches("alice", Some(&loaded), "offline"));
        assert!(!fleet_row_matches("bob", Some(&unloaded), "offline"));
        assert!(fleet_row_matches("bob", Some(&unloaded), "not loaded"));
        assert!(!fleet_row_matches("alice", Some(&loaded), "not loaded"));
        let mut world_login = String::new();
        unloaded.write_world_login(&mut world_login);
        assert_eq!(world_login, "not loaded");
    }

    /// The model's shown rows, counts and generation follow the filter,
    /// order and marks, and only move when one of them changed.
    #[test]
    fn the_model_recounts_only_when_its_inputs_change() {
        use super::FleetModel;
        use crate::selection::ProfileIdentity;
        let mut model = FleetModel::new();
        model.set_universe_for_test(
            vec![
                FleetRow::fixture("carol", Phase::Ready, Some(3), None),
                FleetRow::fixture("alice", Phase::Offline, None, None),
                FleetRow::for_profile("bob"),
            ],
            &[],
        );
        let names = |model: &FleetModel| -> Vec<String> {
            model
                .shown_rows()
                .map(|(_, row)| row.name().to_string())
                .collect()
        };
        assert_eq!(names(&model), ["carol", "alice", "bob"], "load order");
        let generation = model.generation();
        model.set_mark(ProfileIdentity::synthetic("bob"), true);
        assert_eq!(model.generation(), generation + 1);
        model.set_mark(ProfileIdentity::synthetic("bob"), true);
        assert_eq!(model.generation(), generation + 1, "no change, no move");
        model.set_filter_for_test("w3");
        assert_eq!(names(&model), ["carol"]);
        assert_eq!(model.counts().hidden_marked(), 1);
        model.set_filter_for_test("");
        model.set_order_for_test(FleetOrder::Status);
        assert_eq!(
            names(&model),
            ["bob", "alice", "carol"],
            "not loaded < offline < ready"
        );
        model.set_order_for_test(FleetOrder::Name);
        assert_eq!(names(&model), ["alice", "bob", "carol"]);
        assert_eq!(model.counts().shown_loaded(), 2);
        assert_eq!(model.counts().shown_unloaded(), 1);
    }

    /// The unmarked scope is the loaded, shown rows: a filtered-out row, an
    /// unloaded row and an unknown profile are outside it.
    #[test]
    fn the_unmarked_scope_is_the_loaded_shown_rows() {
        use super::FleetModel;
        use crate::selection::ProfileIdentity;
        let mut model = FleetModel::new();
        model.set_universe_for_test(
            vec![
                FleetRow::fixture("carol", Phase::Ready, Some(3), None),
                FleetRow::fixture("alice", Phase::Ready, Some(4), None),
                FleetRow::for_profile("bob"),
            ],
            &[],
        );
        let id = ProfileIdentity::synthetic;
        assert!(model.in_unmarked_scope(id("carol")));
        assert!(model.in_unmarked_scope(id("alice")));
        assert!(!model.in_unmarked_scope(id("bob")), "not loaded");
        assert!(!model.in_unmarked_scope(id("dave")), "no such profile");
        model.set_filter_for_test("w3");
        assert!(model.in_unmarked_scope(id("carol")));
        assert!(!model.in_unmarked_scope(id("alice")), "filtered out");
    }

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

/// The fleet view's filter and row order, as the operator set them. The
/// filter text is a staged input: a front end's text field edits
/// [`Self::staged_mut`] and [`crate::OperatorCommand::SetFleetFilter`]
/// commits it to the filter the rows are shown with ([`Self::text`]);
/// [`crate::OperatorCommand::SetFleetSort`] sets the order. Both buffers
/// keep their capacity, so a commit per keystroke allocates nothing once
/// warm.
#[derive(Debug)]
pub struct FleetFilter {
    staged: String,
    applied: String,
    order: FleetOrder,
}

impl FleetFilter {
    /// An empty filter with `order`.
    pub fn new(order: FleetOrder) -> Self {
        Self {
            staged: String::new(),
            applied: String::new(),
            order,
        }
    }

    /// The staged filter text, for a text field to edit.
    pub fn staged_mut(&mut self) -> &mut String {
        &mut self.staged
    }

    /// The staged filter text.
    pub fn staged(&self) -> &str {
        &self.staged
    }

    /// The filter the rows are shown with.
    pub fn text(&self) -> &str {
        &self.applied
    }

    /// The row order.
    pub fn order(&self) -> FleetOrder {
        self.order
    }

    /// Commit the staged text: `false` when it was already the filter.
    pub(crate) fn commit(&mut self) -> bool {
        if self.applied == self.staged {
            return false;
        }
        self.applied.clear();
        self.applied.push_str(&self.staged);
        true
    }

    pub(crate) fn set_order(&mut self, order: FleetOrder) -> bool {
        std::mem::replace(&mut self.order, order) != order
    }
}

/// The card the fleet buttons use (Start, Assign, Assign & restart and
/// Copy settings). Choosing one changes nothing else: no assignment, no
/// Browse pick, no profile form, no Start. [`Self::OWN`] (the default)
/// is each bot's own saved assignment (Start only; the writing actions
/// need a card), [`Self::FOCUSED`] the focused bot's card (its pending
/// Browse pick, else its assignment) followed as the focus moves, and
/// [`Self::pinned`] one card, fixed whatever the focus.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FleetCard(CardChoice);

/// What a [`FleetCard`] holds.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) enum CardChoice {
    #[default]
    Own,
    Focused,
    Pinned(crate::scripts::ScriptChoice),
}

impl FleetCard {
    /// Each bot's own saved assignment.
    pub const OWN: Self = Self(CardChoice::Own);
    /// The focused bot's card.
    pub const FOCUSED: Self = Self(CardChoice::Focused);

    /// One card, fixed whatever the focus.
    pub fn pinned(card: crate::scripts::ScriptChoice) -> Self {
        Self(CardChoice::Pinned(card))
    }

    /// Whether this is each bot's own card.
    pub fn is_own(&self) -> bool {
        matches!(self.0, CardChoice::Own)
    }

    /// Whether this follows the focused bot's card.
    pub fn is_focused(&self) -> bool {
        matches!(self.0, CardChoice::Focused)
    }

    /// The pinned card, when one is pinned.
    pub fn pinned_card(&self) -> Option<&crate::scripts::ScriptChoice> {
        match &self.0 {
            CardChoice::Pinned(card) => Some(card),
            CardChoice::Own | CardChoice::Focused => None,
        }
    }

    pub(crate) fn choice(&self) -> &CardChoice {
        &self.0
    }

    /// The same choice, copied.
    pub fn duplicate(&self) -> Self {
        Self(match &self.0 {
            CardChoice::Own => CardChoice::Own,
            CardChoice::Focused => CardChoice::Focused,
            CardChoice::Pinned(card) => CardChoice::Pinned(card.duplicate()),
        })
    }
}

/// The label text of [`FleetCard::OWN`].
pub const OWN_CARD_LABEL: &str = "Each bot's own card";
/// The label prefix of [`FleetCard::FOCUSED`].
pub const FOCUSED_CARD_LABEL: &str = "Focused bot's card";

/// The fleet card's label and the inputs it was built from.
#[derive(Debug, Default)]
struct CardLabel {
    text: String,
    /// (card kind, focused name, heading, pending) the text was built
    /// from; `None` forces a rebuild.
    key: Option<(u8, String, String, bool)>,
}

impl CardLabel {
    fn sync(
        &mut self,
        card: &FleetCard,
        focused: Option<&str>,
        heading: Option<&str>,
        pending: bool,
    ) {
        let kind = match card.choice() {
            CardChoice::Own => 0,
            CardChoice::Focused => 1,
            CardChoice::Pinned(_) => 2,
        };
        let focused = focused.unwrap_or("");
        let heading_text = match card.choice() {
            CardChoice::Pinned(choice) => choice.label(),
            _ => heading.unwrap_or(""),
        };
        if let Some((k, f, h, p)) = &self.key {
            if *k == kind && f == focused && h == heading_text && *p == pending {
                return;
            }
        }
        self.text.clear();
        match card.choice() {
            CardChoice::Own => self.text.push_str(OWN_CARD_LABEL),
            CardChoice::Focused => {
                self.text.push_str(FOCUSED_CARD_LABEL);
                self.text.push_str(": ");
                self.text.push_str(heading.unwrap_or("none"));
                if pending {
                    self.text.push_str(" (pending)");
                }
            }
            CardChoice::Pinned(choice) => self.text.push_str(choice.label()),
        }
        match &mut self.key {
            Some((k, f, h, p)) => {
                *k = kind;
                f.clear();
                f.push_str(focused);
                h.clear();
                h.push_str(heading_text);
                *p = pending;
            }
            None => self.key = Some((kind, focused.to_owned(), heading_text.to_owned(), pending)),
        }
    }
}

/// A mark operation over the fleet model's shown rows (rows the filter
/// hides keep their marks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkOp {
    /// Mark every shown row.
    Shown,
    /// Unmark every shown row.
    UnmarkShown,
    /// Flip every shown row's mark.
    InvertShown,
    /// Mark every shown row the predicate matches (other marks stay).
    Where(MarkPredicate),
}

/// A row status for [`MarkOp::Where`]: a predicate over the row's
/// state, never its rendered text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkPredicate {
    /// Loaded and logged in.
    Ready,
    /// Logged in with no script running (idle, stopped or failed).
    Idle,
    /// A script is starting or running.
    Running,
    /// Loaded but not logged in (offline or logged out).
    LoggedOut,
    /// Loaded, and its login failed.
    LoginError,
    /// A vault profile that is not loaded.
    NotLoaded,
}

impl MarkPredicate {
    /// Every predicate, in menu order.
    pub const ALL: [MarkPredicate; 6] = [
        Self::Ready,
        Self::Idle,
        Self::Running,
        Self::LoggedOut,
        Self::LoginError,
        Self::NotLoaded,
    ];

    /// The menu label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Idle => "idle",
            Self::Running => "running",
            Self::LoggedOut => "logged out",
            Self::LoginError => "login error",
            Self::NotLoaded => "not loaded",
        }
    }

    /// Whether `row` is in this status.
    pub fn matches(self, row: &FleetRow) -> bool {
        let loaded = row.loaded();
        let phase = row.phase();
        match self {
            Self::Ready => loaded && phase == Phase::Ready,
            Self::Idle => loaded && phase == Phase::Ready && !row.script().is_live() && !row.busy(),
            Self::Running => loaded && row.busy(),
            Self::LoggedOut => loaded && matches!(phase, Phase::Offline | Phase::LoggedOut),
            Self::LoginError => loaded && matches!(phase, Phase::LoginError | Phase::Failed),
            Self::NotLoaded => !loaded,
        }
    }
}

/// How many of the shown rows are marked (the tri-state box).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShownMarks {
    /// No shown row is marked.
    None,
    /// Some shown rows are marked.
    Some,
    /// Every shown row is marked (and at least one row is shown).
    All,
}

/// Mark and row counts over the fleet model's universe and shown rows,
/// recomputed with the shown rows or the marks, never per frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FleetMarkCounts {
    rows: usize,
    marked: usize,
    shown: usize,
    shown_marked: usize,
    shown_loaded: usize,
    shown_unloaded: usize,
}

impl FleetMarkCounts {
    /// Rows in the universe (every profile, loaded or not).
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Marked rows, shown or hidden by the filter.
    pub fn marked(&self) -> usize {
        self.marked
    }

    /// Rows the filter shows.
    pub fn shown(&self) -> usize {
        self.shown
    }

    /// Marked rows the filter shows.
    pub fn shown_marked(&self) -> usize {
        self.shown_marked
    }

    /// Marked rows the filter hides ("5 marked, 2 not shown").
    pub fn hidden_marked(&self) -> usize {
        self.marked - self.shown_marked
    }

    /// Shown rows of loaded profiles (rail tiles, Grid cells).
    pub fn shown_loaded(&self) -> usize {
        self.shown_loaded
    }

    /// Shown rows of profiles that are not loaded (the rail's folded
    /// "not loaded" group).
    pub fn shown_unloaded(&self) -> usize {
        self.shown_unloaded
    }
}

/// The fleet view model: one row per profile identity, in the saved Load
/// order (loaded and not loaded together; a loaded slot without a vault
/// row last), the marks, the filter and order, and the shown rows. Each front
/// end holds one and lends it to [`crate::OperatorSession::submit`] through
/// [`crate::Models::with_fleet`]; its mutators are crate-private, so marks,
/// filter and order change only through commands.
///
/// [`Self::refresh`] follows the core: it rebuilds the universe only when
/// the projected rows or the vault's profiles moved. The shown rows and the
/// counts are recomputed only when the universe, the filter, the order or
/// the marks changed. [`Self::generation`] moves whenever anything a front
/// end shows from here changed, so a label cache keyed on it rebuilds only
/// then.
///
/// Marks are dropped only when their profile leaves the universe (its
/// vault row was deleted, or an unsaved loaded slot was removed); a filter
/// never drops a mark, and unloading a vault profile keeps it.
#[derive(Debug)]
pub struct FleetModel {
    marks: MarkedSelection,
    filter: FleetFilter,
    rows: Vec<FleetRow>,
    ids: Vec<ProfileIdentity>,
    shown: Vec<usize>,
    /// Parallel to `rows`: whether the row is shown.
    shown_flags: Vec<bool>,
    counts: FleetMarkCounts,
    /// The core's rows generation and profiles generation the universe was
    /// built from.
    seen: Option<(u64, u64)>,
    generation: u64,
    /// Reused sorted identity buffer for de-duplication and mark pruning.
    scratch: Vec<ProfileIdentity>,
    /// Reused flags: which projected rows a rebuild has placed.
    placed: Vec<bool>,
    /// Moves whenever the universe was rebuilt (row contents may have
    /// changed), never for a filter, order or mark change.
    rows_generation: u64,
    /// The card the fleet buttons use, and the one staged for the next
    /// [`crate::OperatorCommand::SetFleetCard`].
    card: FleetCard,
    staged_card: Option<FleetCard>,
    /// The card label and the inputs it was built from
    /// ([`Self::sync_card_label`]).
    card_label: CardLabel,
}

impl Default for FleetModel {
    fn default() -> Self {
        Self::new()
    }
}

impl FleetModel {
    /// An empty model in load order with no filter and no marks.
    pub fn new() -> Self {
        Self {
            marks: MarkedSelection::default(),
            filter: FleetFilter::new(FleetOrder::LoadOrder),
            rows: Vec::new(),
            ids: Vec::new(),
            shown: Vec::new(),
            shown_flags: Vec::new(),
            counts: FleetMarkCounts::default(),
            seen: None,
            generation: 0,
            scratch: Vec::new(),
            placed: Vec::new(),
            rows_generation: 0,
            card: FleetCard::OWN,
            staged_card: None,
            card_label: CardLabel::default(),
        }
    }

    /// Every profile's row, in the universe's order (see the type docs).
    /// Index it with [`Self::shown`].
    pub fn rows(&self) -> &[FleetRow] {
        &self.rows
    }

    /// The identities parallel to [`Self::rows`].
    pub fn identities(&self) -> &[ProfileIdentity] {
        &self.ids
    }

    /// Row `index`'s identity.
    pub fn identity(&self, index: usize) -> Option<ProfileIdentity> {
        self.ids.get(index).copied()
    }

    /// The row index of `identity`.
    pub fn index_of(&self, identity: ProfileIdentity) -> Option<usize> {
        self.ids.iter().position(|id| *id == identity)
    }

    /// The row index of the profile called `name`.
    pub fn index_of_name(&self, name: &str) -> Option<usize> {
        self.rows.iter().position(|row| row.name() == name)
    }

    /// The indices of the rows the filter shows, in the order.
    pub fn shown(&self) -> &[usize] {
        &self.shown
    }

    /// Whether row `index` is shown.
    pub fn is_shown(&self, index: usize) -> bool {
        self.shown_flags.get(index).copied().unwrap_or(false)
    }

    /// Whether `identity` is in the scope a fleet action covers while
    /// nothing is marked: a loaded, shown row.
    pub fn in_unmarked_scope(&self, identity: ProfileIdentity) -> bool {
        self.index_of(identity)
            .is_some_and(|index| self.is_shown(index) && self.rows[index].loaded())
    }

    /// The shown rows with their indices, in the order.
    pub fn shown_rows(&self) -> impl Iterator<Item = (usize, &FleetRow)> + '_ {
        self.shown.iter().map(|&index| (index, &self.rows[index]))
    }

    /// The identities of the shown rows: the rows "mark all shown" marks.
    pub fn shown_identities(&self) -> impl Iterator<Item = ProfileIdentity> + '_ {
        self.shown.iter().map(|&index| self.ids[index])
    }

    /// The marks.
    pub fn marks(&self) -> &MarkedSelection {
        &self.marks
    }

    /// Whether row `index` is marked.
    pub fn is_marked(&self, index: usize) -> bool {
        self.ids
            .get(index)
            .is_some_and(|identity| self.marks.contains(*identity))
    }

    /// Mark and row counts.
    pub fn counts(&self) -> FleetMarkCounts {
        self.counts
    }

    /// The filter text and order.
    pub fn filter(&self) -> &FleetFilter {
        &self.filter
    }

    /// The staged filter text, for a text field to edit; the shown rows
    /// follow it only when [`crate::OperatorCommand::SetFleetFilter`]
    /// commits it.
    pub fn staged_filter_mut(&mut self) -> &mut String {
        self.filter.staged_mut()
    }

    /// The row order.
    pub fn order(&self) -> FleetOrder {
        self.filter.order()
    }

    /// Moves whenever the rows, the shown rows, the marks, the filter or
    /// the order changed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Moves whenever the universe was rebuilt, so a cache of per-row
    /// labels rebuilds only then (not for a filter, order or mark change).
    pub fn rows_generation(&self) -> u64 {
        self.rows_generation
    }

    /// The card the fleet buttons use.
    pub fn card(&self) -> &FleetCard {
        &self.card
    }

    /// Whether every, some or none of the shown rows is marked (the
    /// tri-state box over the shown rows).
    pub fn shown_marks(&self) -> ShownMarks {
        let counts = self.counts;
        if counts.shown_marked == 0 {
            ShownMarks::None
        } else if counts.shown_marked == counts.shown {
            ShownMarks::All
        } else {
            ShownMarks::Some
        }
    }

    /// Bring the card label up to date with the focused bot's heading
    /// (call once per frame; it borrows and compares until an input
    /// changed, then rebuilds the label in its own buffer), and borrow it.
    pub fn sync_card_label<Io>(
        &mut self,
        scripts: &crate::scripts::Scripts,
        core: &crate::OperatorSession<Io>,
    ) -> &str {
        let focused = core.selected();
        let (heading, pending) = match (self.card.choice(), focused) {
            (CardChoice::Focused, Some(name)) => (
                scripts.heading_label(core, name),
                scripts.heading_is_pending(core, name),
            ),
            _ => (None, false),
        };
        self.card_label.sync(&self.card, focused, heading, pending);
        &self.card_label.text
    }

    /// The card label as of the last [`Self::sync_card_label`].
    pub fn card_label(&self) -> &str {
        &self.card_label.text
    }

    /// Follow the core: rebuild the universe when the projected rows or the
    /// vault's profiles moved since the last call. Returns whether it did.
    /// Cheap when nothing moved (two integer comparisons), so a front end
    /// calls it once per frame.
    pub fn refresh<Io>(&mut self, session: &crate::OperatorSession<Io>) -> bool {
        let view = session.fleet_view();
        let seen = (view.rows_generation(), session.profiles_generation());
        if self.seen == Some(seen) {
            return false;
        }
        self.seen = Some(seen);
        let projected = view.rows();
        let mut placed = std::mem::take(&mut self.placed);
        placed.clear();
        placed.resize(projected.len(), false);
        let mut len = 0;
        // The saved Load order: a loaded profile takes its projected row,
        // the others an unloaded row. The members follow the same order,
        // so the projected row is usually the next one.
        let mut hint = 0;
        for name in session.profile_order().names() {
            let loaded = match projected.get(hint) {
                Some(row) if row.name() == name.as_str() => Some(hint),
                _ => projected.iter().position(|row| row.name() == name.as_str()),
            };
            if let Some(index) = loaded.filter(|&index| !placed[index]) {
                placed[index] = true;
                hint = index + 1;
                let row = &projected[index];
                let identity = session.profile_identity_or_synthetic(name);
                self.put_row(len, identity, |dest| dest.copy_from(row), || row.snapshot());
                len += 1;
            } else if let Some(profile) = session.profile(name) {
                let identity = ProfileIdentity::uid(profile.uid());
                self.put_row(
                    len,
                    identity,
                    |dest| dest.reset_unloaded(name),
                    || FleetRow::for_profile(name.as_str()),
                );
                len += 1;
            }
        }
        // A loaded slot the order lacks (no vault row): last, in load
        // order.
        for (index, row) in projected.iter().enumerate() {
            if !placed[index] {
                let identity = session.profile_identity_or_synthetic(row.name());
                self.put_row(len, identity, |dest| dest.copy_from(row), || row.snapshot());
                len += 1;
            }
        }
        self.placed = placed;
        // A vault profile the order does not hold yet (the session
        // reconciles at its next poll).
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.ids[..len]);
        self.scratch.sort_unstable();
        let ordered = len;
        for profile in session.profiles() {
            let identity = ProfileIdentity::uid(profile.uid());
            if self.scratch[..ordered].binary_search(&identity).is_ok() {
                continue;
            }
            let name = profile.username();
            self.put_row(
                len,
                identity,
                |dest| dest.reset_unloaded(name),
                || FleetRow::for_profile(name),
            );
            len += 1;
        }
        self.rows.truncate(len);
        self.ids.truncate(len);
        self.universe_changed();
        true
    }

    /// Write row `index` of the rebuilt universe, reusing the old row's
    /// buffers when there is one.
    fn put_row(
        &mut self,
        index: usize,
        identity: ProfileIdentity,
        reuse: impl FnOnce(&mut FleetRow),
        fresh: impl FnOnce() -> FleetRow,
    ) {
        match self.rows.get_mut(index) {
            Some(dest) => {
                reuse(dest);
                self.ids[index] = identity;
            }
            None => {
                self.rows.push(fresh());
                self.ids.push(identity);
            }
        }
    }

    /// The universe was rebuilt: drop marks of identities that left it,
    /// then recompute the shown rows.
    fn universe_changed(&mut self) {
        self.rows_generation = self.rows_generation.wrapping_add(1);
        if !self.marks.is_empty() {
            self.scratch.clear();
            self.scratch.extend_from_slice(&self.ids);
            self.scratch.sort_unstable();
            self.marks.retain_sorted(&self.scratch);
        }
        self.reshow();
    }

    /// Recompute the shown rows and the counts.
    fn reshow(&mut self) {
        fill_visible_row_indices(
            &self.rows,
            self.filter.text(),
            self.filter.order(),
            &mut self.shown,
        );
        self.shown_flags.clear();
        self.shown_flags.resize(self.rows.len(), false);
        for &index in &self.shown {
            self.shown_flags[index] = true;
        }
        self.recount();
    }

    /// Recompute the counts and move the generation.
    fn recount(&mut self) {
        let mut counts = FleetMarkCounts {
            rows: self.rows.len(),
            shown: self.shown.len(),
            ..FleetMarkCounts::default()
        };
        for (index, row) in self.rows.iter().enumerate() {
            let shown = self.shown_flags[index];
            let marked = self.marks.contains(self.ids[index]);
            counts.marked += usize::from(marked);
            counts.shown_marked += usize::from(marked && shown);
            counts.shown_loaded += usize::from(shown && row.loaded());
            counts.shown_unloaded += usize::from(shown && !row.loaded());
        }
        self.counts = counts;
        self.generation = self.generation.wrapping_add(1);
    }

    pub(crate) fn set_mark(&mut self, identity: ProfileIdentity, marked: bool) {
        if self.marks.contains(identity) != marked {
            self.marks.set(identity, marked);
            self.recount();
        }
    }

    pub(crate) fn mark_all(&mut self, identities: impl IntoIterator<Item = ProfileIdentity>) {
        let before = self.marks.len();
        self.marks.mark_all(identities);
        if self.marks.len() != before {
            self.recount();
        }
    }

    pub(crate) fn clear_marks(&mut self) {
        if !self.marks.is_empty() {
            self.marks.clear();
            self.recount();
        }
    }

    /// Drop a deleted profile's mark. Returns whether a mark was removed.
    pub(crate) fn remove_mark(&mut self, identity: ProfileIdentity) -> bool {
        let removed = self.marks.remove(identity);
        if removed {
            self.recount();
        }
        removed
    }

    /// Apply a mark operation to the shown rows (hidden rows keep their
    /// marks). Returns how many marks changed.
    pub(crate) fn apply_mark_op(&mut self, op: MarkOp) -> usize {
        let mut changed = 0;
        for &index in &self.shown {
            let identity = self.ids[index];
            let marked = self.marks.contains(identity);
            let want = match op {
                MarkOp::Shown => true,
                MarkOp::UnmarkShown => false,
                MarkOp::InvertShown => !marked,
                MarkOp::Where(predicate) => {
                    if !predicate.matches(&self.rows[index]) {
                        continue;
                    }
                    true
                }
            };
            if want != marked {
                self.marks.set(identity, want);
                changed += 1;
            }
        }
        if changed > 0 {
            self.recount();
        }
        changed
    }

    /// Stage the card the next [`crate::OperatorCommand::SetFleetCard`]
    /// commits.
    pub fn stage_card(&mut self, card: FleetCard) {
        self.staged_card = Some(card);
    }

    /// Commit the staged card: `None` when nothing was staged, else
    /// whether the card changed.
    pub(crate) fn commit_card(&mut self) -> Option<bool> {
        let card = self.staged_card.take()?;
        if card == self.card {
            return Some(false);
        }
        self.card = card;
        self.card_label.key = None;
        self.generation = self.generation.wrapping_add(1);
        Some(true)
    }

    /// Commit the staged filter text: `false` when it was already the
    /// filter.
    pub(crate) fn commit_filter(&mut self) -> bool {
        let changed = self.filter.commit();
        if changed {
            self.reshow();
        }
        changed
    }

    pub(crate) fn set_order(&mut self, order: FleetOrder) -> bool {
        let changed = self.filter.set_order(order);
        if changed {
            self.reshow();
        }
        changed
    }
}

#[cfg(any(test, feature = "test-support"))]
impl FleetModel {
    /// Fixture seam: replace the universe with `rows` (and their `ids`,
    /// parallel; a missing id is the name's fixture identity) without a
    /// session, as a refresh would.
    pub fn set_universe_for_test(&mut self, rows: Vec<FleetRow>, ids: &[ProfileIdentity]) {
        self.ids = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                ids.get(index)
                    .copied()
                    .unwrap_or_else(|| ProfileIdentity::synthetic(row.name()))
            })
            .collect();
        self.rows = rows;
        self.seen = None;
        self.universe_changed();
    }

    /// Fixture seam: mark or unmark `identity` without a command.
    pub fn set_mark_for_test(&mut self, identity: ProfileIdentity, marked: bool) {
        self.set_mark(identity, marked);
    }

    /// Fixture seam: mark every identity of `identities` without a command.
    pub fn mark_all_for_test(&mut self, identities: impl IntoIterator<Item = ProfileIdentity>) {
        self.mark_all(identities);
    }

    /// Fixture seam: drop every mark without a command.
    pub fn clear_marks_for_test(&mut self) {
        self.clear_marks();
    }

    /// Fixture seam: set the filter text and commit it without a command.
    pub fn set_filter_for_test(&mut self, text: &str) {
        let staged = self.filter.staged_mut();
        staged.clear();
        staged.push_str(text);
        self.commit_filter();
    }

    /// Fixture seam: set the order without a command.
    pub fn set_order_for_test(&mut self, order: FleetOrder) {
        self.set_order(order);
    }
}
