//! Chooser credential scratch: picker edit buffers and vault read/write helpers.

use vault::{Profile, ProfileSettings};

use super::{fresh_uid, Session};

impl Session {
    /// Save the credentials fields as a vault profile: the username field
    /// is the key, the password field the secret, and an existing profile's
    /// uid/settings are kept. Does not require a focused profile (first-run
    /// empty vault). After the write is queued, spawns the slot via the
    /// existing FIFO if it is not running, then selects it. Returns whether
    /// the write was accepted; a failed durable write is reported on a later
    /// frame and restores the saved profile.
    pub fn save_credentials(&mut self) -> bool {
        if self.core.vault().is_none() {
            self.error = Some("credentials: vault locked".into());
            return false;
        }
        let username = self.cred_user.trim().to_string();
        if username.is_empty() {
            self.error = Some("credentials: username required".into());
            return false;
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
                        }
                        settings
                    })
                    .unwrap_or_else(|| self.cred_settings.clone()),
            }
        };
        // Staged now and written off this thread (a rename as one
        // transaction); a running slot learns the next-handshake settings
        // once the write is durable.
        let mirror = frontend_core::ArmMirror::Remember;
        let saved = match rename_from {
            Some(old) => {
                let old = old.to_string();
                self.core
                    .rename_profile(&old, profile, mirror, "credentials")
            }
            None => self.core.save_profile(profile, mirror, "credentials"),
        };
        let op = match saved {
            Ok(op) => op,
            Err(e) => {
                self.error = Some(e);
                return false;
            }
        };
        self.chooser_edit = None;
        self.error = None;
        // Select (and so spawn) only once the credentials are durable: a
        // failed write must not leave a worker logging in with them.
        self.saving_profile = Some((op, username));
        true
    }

    /// Select the profile a credentials Save wrote once that write settled
    /// successfully. A failed write selects nothing (its error is shown).
    pub(crate) fn settle_profile_save(&mut self) {
        let Some((op, name)) = self.saving_profile.as_ref() else {
            return;
        };
        let outcome = self
            .core
            .operation(*op)
            .and_then(|r| r.outcome(name))
            .cloned();
        match outcome {
            Some(frontend_core::Outcome::Pending) => {}
            Some(frontend_core::Outcome::Completed) => {
                let name = name.clone();
                self.saving_profile = None;
                // `select` builds the arm from the durable auto-login.
                self.select(&name);
            }
            _ => self.saving_profile = None,
        }
    }

    /// Empty the credentials-section fields. The vault entry is untouched.
    pub fn clear_credentials(&mut self) {
        self.cred_user.clear();
        self.cred_pass.clear();
    }

    /// Open the profile picker on `name` (or a blank new row).
    pub fn begin_edit_profile(&mut self, name: Option<&str>) {
        match name {
            Some(n) => {
                if let Some(p) = self.core.vault().and_then(|v| v.get(n)) {
                    self.cred_user = p.username.clone();
                    self.cred_pass = p.password.clone();
                    self.cred_settings = p.settings.clone();
                }
                self.chooser_edit = Some(n.to_string());
            }
            None => {
                self.cred_user.clear();
                self.cred_pass.clear();
                self.cred_settings = ProfileSettings::default();
                self.chooser_edit = Some(String::new());
            }
        }
        self.wall.chooser_open = true;
    }

    pub fn cancel_edit_profile(&mut self) {
        self.chooser_edit = None;
    }
}
