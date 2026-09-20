//! Chooser credential scratch: picker edit buffers and vault read/write helpers.

use vault::{Profile, ProfileSettings};

use super::{fresh_uid, Session};

impl Session {
    fn sync_cred_fields_from_vault(&mut self, name: &str) {
        if let Some(vault) = &self.vault {
            if let Some(p) = vault.get(name) {
                self.cred_user = p.username.clone();
                self.cred_pass = p.password.clone();
            }
        }
    }

    /// Save the credentials fields as a vault profile: the username field
    /// is the key, the password field the secret, and an existing profile's
    /// uid/settings are kept. Does not require a focused profile (first-run
    /// empty vault). After a successful upsert, spawns the slot via the
    /// existing FIFO if it is not running, then selects it. Returns whether
    /// the write landed; failures set [`Session::error`].
    pub fn save_credentials(&mut self) -> bool {
        if self.vault.is_none() {
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
            let vault = self.vault.as_mut().expect("vault checked");
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
                    .map(|p| p.settings)
                    .unwrap_or_else(|| self.cred_settings.clone()),
            }
        };
        match self.vault.as_mut().expect("vault checked").upsert(profile) {
            Ok(()) => {}
            Err(e) => {
                self.error = Some(format!("credentials: {e}"));
                return false;
            }
        }
        if let Some(old) = rename_from {
            match self.vault.as_mut().expect("vault checked").remove(old) {
                Ok(_) => {}
                Err(e) => {
                    self.error = Some(format!("credentials: {e}"));
                    return false;
                }
            }
        }
        self.chooser_edit = None;
        // `select` builds the arm from the vault auto-login setting.
        self.error = None;
        self.select(&username);
        true
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
                if let Some(p) = self.vault.as_ref().and_then(|v| v.get(n)) {
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
