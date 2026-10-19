//! Profile-form and rs2b0t profile-import rows of the command seam.

use api::identity::RequestId;

use super::commands::{Applied, Submitted};
use crate::command::{
    FailureKind, FormClose, OperatorCommand, ProfileDraftField, Rejection, SkipReason, SubjectKind,
};
use crate::profile_editor::{FormLeave, FormOpen};
use crate::rs2b0t_import::{ConflictPolicy, Stage as ImportStage};
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
        surface: &mut S,
        submitted: &mut Submitted,
    ) {
        let command = row.command();
        if command.subject() == SubjectKind::Application {
            if !self.app_admitted(request) {
                return;
            }
            let applied = self.apply_form_row(row, None, surface);
            self.settle_app(request, command, applied, submitted);
        } else {
            for name in self.admitted_names(request) {
                let applied = self.apply_form_row(row, Some(&name), surface);
                self.settle_member(request, command, &name, applied, submitted);
            }
        }
    }

    fn apply_form_row<S: SlotSurface<Io = Io>>(
        &mut self,
        row: FormRow,
        name: Option<&str>,
        surface: &mut S,
    ) -> Applied {
        let mut models = surface.models();
        let no_model = || {
            Applied::Rejected(
                Rejection::NoModel,
                Some("form row requires its frontend model".into()),
            )
        };
        let not_staged = || {
            Applied::Rejected(
                Rejection::NotStaged,
                Some("form row has no staged input".into()),
            )
        };
        let not_applicable = || {
            Applied::Rejected(
                Rejection::NotApplicable,
                Some("form row does not apply in the current state".into()),
            )
        };
        match row {
            FormRow::Open => {
                let Some(name) = name else {
                    return not_applicable();
                };
                if self.profile(name).is_none() {
                    return Applied::Rejected(
                        Rejection::UnknownProfile,
                        Some(format!("profile form: no profile {name}")),
                    );
                }
                if let Some(editor) = models.editor.as_deref_mut() {
                    return match editor.request_open(self, name) {
                        FormOpen::Opened | FormOpen::Kept => Applied::Done,
                        FormOpen::Staged => Applied::Skipped(SkipReason::DecisionStaged),
                    };
                }
                if let Some(save) = models.form_save.as_deref_mut() {
                    save.form_changed();
                    return Applied::Done;
                }
                no_model()
            }
            FormRow::OpenNew => {
                if let Some(editor) = models.editor.as_deref_mut() {
                    return match editor.request_open(self, "") {
                        FormOpen::Opened | FormOpen::Kept => Applied::Done,
                        FormOpen::Staged => Applied::Skipped(SkipReason::DecisionStaged),
                    };
                }
                if let Some(save) = models.form_save.as_deref_mut() {
                    save.form_changed();
                    return Applied::Done;
                }
                no_model()
            }
            FormRow::Edit(_) => {
                if let Some(editor) = models.editor.as_deref_mut() {
                    if !editor.is_open() {
                        return not_applicable();
                    }
                    editor.edited();
                    return Applied::Done;
                }
                if let Some(save) = models.form_save.as_deref_mut() {
                    save.edited();
                    return Applied::Done;
                }
                no_model()
            }
            FormRow::Discard => {
                let Some(editor) = models.editor.as_deref_mut() else {
                    return no_model();
                };
                if editor.discard(self).is_some() {
                    Applied::Done
                } else {
                    not_staged()
                }
            }
            FormRow::KeepEditing => {
                let Some(editor) = models.editor.as_deref_mut() else {
                    return no_model();
                };
                if editor.pending().is_none() {
                    return not_staged();
                }
                editor.keep_editing();
                Applied::Done
            }
            FormRow::Close(close) => {
                if let Some(editor) = models.editor.as_deref_mut() {
                    if !editor.is_open() {
                        return not_applicable();
                    }
                    let leave = match close {
                        FormClose::Form => FormLeave::CloseForm,
                        FormClose::Window => FormLeave::CloseWindow,
                    };
                    return if editor.request_close(self, leave) {
                        Applied::Done
                    } else {
                        Applied::Skipped(SkipReason::DecisionStaged)
                    };
                }
                if let Some(save) = models.form_save.as_deref_mut() {
                    save.form_changed();
                    return Applied::Done;
                }
                no_model()
            }
            FormRow::ImportPath => {
                if models.import.is_none() {
                    return no_model();
                }
                if let Some(editor) = models.editor.as_deref_mut() {
                    if editor.is_open() {
                        if editor.dirty(self) || editor.saving(self) {
                            editor.stage_leave(
                                self,
                                FormLeave::OpenImport,
                                "",
                                "import rs2b0t profiles",
                            );
                            return Applied::Skipped(SkipReason::DecisionStaged);
                        }
                        editor.close();
                    }
                }
                Applied::Done
            }
            FormRow::ImportPreview => {
                let Some(scripts) = models.scripts.as_deref() else {
                    return no_model();
                };
                let Some(import) = models.import.as_deref_mut() else {
                    return no_model();
                };
                if import.stage() != ImportStage::Path {
                    return not_applicable();
                }
                match import.preview(self, scripts) {
                    Ok(()) => Applied::Done,
                    Err(error) => {
                        Applied::Rejected(Rejection::NotApplicable, Some(error.to_string()))
                    }
                }
            }
            FormRow::ImportPolicy(policy) => {
                let Some(scripts) = models.scripts.as_deref() else {
                    return no_model();
                };
                let Some(import) = models.import.as_deref_mut() else {
                    return no_model();
                };
                if import.stage() != ImportStage::Preview {
                    return not_applicable();
                }
                import.set_policy(self, scripts, policy);
                Applied::Done
            }
            FormRow::ImportIncludeUnusable(include) => {
                let Some(scripts) = models.scripts.as_deref() else {
                    return no_model();
                };
                let Some(import) = models.import.as_deref_mut() else {
                    return no_model();
                };
                if import.stage() != ImportStage::Preview {
                    return not_applicable();
                }
                import.set_include_unusable(self, scripts, include);
                Applied::Done
            }
            FormRow::ImportBack => {
                let Some(import) = models.import.as_deref_mut() else {
                    return no_model();
                };
                if import.stage() != ImportStage::Preview {
                    return not_applicable();
                }
                import.back();
                Applied::Done
            }
            FormRow::ImportDeletePrompt(on) => {
                let Some(import) = models.import.as_deref_mut() else {
                    return no_model();
                };
                if on {
                    if !import.can_delete(self) {
                        return not_applicable();
                    }
                    import.ask_delete();
                } else {
                    import.cancel_delete();
                }
                Applied::Done
            }
            FormRow::ImportDeleteExport => {
                let Some(import) = models.import.as_deref_mut() else {
                    return no_model();
                };
                if !import.delete_asked() {
                    return not_staged();
                }
                if !import.can_delete(self) {
                    return not_applicable();
                }
                match import.confirm_delete(self) {
                    Ok(()) => Applied::Done,
                    Err(error) => {
                        Applied::Failed(FailureKind::WriteFailed, Some(error.to_string()))
                    }
                }
            }
        }
    }
}
