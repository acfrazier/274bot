//! Fleet-window commands on the marked rows, shared by the panel and the TUI:
//! change a card (*Assign*, *Assign & restart*) and log in / out. Each marked
//! row is counted once in one [`BulkReport`] (or, for the paced restart, in
//! the running Start report), and a row that cannot take the command is
//! skipped with its reason.

use std::fmt::Write as _;
use std::path::Path;

use crate::bulk::{BulkOutcome, BulkReport, BulkRow};
use crate::scripts::{Scripts, SyncScope};
use crate::selection::{profile_rows, BulkSkip, MarkedSelection, ProfileIdentity};
use crate::session::OperatorSession;
use crate::surface::SlotSurface;
use crate::views::{FleetRow, Phase};

const UNAVAILABLE: &str = "profile unavailable";

/// The report a refused *Assign* leaves when no card is selected, the
/// same on both surfaces.
pub const ASSIGN_NO_CARD: &str = "Assign: select a card in Scripts first";
/// The report a refused *Assign & restart* leaves when no card is selected.
pub const ASSIGN_RESTART_NO_CARD: &str = "Assign & restart: select a card in Scripts first";

/// One marked-bots action of the fleet surfaces (the panel's Fleet
/// action bar, the TUI's marked commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkedAction {
    /// Start the selected card, or each profile's saved assignment.
    Start,
    /// Stop the marked bots' scripts.
    Stop,
    /// Save the selected card as the marked bots' assignment.
    Assign,
    /// Assign the selected card and start it on the marked bots.
    AssignAndRestart,
    /// Log in the marked bots, loading the ones not loaded yet.
    Login,
    /// Log out the marked bots.
    Logout,
    /// Walk the marked bots (open the group-send picker / map send).
    Walk,
    /// Apply the focused bot's settings to the marked bots.
    ApplySettings,
}

/// Why a marked action cannot run now. Both surfaces use the one
/// precedence of [`marked_availability`]: nothing marked first, then a
/// missing focused bot, then no card selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkedBlock {
    /// No row is marked.
    NothingMarked,
    /// The action copies from a focused bot and none is focused.
    NoFocusedBot,
    /// The action needs the selected card and none is selected.
    NoCard,
}

/// Whether `action` can run with `marked` marks, a selected card
/// (`has_card`) and a focused bot (`has_focused`). `Start` needs no
/// card here: without one it starts each marked profile's saved
/// assignment, and [`crate::selection::marked_startable`] says whether
/// any marked profile could start at all.
pub fn marked_availability(
    action: MarkedAction,
    marked: usize,
    has_card: bool,
    has_focused: bool,
) -> Result<(), MarkedBlock> {
    if marked == 0 {
        return Err(MarkedBlock::NothingMarked);
    }
    match action {
        MarkedAction::Start
        | MarkedAction::Stop
        | MarkedAction::Login
        | MarkedAction::Logout
        | MarkedAction::Walk => Ok(()),
        MarkedAction::Assign | MarkedAction::AssignAndRestart => {
            has_card.then_some(()).ok_or(MarkedBlock::NoCard)
        }
        MarkedAction::ApplySettings => {
            if !has_focused {
                return Err(MarkedBlock::NoFocusedBot);
            }
            has_card.then_some(()).ok_or(MarkedBlock::NoCard)
        }
    }
}

/// Who a bulk command acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkScope {
    /// Every member (or, for loading, every vault profile).
    All,
    /// The marked members only.
    Marked,
}

/// The scope of a combined all/marked command (the TUI's *Start all*,
/// *Stop all*, *Load+login all* and *Logout all*): the marked bots
/// when anything is marked, everyone otherwise. The panel keeps its
/// separate all and marked buttons instead of resolving a scope.
pub fn combined_bulk_scope(selection: &MarkedSelection) -> BulkScope {
    if selection.is_empty() {
        BulkScope::All
    } else {
        BulkScope::Marked
    }
}

/// The marked rows resolved to profile names (each once, by name) and the
/// marks whose profile is gone.
struct Resolved {
    names: Vec<String>,
    gone: Vec<String>,
}

