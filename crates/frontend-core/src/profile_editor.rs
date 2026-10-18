//! The profile edit form both front ends bind (T3.3; moved from the
//! panel's Profiles chooser): its target, its staged [`ProfileDraft`], the
//! Discard / Keep editing prompt, and the save feedback.
//!
//! The form only stages. Its Save commits the draft through the command
//! seam: [`OperatorCommand::SaveProfile`] (an existing profile, or a rename
//! of it) or [`OperatorCommand::CreateProfile`] (the blank new-profile row),
//! with the profile staged on the session first
//! ([`OperatorSession::stage_profile_write`]). The front end supplies only
//! its own effects of leaving the form ([`FormLeave`]: closing its window,
//! turning MultiBox off, opening the import).

use crate::command::OperatorCommand;
use crate::profile_draft::ProfileDraft;
use crate::profile_form::{FormSettled, ProfileFormSave};
use crate::scripts::Scripts;
use crate::session::{OperatorSession, StagedProfileWrite, Targets};

/// What Discard does on a staged Discard / Keep editing prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormLeave {
    /// Open the prompt's target over the form.
    Switch,
    /// Close the form, leaving the profiles window open.
    CloseForm,
    /// Close the profiles window, and the form with it.
    CloseWindow,
    /// Remove the prompt's target from the vault; it is the profile the
    /// form edits, so the form closes with it.
    DeleteTarget,
    /// Turn MultiBox off, which closes the profiles window and the form.
    MultiBoxOff,
    /// Close the form and open the rs2b0t import in the editor pane.
    OpenImport,
}

/// A staged Discard / Keep editing prompt: the operator asked to leave the
/// form (a switch of it to [`Self::target`], `""` for a new profile, a
/// close, the delete of its profile, MultiBox off or the import) while it
/// holds unsaved edits or a save that has not settled. The text is built
/// once, when the prompt is staged, so drawing it only borrows.
#[derive(Debug, PartialEq, Eq)]
pub struct LeavePrompt {
    target: String,
    prompt: String,
    leave: FormLeave,
    saving: bool,
}

impl LeavePrompt {
    /// The profile the leave is about (`""`: a new profile or none).
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The prompt text.
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// What Discard does.
    pub fn leave(&self) -> FormLeave {
        self.leave
    }

    /// Whether the prompt was staged because the form's save had not
    /// settled (its text says the save may fail).
    pub fn saving(&self) -> bool {
        self.saving
    }
}

/// What a request to open the form on a target did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormOpen {
    /// The form now edits the target with a fresh draft.
    Opened,
    /// The form already edited the target; its draft is kept.
    Kept,
    /// The form holds unsaved edits or an unsettled save: the Discard /
    /// Keep editing prompt is staged instead.
    Staged,
}

/// The profile edit form.
#[derive(Debug, Default)]
pub struct ProfileEditor {
    /// The profile the form edits: `Some("")` is the blank new-profile row,
    /// `None` no form.
    target: Option<String>,
    draft: ProfileDraft,
    /// Bumped each time the form switches target (a front end resets its
    /// widget state on it).
    generation: usize,
    pending: Option<LeavePrompt>,
    save: ProfileFormSave,
}

impl ProfileEditor {
    /// A closed form.
    pub fn new() -> Self {
        Self::default()
    }

