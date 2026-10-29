//! Script rows of the command seam: Start, the script coordinator's
//! catalogue, parameter, reload and settings rows, and paint input.

use std::path::PathBuf;

use api::identity::RequestId;

use super::commands::{Applied, Submitted, Targets};
#[cfg(feature = "js")]
use crate::command::FailureKind;
use crate::command::{CardKind, OperatorCommand, Rejection, SkipReason, Subject, SubjectKind};
#[cfg(feature = "js")]
use crate::scripts::CatalogActionKind;
use crate::scripts::{
    AssignmentCommandResult, PaintAction, ParameterCommit, ParameterEditKey,
    PendingApplicationWait, ScriptChoice, Scripts, StagedSetting, SyncCommandInput,
};
use crate::selection::MarkedSelection;
use crate::surface::SlotSurface;
use crate::OperatorSession;

/// One script row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptRow {
    Start,
    CancelQueuedStart,
    PickBrowseCard,
    SelectScriptCard,
    PrepareScriptCard,
    Assign,
    AssignAndRestart,
    SaveSettings(CardKind),
    SaveLegacySettings,
    EditParameterText,
    DiscardParameterText,
    ApplySettingsToMarked(CardKind),
    CancelSettingsSync,
    RestartForSettings,
    DeferSettingsRestart,
    LoadScriptFile,
    Reload,
    ConfirmReload,
    CancelReload,
    ImportCatalog,
    TranspileAll,
    DeferCatalogImport,
    PaintInput,
}

impl ScriptRow {
    fn command(self) -> OperatorCommand {
        match self {
            Self::Start => OperatorCommand::StartScript,
            Self::CancelQueuedStart => OperatorCommand::CancelQueuedStart,
            Self::PickBrowseCard => OperatorCommand::PickBrowseCard,
            Self::SelectScriptCard => OperatorCommand::SelectScriptCard,
            Self::PrepareScriptCard => OperatorCommand::PrepareScriptCard,
            Self::Assign => OperatorCommand::AssignScript,
            Self::AssignAndRestart => OperatorCommand::AssignAndRestart,
            Self::SaveSettings(kind) => OperatorCommand::SaveScriptSettings(kind),
            Self::SaveLegacySettings => OperatorCommand::SaveLegacyScriptSettings,
            Self::EditParameterText => OperatorCommand::EditParameterText,
            Self::DiscardParameterText => OperatorCommand::DiscardParameterText,
            Self::ApplySettingsToMarked(kind) => OperatorCommand::ApplySettingsToMarked(kind),
            Self::CancelSettingsSync => OperatorCommand::CancelSettingsSync,
            Self::RestartForSettings => OperatorCommand::RestartForSettings,
            Self::DeferSettingsRestart => OperatorCommand::DeferSettingsRestart,
            Self::LoadScriptFile => OperatorCommand::LoadScriptFile,
            Self::Reload => OperatorCommand::ReloadScripts,
            Self::ConfirmReload => OperatorCommand::ConfirmReload,
            Self::CancelReload => OperatorCommand::CancelReload,
            Self::ImportCatalog => OperatorCommand::ImportCatalog,
            Self::TranspileAll => OperatorCommand::TranspileAll,
            Self::DeferCatalogImport => OperatorCommand::DeferCatalogImport,
            Self::PaintInput => OperatorCommand::ScriptPaintInput,
        }
    }
}

