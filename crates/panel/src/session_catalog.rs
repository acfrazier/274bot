//! Session-owned catalog discovery, file loading, and transpile warmup.
//!
//! This module keeps catalog and source preparation mechanics separate from
//! play, wall, focus, and slot lifecycle control while preserving Session's
//! public controls and their existing behavior.

use std::path::{Path, PathBuf};

use frontend_core::{
    command::OperatorCommand,
    launch::ProfileError,
    scripts::{paths, CardSource, CatalogActionKind, ScriptChoice},
};

use frontend_core::Targets;

use crate::session::{PanelWindow, Session};

fn default_catalog_browse_dir(last: Option<&Path>) -> PathBuf {
    crate::script_picker::default_load_browse_dir(last)
}

/// Why the catalogue root could not be determined: the bound or configured
/// profile failed to resolve, or the persisted root file is unusable.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CatalogRootFailure {
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error(transparent)]
    Root(#[from] paths::CatalogRootError),
}

impl Session {
    pub(crate) fn catalog_root(&self) -> Result<Option<PathBuf>, CatalogRootFailure> {
        if self.launcher.bound().is_some() || self.launcher.configured() {
            Ok(self.launcher.catalog_root(self.ui.server_revision)?)
        } else {
            Ok(paths::rs2b0t_root_checked()?)
        }
    }