    /// The profile the form edits (`""` for a new profile); `None` while no
    /// form is open.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// Whether a form is open.
    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }

    /// The staged draft.
    pub fn draft(&self) -> &ProfileDraft {
        &self.draft
    }

    /// The staged draft, for the form's edit buffers. Call
    /// [`Self::edited`] after a change.
    pub fn draft_mut(&mut self) -> &mut ProfileDraft {
        &mut self.draft
    }

    /// Bumped on every switch of the form's target.
    pub fn generation(&self) -> usize {
        self.generation
    }

    /// The staged Discard / Keep editing prompt.
    pub fn pending(&self) -> Option<&LeavePrompt> {
        self.pending.as_ref()
    }

    /// The form's save feedback.
    pub fn save_state(&self) -> &ProfileFormSave {
        &self.save
    }

    /// A form edit: a stale refusal or save notice no longer describes the
    /// form.
    pub fn edited(&mut self) {
        self.save.edited();
    }

    /// Empty the username and password fields; the vault row is untouched.
    pub fn clear_credentials(&mut self) {
        self.draft.clear_credentials();
    }

    /// Whether the form's own save is still being written.
    pub fn saving<Io>(&self, core: &OperatorSession<Io>) -> bool {
        self.save.saving(core)
    }

    /// Whether the open form holds changes its Save would write (a blank
    /// new-profile form is clean). False with no form open.
    pub fn dirty<Io>(&self, core: &OperatorSession<Io>) -> bool {
        let Some(target) = self.target.as_deref() else {
            return false;
        };
        let target = (!target.is_empty()).then_some(target);
        core.profile_form_dirty(target, &self.draft)
    }

    /// Open the form on `target` (`""`: a new profile). The same target
    /// keeps its draft; unsaved edits or an unsettled save stage the
    /// Discard / Keep editing prompt instead.
    pub fn request_open<Io>(&mut self, core: &OperatorSession<Io>, target: &str) -> FormOpen {
        if self.target.as_deref() == Some(target) {
            return FormOpen::Kept;
        }
        if self.dirty(core) || self.saving(core) {
            let next = if target.is_empty() {
                "start a new profile".to_string()
            } else {
                format!("edit {target}")
            };
            self.stage_leave(core, FormLeave::Switch, target, &next);
            return FormOpen::Staged;
        }
        self.open(core, target);
        FormOpen::Opened
    }

    /// Switch the form to `target`, reloading its draft. Unconditional:
    /// the Discard path and clean opens come through here.
    pub fn open<Io>(&mut self, core: &OperatorSession<Io>, target: &str) {
        if target.is_empty() {
            self.draft = ProfileDraft::default();
            self.target = Some(String::new());
        } else {
            if let Some(profile) = core.profile(target) {
                self.draft = profile.draft();
            }
            self.target = Some(target.to_string());
        }
        self.pending = None;
        self.save.form_changed();
        self.generation = self.generation.wrapping_add(1);
    }

    /// Stage the Discard / Keep editing prompt for leaving the form to
    /// `then` (`target` matters to [`FormLeave::Switch`] and
    /// [`FormLeave::DeleteTarget`]).
    pub fn stage_leave<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        leave: FormLeave,
        target: &str,
        then: &str,
    ) {
        let saving = self.save.saving_name(core);
        let prompt = match saving {
            Some(saving) => format!(
                "Saving {saving} has not finished and may fail; its edits would be lost. Discard them and {then}?"
            ),
            None => format!("Discard unsaved edits and {then}?"),
        };
        self.pending = Some(LeavePrompt {
            target: target.to_string(),
            prompt,
            leave,
            saving: saving.is_some(),
        });
    }

    /// Discard: drop the draft and do the form's part of the staged leave
    /// (open the pending target, or close the form). Returns the prompt so
    /// the front end does its own part ([`FormLeave`]); a
    /// [`FormLeave::DeleteTarget`] still needs its
    /// [`OperatorCommand::DeleteProfile`]. `None` when no prompt waited.
    pub fn discard<Io>(&mut self, core: &OperatorSession<Io>) -> Option<LeavePrompt> {
        let prompt = self.pending.take()?;
        match prompt.leave {
            FormLeave::Switch => self.open(core, &prompt.target),
            FormLeave::CloseForm
            | FormLeave::CloseWindow
            | FormLeave::DeleteTarget
            | FormLeave::MultiBoxOff
            | FormLeave::OpenImport => self.close(),
        }
        Some(prompt)
    }

    /// Keep editing: drop the pending prompt, the form untouched.
    pub fn keep_editing(&mut self) {
        self.pending = None;
    }

    /// Close the form (its Cancel, or the window's Close) unless its save
    /// is still being written, which stages the prompt instead. Returns
    /// whether the form closed.
    pub fn request_close<Io>(&mut self, core: &OperatorSession<Io>, leave: FormLeave) -> bool {
        if self.saving(core) {
            let then = match leave {
                FormLeave::CloseWindow => "close Profiles",
                _ => "close the form",
            };
            self.stage_leave(core, leave, "", then);
            return false;
        }
        self.close();
        true
    }

    /// Close the form, whatever it holds. A save it submitted still
    /// settles, but a failure of it shows only on the banner.
    pub fn close(&mut self) {
        self.target = None;
        self.pending = None;
        self.save.form_changed();
    }

    /// Before the vault row `name` is deleted: the form editing it closes,
    /// unless its save has not settled, which stages the prompt (the write
    /// can still fail and put the profile back). Returns whether the delete
    /// may go ahead.
    pub fn before_delete<Io>(&mut self, core: &OperatorSession<Io>, name: &str) -> bool {
        let editing = self.target.as_deref() == Some(name);
        if editing && self.saving(core) {
            self.stage_leave(
                core,
                FormLeave::DeleteTarget,
                name,
                &format!("delete {name}"),
            );
            return false;
        }
        if editing {
            self.close();
        }
        true
    }

    /// Save the draft: the username is the key, the password the secret,
    /// and an existing profile's uid and settings are kept (the form edits
    /// only its world and clue partner). A new profile or a rename onto a
    /// taken username is refused, never an overwrite. The write is staged
    /// and committed through the command seam; the form keeps its target
    /// until the write is durable ([`Self::settle`]). `Err` is the refusal
    /// (shown in the form and on the banner); a Save while this form's
    /// previous save is unsettled is refused without a notice.
    pub fn save<Io>(
        &mut self,
        core: &mut OperatorSession<Io>,
        scripts: &Scripts,
    ) -> Result<(), SaveRefused> {
        if self.saving(core) {
            return Err(SaveRefused::StillSaving);
        }
        self.pending = None;
        let refused = |editor: &mut Self, reason: String| {
            editor.save.refused(reason);
            Err(SaveRefused::Refused)
        };
        if !core.vault_open() {
            return refused(self, "credentials: vault locked".to_string());
        }
        let username = self.draft.username().trim().to_string();
        if username.is_empty() {
            return refused(self, "credentials: username required".to_string());
        }
        let rename_from = self
            .target
            .as_deref()
            .filter(|old| !old.is_empty() && old.trim() != username)
            .map(str::to_string);
        let creating = rename_from.is_none() && self.target.as_deref() == Some("");
        let Some(profile) = core.compose_profile(
            rename_from.as_deref(),
            &username,
            &self.draft,
            self.target.is_some(),
        ) else {
            return refused(self, "credentials: vault locked".to_string());
        };
        // A running slot learns the next-handshake settings once the write
        // is durable, and its running script the clue duel partner.
        let live = rename_from
            .is_none()
            .then(|| scripts.profile_save_live(core, &profile.username, &profile.settings))
            .flatten();
        let target = rename_from.clone().unwrap_or_else(|| username.clone());
        core.stage_profile_write(StagedProfileWrite::new(
            profile,
            rename_from,
            creating,
            crate::ArmMirror::remember(live),
            "credentials",
        ));
        let command = if creating {
            OperatorCommand::CreateProfile
        } else {
            OperatorCommand::SaveProfile
        };
        let targets = if creating {
            Targets::Application
        } else {
            Targets::Profile(&target)
        };
        let submitted = match core.submit_detached(command, targets) {
            Ok(submitted) => submitted,
            Err(error) => return refused(self, error.to_string()),
        };
        if let Some(reason) = core.command_failure(submitted.request()) {
            let reason = reason.to_string();
            return refused(self, reason);
        }
        let Some(op) = submitted.operation() else {
            return refused(self, "credentials: nothing was queued".to_string());
        };
        let source = self.target.as_deref().filter(|t| !t.is_empty());
        self.save.submitted(core, op, source, username);
        Ok(())
    }

    /// Settle the form's saves being written. A durable one moves the form
    /// it came from (if still showing) onto the saved name; the returned
    /// names are the profiles to select now that they are durable. A
    /// Discard / Keep editing prompt staged because the save had not
    /// settled ends with it.
    pub fn settle<Io>(&mut self, core: &mut OperatorSession<Io>) -> Vec<String> {
        let mut saved = Vec::new();
        for settled in self.save.settle(core) {
            let Some(done) = FormSettled::into_saved(settled) else {
                continue;
            };
            let same_form = done.same_form();
            let name = done.into_destination();
            if same_form {
                self.target = Some(name.clone());
            }
            saved.push(name);
        }
        if self.pending.as_ref().is_some_and(|p| p.saving) && !self.saving(core) {
            self.pending = None;
        }
        saved
    }
}

/// Why [`ProfileEditor::save`] wrote nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SaveRefused {
    /// The form's previous save is still being written.
    #[error("the form's previous save is still being written")]
    StillSaving,
    /// Refused before anything was written; the reason is the form's
    /// notice ([`crate::FormNotice::error`]).
    #[error("the save was refused before anything was written")]
    Refused,
}