impl<Io> OperatorSession<Io> {
    pub(super) fn script_row<S: SlotSurface<Io = Io>>(
        &mut self,
        row: ScriptRow,
        targets: Targets<'_>,
        request: RequestId,
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        if command.subject() == SubjectKind::Application {
            if !self.app_admitted(request) {
                return;
            }
            let mut models = surface.models();
            let applied = match models.scripts.as_deref_mut() {
                Some(scripts) => dispatch_application(self, scripts, row, request),
                None => Applied::Rejected(Rejection::NoModel, None),
            };
            self.settle_app(request, command, applied, submitted);
            return;
        }

        let names = self.admitted_names(request);
        // A planned command runs over its plan's frozen marks and card.
        let planned_marks = self.planned_marks();
        let planned_card = self.planned_card();
        let mut models = surface.models();
        let marks = match planned_marks.as_ref() {
            Some(marks) => Some(marks),
            None => models.fleet.as_deref().map(crate::FleetModel::marks),
        };
        let Some(scripts) = models.scripts.as_deref_mut() else {
            for name in names {
                self.settle_member(
                    request,
                    command,
                    &name,
                    Applied::Rejected(Rejection::NoModel, None),
                    submitted,
                );
            }
            return;
        };

        if let ScriptRow::ApplySettingsToMarked(kind) = row {
            if planned_marks.is_some() {
                // A planned copy applies the scope its plan prepared.
                scripts.stage_settings_sync_apply();
            }
            for (name, applied) in
                dispatch_settings_sync(self, scripts, kind, targets, &names, marks)
            {
                self.settle_member(request, command, &name, applied, submitted);
            }
            return;
        }

        if row == ScriptRow::Start && matches!(targets, Targets::Everyone | Targets::Marked) {
            let results =
                dispatch_start_group(self, scripts, targets, request, &names, marks, planned_card);
            let marked_target = matches!(targets, Targets::Marked);
            let mut bulk_rows = Vec::new();
            for (name, applied) in results {
                if marked_target {
                    bulk_rows.push(crate::BulkRow::new(name.clone(), bulk_outcome(&applied)));
                }
                self.settle_member(request, command, &name, applied, submitted);
            }
            if marked_target {
                submitted.set_bulk(crate::BulkReport::new(
                    "Start marked",
                    "started",
                    bulk_rows,
                    None,
                ));
            }
            return;
        }

        let marked_target = matches!(targets, Targets::Marked);
        let start_root = (row == ScriptRow::Start)
            .then(|| scripts.take_staged_start_catalog_root())
            .flatten();
        let cancel_reason = if row == ScriptRow::CancelQueuedStart {
            scripts.take_staged_cancel_reason()
        } else {
            None
        };
        let assignment = marked_assignment_label(row).map(|_| {
            (
                match &planned_card {
                    Some(card) => card.as_ref().map(ScriptChoice::duplicate),
                    None => scripts.browse_selection().map(ScriptChoice::duplicate),
                },
                scripts.take_staged_start_catalog_root(),
            )
        });
        let mut bulk_rows = Vec::new();
        for name in names {
            let saving = planned_marks.is_some() && self.profile_write_pending(&name);
            let applied = if saving {
                Applied::Skipped(SkipReason::ProfileSaving)
            } else if let Some((selection, root)) = &assignment {
                match selection {
                    Some(selection) => assign_profile(
                        self,
                        scripts,
                        request,
                        &name,
                        selection.duplicate(),
                        root.clone(),
                        row == ScriptRow::AssignAndRestart,
                    ),
                    None => Applied::Rejected(Rejection::NotStaged, None),
                }
            } else {
                dispatch_profile(
                    self,
                    scripts,
                    row,
                    request,
                    &name,
                    start_root.as_deref(),
                    cancel_reason,
                )
            };
            if marked_target && marked_assignment_label(row).is_some() {
                bulk_rows.push(crate::BulkRow::new(name.clone(), bulk_outcome(&applied)));
            }
            self.settle_member(request, command, &name, applied, submitted);
        }
        if marked_target {
            if let Some((label, done)) = marked_assignment_label(row) {
                submitted.set_bulk(crate::BulkReport::new(label, done, bulk_rows, None));
            }
        }
    }
}
fn dispatch_application<Io>(
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    row: ScriptRow,
    request: RequestId,
) -> Applied {
    match row {
        ScriptRow::PickBrowseCard => match scripts.take_staged_browse_pick() {
            Some(selection) => {
                scripts.pick_browse_card(selection);
                Applied::Done
            }
            None => Applied::Rejected(Rejection::NotStaged, None),
        },
        #[cfg(feature = "js")]
        ScriptRow::PrepareScriptCard => {
            let Some((source, lookup)) = scripts.take_staged_transpile_target() else {
                return Applied::Rejected(Rejection::NotStaged, None);
            };
            match scripts.queue_transpile_command(request, source, &lookup) {
                Ok(true) => Applied::Later,
                Ok(false) => Applied::Done,
                Err(error) => Applied::Rejected(Rejection::NotApplicable, Some(error)),
            }
        }
        #[cfg(feature = "js")]
        ScriptRow::SaveLegacySettings => {
            if let Some(staged) = scripts.take_staged_setting() {
                return commit_staged_setting(core, scripts, staged, None, None);
            }
            commit_parameter_validation(core, scripts, None, None, false)
        }
        ScriptRow::EditParameterText => {
            commit_parameter_validation(core, scripts, None, None, true)
        }
        ScriptRow::DiscardParameterText => match scripts.take_staged_parameter_discard() {
            Some(staged) => {
                match staged {
                    crate::scripts::StagedParameterDiscard::Field(key) => {
                        scripts.discard_parameter_text(&key);
                    }
                    crate::scripts::StagedParameterDiscard::Profile(profile) => {
                        scripts.discard_profile_parameter_text(&profile);
                    }
                }
                Applied::Done
            }
            None => Applied::Rejected(Rejection::NotStaged, None),
        },
        ScriptRow::CancelSettingsSync => {
            scripts.cancel_settings_sync();
            Applied::Done
        }
        ScriptRow::RestartForSettings => {
            let root = scripts.take_staged_start_catalog_root();
            match scripts.restart_pending_settings(core, root.as_deref()) {
                Ok(()) => {
                    scripts.await_application_command(
                        request,
                        PendingApplicationWait::RestartSettings,
                    );
                    Applied::Later
                }
                Err(error) => Applied::Rejected(Rejection::NotApplicable, Some(error)),
            }
        }
        ScriptRow::DeferSettingsRestart => {
            scripts.dismiss_restart_prompt();
            Applied::Done
        }
        #[cfg(feature = "js")]
        ScriptRow::LoadScriptFile => {
            let Some(path) = scripts.take_staged_file_path() else {
                return Applied::Rejected(Rejection::NotStaged, None);
            };
            let loaded = scripts.load_file(&path).map(|card| card.choice());
            match loaded {
                Ok(selection) => {
                    scripts.pick_browse_card(Some(selection));
                    Applied::Done
                }
                Err(error) => Applied::Rejected(Rejection::NotApplicable, Some(error)),
            }
        }
        #[cfg(feature = "js")]
        ScriptRow::Reload | ScriptRow::ConfirmReload => {
            let target = scripts.take_staged_reload_target();
            scripts.begin_reload(core, target);
            settle_reload_command(scripts, request)
        }
        #[cfg(feature = "js")]
        ScriptRow::CancelReload => {
            scripts.cancel_reload();
            Applied::Done
        }
        #[cfg(feature = "js")]
        ScriptRow::ImportCatalog => {
            let Some(action) = scripts.take_staged_catalog_action() else {
                return Applied::Rejected(Rejection::NotStaged, None);
            };
            let Some(root) = action.root.as_deref() else {
                if action.kind == CatalogActionKind::Fill {
                    return Applied::Skipped(SkipReason::NotReady);
                }
                return Applied::Rejected(
                    Rejection::NotApplicable,
                    Some("catalog root is unavailable".into()),
                );
            };
            match action.kind {
                CatalogActionKind::Fill => {
                    match scripts.request_catalog_fill(root, &action.path_file) {
                        Ok(()) if scripts.catalog_work_pending() => {
                            scripts.await_application_command(
                                request,
                                PendingApplicationWait::CatalogFill,
                            );
                            Applied::Later
                        }
                        Ok(()) => Applied::Done,
                        Err(error) => {
                            Applied::Rejected(Rejection::NotApplicable, Some(error.to_string()))
                        }
                    }
                }
                CatalogActionKind::Import => {
                    match scripts.request_catalog_import(root, &action.path_file) {
                        Ok(()) if scripts.catalog_work_pending() => {
                            scripts.await_application_command(
                                request,
                                PendingApplicationWait::CatalogImport,
                            );
                            Applied::Later
                        }
                        Ok(()) => Applied::Done,
                        Err(error) => {
                            Applied::Rejected(Rejection::NotApplicable, Some(error.to_string()))
                        }
                    }
                }
                CatalogActionKind::Refresh => {
                    scripts.begin_refresh_catalog(core, root);
                    settle_reload_command(scripts, request)
                }
            }
        }
        #[cfg(feature = "js")]
        ScriptRow::TranspileAll => match scripts.queue_transpile_all_command(request) {
            Ok(true) => Applied::Later,
            Ok(false) => Applied::Done,
            Err(error) => Applied::Rejected(Rejection::NotApplicable, Some(error)),
        },
        #[cfg(feature = "js")]
        ScriptRow::DeferCatalogImport => {
            let Some(path) = scripts.take_staged_defer_catalog() else {
                return Applied::Rejected(Rejection::NotStaged, None);
            };
            match crate::scripts::paths::set_rs2b0t_import_deferred_at(&path) {
                Ok(()) => Applied::Done,
                Err(error) => Applied::Rejected(Rejection::NotApplicable, Some(error.to_string())),
            }
        }
        #[cfg(not(feature = "js"))]
        ScriptRow::PrepareScriptCard
        | ScriptRow::SaveLegacySettings
        | ScriptRow::LoadScriptFile
        | ScriptRow::Reload
        | ScriptRow::ConfirmReload
        | ScriptRow::CancelReload
        | ScriptRow::ImportCatalog
        | ScriptRow::TranspileAll
        | ScriptRow::DeferCatalogImport => Applied::Rejected(
            Rejection::NotApplicable,
            Some(
                script::surface::SurfaceError::NoFactory(script::surface::SurfaceKind::Js)
                    .to_string(),
            ),
        ),
        ScriptRow::Start
        | ScriptRow::CancelQueuedStart
        | ScriptRow::SelectScriptCard
        | ScriptRow::Assign
        | ScriptRow::AssignAndRestart
        | ScriptRow::SaveSettings(_)
        | ScriptRow::ApplySettingsToMarked(_)
        | ScriptRow::PaintInput => Applied::Rejected(Rejection::NotApplicable, None),
    }
}