fn resolve<Io>(selection: &MarkedSelection, core: &OperatorSession<Io>) -> Resolved {
    let profiles = profile_rows(core);
    let mut names: Vec<String> = Vec::with_capacity(selection.len());
    let mut gone = Vec::new();
    for identity in selection.iter() {
        match profiles.iter().find(|(_, id)| *id == identity) {
            Some((name, _)) if !names.contains(name) => names.push(name.clone()),
            Some(_) => {}
            None => gone.push(format!("profile#{}", identity.raw())),
        }
    }
    names.sort();
    Resolved { names, gone }
}

/// The outcome of one command admitted against a frozen marked-row snapshot.
///
/// `accepted + skipped.len()` is the number of rows in the snapshot that were
/// selected. Callers should retain this report rather than inferring success
/// from the number of marked rows after the command: a profile can disappear
/// or change readiness while the per-row admission closure runs.
#[derive(Debug, PartialEq, Eq)]
pub struct MarkedCommandReport {
    accepted: usize,
    skipped: Vec<BulkSkip>,
}

impl MarkedCommandReport {
    /// Build a report from already-counted admissions.
    pub fn from_parts(accepted: usize, skipped: Vec<BulkSkip>) -> Self {
        Self { accepted, skipped }
    }

    /// How many marked rows the command admitted.
    pub fn accepted(&self) -> usize {
        self.accepted
    }

    /// Rows that could not take the command, with their reasons.
    pub fn skipped(&self) -> &[BulkSkip] {
        &self.skipped
    }

    /// Take the accepted count and skipped rows.
    pub fn into_parts(self) -> (usize, Vec<BulkSkip>) {
        (self.accepted, self.skipped)
    }

    pub fn total(&self) -> usize {
        self.accepted + self.skipped.len()
    }

    pub fn summary(&self, action: &str) -> String {
        let mut text = format!(
            "{action}: accepted {}, skipped {}",
            self.accepted,
            self.skipped.len()
        );
        let mut sep = ": ";
        for skip in &self.skipped {
            let _ = write!(text, "{sep}{}: {}", skip.profile(), skip.reason());
            sep = ", ";
        }
        text
    }
}

/// Admit one command on each marked row in a frozen identity/name snapshot.
///
/// The caller supplies rows as owned values before invoking the closure. This
/// is intentional: admission may wake workers and publish status changes, but
/// those changes must not alter which names this command reports or targets.
pub fn run_marked_command<I, F>(
    selection: &MarkedSelection,
    rows: I,
    mut admit: F,
) -> MarkedCommandReport
where
    I: IntoIterator<Item = (ProfileIdentity, String)>,
    F: FnMut(&str) -> Result<(), String>,
{
    let rows: Vec<_> = rows.into_iter().collect();
    let mut report = MarkedCommandReport {
        accepted: 0,
        skipped: Vec::new(),
    };
    for identity in selection.iter() {
        let Some((_, profile)) = rows.iter().find(|(row_id, _)| *row_id == identity) else {
            report.skipped.push(BulkSkip::new(
                format!("profile#{}", identity.raw()),
                "profile unavailable",
            ));
            continue;
        };
        match admit(profile) {
            Ok(()) => report.accepted += 1,
            Err(reason) => report.skipped.push(BulkSkip::new(profile.clone(), reason)),
        }
    }
    report
}

fn unavailable_rows(gone: Vec<String>) -> Vec<BulkRow> {
    gone.into_iter()
        .map(|profile| BulkRow::new(profile, BulkOutcome::skipped(UNAVAILABLE)))
        .collect()
}

fn state<Io>(core: &OperatorSession<Io>, name: &str) -> script::RunState {
    core.play()
        .map_or(script::RunState::Idle, |play| play.script_state(name))
}

fn active(state: script::RunState) -> bool {
    !matches!(state, script::RunState::Idle | script::RunState::Error)
}

