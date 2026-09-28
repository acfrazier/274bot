//! Chooser credential scratch: picker edit buffers and vault read/write helpers.

use vault::{Profile, ProfileSettings};

use super::{fresh_uid, Session};

/// An explicit switch of an unsaved edit form to `target` (`""` is a new
/// profile), waiting on the Discard / Keep editing prompt. The prompt text
/// is built once, when the switch is staged, so drawing it only borrows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSwitch {
    pub target: String,
    pub prompt: String,
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
    /// target, so Save retries the same rename or create; its error is on
    /// the banner. Returns whether the write was accepted: a refusal shows
    /// inline and writes nothing, and a Save while this form's previous save
    /// is still being written is not accepted.
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
        self.chooser_save.submitted(op, username);
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

    /// Settle the credentials Save being written. Once it is durable, the
    /// form it came from, if still showing, follows the saved name (a
    /// rename's or a new profile's) and shows `Saved <name>.`, and the
    /// profile is selected. A failed write selects nothing and leaves the
    /// form alone: its error is on the banner.
    pub(crate) fn settle_profile_save(&mut self) {
        let Some(saved) = self.chooser_save.settle(&self.core) else {
            return;
        };
        if saved.same_form {
            self.chooser_edit = Some(saved.name.clone());
        }
        // `select` builds the arm from the durable auto-login.
        self.select(&saved.name);
    }

    /// Empty the credentials-section fields. The vault entry is untouched.
    pub fn clear_credentials(&mut self) {
        self.cred_user.clear();
        self.cred_pass.clear();
    }

    /// Open the profile picker on `name` (or a blank new row). With unsaved
    /// changes on another target the switch waits: the form stays on its
    /// target with its buffers, and the pending target is staged for the
    /// Discard / Keep editing prompt ([`Self::confirm_pending_edit_switch`],
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
        if self.edit_dirty() {
            let prompt = if target.is_empty() {
                "Discard unsaved edits and start a new profile?".to_string()
            } else {
                format!("Discard unsaved edits and edit {target}?")
            };
            self.pending_edit_switch = Some(EditSwitch {
                target: target.to_string(),
                prompt,
            });
            self.wall.chooser_open = true;
            return;
        }
        self.open_edit_profile(target);
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

    /// Discard the draft and switch to the pending target. No-op when no
    /// switch is waiting.
    pub fn confirm_pending_edit_switch(&mut self) {
        if let Some(switch) = self.pending_edit_switch.take() {
            self.open_edit_profile(&switch.target);
        }
    }

    /// Keep editing: drop the pending target switch, the form untouched.
    pub fn cancel_pending_edit_switch(&mut self) {
        self.pending_edit_switch = None;
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

    /// Close the edit form. A save it submitted still settles (selecting
    /// its profile once durable) but never shows a notice again.
    pub fn cancel_edit_profile(&mut self) {
        self.chooser_edit = None;
        self.pending_edit_switch = None;
        self.chooser_save.form_changed();
    }
}