#[cfg(feature = "js")]
fn settle_reload_command(scripts: &mut Scripts, request: RequestId) -> Applied {
    if scripts.reload_validation_pending() {
        scripts.await_application_command(request, PendingApplicationWait::Reload);
        return Applied::Later;
    }
    match scripts
        .reload_outcome()
        .and_then(|outcome| outcome.failure())
    {
        Some(error) => Applied::Failed(FailureKind::JobFailed, Some(error.to_owned())),
        None => Applied::Done,
    }
}

fn commit_staged_setting<Io>(
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    staged: StagedSetting,
    profile: Option<&str>,
    kind: Option<CardKind>,
) -> Applied {
    if staged.profile.as_deref() != profile {
        return Applied::Rejected(Rejection::NotApplicable, None);
    }
    let native = staged.selection.compiled_id();
    if kind.is_some_and(|kind| (kind == CardKind::Native) != native.is_some()) {
        return Applied::Rejected(Rejection::NotApplicable, None);
    }
    let key = ParameterEditKey::new(profile, &staged.selection, &staged.field);
    let current = scripts.durable_setting_value(core, profile, &staged.selection, &staged.field);
    let current_text = current
        .as_ref()
        .map(script::format_setting_value)
        .unwrap_or_default();
    *scripts.parameter_text_mut(&key, &current_text, current.clone()) =
        script::format_setting_value(&staged.value);
    match scripts.commit_parameter_value(
        core,
        &key,
        &staged.selection,
        &staged.field,
        staged.value,
        current,
    ) {
        Ok(ParameterCommit::Submitted(operation) | ParameterCommit::Pending(operation)) => {
            Applied::Operation(operation)
        }
        Ok(ParameterCommit::Unchanged | ParameterCommit::Saved) => Applied::Done,
        Err(error) => Applied::Rejected(Rejection::NotApplicable, Some(error)),
    }
}