/// Save `card` as the assignment of every marked profile, without starting
/// anything: the operator can adjust each bot's settings before Start. A bot
/// whose script is active is skipped (use [`assign_and_restart_marked`]);
/// per-bot settings are untouched.
pub fn assign_marked<Io>(
    selection: &MarkedSelection,
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    card: &script::ScriptSel,
    catalog_root: Option<&Path>,
) -> BulkReport {
    const LABEL: &str = "Assign";
    const DONE: &str = "assigned";
    let Resolved { names, gone } = resolve(selection, core);
    let mut rows = unavailable_rows(gone);
    let assignment = match scripts.assignment_for(card, catalog_root) {
        Ok(assignment) => assignment,
        Err(reason) => {
            rows.extend(
                names
                    .iter()
                    .map(|name| BulkRow::new(name, BulkOutcome::failed(&reason))),
            );
            return BulkReport::new(LABEL, DONE, rows, Some(reason));
        }
    };
    for name in names {
        let outcome = if active(state(core, &name)) {
            BulkOutcome::skipped("script active: use Assign & restart")
        } else if scripts.persist_assignment_with_action(core, &name, assignment.clone(), "Assign")
        {
            scripts.clear_pending_browse(&name);
            BulkOutcome::Done
        } else {
            BulkOutcome::failed("could not save")
        };
        rows.push(BulkRow::new(name, outcome));
    }
    BulkReport::new(LABEL, DONE, rows, None)
}

/// Which marked bots *Assign & restart* would interrupt, for the
/// confirmation that names them.
#[derive(Debug, PartialEq, Eq, Default)]
pub struct RestartScope {
    /// Marked bots with an active script: it is stopped, then the card
    /// starts.
    interrupted: Vec<String>,
    /// Marked, loaded bots with no active script: the card just starts.
    starting: Vec<String>,
}

/// How many names one confirmation line spells out before the rest
/// are counted as "+N more" (the [`RestartScope::prompt_lines`]
/// lines and every surface list built with [`bounded_names`]).
pub const PROMPT_NAMES_SHOWN: usize = 6;

/// `names` joined for one confirmation line: at most
/// [`PROMPT_NAMES_SHOWN`] names, then "+N more" for the remainder.
/// The one bounded-names list every surface renders, so the cap
/// cannot drift between them.
pub fn bounded_names(names: &[String]) -> String {
    let mut text = String::new();
    for (index, name) in names.iter().take(PROMPT_NAMES_SHOWN).enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        text.push_str(name);
    }
    if names.len() > PROMPT_NAMES_SHOWN {
        let _ = write!(text, " +{} more", names.len() - PROMPT_NAMES_SHOWN);
    }
    text
}

impl RestartScope {
    /// Marked bots with an active script: it is stopped, then the card starts.
    pub fn interrupted(&self) -> &[String] {
        &self.interrupted
    }

    /// Marked, loaded bots with no active script: the card just starts.
    pub fn starting(&self) -> &[String] {
        &self.starting
    }

    /// No loaded marked bot would take the card.
    pub fn is_empty(&self) -> bool {
        self.interrupted.is_empty() && self.starting.is_empty()
    }

    /// The scope over `names` (the marked, loaded bots) from their
    /// projected `rows`, for a surface that renders from the fleet
    /// projection instead of the session. A name with no row yet has no
    /// active script and counts as starting.
    pub fn from_rows<'a>(names: impl IntoIterator<Item = &'a str>, rows: &[FleetRow]) -> Self {
        let mut scope = Self::default();
        for name in names {
            let state = rows
                .iter()
                .find(|row| row.name() == name)
                .map_or(script::RunState::Idle, |row| row.script());
            if active(state) {
                scope.interrupted.push(name.to_string());
            } else {
                scope.starting.push(name.to_string());
            }
        }
        scope
    }

    /// The confirmation text, one line per part. Each line names at
    /// most [`PROMPT_NAMES_SHOWN`] bots and counts the rest
    /// ("+N more"), so no bot is hidden silently however large the
    /// fleet: the line's count always covers every bot in its part.
    pub fn prompt_lines(&self) -> Vec<String> {
        if self.is_empty() {
            return vec!["No loaded marked bot to start the card on.".to_string()];
        }
        let mut lines = Vec::new();
        if !self.interrupted.is_empty() {
            lines.push(format!(
                "Stop and restart {} running {}: {}",
                self.interrupted.len(),
                if self.interrupted.len() == 1 {
                    "bot"
                } else {
                    "bots"
                },
                bounded_names(&self.interrupted)
            ));
        }
        if !self.starting.is_empty() {
            lines.push(format!(
                "Start the card on {} idle {}: {}",
                self.starting.len(),
                if self.starting.len() == 1 {
                    "bot"
                } else {
                    "bots"
                },
                bounded_names(&self.starting)
            ));
        }
        lines
    }
}

