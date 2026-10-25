//! Profiles window over the core's profile edit form
//! ([`frontend_core::ProfileEditor`]): the panel adds its own effects
//! (the window open, the focus request, the rs2b0t import sharing the editor
//! pane, MultiBox off) around the form's staging and saves.

use frontend_core::command::{FormClose, ProfileDraftField};
use frontend_core::{FormLeave, OperatorCommand, Submitted, Targets};

use super::Session;

impl Session {
    fn command_done(&self, submitted: &Submitted) -> bool {
        self.core
            .commands()
            .report(submitted.request())
            .and_then(|report| report.members().first())
            .and_then(|member| member.terminal())
            .is_some_and(|terminal| {
                matches!(
                    terminal,
                    api::identity::Settlement::Completed(frontend_core::command::Completion::Done)
                )
            })
    }

    fn command_staged(&self, submitted: &Submitted) -> bool {
        self.core
            .commands()
            .report(submitted.request())
            .and_then(|report| report.members().first())
            .and_then(|member| member.terminal())
            .is_some_and(|terminal| {
                matches!(
                    terminal,
                    api::identity::Settlement::Completed(
                        frontend_core::command::Completion::Skipped(
                            frontend_core::command::SkipReason::DecisionStaged
                        )
                    )
                )
            })
    }
    /// Save the credentials form through its profile-writer command.
    pub fn save_credentials(&mut self) -> bool {
        let (command, target) = match self.profile_editor.target() {
            Some("") | None => (OperatorCommand::CreateProfile, None),
            Some(name) => (OperatorCommand::SaveProfile, Some(name.to_owned())),
        };
        let targets = target
            .as_deref()
            .map_or(Targets::Application, Targets::Profile);
        let Some(submitted) = self.submit(command, targets) else {
            return false;
        };
        if submitted.operation().is_some() || self.command_done(&submitted) {
            self.error = None;
            true
        } else {
            if let Some(error) = self.core.command_failure(submitted.request()) {
                self.error = Some(error.to_owned());
            }
            false
        }
    }

    /// Settle durable credential saves and follow their saved names. Automatic
    /// selection waits for preference adoption through the startup gate.
    /// A failed write keeps its form and banner error and selects nothing.
    pub(crate) fn settle_profile_save(&mut self) {
        for name in self.profile_editor.settle(&mut self.core) {
            self.focus_when_ready(super::StartupFocus::SavedProfile(name));
        }
    }

    /// Empty the staged credentials fields, then notify the form row.
    pub fn clear_credentials(&mut self) {
        self.profile_editor.username_mut().clear();
        self.profile_editor.password_mut().clear();
        let _ = self.submit(
            OperatorCommand::EditProfileDraft(ProfileDraftField::Username),
            Targets::Application,
        );
        let _ = self.submit(
            OperatorCommand::EditProfileDraft(ProfileDraftField::Password),
            Targets::Application,
        );
    }

    /// Open the profile picker on `name` (or a blank new row). Any dirty
    /// form waits behind the Discard / Keep editing prompt.
    pub fn begin_edit_profile(&mut self, name: Option<&str>) {
        #[cfg(feature = "js")]
        if self.import_writing() {
            self.wall.chooser_open = true;
            return;
        }
        #[cfg(feature = "js")]
        if self.profile_import_open {
            self.close_profile_import();
        }
        let (command, targets) = match name {
            Some(name) => (OperatorCommand::OpenProfileForm, Targets::Profile(name)),
            None => (OperatorCommand::OpenNewProfileForm, Targets::Application),
        };
        if let Some(submitted) = self.submit(command, targets) {
            if self.command_staged(&submitted) {
                self.request_window_focus(super::PanelWindow::Profiles);
            }
        }
        self.wall.chooser_open = true;
    }

    #[cfg(feature = "js")]
    /// Open the rs2b0t import in the editor pane, in place of the edit
    /// form. With unsaved changes on the form, or its save still being
    /// written, the Discard / Keep editing prompt asks first.
    pub fn request_open_import(&mut self) {
        if self.profile_import_open {
            return;
        }
        let Some(submitted) =
            self.submit(OperatorCommand::EditProfileImportPath, Targets::Application)
        else {
            return;
        };
        if self.command_done(&submitted) {
            self.open_import();
        } else if self.command_staged(&submitted) {
            self.request_window_focus(super::PanelWindow::Profiles);
            self.wall.chooser_open = true;
        }
    }

    #[cfg(feature = "js")]
    fn open_import(&mut self) {
        self.profile_import_open = true;
        self.refresh_profile_import();
    }