fn commit_parameter_validation<Io>(
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    profile: Option<&str>,
    kind: Option<CardKind>,
    validate_only: bool,
) -> Applied {
    let Some(staged) = scripts.take_staged_parameter_validation() else {
        return Applied::Rejected(Rejection::NotStaged, None);
    };
    let validated = scripts.validate_staged_parameter_validation(staged);
    let (key, selection, field, value, current) = match validated {
        Ok(validated) => validated,
        Err(error) => return Applied::Rejected(Rejection::NotApplicable, Some(error)),
    };
    if validate_only {
        return Applied::Done;
    }
    if key != ParameterEditKey::new(profile, &selection, &field)
        || kind.is_some_and(|kind| (kind == CardKind::Native) != selection.compiled_id().is_some())
    {
        return Applied::Rejected(Rejection::NotApplicable, None);
    }
    match scripts.commit_parameter_value(core, &key, &selection, &field, value, current) {
        Ok(ParameterCommit::Submitted(operation) | ParameterCommit::Pending(operation)) => {
            Applied::Operation(operation)
        }
        Ok(ParameterCommit::Unchanged | ParameterCommit::Saved) => Applied::Done,
        Err(error) => Applied::Rejected(Rejection::NotApplicable, Some(error)),
    }
}

fn dispatch_profile<Io>(
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    row: ScriptRow,
    request: RequestId,
    profile: &str,
    start_root: Option<&std::path::Path>,
    cancel_reason: Option<&str>,
) -> Applied {
    match row {
        ScriptRow::Start => start_profile(core, scripts, request, profile, start_root),
        ScriptRow::CancelQueuedStart => {
            let cancelled = scripts.cancel_queued_as(profile, cancel_reason.unwrap_or("cancelled"));
            scripts.publish_start_places(core);
            if cancelled {
                Applied::Done
            } else {
                Applied::Skipped(SkipReason::NotReady)
            }
        }
        ScriptRow::SelectScriptCard => match scripts.browse_selection() {
            Some(selection) => {
                scripts.set_pending_browse(profile, selection.duplicate());
                Applied::Done
            }
            None => Applied::Rejected(Rejection::NotStaged, None),
        },
        ScriptRow::Assign | ScriptRow::AssignAndRestart => {
            Applied::Rejected(Rejection::NotStaged, None)
        }
        ScriptRow::SaveSettings(kind) => {
            if let Some(staged) = scripts.take_staged_setting() {
                commit_staged_setting(core, scripts, staged, Some(profile), Some(kind))
            } else {
                commit_parameter_validation(core, scripts, Some(profile), Some(kind), false)
            }
        }
        ScriptRow::PaintInput => {
            let Some(input) = scripts.take_staged_paint_input() else {
                return Applied::Rejected(Rejection::NotStaged, None);
            };
            if core.play().is_none() {
                return Applied::Rejected(Rejection::NoHost, None);
            }
            match input.0 {
                // A host control id is never a script click: it reaches the
                // run only as its own lifecycle row.
                PaintAction::Click { id, .. } if crate::scripts::is_host_control(&id) => {
                    return Applied::Rejected(
                        Rejection::NotApplicable,
                        Some("a host paint control is not a script click".into()),
                    );
                }
                PaintAction::Click { id, generation } => {
                    core.script_paint_click(profile, &id, generation)
                }
                PaintAction::Select {
                    key,
                    name,
                    generation,
                } => core.script_paint_select(profile, &key, &name, generation),
                // The seam routes a holding binding to its own row before
                // dispatch; one that reaches this row was refused.
                PaintAction::Refused(reason) => {
                    return Applied::Rejected(Rejection::NotApplicable, Some(reason.into()));
                }
                PaintAction::Control(_) => {
                    return Applied::Rejected(
                        Rejection::NotApplicable,
                        Some("the paint control names no single profile".into()),
                    );
                }
            }
            Applied::Done
        }
        ScriptRow::PickBrowseCard
        | ScriptRow::PrepareScriptCard
        | ScriptRow::SaveLegacySettings
        | ScriptRow::EditParameterText
        | ScriptRow::DiscardParameterText
        | ScriptRow::ApplySettingsToMarked(_)
        | ScriptRow::CancelSettingsSync
        | ScriptRow::RestartForSettings
        | ScriptRow::DeferSettingsRestart
        | ScriptRow::LoadScriptFile
        | ScriptRow::Reload
        | ScriptRow::ConfirmReload
        | ScriptRow::CancelReload
        | ScriptRow::ImportCatalog
        | ScriptRow::TranspileAll
        | ScriptRow::DeferCatalogImport => Applied::Rejected(Rejection::NotApplicable, None),
    }
}

