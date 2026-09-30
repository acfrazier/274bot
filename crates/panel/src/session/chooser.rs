//! Chooser credential scratch: picker edit buffers and vault read/write helpers.

use vault::{Profile, ProfileSettings};

use super::{fresh_uid, Session};

/// What Discard does on a staged Discard / Keep editing prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditLeave {
    /// Open [`EditSwitch::target`] over the form.
    Switch,
    /// Close the form, leaving Profiles open.
    CloseForm,
    /// Close Profiles, and the form with it.
    CloseWindow,
}

/// A staged Discard / Keep editing prompt: the operator asked to leave the
/// form (an explicit switch of it to `target`, `""` for a new profile, or a
/// close) while it holds unsaved edits or a save that has not settled. The
/// prompt text is built once, when the prompt is staged, so drawing it only
/// borrows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSwitch {
    pub target: String,
    pub prompt: String,
    pub leave: EditLeave,
}

impl Session {
    /// Save the credentials fields as a vault profile: the username field
    /// is the key, the password field the secret, and an existing profile's
    /// uid/settings are kept. Does not require a focused profile (first-run
    /// empty vault). A new profile or a rename onto a username another
    /// profile already has is refused with an error, never an overwrite.
    /// The write is only queued: the form stays open on its target until the
    /// write is durable, then follows a renamed or new profile to its saved
    /// name and shows `Saved <name>.`; the profile is selected (and so
    /// spawned) only then. A write that fails later leaves the form on its
    /// target with its draft, so Save retries the same rename or create; its
    /// error shows in the form and on the banner. Returns whether the write
    /// was accepted: a refusal shows inline and writes nothing, and a Save
    /// while this form's previous save is still being written is not
    /// accepted.
    pub fn save_credentials(&mut self) -> bool {
        // The form keeps its target until the write is durable, so a second
        // Save before then would not follow a rename or create.
        if self.chooser_save.saving() {
            return false;
        }
        self.pending_edit_switch = None;
        if self.core.vault().is_none() {
            return self.refuse_credentials("credentials: vault locked");
        }
        let username = self.cred_user.trim().to_string();
        if username.is_empty() {
            return self.refuse_credentials("credentials: username required");
        }
        let rename_from = self
            .chooser_edit
            .as_deref()
            .filter(|old| !old.is_empty() && old.trim() != username);
        let profile = {
            let vault = self.core.vault().expect("vault checked");
            let existing = if let Some(old) = rename_from {
                vault.get(old).cloned()
            } else {
                vault.get(&username).cloned()
            };
            Profile {
                uid: existing
                    .as_ref()
                    .map(|p| p.uid)
                    .unwrap_or_else(|| fresh_uid(vault)),
                username: username.clone(),
                password: self.cred_pass.clone(),
                settings: existing
                    .map(|p| {
                        let mut settings = p.settings;
                        if self.chooser_edit.is_some() {
                            settings.world = self.cred_settings.world;
                            settings.clue_duel_partner =
                                self.cred_settings.clue_duel_partner.trim().to_string();
                        }
                        settings
                    })
                    .unwrap_or_else(|| self.cred_settings.clone()),
            }
        };
        // Staged now and written off this thread (a rename as one
        // transaction); a running slot learns the next-handshake settings
        // once the write is durable, and its running script the profile's
        // clue duel partner (frozen reads it live).
        let live = rename_from
            .is_none()
            .then(|| {
                self.scripts
                    .profile_save_live(&self.core, &profile.username, &profile.settings)
            })
            .flatten();
        let mirror = frontend_core::ArmMirror::Remember(live);
        let saved = match rename_from {
            Some(old) => {
                let old = old.to_string();
                self.core
                    .rename_profile(&old, profile, mirror, "credentials")
            }
            None if self.chooser_edit.as_deref() == Some("") => {
                self.core.create_profile(profile, mirror, "credentials")
            }
            None => self.core.save_profile(profile, mirror, "credentials"),
        };
        let op = match saved {
            Ok(op) => op,
            Err(e) => return self.refuse_credentials(e),
        };
        self.error = None;
        // Select (and so spawn) only once the credentials are durable: a
        // failed write must not leave a worker logging in with them.
        let source = self.chooser_edit.as_deref().filter(|t| !t.is_empty());
        self.chooser_save
            .submitted(&mut self.core, op, source, username);
        true
    }

