//! Panel adapter over the shared script coordinator
//! ([`frontend_core::Scripts`]): it resolves the focused profile, its
//! script heading and the catalog root, shows the coordinator's notices on
//! the banner and attributes Start failures to the external-loader watch.
//! Assignment, parameters, Start / Start all / Stop all, reload, catalog
//! refresh and Apply to all are the coordinator's.

use std::path::{Path, PathBuf};

#[cfg(test)]
use frontend_core::scripts::ReloadOutcome;
#[cfg(test)]
use frontend_core::test_support::scripts::Assignment;
use frontend_core::{
    command::{CardKind, OperatorCommand},
    scripts::{CardSource, CompiledCardId, SchemaRef, ScriptChoice},
    Targets,
};
use serde_json::{Map, Value};

use crate::session::Session;

impl Session {
    /// Show the coordinator's latest notice on the banner.
    pub(crate) fn apply_script_notice(&mut self) {
        if let Some(notice) = self.scripts.take_notice() {
            notice.apply(&mut self.error);
        }
    }

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
        let needs_catalog = heading
            .as_ref()
            .is_some_and(|sel| sel.source() == CardSource::Catalog);
        self.script_sel = heading;
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
        let selection = CompiledCardId::lookup(name)
            .map(ScriptChoice::compiled)
            .unwrap_or_else(|| ScriptChoice::loaded(source, name));
        let kind = if selection.compiled_id().is_some() {
            CardKind::Native
        } else {
            CardKind::Script
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
        let root = self.start_catalog_root();
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

    pub fn script_start_all(&mut self) {
        let root = self.start_catalog_root();
        self.scripts.stage_start_catalog_root(root);
        self.submit_script_command(OperatorCommand::StartScript, Targets::Everyone);
        self.apply_script_notice();
    }

    pub fn script_stop_all(&mut self) {
        self.submit_script_command(OperatorCommand::StopScript, Targets::Everyone);
        self.apply_script_notice();
    }

    /// Start the selected card across the marked fleet rows. The fleet
    /// report then follows the running Start report, which also covers a
    /// Start all that lands while these rows still wait.
    pub fn fleet_start_selected(&mut self) {
        if let Some(selection) = self.script_sel.as_ref().map(ScriptChoice::duplicate) {
            self.scripts.stage_browse_pick(selection);
        } else {
            self.scripts.stage_browse_pick_clear();
        }
        let Some(picked) =
            self.submit_script_command(OperatorCommand::PickBrowseCard, Targets::Application)
        else {
            return;
        };
        if self.core.command_failure(picked.request()).is_some() {
            return;
        }
        let root = self.start_catalog_root();
        self.scripts.stage_start_catalog_root(root);
        self.submit_script_command(OperatorCommand::StartScript, Targets::Marked);
        self.apply_script_notice();
        self.fleet_report_follows_start = true;
        self.follow_fleet_report();
    }

    /// Stop marked rows through the shared core stop path. The core skips
    /// idle/already-stopping rows and records those reasons in the report.
    pub fn fleet_stop_selected(&mut self) {
        let submitted = self.submit_script_command(OperatorCommand::StopScript, Targets::Marked);
        self.apply_script_notice();
        self.fleet_report_follows_start = false;
        self.fleet_report =
            submitted.and_then(|submitted| submitted.bulk().map(|report| report.summary()));
    }

    /// Fold settled script work (reload validation, Start setup, parameter
    /// writes) once per UI frame, after the core poll.
    pub fn poll_scripts(&mut self) {
        self.scripts.poll(&mut self.core);
        if let Some((root, result)) = self.scripts.take_catalog_import_update() {
            match result {
                Ok(_) => {
                    let _ = frontend_core::scripts::paths::clear_rs2b0t_import_at(
                        &frontend_core::scripts::paths::rs2b0t_import_file(),
                    );
                    self.ui.script_catalog_last_dir = Some(root);
                    if self.persist_ui {
                        crate::ui_state::save(&self.preferences, &self.ui);
                    }
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
        self.apply_script_notice();
        if self.fleet_report_follows_start {
            self.follow_fleet_report();
        }
    }

    /// Copy the latest bulk report into the fleet report when it changed.
    pub(crate) fn follow_fleet_report(&mut self) {
        let Some(report) = self.scripts.last_bulk_report() else {
            return;
        };
        match &mut self.fleet_report {
            Some(shown) if shown == report => {}
            Some(shown) => {
                shown.clear();
                shown.push_str(report);
            }
            None => self.fleet_report = Some(report.to_string()),
        }
    }

    pub fn reload_validation_pending(&self) -> bool {
        self.scripts.reload_validation_pending()
    }

    #[cfg(test)]
    pub(crate) fn take_reload_outcome(&mut self) -> Option<ReloadOutcome> {
        self.scripts.take_reload_outcome()
    }

    fn current_reload_target(&self) -> Option<(CardSource, String)> {
        let focused = self.focused_name();
        self.scripts
            .reload_target(&self.core, focused.as_deref(), self.script_sel.as_ref())
    }

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

    pub fn script_reload_confirmation_pending(&self) -> bool {
        self.scripts
            .reload_confirmation_pending(self.current_reload_target().as_ref())
    }

    /// Discard the prepared candidate without touching any execution.
    pub fn cancel_reload(&mut self) {
        self.submit_script_command(OperatorCommand::CancelReload, Targets::Application);
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
        self.scripts.stage_catalog_action(
            frontend_core::scripts::CatalogActionKind::Refresh,
            Some(root.to_path_buf()),
            frontend_core::scripts::paths::rs2b0t_path_file(),
        );
        self.submit_script_command(OperatorCommand::ImportCatalog, Targets::Application);
        self.apply_script_notice();
    }

    /// Prepare Apply to all for the focused profile's selected card.
    pub fn prepare_settings_sync(&mut self) {
        let Some(profile) = self.focused_name() else {
            self.error = Some("Apply to all: no focused profile".into());
            return;
        };
        let Some(selection) = self.script_sel.as_ref() else {
            self.error = Some("Apply to all: select a script".into());
            return;
        };
        let kind = if selection.compiled_id().is_some() {
            CardKind::Native
        } else {
            CardKind::Script
        };
        self.scripts
            .stage_settings_sync_prepare(&profile, selection);
        self.submit_script_command(
            OperatorCommand::ApplySettingsToMarked(kind),
            Targets::Profile(&profile),
        );
        self.apply_script_notice();
    }

    pub fn apply_settings_sync(&mut self) {
        let Some(kind) = self.scripts.stage_settings_sync_apply() else {
            self.error = Some("Apply to all: no prepared copy".into());
            return;
        };
        self.submit_script_command(
            OperatorCommand::ApplySettingsToMarked(kind),
            Targets::Everyone,
        );
        self.apply_script_notice();
    }

    pub fn cancel_settings_sync(&mut self) {
        self.submit_script_command(OperatorCommand::CancelSettingsSync, Targets::Application);
    }

    pub fn restart_pending_settings(&mut self) {
        let root = self.start_catalog_root();
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