fn start_profile<Io>(
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    request: RequestId,
    profile: &str,
    catalog_root: Option<&std::path::Path>,
) -> Applied {
    let waiting = scripts.start_queue_place(profile).is_some();
    #[cfg(feature = "js")]
    let waiting = waiting || scripts.has_start_preparation(profile);
    if waiting {
        register_start_command(core, scripts, request, profile);
        return Applied::Later;
    }
    match scripts.start_selected(core, profile, None, catalog_root) {
        Ok(Some(operation)) => Applied::Operation(operation),
        Ok(None) => {
            let waiting = scripts.start_queue_place(profile).is_some();
            #[cfg(feature = "js")]
            let waiting = waiting || scripts.has_start_preparation(profile);
            if waiting {
                register_start_command(core, scripts, request, profile);
                Applied::Later
            } else {
                Applied::Done
            }
        }
        Err(error) => Applied::Rejected(Rejection::StartRefused, Some(error.to_string())),
    }
}

fn register_start_command<Io>(
    core: &OperatorSession<Io>,
    scripts: &mut Scripts,
    request: RequestId,
    profile: &str,
) {
    scripts.register_start_command(
        profile.to_owned(),
        request,
        Subject::Profile(core.profile_identity_or_synthetic(profile)),
    );
}

fn start_group_member<Io>(
    core: &OperatorSession<Io>,
    scripts: &mut Scripts,
    request: RequestId,
    profile: &str,
    accepted: bool,
) -> Applied {
    if accepted {
        register_start_command(core, scripts, request, profile);
        return Applied::Later;
    }
    if core.play().is_none() {
        return Applied::Skipped(SkipReason::NotReady);
    }
    match core
        .play()
        .map_or(script::RunState::Idle, |play| play.script_state(profile))
    {
        script::RunState::Starting
        | script::RunState::Running
        | script::RunState::Paused
        | script::RunState::Stopping => Applied::Skipped(SkipReason::AlreadyActive),
        script::RunState::Idle | script::RunState::Error => Applied::Rejected(
            Rejection::StartRefused,
            Some("Start was not admitted".into()),
        ),
    }
}