pub fn restart_scope<Io>(selection: &MarkedSelection, core: &OperatorSession<Io>) -> RestartScope {
    let mut scope = RestartScope::default();
    for name in resolve(selection, core).names {
        if !core.members().contains(&name) {
            continue;
        }
        if active(state(core, &name)) {
            scope.interrupted.push(name);
        } else {
            scope.starting.push(name);
        }
    }
    scope
}

/// Save `card` on every marked profile and start it there: an active script
/// is stopped first, and every Start waits for the same paced permit as Start
/// all. The running Start report ([`Scripts::last_bulk_report`]) names each
/// bot as it is queued, started, skipped or failed.
pub fn assign_and_restart_marked<Io>(
    selection: &MarkedSelection,
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    card: &script::ScriptSel,
    catalog_root: Option<&Path>,
) {
    let Resolved { names, gone } = resolve(selection, core);
    assign_and_restart_names(&names, &gone, core, scripts, card, catalog_root);
}

fn assign_and_restart_names<Io>(
    names: &[String],
    gone: &[String],
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    card: &script::ScriptSel,
    catalog_root: Option<&Path>,
) {
    scripts.open_tally("Assign & restart");
    for profile in gone {
        scripts.tally_skip_unavailable(profile, UNAVAILABLE);
    }
    let assignment = match scripts.assignment_for(card, catalog_root) {
        Ok(assignment) => assignment,
        Err(reason) => {
            for name in names {
                scripts.tally_fail(name, &reason);
            }
            scripts.publish_bulk();
            return;
        }
    };
    let mut stopping = Vec::new();
    for name in names {
        if !core.members().contains(name) {
            scripts.tally_skip(name, "not loaded");
            continue;
        }
        if !scripts.persist_assignment_with_action(
            core,
            name,
            assignment.clone(),
            "Assign & restart",
        ) {
            scripts.tally_fail(name, "could not save");
            continue;
        }
        scripts.clear_pending_browse(name);
        let running = state(core, name);
        if matches!(
            running,
            script::RunState::Starting | script::RunState::Running | script::RunState::Paused
        ) {
            stopping.push(name.clone());
        }
        if let Err(reason) = scripts.queue_restart(core, name, card.clone(), active(running)) {
            scripts.tally_fail(name, &reason);
        }
    }
    if !stopping.is_empty() {
        core.stop_scripts(&stopping);
    }
    scripts.admit_starts(core, catalog_root);
    scripts.publish_bulk();
}

/// Log in every marked profile, loading the ones not loaded yet. A bot that
/// is already logging in or logged in is skipped.
pub fn login_marked<Io, S: SlotSurface<Io = Io>>(
    selection: &MarkedSelection,
    core: &mut OperatorSession<Io>,
    surface: &mut S,
) -> BulkReport {
    const LABEL: &str = "Log in marked";
    const DONE: &str = "logging in";
    let Resolved { names, gone } = resolve(selection, core);
    let mut rows = unavailable_rows(gone);
    let mut eligible = Vec::new();
    for name in names {
        let phase = core.fleet_view().row(&name).map(|row| row.phase());
        let member = core.members().contains(&name);
        match phase.filter(|_| member) {
            Some(Phase::Ready) => rows.push(BulkRow::new(
                name,
                BulkOutcome::skipped("already logged in"),
            )),
            Some(
                Phase::Preparing
                | Phase::Waiting
                | Phase::Queued
                | Phase::Connecting
                | Phase::Loading,
            ) => rows.push(BulkRow::new(
                name,
                BulkOutcome::skipped("already logging in"),
            )),
            _ => {
                if !member {
                    core.load(&name, surface);
                }
                eligible.push(name);
            }
        }
    }
    if !eligible.is_empty() {
        core.login_members(&eligible, surface);
        rows.extend(
            eligible
                .into_iter()
                .map(|name| BulkRow::new(name, BulkOutcome::Done)),
        );
    }
    BulkReport::new(LABEL, DONE, rows, None)
}