    #[cfg(feature = "js")]
    /// Whether a confirmed rs2b0t import's writes are still being saved:
    /// the editor pane stays on its report until they settle.
    pub fn import_writing(&self) -> bool {
        self.profile_import_open && self.profile_import.writing(&self.core)
    }

    /// Discard the draft and leave the form as the prompt asked. No-op when
    /// no prompt is waiting.
    pub fn confirm_pending_edit_switch(&mut self) {
        let Some((leave, target)) = self
            .profile_editor
            .pending()
            .map(|prompt| (prompt.leave(), prompt.target().to_owned()))
        else {
            let _ = self.submit(OperatorCommand::DiscardProfileEdits, Targets::Application);
            return;
        };
        let Some(submitted) =
            self.submit(OperatorCommand::DiscardProfileEdits, Targets::Application)
        else {
            return;
        };
        if !self.command_done(&submitted) {
            return;
        }
        match leave {
            FormLeave::Switch => self.wall.chooser_open = true,
            FormLeave::CloseForm => {}
            FormLeave::CloseWindow => self.wall.chooser_open = false,
            FormLeave::DeleteTarget => {
                self.vault_remove(&target);
            }
            FormLeave::MultiBoxOff => {
                self.set_multibox(false);
            }
            #[cfg(feature = "js")]
            FormLeave::OpenImport => self.open_import(),
        }
    }

    /// Keep editing: drop the pending prompt, the form untouched.
    pub fn cancel_pending_edit_switch(&mut self) {
        let _ = self.submit(OperatorCommand::KeepEditing, Targets::Application);
    }

    /// The form's Cancel: closes the form, unless its save is still being
    /// written (the prompt is staged instead). Returns whether it closed.
    pub fn request_cancel_edit(&mut self) -> bool {
        let Some(submitted) = self.submit(
            OperatorCommand::CloseProfileForm(FormClose::Form),
            Targets::Application,
        ) else {
            return false;
        };
        if self.command_done(&submitted) {
            true
        } else {
            if self.command_staged(&submitted) {
                self.request_window_focus(super::PanelWindow::Profiles);
            }
            false
        }
    }

    /// Close Profiles (its Close button and its ✕), and the form with it.
    pub fn request_close_profiles(&mut self) -> bool {
        let Some(submitted) = self.submit(
            OperatorCommand::CloseProfileForm(FormClose::Window),
            Targets::Application,
        ) else {
            return false;
        };
        if self.command_done(&submitted) {
            self.wall.chooser_open = false;
            true
        } else {
            if self.command_staged(&submitted) {
                self.request_window_focus(super::PanelWindow::Profiles);
                self.wall.chooser_open = true;
            }
            false
        }
    }

    /// Whether the open edit form holds changes its Save would write.
    pub fn edit_dirty(&self) -> bool {
        self.profile_editor.dirty(&self.core)
    }

    /// Chooser row pick: focus the profile (loading it onto the wall in
    /// MultiBox) without disturbing an open edit form.
    pub fn pick_profile(&mut self, name: &str) {
        if self.multibox {
            self.load(name);
        } else {
            self.select(name);
        }
    }

    /// Close the form through its command row.
    pub fn cancel_edit_profile(&mut self) {
        let _ = self.submit(
            OperatorCommand::CloseProfileForm(FormClose::Window),
            Targets::Application,
        );
    }

    /// The chooser's confirmed delete of `name`.
    pub fn delete_profile(&mut self, name: &str) -> bool {
        self.vault_remove(name)
    }

    /// Whether the open edit form's own save is still being written.
    pub fn form_saving(&self) -> bool {
        self.profile_editor.saving(&self.core)
    }

    #[cfg(feature = "js")]
    /// Rebuild the rs2b0t import text from its flow: after each step, and
    /// each frame while a confirmed import's writes are still settling.
    pub fn refresh_profile_import(&mut self) {
        use frontend_core::rs2b0t_import::Stage;
        let flow = &self.profile_import;
        self.profile_import_lines = match flow.stage() {
            Stage::Path => Vec::new(),
            Stage::Preview => flow.preview_lines(),
            Stage::Report => flow.report_lines(&self.core),
        };
        self.profile_import_settled = flow.settled(&self.core);
    }

    #[cfg(feature = "js")]
    /// Close the rs2b0t import, dropping its parsed file (and passwords).
    pub fn close_profile_import(&mut self) {
        self.profile_import = frontend_core::rs2b0t_import::ProfileImport::new();
        self.profile_import_open = false;
        self.profile_import_lines.clear();
        self.profile_import_settled = false;
        self.import_delete_understood = false;
    }
}