fn dispatch_start_group<Io>(
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    targets: Targets<'_>,
    request: RequestId,
    names: &[String],
    marks: Option<&MarkedSelection>,
    planned_card: Option<Option<ScriptChoice>>,
) -> Vec<(String, Applied)> {
    let root = scripts.take_staged_start_catalog_root();
    let accepted = match targets {
        Targets::Everyone => scripts.start_all(core, root.as_deref()),
        Targets::Marked => {
            let Some(marks) = marks else {
                return names
                    .iter()
                    .map(|name| (name.clone(), Applied::Rejected(Rejection::NoModel, None)))
                    .collect();
            };
            let selection = match planned_card {
                Some(card) => card,
                None => scripts.browse_selection().map(ScriptChoice::duplicate),
            };
            crate::selection::start_marked(
                marks,
                core,
                scripts,
                selection.as_ref(),
                root.as_deref(),
            )
        }
        Targets::Application
        | Targets::Profile(_)
        | Targets::Profiles(_)
        | Targets::Selected
        | Targets::Plan => {
            return names
                .iter()
                .map(|name| {
                    (
                        name.clone(),
                        Applied::Rejected(Rejection::NotApplicable, None),
                    )
                })
                .collect()
        }
    };
    names
        .iter()
        .map(|name| {
            (
                name.clone(),
                start_group_member(core, scripts, request, name, accepted.contains(name)),
            )
        })
        .collect()
}

