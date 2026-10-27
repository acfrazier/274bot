//! Panel adapter over the shared script coordinator
//! ([`frontend_core::Scripts`]): it resolves the focused profile, its
//! script heading and the catalog root, shows the coordinator's notices on
//! the banner and attributes Start failures to the external-loader watch.
//! Assignment, parameters, Start / Start all / Stop all, reload, catalog
//! refresh and Apply to all are the coordinator's.

use std::path::Path;
#[cfg(feature = "js")]
use std::path::PathBuf;

#[cfg(all(test, feature = "js"))]
use frontend_core::scripts::ReloadOutcome;
#[cfg(feature = "js")]
use frontend_core::scripts::SchemaRef;
#[cfg(test)]
use frontend_core::test_support::scripts::Assignment;
use frontend_core::{
    command::{CardKind, OperatorCommand},
    scripts::{CardSource, CompiledCardId, ScriptChoice},
    Targets,
};
#[cfg(feature = "js")]
use serde_json::Map;
use serde_json::Value;

use crate::session::Session;

impl Session {
    /// Show the coordinator's latest notice on the banner.
    pub(crate) fn apply_script_notice(&mut self) {
        if let Some(notice) = self.scripts.take_notice() {
            notice.apply(&mut self.error);
        }
    }

    #[cfg(feature = "js")]
    /// The catalog root a Start may fill the catalog from; a server-profile
    /// error is shown and fills nothing.
    pub(crate) fn start_catalog_root(&mut self) -> Option<PathBuf> {
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
        let heading = self.scripts.heading(&self.core, profile);
        #[cfg(feature = "js")]
        let needs_catalog = heading
            .as_ref()
            .is_some_and(|sel| sel.source() == CardSource::Catalog);
        self.script_sel = heading;
        #[cfg(feature = "js")]
        if needs_catalog {
            self.fill_rs2b0t_cards_once();
        }
    }

    #[cfg(test)]
    pub fn profile_assignment(&self, profile: &str) -> Option<Assignment> {
        frontend_core::Scripts::test_assignment(&self.core, profile)
    }

    pub fn heading_is_pending(&self) -> bool {
        self.focused_name()
            .is_some_and(|name| self.scripts.heading_is_pending(&self.core, &name))
    }

    #[cfg(test)]
    pub fn persist_successful_assignment(&mut self, profile: &str, assignment: Assignment) {
        self.scripts
            .persist_test_assignment(&mut self.core, profile, assignment);
        self.apply_script_notice();
    }

    #[cfg(feature = "js")]
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

    #[cfg(feature = "js")]
    pub fn merged_profile_bag(
        &mut self,
        profile: &str,
        source: CardSource,
        name: &str,
        path: &Path,
        schema: SchemaRef<'_>,
    ) -> Map<String, Value> {
        let bag =
            self.scripts
                .merged_profile_bag(&mut self.core, profile, source, name, path, schema);
        self.apply_script_notice();
        bag
    }

    /// Set one parameter on `profile`'s bag through the settings command.
    /// Returns whether the row accepted the write.
    pub fn set_profile_setting(
        &mut self,
        profile: &str,
        source: CardSource,
        name: &str,
        _path: &Path,
        id: &str,
        value: Value,
    ) -> bool {
        let (selection, kind) = match CompiledCardId::lookup(name) {
            Some(card) => (ScriptChoice::compiled(card), CardKind::Native),
            None => {
                #[cfg(feature = "js")]
                {
                    (ScriptChoice::loaded(source, name), CardKind::Script)
                }
                #[cfg(not(feature = "js"))]
                {
                    let _ = source;
                    return false;
                }
            }
        };
        self.scripts
            .stage_setting(Some(profile), &selection, id, value);
        let Some(submitted) = self.submit_script_command(
            OperatorCommand::SaveScriptSettings(kind),
            Targets::Profile(profile),
        ) else {
            return false;
        };
        let accepted = self.core.command_failure(submitted.request()).is_none();
        self.apply_script_notice();
        accepted
    }

    /// Start the given profile using its last successful assignment only.
    pub fn script_start_profile(&mut self, profile: &str) -> Result<(), String> {
        #[cfg(feature = "js")]
        let root = self.start_catalog_root();
        #[cfg(feature = "js")]
        self.scripts.stage_start_catalog_root(root);
        let submitted = self
            .submit_script_command(OperatorCommand::StartScript, Targets::Profile(profile))
            .ok_or_else(|| {
                self.error
                    .clone()
                    .unwrap_or_else(|| "script Start was not submitted".into())
            })?;
        self.apply_script_notice();
        self.core
            .command_failure(submitted.request())
            .map_or(Ok(()), |error| Err(error.to_owned()))
    }

