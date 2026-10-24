//! Fleet plans on the command seam: `PlanFleetAction` freezes a fleet
//! action ([`crate::fleet_plan`]), `CancelFleetPlan` drops it, a command
//! with [`super::commands::Targets::Plan`] runs it, and the one result
//! view follows that command's record until it settles.

use std::fmt::Write as _;

use api::identity::{Cancellation, RequestId, Settlement};

use super::commands::Applied;
use crate::bot_projection::{BotProjection, StopEligibility};
use crate::bulk::{BulkOutcome, BulkRow};
use crate::command::{Completion, Rejection};
use crate::fleet::CardChoice;
use crate::fleet_plan::{FleetAction, FleetPlan, FleetResult, PlanScope, ResultCounts};
use crate::scripts::{ScriptChoice, Scripts};
use crate::selection::{MarkedSelection, ProfileIdentity};
use crate::surface::SlotSurface;
use crate::views::{FleetRow, Phase};
use crate::OperatorSession;

/// Names a prompt or report spells out before "+N more".
const NAMES_SHOWN: usize = 8;

/// A cancelled member's reason in the result line when its owner gave
/// none, in the operator's words. A member a posted row superseded names
/// that row (`OperatorCommand::took_over_words`) and a login the slot's own
/// latch withdrew names the latch (`CommandBook::observe_intent`); with no
/// recorded cause a superseded member says only "cancelled", since neither
/// a later command nor any other cause is known.
fn cancellation_words(cause: Cancellation) -> &'static str {
    match cause {
        Cancellation::Superseded | Cancellation::Operator => "cancelled",
        Cancellation::ScopeInvalidated => "the session or slot it was for ended",
        Cancellation::Shutdown => "shutting down",
    }
}