fn assign_profile<Io>(
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    request: RequestId,
    profile: &str,
    selection: ScriptChoice,
    catalog_root: Option<PathBuf>,
    restart: bool,
) -> Applied {
    let subject = Subject::Profile(core.profile_identity_or_synthetic(profile));
    match scripts.dispatch_assignment_command(
        core,
        crate::scripts::PendingAssignmentWrite {
            request,
            subject,
            profile: profile.to_owned(),
            selection,
            catalog_root,
            restart,
        },
    ) {
        AssignmentCommandResult::Done => Applied::Done,
        AssignmentCommandResult::Operation(operation) => Applied::Operation(operation),
        AssignmentCommandResult::Later => Applied::Later,
        AssignmentCommandResult::Skipped(reason) => Applied::Skipped(reason),
        AssignmentCommandResult::Rejected(error) => {
            Applied::Rejected(Rejection::NotApplicable, Some(error))
        }
    }
}

fn dispatch_settings_sync<Io>(
    core: &mut OperatorSession<Io>,
    scripts: &mut Scripts,
    kind: CardKind,
    targets: Targets<'_>,
    names: &[String],
    marks: Option<&MarkedSelection>,
) -> Vec<(String, Applied)> {
    let Some(staged) = scripts.take_staged_sync_command() else {
        return broadcast_script_result(names, Applied::Rejected(Rejection::NotStaged, None));
    };
    if (kind == CardKind::Native) != staged.selection.compiled_id().is_some() {
        return broadcast_script_result(names, Applied::Rejected(Rejection::NotApplicable, None));
    }
    match staged.command {
        SyncCommandInput::Prepare if matches!(targets, Targets::Marked) => {
            let Some(marks) = marks else {
                return broadcast_script_result(names, Applied::Rejected(Rejection::NoModel, None));
            };
            let applied = match crate::marked::prepare_apply_settings_marked(
                marks,
                core,
                scripts,
                &staged.source,
                &staged.selection,
            ) {
                Ok(_) => Applied::Done,
                Err(error) => Applied::Rejected(Rejection::NotApplicable, Some(error)),
            };
            broadcast_script_result(names, applied)
        }
        SyncCommandInput::Prepare => {
            let applied = if let Some(id) = staged.selection.compiled_id() {
                match scripts.prepare_compiled_settings_sync(core, &staged.source, id, None) {
                    Ok(_) => Applied::Done,
                    Err(error) => Applied::Rejected(Rejection::NotApplicable, Some(error)),
                }
            } else {
                #[cfg(feature = "js")]
                {
                    if let Some((source, lookup)) = staged.selection.loaded_card() {
                        match scripts.card(source, lookup) {
                            Some(card) => {
                                let name = card.name().to_owned();
                                let path = card.path().to_path_buf();
                                scripts.prepare_settings_sync(
                                    core,
                                    &staged.source,
                                    source,
                                    &name,
                                    &path,
                                );
                                Applied::Done
                            }
                            None => Applied::Rejected(
                                Rejection::NotApplicable,
                                Some("script card is unavailable".into()),
                            ),
                        }
                    } else {
                        Applied::Rejected(Rejection::NotApplicable, None)
                    }
                }
                #[cfg(not(feature = "js"))]
                {
                    Applied::Rejected(
                        Rejection::NotApplicable,
                        Some(
                            script::surface::SurfaceError::NoFactory(
                                script::surface::SurfaceKind::Js,
                            )
                            .to_string(),
                        ),
                    )
                }
            };
            broadcast_script_result(names, applied)
        }
        SyncCommandInput::Apply => {
            let source = scripts
                .prepared_settings_sync()
                .map(|scope| scope.source().to_owned());
            match scripts.apply_settings_sync(core) {
                // A member settles from the copy's own outcome for it, under
                // the name the copy wrote (a planned copy follows a rename).
                Ok(operation) => names
                    .iter()
                    .map(|name| {
                        let copied = core
                            .operations
                            .get(operation)
                            .is_some_and(|report| report.outcome(name).is_some());
                        let applied = if source.as_deref() == Some(name.as_str()) {
                            Applied::Done
                        } else if copied {
                            Applied::Operation(operation)
                        } else {
                            Applied::Skipped(SkipReason::NotReady)
                        };
                        (name.clone(), applied)
                    })
                    .collect(),
                Err(error) => broadcast_script_result(
                    names,
                    Applied::Rejected(Rejection::NotApplicable, Some(error)),
                ),
            }
        }
    }
}