    pub(crate) fn submit_script_command(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Option<frontend_core::Submitted> {
        let submitted = self.submit(command, targets)?;
        let error = submitted
            .operation()
            .and_then(|operation| self.core.failure(operation))
            .or_else(|| {
                self.core
                    .command_failure(submitted.request())
                    .map(str::to_owned)
            });
        if let Some(error) = error {
            self.error = Some(error);
        }
        Some(submitted)
    }

    /// First Load/Browse: fill the JS library from the `$RS2B0T` catalog
    /// exactly once per session. `$RS2B0T` (or the persisted root path) is
    /// read only here — never in [`Session::new`] — so boot and unit tests
    /// that merely construct a `Session` do not parse a real catalog or
    /// rewrite `~/.274bot/rs2b0t-path`. Source scanning and card preparation
    /// use WorkAdmission rather than the panel frame.
    pub fn fill_rs2b0t_cards_once(&mut self) {
        if self.scripts.catalog_filled() {
            return;
        }
        match self.catalog_root() {
            Ok(root) => {
                self.scripts.stage_catalog_action(
                    CatalogActionKind::Fill,
                    root,
                    self.rs2b0t_path_file(),
                );
                self.submit_script_command(OperatorCommand::ImportCatalog, Targets::Application);
            }
            Err(error) => self.error = Some(format!("server profile: {error}")),
        }
    }

    fn rs2b0t_import_file(&self) -> PathBuf {
        paths::rs2b0t_import_file()
    }

    pub(crate) fn rs2b0t_path_file(&self) -> PathBuf {
        paths::rs2b0t_path_file()
    }

    /// Opening Browse: fill from `$RS2B0T`/persisted root, or prompt for a
    /// clone root, or honour a prior defer.
    pub fn on_script_browse_open(&mut self) {
        if self.scripts.catalog_filled() || self.rs2b0t_browse_prompted {
            return;
        }
        let selected = match self.catalog_root() {
            Ok(selected) => selected,
            Err(error) => {
                self.error = Some(format!("server profile: {error}"));
                return;
            }
        };
        if selected.is_some() {
            self.scripts.stage_catalog_action(
                CatalogActionKind::Fill,
                selected,
                self.rs2b0t_path_file(),
            );
            self.submit_script_command(OperatorCommand::ImportCatalog, Targets::Application);
            return;
        }
        self.rs2b0t_browse_prompted = true;
        if paths::rs2b0t_import_deferred_at(&self.rs2b0t_import_file()) {
            return;
        }
        self.open_rs2b0t_catalog_picker(true);
    }

    /// Open (or bring forward) the rs2b0t catalog folder picker. Only a
    /// closed picker is initialised: the last catalog folder, a clear
    /// search, and **Not now** when `defer_ok` (the first-run prompt).
    pub fn open_rs2b0t_catalog_picker(&mut self, defer_ok: bool) {
        if !self.open_window(PanelWindow::ImportCatalog) {
            return;
        }
        self.rs2b0t_catalog_defer_ok = defer_ok;
        self.script_dialog_search.clear();
        self.rs2b0t_catalog_dir =
            default_catalog_browse_dir(self.ui.script_catalog_last_dir.as_deref());
    }

    /// Operator chose **Not now** on the first-run catalog prompt.
    pub fn defer_rs2b0t_catalog(&mut self) {
        self.scripts.stage_defer_catalog(self.rs2b0t_import_file());
        self.submit_script_command(OperatorCommand::DeferCatalogImport, Targets::Application);
        self.rs2b0t_catalog_open = false;
    }

    /// Import catalog cards from a clone root that contains
    /// `src/bot/scripts/index.ts`. Preparation runs through WorkAdmission;
    /// the picker closes and the root is persisted when the result settles.
    pub fn import_rs2b0t_catalog(&mut self, root: &Path) -> Result<(), String> {
        if !crate::script_picker::rs2b0t_root_has_index(root) {
            return Err(format!(
                "no catalog at {}",
                paths::registry_index_path(root).display()
            ));
        }
        self.scripts.stage_catalog_action(
            CatalogActionKind::Import,
            Some(root.to_path_buf()),
            self.rs2b0t_path_file(),
        );
        let Some(submitted) =
            self.submit_script_command(OperatorCommand::ImportCatalog, Targets::Application)
        else {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "catalog import was not submitted".into()));
        };
        self.core
            .command_failure(submitted.request())
            .map_or(Ok(()), |error| Err(error.to_owned()))
    }

    /// The card currently at the front of the warmup queue, if any.
    pub fn transpile_front(&self) -> Option<(CardSource, &str)> {
        self.scripts.transpile_front()
    }

    pub fn transpile_progress(&self) -> (usize, usize) {
        self.scripts.transpile_progress()
    }

    /// Select a shared Browse card. Compiled cards only update the pending
    /// selection; loaded cards also queue their source for transpile.
    pub fn select_script_selection(&mut self, sel: ScriptChoice) {
        self.scripts.stage_browse_pick(sel.duplicate());
        let Some(picked) =
            self.submit_script_command(OperatorCommand::PickBrowseCard, Targets::Application)
        else {
            return;
        };
        if self.core.command_failure(picked.request()).is_some() {
            return;
        }
        self.script_sel = Some(sel.duplicate());
        if let Some(profile) = self.focused_name() {
            self.submit_script_command(
                OperatorCommand::SelectScriptCard,
                Targets::Profile(&profile),
            );
        }
        if let Some((source, lookup)) = sel.loaded_card() {
            self.scripts.stage_transpile_target(source, lookup);
            self.submit_script_command(OperatorCommand::PrepareScriptCard, Targets::Application);
        }
    }

    /// Select a loaded card by its library lookup identity.
    pub fn select_script_card(&mut self, source: CardSource, name: impl AsRef<str>) {
        self.select_script_selection(ScriptChoice::loaded(source, name.as_ref()));
    }

    /// Operator opted into warming every unwarmed card.
    pub fn queue_transpile_all(&mut self) {
        self.submit_script_command(OperatorCommand::TranspileAll, Targets::Application);
    }

    /// Load a local JS/TS file into the library (registers a picker card,
    /// persists `~/.274bot/js-scripts.json`), select it for Start, and
    /// remember the parent directory. Errors set [`Session::error`].
    pub fn load_js(&mut self, path: &Path) {
        if !path.is_file() {
            self.error = Some(format!("script: not a file: {}", path.display()));
            return;
        }
        self.scripts.stage_load_file(path.to_path_buf());
        let Some(submitted) =
            self.submit_script_command(OperatorCommand::LoadScriptFile, Targets::Application)
        else {
            return;
        };
        if self.core.command_failure(submitted.request()).is_some() {
            return;
        }
        let Some(selection) = self.scripts.browse_selection().map(ScriptChoice::duplicate) else {
            return;
        };
        self.error = None;
        self.script_sel = Some(selection.duplicate());
        if let Some(profile) = self.focused_name() {
            self.submit_script_command(
                OperatorCommand::SelectScriptCard,
                Targets::Profile(&profile),
            );
        }
        if let Some(parent) = path.parent() {
            self.ui.script_load_last_dir = Some(parent.to_path_buf());
            if self.persist_ui {
                crate::ui_state::save(&self.preferences, &self.ui);
            }
        }
        self.script_load_open = false;
    }

    /// Open the Load file browser at the last visited directory, else the
    /// process working directory (where the OS started the app). An open
    /// browser is only brought forward, keeping its folder and search.
    pub fn open_script_load_browser(&mut self) {
        if !self.open_window(PanelWindow::LoadScript) {
            return;
        }
        self.script_load_dir =
            crate::script_picker::default_load_browse_dir(self.ui.script_load_last_dir.as_deref());
        self.script_load_sel = 0;
        self.script_dialog_search.clear();
    }
}
