//! Fleet-window commands on the marked rows, shared by the panel and the TUI:
//! change a card (*Assign*, *Assign & restart*) and log in / out. Each marked
//! row is counted once in one [`BulkReport`] (or, for the paced restart, in
//! the running Start report), and a row that cannot take the command is
//! skipped with its reason.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::bulk::{BulkOutcome, BulkReport, BulkRow};
use crate::scripts::{Scripts, SyncScope};
use crate::selection::{profile_rows, BulkSkip, MarkedSelection, ProfileIdentity};
use crate::session::OperatorSession;
use crate::surface::SlotSurface;
use crate::views::Phase;

const UNAVAILABLE: &str = "profile unavailable";

/// The marked rows resolved to profile names (each once, by name) and the
/// marks whose profile is gone.
struct Resolved {
    names: Vec<String>,
    gone: Vec<String>,
}

/// A marked Assign/Restart command held until its catalog prerequisite settles.
#[derive(Debug, Clone)]
pub(crate) struct PendingCatalogAssignment {
    pub(crate) profiles: Vec<String>,
    pub(crate) unavailable: Vec<String>,
    pub(crate) card: script::ScriptSel,
    pub(crate) root: Option<PathBuf>,
    pub(crate) restart: bool,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkedCommandReport {
    pub accepted: usize,
    pub skipped: Vec<BulkSkip>,
}

impl MarkedCommandReport {
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
            let _ = write!(text, "{sep}{}: {}", skip.profile, skip.reason);
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
            report.skipped.push(BulkSkip {
                profile: format!("profile#{}", identity.raw()),
                reason: "profile unavailable".into(),
            });
            continue;
        };
        match admit(profile) {
            Ok(()) => report.accepted += 1,
            Err(reason) => report.skipped.push(BulkSkip {
                profile: profile.clone(),
                reason,
            }),
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
    match scripts.assignment_for(card, catalog_root) {
        Ok(Some(assignment)) => assign_ready_names(&names, &gone, core, scripts, assignment),
        Ok(None) => {
            let mut rows = unavailable_rows(gone.clone());
            let action = PendingCatalogAssignment {
                profiles: names.clone(),
                unavailable: gone,
                card: card.clone(),
                root: catalog_root.map(Path::to_path_buf),
                restart: false,
            };
            match scripts.defer_pending_catalog_assignment(action) {
                Ok(()) => {
                    rows.extend(names.iter().map(|name| {
                        BulkRow::new(name, BulkOutcome::Pending).with_detail("catalog scan pending")
                    }));
                    BulkReport::new(LABEL, DONE, rows, None)
                }
                Err(reason) => {
                    rows.extend(
                        names
                            .iter()
                            .map(|name| BulkRow::new(name, BulkOutcome::failed(&reason))),
                    );
                    BulkReport::new(LABEL, DONE, rows, Some(reason))
                }
            }
        }
        Err(reason) => {
            let mut rows = unavailable_rows(gone);
            rows.extend(
                names
                    .iter()
                    .map(|name| BulkRow::new(name, BulkOutcome::failed(&reason))),
            );
            BulkReport::new(LABEL, DONE, rows, Some(reason))
        }
    }
}

fn assign_ready_names<Io>(
    names: &[String],
    gone: &[String],
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    assignment: vault::ScriptAssignment,
) -> BulkReport {
    let mut rows = unavailable_rows(gone.to_vec());
    for name in names {
        let outcome = if active(state(core, name)) {
            BulkOutcome::skipped("script active: use Assign & restart")
        } else if scripts.persist_assignment_with_action(core, name, assignment.clone(), "Assign") {
            scripts.clear_pending_browse(name);
            BulkOutcome::Done
        } else {
            BulkOutcome::failed("could not save")
        };
        rows.push(BulkRow::new(name, outcome));
    }
    BulkReport::new("Assign", "assigned", rows, None)
}

/// Which marked bots *Assign & restart* would interrupt, for the
/// confirmation that names them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RestartScope {
    /// Marked bots with an active script: it is stopped, then the card
    /// starts.
    pub interrupted: Vec<String>,
    /// Marked, loaded bots with no active script: the card just starts.
    pub starting: Vec<String>,
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
    match scripts.assignment_for(card, catalog_root) {
        Ok(Some(assignment)) => {
            enqueue_assigned_restarts(names, core, scripts, card, catalog_root, assignment);
        }
        Ok(None) => {
            let action = PendingCatalogAssignment {
                profiles: names.to_vec(),
                unavailable: gone.to_vec(),
                card: card.clone(),
                root: catalog_root.map(Path::to_path_buf),
                restart: true,
            };
            if let Err(reason) = scripts.defer_pending_catalog_assignment(action) {
                for name in names {
                    scripts.tally_fail(name, &reason);
                }
            } else {
                for name in names {
                    scripts.tally_queued_pending(name);
                }
            }
            scripts.publish_bulk();
        }
        Err(reason) => {
            for name in names {
                scripts.tally_fail(name, &reason);
            }
            scripts.publish_bulk();
        }
    }
}