fn write_names<'a>(text: &mut String, names: impl ExactSizeIterator<Item = &'a str>) {
    let total = names.len();
    for (index, name) in names.take(NAMES_SHOWN).enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        text.push_str(name);
    }
    if total > NAMES_SHOWN {
        let _ = write!(text, " +{} more", total - NAMES_SHOWN);
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

impl<Io> OperatorSession<Io> {
    /// The fleet plan waiting for Confirm or Cancel.
    pub fn fleet_plan(&self) -> Option<&FleetPlan> {
        self.commands.fleet_plan.as_ref()
    }

    /// The latest fleet action's result.
    pub fn fleet_result(&self) -> Option<&FleetResult> {
        self.commands.fleet_result.as_ref()
    }

    /// Stage the card the next [`crate::OperatorCommand::PlanFleetAction`]
    /// uses instead of the fleet card: an entry such as the Scripts
    /// window's copy shortcut, which copies the focused bot's own card.
    pub fn stage_plan_card(&mut self, card: ScriptChoice) {
        self.commands.plan_card = Some(card);
    }

    /// Stage the one profile the next
    /// [`crate::OperatorCommand::PlanFleetAction`] acts on instead of the
    /// marks or the shown rows: a world bubble's bot, the TUI's cursor row.
    pub fn stage_plan_target(&mut self, name: &str) {
        self.commands.plan_target = Some(self.profile_identity_or_synthetic(name));
    }

    /// A copy of the running plan's frozen mark set, if a planned command
    /// is being dispatched.
    pub(super) fn planned_marks(&self) -> Option<MarkedSelection> {
        self.commands.planned.as_ref().map(|plan| {
            let mut marks = MarkedSelection::default();
            marks.mark_all(plan.marks.iter());
            marks
        })
    }

    /// The running plan's card (`Some(None)`: each bot's own assignment),
    /// if a planned command is being dispatched.
    pub(super) fn planned_card(&self) -> Option<Option<ScriptChoice>> {
        self.commands
            .planned
            .as_ref()
            .map(|plan| plan.card.as_ref().map(ScriptChoice::duplicate))
    }

    /// Whether `name` has a profile save or rename still being written
    /// (a form save of it, or its first durable row).
    pub(crate) fn profile_write_pending(&self, name: &str) -> bool {
        self.profile_saving(name)
            || self
                .saves_in_flight()
                .iter()
                .any(|save| save.destination() == name || save.source() == Some(name))
    }

    /// `PlanFleetAction`: freeze `action` over the lent fleet model.
    pub(super) fn plan_fleet_action<S: SlotSurface<Io = Io>>(
        &mut self,
        action: FleetAction,
        surface: &mut S,
    ) -> Applied {
        let mut models = surface.models();
        let mut scripts = models.scripts.as_deref_mut();
        // A newer plan replaces the waiting one; its prompt says so.
        let mut replaced = None;
        if let Some(old) = self.commands.fleet_plan.take() {
            if old.action == FleetAction::CopySettings {
                if let Some(scripts) = scripts.as_deref_mut() {
                    scripts.cancel_settings_sync();
                }
            }
            api::host_log!(
                api::hostlog::Category::Lifecycle,
                api::hostlog::Level::Info,
                "fleet: the waiting {} plan was replaced",
                old.action.verb()
            );
            replaced = Some(old.action);
        }
        let staged_target = self.commands.plan_target.take();
        let Some(fleet) = models.fleet.as_deref() else {
            return no_model("fleet");
        };
        if let FleetAction::SwitchWorld(world) = action {
            if let Some(why) = self.switch_world_unavailable(world) {
                return reject(format!("{}: {why}", action.verb()));
            }
        }
        let staged_card = self.commands.plan_card.take();
        // The catalog root a front end staged with the click: frozen with
        // the plan for the Start or assignment that runs it.
        let staged_root = scripts
            .as_deref_mut()
            .and_then(Scripts::take_staged_start_catalog_root);
        let catalog_root = matches!(
            action,
            FleetAction::Start | FleetAction::Assign | FleetAction::AssignRestart
        )
        .then_some(staged_root)
        .flatten();
        let card = match (staged_card, fleet.card().choice()) {
            (Some(card), _) => Some(card),
            (None, CardChoice::Own) => None,
            (None, CardChoice::Pinned(card)) => Some(card.duplicate()),
            (None, CardChoice::Focused) => match (scripts.as_deref(), self.selected()) {
                (Some(scripts), Some(name)) => scripts.heading(self, name),
                _ => None,
            },
        };
        if action.needs_card() && card.is_none() {
            let why = if fleet.card().is_own() {
                "pick a card: each bot's own card cannot be assigned or copied"
            } else {
                "the focused bot has no card"
            };
            return reject(format!("{}: {why}", action.verb()));
        }
        if (action == FleetAction::Start || action.needs_card()) && scripts.is_none() {
            return no_model("scripts");
        }

        // Freeze the candidates: the staged profile, else every mark
        // (hidden too), else the loaded shown rows.
        let marks = fleet.marks();
        let mut skipped = Vec::new();
        let mut candidates: Vec<(ProfileIdentity, usize)> = Vec::new();
        let scope = if let Some(identity) = staged_target {
            let Some(index) = fleet.index_of(identity) else {
                return reject(format!("{}: the profile is gone", action.verb()));
            };
            candidates.push((identity, index));
            PlanScope::One
        } else if marks.is_empty() {
            for (index, row) in fleet.shown_rows() {
                if row.loaded() {
                    candidates.push((fleet.identities()[index], index));
                }
            }
            PlanScope::AllShown
        } else {
            for (index, &identity) in fleet.identities().iter().enumerate() {
                if marks.contains(identity) {
                    candidates.push((identity, index));
                }
            }
            for identity in marks.iter() {
                if fleet.index_of(identity).is_none() {
                    skipped.push(BulkRow::new(
                        format!("profile#{}", identity.raw()),
                        BulkOutcome::skipped("profile unavailable"),
                    ));
                }
            }
            PlanScope::Marked {
                hidden: fleet.counts().hidden_marked(),
            }
        };

        let mut targets = Vec::new();
        let mut interrupted = Vec::new();
        for &(identity, index) in &candidates {
            let row = &fleet.rows()[index];
            let name = row.name();
            let verdict = if self.profile_write_pending(name) && action != FleetAction::Stop {
                Err("profile is being saved")
            } else {
                classify(action, row, card.is_some(), scripts.as_deref(), self)
            };
            match verdict {
                Ok(interrupts) => {
                    if interrupts {
                        interrupted.push(name.to_owned());
                    }
                    targets.push((identity, name.to_owned()));
                }
                Err(reason) => skipped.push(BulkRow::new(name, BulkOutcome::skipped(reason))),
            }
        }

        let mut copy_kind = None;
        let mut copy_source = None;
        if action == FleetAction::CopySettings {
            let Some(source) = self.selected().map(str::to_owned) else {
                return reject("Copy settings: focus the bot to copy from".into());
            };
            let (Some(scripts), Some(card)) = (scripts.as_deref_mut(), card.as_ref()) else {
                return no_model("scripts");
            };
            let mut frozen = MarkedSelection::default();
            frozen.mark_all(targets.iter().map(|(identity, _)| *identity));
            match prepare_copy(self, scripts, &source, card, &frozen, scope) {
                Ok(()) => {}
                Err(error) => return reject(error),
            }
            let Some(scope_view) = scripts.prepared_settings_sync() else {
                return reject("Copy settings: nothing prepared".into());
            };
            // The prepared scope decides who receives the copy.
            let receivers: Vec<String> = scope_view.targets().to_vec();
            for (profile, why) in scope_view.skipped() {
                if !skipped.iter().any(|row| row.profile() == profile) {
                    skipped.push(BulkRow::new(profile, BulkOutcome::skipped(why.as_str())));
                }
            }
            targets.retain(|(_, name)| receivers.iter().any(|r| r == name));
            interrupted.retain(|name| receivers.iter().any(|r| r == name));
            scripts.freeze_settings_sync_targets(targets.clone());
            copy_kind = Some(if card.compiled_id().is_some() {
                crate::command::CardKind::Native
            } else {
                crate::command::CardKind::Script
            });
            copy_source = Some(source);
        }

        if targets.is_empty() {
            if action == FleetAction::CopySettings {
                if let Some(scripts) = scripts {
                    scripts.cancel_settings_sync();
                }
            }
            let mut why = format!("{}: no bot to act on", action.verb());
            if !skipped.is_empty() {
                why.push_str(" (");
                write_skips(&mut why, &skipped);
                why.push(')');
            }
            return reject(why);
        }

        let mut frozen = MarkedSelection::default();
        frozen.mark_all(targets.iter().map(|(identity, _)| *identity));
        let needs_confirm = matches!(scope, PlanScope::AllShown)
            || !interrupted.is_empty()
            || matches!(
                action,
                FleetAction::CopySettings | FleetAction::SwitchWorld(_)
            );
        let mut prompt = String::new();
        if let Some(old) = replaced {
            let _ = write!(prompt, "Replaces the waiting {} plan. ", old.verb());
        }
        prompt += &prompt_text(
            action,
            scope,
            card.as_ref().map(ScriptChoice::label),
            copy_source.as_deref(),
            &targets,
            &interrupted,
            &skipped,
        );
        self.commands.fleet_plan = Some(FleetPlan {
            action,
            scope,
            targets,
            marks: frozen,
            card,
            copy_kind,
            interrupted,
            skipped,
            prompt,
            needs_confirm,
            catalog_root,
        });
        Applied::Done
    }

    /// `CancelFleetPlan`: drop the waiting plan; a settings copy releases
    /// its start hold. Writes nothing.
    pub(super) fn cancel_fleet_plan<S: SlotSurface<Io = Io>>(
        &mut self,
        surface: &mut S,
    ) -> Applied {
        let Some(plan) = self.commands.fleet_plan.take() else {
            return Applied::Rejected(
                Rejection::NotStaged,
                Some("fleet: no plan is waiting".into()),
            );
        };
        if plan.action == FleetAction::CopySettings {
            if let Some(scripts) = surface.models().scripts.as_deref_mut() {
                scripts.cancel_settings_sync();
            }
        }
        Applied::Done
    }

    /// Take the waiting plan to run as `command`: `Err` when none waits or
    /// it runs another command (the plan then stays).
    pub(super) fn take_plan_for(
        &mut self,
        command: crate::command::OperatorCommand,
    ) -> Result<FleetPlan, super::commands::SubmitError> {
        match self.commands.fleet_plan.take() {
            Some(plan) if plan.command() == command => Ok(plan),
            Some(plan) => {
                self.commands.fleet_plan = Some(plan);
                Err(super::commands::SubmitError::WrongTargets(command))
            }
            None => Err(super::commands::SubmitError::NoTarget(command)),
        }
    }

    /// After a planned command opened as `request`: keep its result view
    /// (the plan's skips and the profiles gone since the plan included).
    /// `None`: no target remained, so the result is terminal at once.
    pub(super) fn open_fleet_result(&mut self, request: Option<RequestId>, plan: FleetPlan) {
        let mut extra = plan.skipped;
        for (identity, name) in &plan.targets {
            let present = self
                .profiles()
                .any(|profile| ProfileIdentity::uid(profile.uid()) == *identity)
                || self.members().iter().any(|member| {
                    member == name && self.profile_identity_or_synthetic(member) == *identity
                });
            if !present {
                extra.push(BulkRow::new(
                    name.as_str(),
                    BulkOutcome::skipped("profile no longer exists"),
                ));
            }
        }
        self.commands.fleet_result = Some(FleetResult {
            request,
            action: plan.action,
            extra,
            text: String::new(),
            settled: false,
            logged: false,
            counts: ResultCounts {
                pending: usize::MAX,
                ..ResultCounts::default()
            },
        });
        self.refresh_fleet_result();
    }

    /// Bring the result view up to date with its command's members: the
    /// text is rebuilt only when a count moved; one Log line when every
    /// member has settled.
    pub(super) fn refresh_fleet_result(&mut self) {
        let Some(result) = self.commands.fleet_result.as_ref() else {
            return;
        };
        if result.settled {
            return;
        }
        let report = match result.request {
            Some(request) => match self.commands.book().report(request) {
                Some(report) => Some(report),
                None => return,
            },
            None => None,
        };
        let members = report.map_or(&[][..], |report| report.members());
        let mut counts = ResultCounts::default();
        for row in &result.extra {
            match row.outcome() {
                outcome if outcome.is_failed() => counts.failed += 1,
                outcome if outcome.is_skipped() => counts.skipped += 1,
                _ => counts.done += 1,
            }
        }
        for member in members {
            match member.terminal() {
                None => counts.pending += 1,
                Some(Settlement::Completed(Completion::Done)) => counts.done += 1,
                Some(Settlement::Completed(_)) => counts.skipped += 1,
                Some(Settlement::Cancelled(_)) => counts.cancelled += 1,
                Some(Settlement::Failed(_)) => counts.failed += 1,
            }
        }
        if counts == result.counts {
            return;
        }
        // Rebuild the text: failures, then cancellations, then skips, then
        // done.
        let mut failed = Vec::new();
        let mut cancelled: Vec<(String, String)> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();
        let mut done = Vec::new();
        for row in &result.extra {
            let reason = row.outcome().reason().unwrap_or("").to_owned();
            if row.outcome().is_failed() {
                failed.push((row.profile().to_owned(), reason));
            } else {
                skipped.push((row.profile().to_owned(), reason));
            }
        }
        for member in members {
            let crate::command::Subject::Profile(identity) = member.subject() else {
                continue;
            };
            let name = self.name_for_identity(identity);
            match member.terminal() {
                None => {}
                Some(Settlement::Completed(Completion::Done)) => done.push(name),
                Some(Settlement::Completed(other)) => {
                    let why = member
                        .detail()
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{other:?}"));
                    skipped.push((name, why));
                }
                Some(Settlement::Cancelled(cause)) => {
                    let why = member
                        .detail()
                        .unwrap_or_else(|| cancellation_words(*cause))
                        .to_owned();
                    cancelled.push((name, why));
                }
                Some(other) => {
                    let why = member
                        .detail()
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{other:?}"));
                    failed.push((name, why));
                }
            }
        }
        let action = result.action;
        let mut text = String::new();
        let _ = write!(text, "{}: ", action.verb());
        let mut first = true;
        let mut part = |text: &mut String, label: &str, rows: &[(String, String)]| {
            if rows.is_empty() {
                return;
            }
            if !first {
                text.push_str("; ");
            }
            first = false;
            let _ = write!(text, "{label} {}: ", rows.len());
            for (index, (name, why)) in rows.iter().take(NAMES_SHOWN).enumerate() {
                if index > 0 {
                    text.push_str(", ");
                }
                text.push_str(name);
                if !why.is_empty() {
                    let _ = write!(text, " ({why})");
                }
            }
            if rows.len() > NAMES_SHOWN {
                let _ = write!(text, " +{} more", rows.len() - NAMES_SHOWN);
            }
        };
        part(&mut text, "failed", &failed);
        part(&mut text, "cancelled", &cancelled);
        part(&mut text, "skipped", &skipped);
        if !done.is_empty() {
            if !first {
                text.push_str("; ");
            }
            first = false;
            let _ = write!(text, "done {}: ", done.len());
            write_names(&mut text, done.iter().map(String::as_str));
        }
        if counts.pending > 0 {
            if !first {
                text.push_str("; ");
            }
            let _ = write!(text, "pending {}", counts.pending);
        }
        let settled = counts.pending == 0;
        let Some(result) = self.commands.fleet_result.as_mut() else {
            return;
        };
        result.counts = counts;
        result.text = text;
        result.settled = settled;
        if settled && !result.logged {
            result.logged = true;
            api::host_log!(
                api::hostlog::Category::Lifecycle,
                api::hostlog::Level::Info,
                "fleet: {}",
                result.text
            );
        }
    }

    /// The profile name `identity` names now, else its raw number.
    pub(super) fn name_for_identity(&self, identity: ProfileIdentity) -> String {
        self.profiles()
            .find(|profile| ProfileIdentity::uid(profile.uid()) == identity)
            .map(|profile| profile.username().to_owned())
            .or_else(|| {
                self.members()
                    .iter()
                    .find(|name| self.profile_identity_or_synthetic(name) == identity)
                    .cloned()
            })
            .unwrap_or_else(|| format!("profile#{}", identity.raw()))
    }
}

/// Whether `row` is admitted to `action` (`Ok(true)` when the action
/// would interrupt its running script), or the skip reason.
fn classify<Io>(
    action: FleetAction,
    row: &FleetRow,
    has_card: bool,
    scripts: Option<&Scripts>,
    core: &OperatorSession<Io>,
) -> Result<bool, &'static str> {
    let bot = BotProjection::new(row, scripts, core);
    let name = row.name();
    let active = row.script().is_live();
    let logged_in = bot.in_game();
    let logging_in = row.loaded()
        && matches!(
            row.phase(),
            Phase::Preparing | Phase::Waiting | Phase::Queued | Phase::Connecting | Phase::Loading
        );
    match action {
        FleetAction::Start => {
            if active {
                return Err("already active");
            }
            if !has_card && Scripts::assignment_name(core, name).is_none() {
                return Err("no assignment");
            }
            Ok(false)
        }
        FleetAction::Stop => match bot.stop_eligibility() {
            StopEligibility::Running => Ok(true),
            StopEligibility::StartQueued => Ok(false),
            StopEligibility::NoScript => Err(StopEligibility::REFUSAL),
        },
        FleetAction::Assign | FleetAction::CopySettings => Ok(false),
        FleetAction::AssignRestart => Ok(active),
        FleetAction::Login => {
            if logged_in {
                Err("already logged in")
            } else if logging_in {
                Err("already logging in")
            } else {
                Ok(false)
            }
        }
        FleetAction::Logout => {
            if logged_in || logging_in {
                Ok(active)
            } else {
                Err("not logged in")
            }
        }
        // The relog keeps a running script's run: nothing is interrupted.
        FleetAction::SwitchWorld(world) => match super::world_switch::refusal(Some(row), world) {
            Some((_, why)) => Err(why),
            None => Ok(false),
        },
    }
}