    /// Start the focused profile through the same single-profile Start row.
    pub(crate) fn script_start_focused(&mut self, profile: &str) -> Result<(), String> {
        self.script_start_profile(profile)
    }

    /// Fold settled script work (reload validation, Start setup, parameter
    /// writes) once per UI frame, after the core poll.
    pub fn poll_scripts(&mut self) {
        self.scripts.poll(&mut self.core);
        #[cfg(feature = "js")]
        {
            if let Some((root, result)) = self.scripts.take_catalog_import_update() {
                match result {
                    Ok(_) => {
                        let _ = frontend_core::scripts::paths::clear_rs2b0t_import_at(
                            &frontend_core::scripts::paths::rs2b0t_import_file(),
                        );
                        let _ =
                            self.persist_panel_serializable(&["script_catalog_last_dir"], &root);
                        self.ui.script_catalog_last_dir = Some(root);
                        self.rs2b0t_catalog_open = false;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            for (name, message) in self.scripts.take_start_failures() {
                self.core
                    .live_runner()
                    .fail_external_loader_start(&name, message);
            }
        }
        self.apply_script_notice();
    }

    #[cfg(feature = "js")]
    pub fn reload_validation_pending(&self) -> bool {
        self.scripts.reload_validation_pending()
    }

    #[cfg(feature = "js")]
    #[cfg(test)]
    pub(crate) fn take_reload_outcome(&mut self) -> Option<ReloadOutcome> {
        self.scripts.take_reload_outcome()
    }

    #[cfg(feature = "js")]
    fn current_reload_target(&self) -> Option<(CardSource, String)> {
        let focused = self.focused_name();
        self.scripts
            .reload_target(&self.core, focused.as_deref(), self.script_sel.as_ref())
    }

    #[cfg(feature = "js")]
    /// Product UI Reload click (or Confirm of a shown warning).
    pub fn begin_script_reload_clicked(&mut self) {
        let target = self.current_reload_target();
        let confirming = self.scripts.reload_confirmation_pending(target.as_ref());
        self.scripts.stage_reload_target(target);
        let command = if confirming {
            OperatorCommand::ConfirmReload
        } else {
            OperatorCommand::ReloadScripts
        };
        self.submit_script_command(command, Targets::Application);
        self.apply_script_notice();
    }

    #[cfg(feature = "js")]
    pub fn script_reload_confirmation_pending(&self) -> bool {
        self.scripts
            .reload_confirmation_pending(self.current_reload_target().as_ref())
    }

    /// Discard the prepared candidate without touching any execution.
    #[cfg(feature = "js")]
    pub fn cancel_reload(&mut self) {
        self.submit_script_command(OperatorCommand::CancelReload, Targets::Application);
        self.apply_script_notice();
    }

    #[cfg(feature = "js")]
    /// Product UI catalog refresh. With no catalog root to read, the refresh
    /// is refused at the shared command boundary, like every other refusal.
    pub fn begin_refresh_catalog(&mut self) {
        let reason = match self.catalog_root() {
            Ok(Some(root)) => return self.begin_refresh_catalog_at(&root),
            Ok(None) => "Refresh catalog: no catalog configured".to_owned(),
            Err(error) => format!("Refresh catalog: {error}"),
        };
        self.refuse_command(OperatorCommand::ImportCatalog, &[], None, &reason);
    }

    #[cfg(feature = "js")]
    pub(crate) fn begin_refresh_catalog_at(&mut self, root: &Path) {
        self.scripts.stage_catalog_action(
            frontend_core::scripts::CatalogActionKind::Refresh,
            Some(root.to_path_buf()),
            frontend_core::scripts::paths::rs2b0t_path_file(),
        );
        self.submit_script_command(OperatorCommand::ImportCatalog, Targets::Application);
        self.apply_script_notice();
    }

    pub fn restart_pending_settings(&mut self) {
        #[cfg(feature = "js")]
        let root = self.start_catalog_root();
        #[cfg(feature = "js")]
        self.scripts.stage_start_catalog_root(root);
        self.submit_script_command(OperatorCommand::RestartForSettings, Targets::Application);
        self.apply_script_notice();
    }

    pub fn defer_settings_restart(&mut self) {
        self.submit_script_command(OperatorCommand::DeferSettingsRestart, Targets::Application);
        self.apply_script_notice();
    }
}

#[cfg(test)]
#[path = "profile_script_tests.rs"]
mod tests;