    /// A Save refused before anything was written: on the banner, and
    /// inline next to Save, since the banner is hidden while Profiles is
    /// open.
    fn refuse_credentials(&mut self, reason: impl Into<String>) -> bool {
        let reason = reason.into();
        self.chooser_save.refused(reason.clone());
        self.error = Some(reason);
        false
    }

    /// Settle the credentials Saves being written. Once one is durable, the
    /// form it came from, if still showing, follows the saved name (a
    /// rename's or a new profile's) and shows `Saved <name>.`, and the
    /// profile is selected. A write that failed after it was queued shows
    /// in the form it came from, which keeps its draft and its target, so
    /// Save retries; it selects nothing. Its error is on the banner too.
    pub(crate) fn settle_profile_save(&mut self) {
        for settled in self.chooser_save.settle(&mut self.core) {
            let frontend_core::FormSettled::Saved(saved) = settled else {
                continue;
            };
            if saved.same_form {
                self.chooser_edit = Some(saved.name.clone());
            }
            // `select` builds the arm from the durable auto-login.
            self.select(&saved.name);
        }
    }

    /// Empty the credentials-section fields. The vault entry is untouched.
    pub fn clear_credentials(&mut self) {
        self.cred_user.clear();
        self.cred_pass.clear();
    }

    /// Open the profile picker on `name` (or a blank new row). With unsaved
    /// changes on another target, or this form's save still being written
    /// (it can still fail, and the draft is all that is left of it), the
    /// switch waits: the form stays on its target with its buffers, and the
    /// pending target is staged for the Discard / Keep editing prompt
    /// ([`Self::confirm_pending_edit_switch`],
    /// [`Self::cancel_pending_edit_switch`]). Focus changes never come
    /// through here, so they can never retarget the form.
    pub fn begin_edit_profile(&mut self, name: Option<&str>) {
        let target = name.unwrap_or("");
        if self.chooser_edit.as_deref() == Some(target) {
            // Same target: keep the buffers — an explicit re-open never
            // discards what was typed.
            self.wall.chooser_open = true;
            return;
        }
        if self.edit_dirty() || self.chooser_save.saving() {
            let next = if target.is_empty() {
                "start a new profile".to_string()
            } else {
                format!("edit {target}")
            };
            self.stage_edit_exit(EditLeave::Switch, target, &next);
            self.wall.chooser_open = true;
            return;
        }
        self.open_edit_profile(target);
    }

    /// Stage the Discard / Keep editing prompt for leaving the form to
    /// `then` (`target` only matters to [`EditLeave::Switch`]).
    fn stage_edit_exit(&mut self, leave: EditLeave, target: &str, then: &str) {
        let prompt = match self.chooser_save.in_flight().filter(|_| self.chooser_save.saving()) {
            Some(saving) => format!(
                "Saving {saving} has not finished and may fail; its edits would be lost. Discard them and {then}?"
            ),
            None => format!("Discard unsaved edits and {then}?"),
        };
        self.pending_edit_switch = Some(EditSwitch {
            target: target.to_string(),
            prompt,
            leave,
        });
    }

    /// Switch the form to `target` (`""` is the blank new-profile row),
    /// reloading its buffers. Unconditional: the Discard path and clean
    /// opens come through here.
    fn open_edit_profile(&mut self, target: &str) {
        if target.is_empty() {
            self.cred_user.clear();
            self.cred_pass.clear();
            self.cred_settings = ProfileSettings::default();
            self.chooser_edit = Some(String::new());
        } else {
            if let Some(p) = self.core.vault().and_then(|v| v.get(target)) {
                self.cred_user = p.username.clone();
                self.cred_pass = p.password.clone();
                self.cred_settings = p.settings.clone();
            }
            self.chooser_edit = Some(target.to_string());
        }
        self.pending_edit_switch = None;
        self.chooser_save.form_changed();
        self.chooser_form = self.chooser_form.wrapping_add(1);
        self.wall.chooser_open = true;
    }