/// Prepare the settings copy for the frozen targets: marked copies may
/// assign the card to unassigned targets (named in the prompt); the
/// no-marks scope copies to the shown bots already on the card.
fn prepare_copy<Io>(
    core: &OperatorSession<Io>,
    scripts: &mut Scripts,
    source: &str,
    card: &ScriptChoice,
    frozen: &MarkedSelection,
    scope: PlanScope,
) -> Result<(), String> {
    match scope {
        PlanScope::Marked { .. } | PlanScope::One => {
            crate::marked::prepare_apply_settings_marked(frozen, core, scripts, source, card)
                .map(|_| ())
        }
        PlanScope::AllShown => {
            crate::marked::prepare_apply_settings_shown(frozen, core, scripts, source, card)
                .map(|_| ())
        }
    }
}

fn write_skips(text: &mut String, skipped: &[BulkRow]) {
    for (index, row) in skipped.iter().take(NAMES_SHOWN).enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        text.push_str(row.profile());
        if let Some(reason) = row.outcome().reason() {
            let _ = write!(text, ": {reason}");
        }
    }
    if skipped.len() > NAMES_SHOWN {
        let _ = write!(text, " +{} more", skipped.len() - NAMES_SHOWN);
    }
}

/// The plan's prompt: what runs on whom, who is interrupted, who is
/// skipped. One wording for every front end.
fn prompt_text(
    action: FleetAction,
    scope: PlanScope,
    card: Option<&str>,
    copy_source: Option<&str>,
    targets: &[(ProfileIdentity, String)],
    interrupted: &[String],
    skipped: &[BulkRow],
) -> String {
    let count = targets.len();
    let noun = match scope {
        PlanScope::Marked { .. } => "marked bot",
        PlanScope::AllShown => "shown bot",
        PlanScope::One => "bot",
    };
    let mut text = String::new();
    match action {
        FleetAction::Start => match card {
            Some(card) => {
                let _ = write!(text, "Start {card} on {count} {noun}{}", plural(count));
            }
            None => {
                let _ = write!(
                    text,
                    "Start each bot's own card on {count} {noun}{}",
                    plural(count)
                );
            }
        },
        FleetAction::Stop => {
            let _ = write!(text, "Stop {count} {noun}{}", plural(count));
        }
        FleetAction::Assign => {
            let _ = write!(
                text,
                "Assign {} to {count} {noun}{}",
                card.unwrap_or(""),
                plural(count)
            );
        }
        FleetAction::AssignRestart => {
            let _ = write!(
                text,
                "Assign {} and restart it on {count} {noun}{}",
                card.unwrap_or(""),
                plural(count)
            );
        }
        FleetAction::Login => {
            let _ = write!(text, "Log in {count} {noun}{}", plural(count));
        }
        FleetAction::Logout => {
            let _ = write!(text, "Log out {count} {noun}{}", plural(count));
        }
        FleetAction::CopySettings => {
            let _ = write!(
                text,
                "Copy {}'s {} settings to {count} {noun}{}",
                copy_source.unwrap_or(""),
                card.unwrap_or(""),
                plural(count)
            );
        }
        FleetAction::SwitchWorld(world) => {
            let _ = write!(
                text,
                "Move {count} {noun}{} to world {world}",
                plural(count)
            );
        }
    }
    if let PlanScope::Marked { hidden } = scope {
        if hidden > 0 {
            let _ = write!(text, " ({hidden} not shown)");
        }
    }
    text.push_str(": ");
    write_names(&mut text, targets.iter().map(|(_, name)| name.as_str()));
    text.push('.');
    if !interrupted.is_empty() {
        text.push_str(" This stops the running script of ");
        write_names(&mut text, interrupted.iter().map(String::as_str));
        text.push('.');
    }
    if matches!(action, FleetAction::SwitchWorld(_)) {
        text.push_str(
            " Each logs out and back in through the login queue; a running script continues after it reconnects.",
        );
    }
    if !skipped.is_empty() {
        let _ = write!(text, " Skipped {}: ", skipped.len());
        write_skips(&mut text, skipped);
        text.push('.');
    }
    text
}