/// Log out every marked profile that is logged in or logging in. A bot still
/// waiting for its Start is cancelled with the logout.
pub fn logout_marked<Io>(
    selection: &MarkedSelection,
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
) -> BulkReport {
    const LABEL: &str = "Log out marked";
    const DONE: &str = "logging out";
    let Resolved { names, gone } = resolve(selection, core);
    let mut rows = unavailable_rows(gone);
    let mut eligible = Vec::new();
    for name in names {
        let phase = core.fleet_view().row(&name).map(|row| row.phase());
        let member = core.members().contains(&name);
        match phase.filter(|_| member) {
            None | Some(Phase::Offline | Phase::LoggedOut | Phase::Failed) => {
                rows.push(BulkRow::new(name, BulkOutcome::skipped("not logged in")))
            }
            Some(_) => eligible.push(name),
        }
    }
    if !eligible.is_empty() {
        for name in &eligible {
            scripts.cancel_queued_as(name, "logged out");
        }
        scripts.publish_start_places(core);
        core.logout_members(&eligible);
        rows.extend(
            eligible
                .into_iter()
                .map(|name| BulkRow::new(name, BulkOutcome::Done)),
        );
    }
    BulkReport::new(LABEL, DONE, rows, None)
}

/// Freeze *Apply focused bot's settings to marked* for `card`: the parameters
/// `source` (the focused bot) holds for the card, and the marked bots that
/// would take them (already on the card, or unassigned — Apply assigns the
/// card, and the confirmation names that assignment). Unmarked bots are never
/// targets and are only counted; every marked row that cannot take the copy
/// is named with its reason. Read-only (`core` is borrowed shared): nothing
/// is written or migrated until [`Scripts::apply_settings_sync`]; a scope
/// with no marked target to copy to is refused instead of prepared.
pub fn prepare_apply_settings_marked<'a, Io>(
    selection: &MarkedSelection,
    core: &OperatorSession<Io>,
    scripts: &'a mut Scripts,
    source: &str,
    card: &script::ScriptSel,
) -> Result<&'a SyncScope, String> {
    const LABEL: &str = "Apply to marked";
    scripts.clear_prepared_settings_sync();
    if selection.is_empty() {
        return Err(format!("{LABEL}: mark fleet rows first"));
    }
    match card {
        script::ScriptSel::Compiled(id) => {
            scripts.prepare_compiled_settings_sync(core, source, *id, None)?;
        }
        script::ScriptSel::Loaded(card_source, lookup) => {
            let (name, path) = scripts
                .js()
                .get(*card_source, lookup)
                .map(|card| (card.name.clone(), card.path.clone()))
                .ok_or_else(|| format!("{LABEL}: unavailable: {lookup}"))?;
            scripts.prepare_settings_sync(core, source, *card_source, &name, &path);
        }
    }
    let Resolved { names, gone } = resolve(selection, core);
    scripts.restrict_prepared_settings_sync(&names, gone);
    let refusal = scripts.prepared_settings_sync().and_then(|scope| {
        scope.targets.is_empty().then(|| {
            let why = if scope.skipped.is_empty() {
                "the only marked bot is the source".to_string()
            } else {
                scope
                    .skipped
                    .iter()
                    .map(|(name, why)| format!("{name}: {why}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "{LABEL}: no marked bot on {} to copy {source}'s settings to ({why})",
                scope.card_name
            )
        })
    });
    if let Some(refusal) = refusal {
        scripts.clear_prepared_settings_sync();
        return Err(refusal);
    }
    scripts
        .prepared_settings_sync()
        .ok_or_else(|| format!("{LABEL}: nothing prepared"))
}

#[cfg(test)]
#[path = "marked_tests.rs"]
mod tests;