    /// Discard the draft and leave the form as the prompt asked (open the
    /// pending target, or close the form or Profiles). No-op when no prompt
    /// is waiting.
    pub fn confirm_pending_edit_switch(&mut self) {
        let Some(switch) = self.pending_edit_switch.take() else {
            return;
        };
        match switch.leave {
            EditLeave::Switch => self.open_edit_profile(&switch.target),
            EditLeave::CloseForm => self.cancel_edit_profile(),
            EditLeave::CloseWindow => {
                self.wall.chooser_open = false;
                self.cancel_edit_profile();
            }
        }
    }

    /// Keep editing: drop the pending prompt, the form untouched.
    pub fn cancel_pending_edit_switch(&mut self) {
        self.pending_edit_switch = None;
    }

    /// The form's Cancel: closes the form. Refused while the form's save is
    /// still being written: the Discard / Keep editing prompt is staged
    /// instead. Returns whether the form closed.
    pub fn request_cancel_edit(&mut self) -> bool {
        if self.chooser_save.saving() {
            self.stage_edit_exit(EditLeave::CloseForm, "", "close the form");
            return false;
        }
        self.cancel_edit_profile();
        true
    }

    /// Close Profiles (its Close button and its ✕), and the form with it.
    /// Refused while the form's save is still being written: the Discard /
    /// Keep editing prompt is staged and Profiles stays open. Returns
    /// whether Profiles closed.
    pub fn request_close_profiles(&mut self) -> bool {
        if self.chooser_save.saving() {
            self.stage_edit_exit(EditLeave::CloseWindow, "", "close Profiles");
            self.wall.chooser_open = true;
            return false;
        }
        self.wall.chooser_open = false;
        self.cancel_edit_profile();
        true
    }

    /// Whether the open edit form holds changes its Save would write (user,
    /// pass or settings against the vault row; a blank new-profile form is
    /// clean). False with no form open.
    pub fn edit_dirty(&self) -> bool {
        let Some(target) = self.chooser_edit.as_deref() else {
            return false;
        };
        let target = if target.is_empty() {
            None
        } else {
            Some(target)
        };
        self.core.profile_form_dirty(
            target,
            &self.cred_user,
            &self.cred_pass,
            &self.cred_settings,
        )
    }

    /// A form edit (user/pass/world/clue/toggle): a stale inline refusal or
    /// save confirmation no longer describes the form, so it goes. The
    /// vault row is untouched.
    pub fn note_chooser_edited(&mut self) {
        self.chooser_save.edited();
    }

    /// Chooser row pick: focus the profile (loading it onto the wall in
    /// MultiBox) without disturbing an open edit form — a focus change
    /// never retargets the form, rewrites its buffers, or closes the
    /// picker.
    pub fn pick_profile(&mut self, name: &str) {
        if self.multibox {
            self.load(name);
        } else {
            self.select(name);
        }
    }

    /// Close the edit form, whatever it holds. A save it submitted still
    /// settles (selecting its profile once durable), but a failure of it no
    /// longer shows in any form: only on the banner. Operator closes ask
    /// first while the save is unsettled ([`Self::request_cancel_edit`],
    /// [`Self::request_close_profiles`]).
    pub fn cancel_edit_profile(&mut self) {
        self.chooser_edit = None;
        self.pending_edit_switch = None;
        self.chooser_save.form_changed();
    }

    /// The chooser's confirmed delete of `name`: the form editing it closes
    /// (a save it still has queued settles behind the delete), then the
    /// vault row goes. Returns whether a row was removed.
    pub fn delete_profile(&mut self, name: &str) -> bool {
        if self.chooser_edit.as_deref() == Some(name) {
            self.cancel_edit_profile();
        }
        self.vault_remove(name)
    }
}
