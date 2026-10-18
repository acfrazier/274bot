//! Profiles window over the core's profile edit form
//! ([`frontend_core::ProfileEditor`]): the panel adds its own effects
//! (the window open, the focus request, the rs2b0t import sharing the editor
//! pane, MultiBox off) around the form's staging and saves.

use frontend_core::{FormLeave, FormOpen, SaveRefused};

use super::Session;

impl Session {
    /// Save the credentials form as a vault profile through the core form
    /// ([`frontend_core::ProfileEditor::save`]): the username field is the
    /// key, the password field the secret, and an existing profile's
    /// uid/settings are kept. A new profile or a rename onto a taken
    /// username is refused, never an overwrite. The form keeps its target
    /// until the write is durable, then follows a renamed or new profile to
    /// its saved name and shows `Saved <name>.`; the profile is selected
    /// (and so spawned) only then. Returns whether the write was accepted:
    /// a refusal shows inline and on the banner and writes nothing, and a
    /// Save while this form's previous save is unsettled is not accepted.
    pub fn save_credentials(&mut self) -> bool {
        match self.profile_editor.save(&mut self.core, &self.scripts) {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(SaveRefused::StillSaving) => false,
            Err(SaveRefused::Refused) => {
                // The banner is hidden while Profiles is open, so the form
                // shows the refusal too.
                self.error = self
                    .profile_editor
                    .save_state()
                    .notice()
                    .and_then(|notice| notice.error())
                    .map(str::to_string);
                false
            }
        }
    }

    /// Settle the credentials Saves being written: once one is durable, the
    /// form it came from follows the saved name and the profile is selected
    /// (`select` builds the arm from the durable auto-login). A failed write
    /// shows in the form it came from and on the banner; it selects nothing.
    pub(crate) fn settle_profile_save(&mut self) {
        for name in self.profile_editor.settle(&mut self.core) {
            self.select(&name);
        }
    }

    /// Empty the credentials-section fields. The vault entry is untouched.
    pub fn clear_credentials(&mut self) {
        self.profile_editor.clear_credentials();
    }

    /// Open the profile picker on `name` (or a blank new row). With unsaved
    /// changes on another target, or this form's save still being written,
    /// the switch waits behind the Discard / Keep editing prompt. Focus
    /// changes never come through here. The form and the rs2b0t import
    /// share the editor pane: an import whose writes are still being saved
    /// keeps it, and any other import closes.
    pub fn begin_edit_profile(&mut self, name: Option<&str>) {
        if self.import_writing() {
            self.wall.chooser_open = true;
            return;
        }
        if self.profile_import_open {
            self.close_profile_import();
        }
        let target = name.unwrap_or("");
        if self.profile_editor.request_open(&self.core, target) == FormOpen::Staged {
            self.request_window_focus(super::PanelWindow::Profiles);
        }
        self.wall.chooser_open = true;
    }

    /// Open the rs2b0t import in the editor pane, in place of the edit
    /// form. With unsaved changes on the form, or its save still being
    /// written, the Discard / Keep editing prompt asks first.
    pub fn request_open_import(&mut self) {
        if self.profile_import_open {
            return;
        }
        if self.profile_editor.is_open() {
            if self.edit_dirty() || self.form_saving() {
                self.stage_edit_exit(FormLeave::OpenImport, "", "import rs2b0t profiles");
                return;
            }
            self.cancel_edit_profile();
        }
        self.open_import();
    }

    fn open_import(&mut self) {
        self.profile_import_open = true;
        self.refresh_profile_import();
    }

    /// Whether a confirmed rs2b0t import's writes are still being saved:
    /// the editor pane stays on its report until they settle.
    pub fn import_writing(&self) -> bool {
        self.profile_import_open && self.profile_import.writing(&self.core)
    }

    /// Stage the Discard / Keep editing prompt for leaving the form to
    /// `then`, and bring Profiles to the front.
    pub(super) fn stage_edit_exit(&mut self, leave: FormLeave, target: &str, then: &str) {
        self.profile_editor
            .stage_leave(&self.core, leave, target, then);
        self.request_window_focus(super::PanelWindow::Profiles);
    }

    /// Discard the draft and leave the form as the prompt asked. No-op when
    /// no prompt is waiting.
    pub fn confirm_pending_edit_switch(&mut self) {
        let Some(prompt) = self.profile_editor.discard(&self.core) else {
            return;
        };
        match prompt.leave() {
            FormLeave::Switch => self.wall.chooser_open = true,
            FormLeave::CloseForm => {}
            FormLeave::CloseWindow => self.wall.chooser_open = false,
            FormLeave::DeleteTarget => {
                self.vault_remove(prompt.target());
            }
            FormLeave::MultiBoxOff => self.apply_multibox(false),
            FormLeave::OpenImport => self.open_import(),
        }
    }

    /// Keep editing: drop the pending prompt, the form untouched.
    pub fn cancel_pending_edit_switch(&mut self) {
        self.profile_editor.keep_editing();
    }

    /// The form's Cancel: closes the form, unless its save is still being
    /// written (the prompt is staged instead). Returns whether it closed.
    pub fn request_cancel_edit(&mut self) -> bool {
        let closed = self
            .profile_editor
            .request_close(&self.core, FormLeave::CloseForm);
        if !closed {
            self.request_window_focus(super::PanelWindow::Profiles);
        }
        closed
    }

    /// Close Profiles (its Close button and its ✕), and the form with it,
    /// unless the form's save is still being written (the prompt is staged
    /// and Profiles stays open). Returns whether Profiles closed.
    pub fn request_close_profiles(&mut self) -> bool {
        if !self
            .profile_editor
            .request_close(&self.core, FormLeave::CloseWindow)
        {
            self.request_window_focus(super::PanelWindow::Profiles);
            self.wall.chooser_open = true;
            return false;
        }
        self.wall.chooser_open = false;
        true
    }

    /// Whether the open edit form holds changes its Save would write.
    pub fn edit_dirty(&self) -> bool {
        self.profile_editor.dirty(&self.core)
    }

    /// A form edit: a stale inline refusal or save notice goes.
    pub fn note_chooser_edited(&mut self) {
        self.profile_editor.edited();
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

    /// Close the edit form, whatever it holds. A save it submitted still
    /// settles, but a failure of it shows only on the banner.
    pub fn cancel_edit_profile(&mut self) {
        self.profile_editor.close();
    }

    /// The chooser's confirmed delete of `name`: the vault row goes, and
    /// the form editing it closes with it. While that form's save has not
    /// settled the delete waits behind the prompt instead. Returns whether
    /// a row was removed.
    pub fn delete_profile(&mut self, name: &str) -> bool {
        if !self.profile_editor.before_delete(&self.core, name) {
            self.request_window_focus(super::PanelWindow::Profiles);
            self.wall.chooser_open = true;
            return false;
        }
        self.vault_remove(name)
    }

    /// Whether the open edit form's own save is still being written.
    pub fn form_saving(&self) -> bool {
        self.profile_editor.saving(&self.core)
    }

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

    /// Close the rs2b0t import, dropping its parsed file (and passwords).
    pub fn close_profile_import(&mut self) {
        self.profile_import = frontend_core::rs2b0t_import::ProfileImport::new();
        self.profile_import_open = false;
        self.profile_import_lines.clear();
        self.profile_import_settled = false;
        self.import_delete_understood = false;
    }
}
