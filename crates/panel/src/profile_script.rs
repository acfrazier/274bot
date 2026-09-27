//! Panel adapter over the shared script coordinator
//! ([`frontend_core::Scripts`]): it resolves the focused profile, its
//! script heading and the catalog root, shows the coordinator's notices on
//! the banner and attributes Start failures to the external-loader watch.
//! Assignment, parameters, Start / Start all / Stop all, reload, catalog
//! refresh and Apply to all are the coordinator's.

use std::path::{Path, PathBuf};

use frontend_core::scripts::ReloadOutcome;
use frontend_core::Scripts;
use serde_json::{Map, Value};
use vault::ScriptAssignment;

use crate::session::Session;

impl Session {
    /// Show the coordinator's latest notice on the banner.
    fn apply_script_notice(&mut self) {
        if let Some(notice) = self.scripts.take_notice() {
            notice.apply(&mut self.error);
        }
    }

    /// The catalog root a Start may fill the catalog from; a server-profile
    /// error is shown and fills nothing.
    fn start_catalog_root(&mut self) -> Option<PathBuf> {
        if self.scripts.catalog_filled() {
            return None;
        }
        match self.catalog_root() {
            Ok(root) => root,
            Err(error) => {
                self.error = Some(format!("server profile: {error}"));
                None
            }
        }
    }

    pub fn restore_script_heading(&mut self, profile: &str) {
        self.script_sel = self.scripts.heading(&self.core, profile);
    }

    pub fn profile_assignment(&self, profile: &str) -> Option<ScriptAssignment> {
        Scripts::assignment(&self.core, profile)
    }

    pub fn heading_is_pending(&self) -> bool {
        self.focused_name()
            .is_some_and(|name| self.scripts.heading_is_pending(&self.core, &name))
    }

    pub fn set_pending_browse(&mut self, profile: &str, sel: script::ScriptSel) {
        self.scripts.set_pending_browse(profile, sel);
    }

    pub fn persist_successful_assignment(&mut self, profile: &str, assignment: ScriptAssignment) {
        self.scripts
            .persist_assignment(&mut self.core, profile, assignment);
        self.apply_script_notice();
    }

    pub fn profile_overrides(
        &mut self,
        profile: &str,
        key: &str,
        card_name: &str,
    ) -> Map<String, Value> {
        let bag = self
            .scripts
            .profile_overrides(&mut self.core, profile, key, card_name);
        self.apply_script_notice();
        bag
    }

    pub fn merged_profile_bag(
        &mut self,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        schema: &[script::SettingDef],
    ) -> Map<String, Value> {
        let bag =
            self.scripts
                .merged_profile_bag(&mut self.core, profile, source, name, path, schema);
        self.apply_script_notice();
        bag
    }

    /// Set one parameter on `profile`'s bag. The matching run receives it
    /// once the write is durable. Returns whether the write was queued.
    pub fn set_profile_setting(
        &mut self,
        profile: &str,
        source: script::ScriptSource,
        name: &str,
        path: &Path,
        id: &str,
        value: Value,
    ) -> bool {
        let queued = self
            .scripts
            .set_profile_setting(&mut self.core, profile, source, name, path, id, value)
            .is_ok();
        self.apply_script_notice();
        queued
    }

    /// Start the given profile using its last successful assignment only.
    pub fn script_start_profile(&mut self, profile: &str) -> Result<(), String> {
        let root = self.start_catalog_root();
        let result = self
            .scripts
            .start_profile(&mut self.core, profile, root.as_deref());
        self.apply_script_notice();
        result
    }

    /// Start the focused heading: the profile's pending Browse selection,
    /// else the heading, else its assignment.
    pub(crate) fn script_start_focused(&mut self, profile: &str) -> Result<(), String> {
        let root = self.start_catalog_root();
        let heading = self.script_sel.clone();
        let result =
            self.scripts
                .start_selected(&mut self.core, profile, heading.as_ref(), root.as_deref());
        self.apply_script_notice();
        result
    }

