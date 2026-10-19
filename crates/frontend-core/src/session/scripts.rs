//! Script rows of the command seam: Start, the script coordinator's
//! catalogue, parameter, reload and settings rows, and paint input.

use api::identity::RequestId;

use super::commands::{Applied, Submitted, Targets};
use crate::command::{CardKind, OperatorCommand, Rejection, SubjectKind};
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
        _targets: Targets<'_>,
        request: RequestId,
        _surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        let applied = || Applied::Rejected(Rejection::NoModel, Some("script rows: in progress".into()));
        if command.subject() == SubjectKind::Application {
            if self.app_admitted(request) {
                self.settle_app(request, command, applied(), submitted);
            }
        } else {
            for name in self.admitted_names(request) {
                self.settle_member(request, command, &name, applied(), submitted);
            }
        }
    }
}