fn enqueue_assigned_restarts<Io>(
    names: &[String],
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    card: &script::ScriptSel,
    catalog_root: Option<&Path>,
    assignment: vault::ScriptAssignment,
) {
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

/// Complete a marked Assign/Restart receipt once the catalog worker settles.
pub(crate) fn poll_pending_catalog_assignment<Io>(
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
) {
    if scripts.catalog_work_pending() {
        return;
    }
    let Some(action) = scripts.take_pending_catalog_assignment() else {
        return;
    };
    let published_root = scripts.catalog_scan_root().map(Path::to_path_buf);
    if action
        .root
        .as_deref()
        .is_some_and(|root| published_root.as_deref() != Some(root))
    {
        fail_pending_catalog_assignment(
            &action,
            core,
            scripts,
            "catalog scan did not publish the requested root",
        );
        return;
    }
    let Some(root) = action.root.as_deref().or(published_root.as_deref()) else {
        fail_pending_catalog_assignment(&action, core, scripts, "catalog root is not available");
        return;
    };
    match scripts.assignment_for(&action.card, Some(root)) {
        Ok(Some(assignment)) if action.restart => {
            scripts.open_tally("Assign & restart");
            for profile in &action.unavailable {
                scripts.tally_skip_unavailable(profile, UNAVAILABLE);
            }
            enqueue_assigned_restarts(
                &action.profiles,
                core,
                scripts,
                &action.card,
                Some(root),
                assignment,
            );
        }
        Ok(Some(assignment)) => {
            let report = assign_ready_names(
                &action.profiles,
                &action.unavailable,
                core,
                scripts,
                assignment,
            );
            scripts.show(report.summary());
        }
        Ok(None) => {
            let _ = scripts.defer_pending_catalog_assignment(action);
        }
        Err(error) => fail_pending_catalog_assignment(&action, core, scripts, &error),
    }
}

fn fail_pending_catalog_assignment<Io>(
    action: &PendingCatalogAssignment,
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    reason: &str,
) {
    if action.restart {
        scripts.open_tally("Assign & restart");
        for profile in &action.unavailable {
            scripts.tally_skip_unavailable(profile, UNAVAILABLE);
        }
        for profile in &action.profiles {
            scripts.tally_fail(profile, reason);
        }
        scripts.publish_bulk();
    } else {
        let mut rows = unavailable_rows(action.unavailable.clone());
        rows.extend(
            action
                .profiles
                .iter()
                .map(|profile| BulkRow::new(profile, BulkOutcome::failed(reason))),
        );
        let report = BulkReport::new("Assign", "assigned", rows, Some(reason.to_string()));
        scripts.show(report.summary());
    }
    let _ = core;
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
        let phase = core.fleet_view().row(&name).map(|row| row.phase);
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
        let phase = core.fleet_view().row(&name).map(|row| row.phase);
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
                .js
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