    pub fn script_start_all(&mut self) {
        let root = self.start_catalog_root();
        self.scripts.start_all(&mut self.core, root.as_deref());
        self.apply_script_notice();
    }

    pub fn script_stop_all(&mut self) {
        self.scripts.stop_all(&mut self.core);
        self.apply_script_notice();
    }

    /// Fold settled script work (reload validation, Start setup, parameter
    /// writes) once per UI frame, after the core poll.
    pub fn poll_scripts(&mut self) {
        self.scripts.poll(&mut self.core);
        for (name, message) in self.scripts.take_start_failures() {
            if let Some(watch) = self.external_core_watch() {
                if watch.configured() && watch.account() == name {
                    watch.fail_start(message);
                }
            }
        }
        self.apply_script_notice();
    }

    pub fn reload_validation_pending(&self) -> bool {
        self.scripts.reload_validation_pending()
    }

    pub(crate) fn take_reload_outcome(&mut self) -> Option<ReloadOutcome> {
        self.scripts.take_reload_outcome()
    }

    fn current_reload_target(&self) -> Option<(script::ScriptSource, String)> {
        let focused = self.focused_name();
        self.scripts
            .reload_target(&self.core, focused.as_deref(), self.script_sel.as_ref())
    }

    /// Product UI Reload click (or Confirm of a shown warning).
    pub fn begin_script_reload_clicked(&mut self) {
        let target = self.current_reload_target();
        self.scripts.begin_reload(&mut self.core, target);
        self.apply_script_notice();
    }

    pub fn script_reload_confirmation_pending(&self) -> bool {
        self.scripts
            .reload_confirmation_pending(self.current_reload_target().as_ref())
    }

    /// Discard the prepared candidate without touching any execution.
    pub fn cancel_reload(&mut self) {
        self.scripts.cancel_reload();
        self.apply_script_notice();
    }

    /// Product UI catalog refresh.
    pub fn begin_refresh_catalog(&mut self) {
        match self.catalog_root() {
            Ok(Some(root)) => self.begin_refresh_catalog_at(&root),
            Ok(None) => self.error = Some("Refresh catalog: no catalog configured".into()),
            Err(error) => self.error = Some(format!("Refresh catalog: {error}")),
        }
    }

    pub(crate) fn begin_refresh_catalog_at(&mut self, root: &Path) {
        let added = self.scripts.begin_refresh_catalog(&mut self.core, root);
        for name in added {
            self.enqueue_transpile(script::ScriptSource::Catalog, name, false);
        }
        self.apply_script_notice();
    }

    /// Prepare Apply to all for the focused profile's selected card.
    pub fn prepare_settings_sync(&mut self) {
        let Some(profile) = self.focused_name() else {
            self.error = Some("Apply to all: no focused profile".into());
            return;
        };
        let Some(script::ScriptSel::Loaded(source, lookup)) = self.script_sel.clone() else {
            self.error = Some("Apply to all: select a loaded script".into());
            return;
        };
        let Some((name, path)) = self
            .scripts
            .js
            .get(source, &lookup)
            .map(|card| (card.name.clone(), card.path.clone()))
        else {
            self.error = Some(format!("Apply to all: unavailable: {lookup}"));
            return;
        };
        self.scripts
            .prepare_settings_sync(&mut self.core, &profile, source, &name, &path);
        self.apply_script_notice();
    }

    pub fn apply_settings_sync(&mut self) {
        if let Err(error) = self.scripts.apply_settings_sync(&mut self.core) {
            self.error = Some(error);
        }
        self.apply_script_notice();
    }

    pub fn cancel_settings_sync(&mut self) {
        self.scripts.cancel_settings_sync();
    }
}

#[cfg(test)]
#[path = "profile_script_tests.rs"]
mod tests;