fn reject(reason: String) -> Applied {
    Applied::Rejected(Rejection::NotApplicable, Some(reason))
}

fn no_model(what: &str) -> Applied {
    Applied::Rejected(
        Rejection::NoModel,
        Some(format!("{what}: this front end holds none")),
    )
}

#[cfg(test)]
mod tests {
    use super::{cancellation_words, classify, prompt_text};
    use crate::bulk::{BulkOutcome, BulkRow};
    use crate::fleet_plan::{FleetAction, PlanScope};
    use crate::scripts::ScriptRunState as RunState;
    use crate::selection::ProfileIdentity;
    use crate::views::{FleetRow, Phase};

    fn ready(name: &str, script: RunState) -> FleetRow {
        let mut row = FleetRow::fixture(name, Phase::Ready, Some(1), None);
        row.set_script(script);
        row
    }

    fn core() -> crate::OperatorSession<()> {
        crate::OperatorSession::new(host_play::InstancePermit::SkipLock)
    }

    /// The confirmation policy's interruption: Stop, Assign & restart and
    /// Log out of a running bot interrupt it; Start, Assign and Log in
    /// never do.
    #[test]
    fn running_scripts_are_interrupted_only_by_the_interrupting_actions() {
        let core = core();
        let running = ready("a", RunState::Running);
        for (action, interrupts) in [
            (FleetAction::Stop, Ok(true)),
            (FleetAction::AssignRestart, Ok(true)),
            (FleetAction::Logout, Ok(true)),
            (FleetAction::Assign, Ok(false)),
            (FleetAction::Start, Err("already active")),
            (FleetAction::Login, Err("already logged in")),
        ] {
            assert_eq!(
                classify(action, &running, true, None, &core),
                interrupts,
                "{action:?}"
            );
        }
        let idle = ready("b", RunState::Idle);
        assert_eq!(
            classify(FleetAction::Stop, &idle, true, None, &core),
            Err("no script running")
        );
        let unloaded = FleetRow::for_profile("c");
        assert_eq!(
            classify(FleetAction::Logout, &unloaded, true, None, &core),
            Err("not logged in")
        );
        assert_eq!(
            classify(FleetAction::Login, &unloaded, true, None, &core),
            Ok(false)
        );
    }