fn broadcast_script_result(names: &[String], applied: Applied) -> Vec<(String, Applied)> {
    names
        .iter()
        .map(|name| (name.clone(), duplicate_applied(&applied)))
        .collect()
}

fn duplicate_applied(applied: &Applied) -> Applied {
    match applied {
        Applied::Done => Applied::Done,
        #[cfg(feature = "test-support")]
        Applied::DoneWithDetail(detail) => Applied::DoneWithDetail(detail.clone()),
        Applied::Skipped(reason) => Applied::Skipped(*reason),
        Applied::Rejected(reason, detail) => Applied::Rejected(*reason, detail.clone()),
        Applied::Failed(reason, detail) => Applied::Failed(*reason, detail.clone()),
        Applied::Operation(operation) => Applied::Operation(*operation),
        Applied::Later => Applied::Later,
        Applied::NotAdmitted => Applied::NotAdmitted,
    }
}

fn bulk_outcome(applied: &Applied) -> crate::BulkOutcome {
    match applied {
        Applied::Done => crate::BulkOutcome::Done,
        #[cfg(feature = "test-support")]
        Applied::DoneWithDetail(_) => crate::BulkOutcome::Done,
        Applied::Skipped(reason) => crate::BulkOutcome::skipped(skip_label(*reason)),
        Applied::Rejected(_, detail) | Applied::Failed(_, detail) => {
            crate::BulkOutcome::failed(detail.as_deref().unwrap_or("command failed"))
        }
        Applied::Operation(_) | Applied::Later => crate::BulkOutcome::Pending,
        Applied::NotAdmitted => crate::BulkOutcome::skipped("not admitted"),
    }
}

fn skip_label(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::NoScript => "no script",
        SkipReason::NoRunningScript => "no running script",
        SkipReason::AlreadyActive => "already active",
        SkipReason::AlreadyStopping => "already stopping",
        SkipReason::AlreadyLoggedIn => "already logged in",
        SkipReason::AlreadyLoggingIn => "already logging in",
        SkipReason::NotLoggedIn => "not logged in",
        SkipReason::NotLoaded => "not loaded",
        SkipReason::NotReady => "not ready",
        SkipReason::ProfileUnavailable => "profile unavailable",
        SkipReason::ProfileSaving => "profile still saving",
        SkipReason::ScriptActive => "script active",
        SkipReason::WalkExcluded => "walk excluded",
        SkipReason::NothingSelected => "nothing selected",
        SkipReason::DecisionStaged => "decision staged",
        SkipReason::AlreadyOnWorld => "already on this world",
    }
}

fn marked_assignment_label(row: ScriptRow) -> Option<(&'static str, &'static str)> {
    match row {
        ScriptRow::Assign => Some(("Assign marked", "assigned")),
        ScriptRow::AssignAndRestart => Some(("Assign & restart marked", "restarted")),
        _ => None,
    }
}
