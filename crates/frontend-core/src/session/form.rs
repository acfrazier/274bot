//! Profile-form and rs2b0t profile-import rows of the command seam.

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{FormClose, OperatorCommand, ProfileDraftField, Rejection, SubjectKind};
use crate::rs2b0t_import::ConflictPolicy;
use crate::surface::SlotSurface;
use crate::OperatorSession;

/// One profile-form or profile-import row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormRow {
    Open,
    OpenNew,
    Edit(ProfileDraftField),
    Discard,
    KeepEditing,
    Close(FormClose),
    ImportPath,
    ImportPreview,
    ImportPolicy(ConflictPolicy),
    ImportIncludeUnusable(bool),
    ImportBack,
    ImportDeletePrompt(bool),
    ImportDeleteExport,
}

impl FormRow {
    fn command(self) -> OperatorCommand {
        match self {
            Self::Open => OperatorCommand::OpenProfileForm,
            Self::OpenNew => OperatorCommand::OpenNewProfileForm,
            Self::Edit(field) => OperatorCommand::EditProfileDraft(field),
            Self::Discard => OperatorCommand::DiscardProfileEdits,
            Self::KeepEditing => OperatorCommand::KeepEditing,
            Self::Close(close) => OperatorCommand::CloseProfileForm(close),
            Self::ImportPath => OperatorCommand::EditProfileImportPath,
            Self::ImportPreview => OperatorCommand::PreviewProfileImport,
            Self::ImportPolicy(policy) => OperatorCommand::SetProfileImportPolicy(policy),
            Self::ImportIncludeUnusable(on) => OperatorCommand::SetProfileImportIncludeUnusable(on),
            Self::ImportBack => OperatorCommand::ReturnToProfileImportPath,
            Self::ImportDeletePrompt(on) => OperatorCommand::SetProfileImportDeletePrompt(on),
            Self::ImportDeleteExport => OperatorCommand::DeleteProfileImportExport,
        }
    }
}

impl<Io> OperatorSession<Io> {
    pub(super) fn form_row<S: SlotSurface<Io = Io>>(
        &mut self,
        row: FormRow,
        request: RequestId,
        _surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        let applied = || Applied::Rejected(Rejection::NoModel, Some("form rows: in progress".into()));
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