    /// Review F4 r3 (Sol 2): a cancelled member with no recorded cause
    /// names no command; a superseded one reads only "cancelled".
    #[test]
    fn a_cancellation_without_a_recorded_cause_names_no_command() {
        use api::identity::Cancellation;
        assert_eq!(cancellation_words(Cancellation::Superseded), "cancelled");
        assert_eq!(cancellation_words(Cancellation::Operator), "cancelled");
    }

    #[test]
    fn the_prompt_names_targets_interruptions_skips_and_hidden_marks() {
        let targets = vec![
            (ProfileIdentity::synthetic("aaa"), "aaa".to_string()),
            (ProfileIdentity::synthetic("bbb"), "bbb".to_string()),
        ];
        let skipped = vec![BulkRow::new(
            "ccc",
            BulkOutcome::skipped("no script running"),
        )];
        let prompt = prompt_text(
            FleetAction::Stop,
            PlanScope::Marked { hidden: 1 },
            None,
            None,
            &targets,
            &["aaa".to_string()],
            &skipped,
        );
        assert_eq!(
            prompt,
            "Stop 2 marked bots (1 not shown): aaa, bbb. This stops the running script of aaa. \
             Skipped 1: ccc: no script running."
        );
        let all = prompt_text(
            FleetAction::Start,
            PlanScope::AllShown,
            None,
            None,
            &targets[..1],
            &[],
            &[],
        );
        assert_eq!(all, "Start each bot's own card on 1 shown bot: aaa.");
    }
}
