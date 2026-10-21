//! Headless operator driver (T3.3): issue [`OperatorCommand`]s against a
//! real [`OperatorSession`] with no window and no terminal.
//!
//! The driver adds no dispatch of its own. [`HeadlessDriver::submit`] calls
//! [`OperatorSession::submit`], the entry point the panel and the TUI
//! handlers call, with a surface that has no per-slot devices (raster off,
//! no frame mailbox, no input channel) and lends the driver's own operator
//! models ([`DriverModels`]). [`HeadlessDriver::poll`] is
//! [`OperatorSession::poll`], which settles every pending command by its
//! own request id.
//!
//! [`HeadlessDriver::open`] composes it as the headed front ends do: the
//! process local-state service over a state root (the one writer of the
//! preferences and loadouts), the operator preferences adopted from it, and
//! the operator models over them. The vault and the play session come from
//! the seam itself: [`HeadlessDriver::unlock`] stages the vault and the
//! play launch and submits `UnlockVault` (or `CreateVault` for a new vault
//! file), which opens the vault and starts a real `Play`.

use std::path::{Path, PathBuf};

use api::identity::RequestId;

use crate::command::{CommandReport, DrawSwitch, OperatorCommand};
use crate::fleet::FleetFilter;
use crate::launch::{InstancePermit, PlayConfig, PlayLaunch};
use crate::models::Models;
use crate::profile_editor::ProfileEditor;
use crate::quester_paths::QuesterPathsController;
use crate::rs2b0t_import::ProfileImport;
use crate::scripts::{LoadoutForm, Scripts};
use crate::selection::MarkedSelection;
use crate::session::prefs::DrawPolicy;
use crate::session::{OperatorSession, SubmitError, Submitted, Targets};
use crate::surface::{SlotAttach, SlotSurface, SpawnProfile};
use crate::walk_permissions::WalkGlobalsView;

/// The operator models a headless driver holds, as a front end does.
pub struct DriverModels {
    scripts: Scripts,
    marks: MarkedSelection,
    fleet_filter: FleetFilter,
    editor: ProfileEditor,
    import: ProfileImport,
    loadouts: LoadoutForm,
    quester: QuesterPathsController,
    walk_globals: WalkGlobalsView,
    draw: DrawPolicy,
    map: crate::session::map::MapDesk,
}

impl DriverModels {
    /// The script coordinator.
    pub fn scripts(&self) -> &Scripts {
        &self.scripts
    }

    /// The script coordinator, for staging a command's inputs (a paint
    /// activation for `ScriptPaintInput`).
    pub fn scripts_mut(&mut self) -> &mut Scripts {
        &mut self.scripts
    }

    /// The fleet marks.
    pub fn marks(&self) -> &MarkedSelection {
        &self.marks
    }

    /// The fleet filter and order.
    pub fn fleet_filter(&self) -> &FleetFilter {
        &self.fleet_filter
    }

    /// The fleet filter's staged text, for a command to commit.
    pub fn fleet_filter_mut(&mut self) -> &mut FleetFilter {
        &mut self.fleet_filter
    }

    /// The profile form.
    pub fn editor(&self) -> &ProfileEditor {
        &self.editor
    }

    /// The rs2b0t profile import.
    pub fn import(&self) -> &ProfileImport {
        &self.import
    }

    /// The loadout form.
    pub fn loadouts(&self) -> &LoadoutForm {
        &self.loadouts
    }

    /// The Quester Paths settings and reload.
    pub fn quester(&self) -> &QuesterPathsController {
        &self.quester
    }

    /// The walk permissions as last adopted.
    pub fn walk_globals(&self) -> WalkGlobalsView {
        self.walk_globals
    }

    /// The draw policy.
    pub fn draw(&self) -> &DrawPolicy {
        &self.draw
    }

    /// The map desk: the staged walk destination, the map demand and the
    /// bake gate.
    pub fn map(&self) -> &crate::MapDesk {
        &self.map
    }

    /// The map desk, for staging a map or movement command.
    pub fn map_mut(&mut self) -> &mut crate::MapDesk {
        &mut self.map
    }

    fn lend(&mut self) -> Models<'_> {
        Models::none()
            .with_scripts(&mut self.scripts)
            .with_marks(&mut self.marks)
            .with_fleet_filter(&mut self.fleet_filter)
            .with_editor(&mut self.editor)
            .with_import(&mut self.import)
            .with_loadouts(&mut self.loadouts)
            .with_quester(&mut self.quester)
            .with_walk_globals(&mut self.walk_globals)
            .with_draw(&mut self.draw)
            .with_map(&mut self.map)
    }
}

/// The driver's surface: no per-slot devices, every spawn raster-off, the
/// driver's models lent to each command.
struct DriverSurface<'a> {
    models: &'a mut DriverModels,
}

impl SlotSurface for DriverSurface<'_> {
    type Io = ();

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut SpawnProfile<'_>,
        _retained: Option<()>,
    ) -> SlotAttach<()> {
        profile.set_raster(crate::command::RasterMode::Off);
        SlotAttach::new((), None, None)
    }

    fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {}

    fn released(&mut self, _name: &str) {}

    fn models(&mut self) -> Models<'_> {
        self.models.lend()
    }

    /// No renderer and no input devices: nothing to apply.
    fn draw_changed(&mut self, _switch: DrawSwitch, _on: bool) {}
}

/// An operator session driven without a window.
pub struct HeadlessDriver {
    session: OperatorSession<()>,
    models: DriverModels,
}

impl HeadlessDriver {
    /// Compose a locked session as the front ends do: the process
    /// local-state service over `state_root`, the operator preferences it
    /// holds, and the operator models over them. The vault opens through
    /// [`Self::unlock`].
    pub fn open(state_root: &Path, permit: InstancePermit) -> Self {
        let mut session = OperatorSession::open(permit);
        session.bind_local_state(host_play::local_state::LocalStateService::open_process(
            host_play::local_state::StateRoot::new(state_root.to_path_buf()),
        ));
        let walk_globals = session.adopt_operator_preferences();
        let preferences = session
            .preferences()
            .expect("the driver bound its local state just now");
        let mut quester = QuesterPathsController::default();
        quester.restore(&preferences);
        let (scripts, _library_error) = Scripts::open_default();
        let models = DriverModels {
            scripts,
            marks: MarkedSelection::default(),
            fleet_filter: FleetFilter::new(crate::FleetOrder::LoadOrder),
            editor: ProfileEditor::new(),
            import: ProfileImport::new(),
            loadouts: LoadoutForm::operator_of(&session),
            quester,
            walk_globals,
            draw: DrawPolicy::restore(&preferences, true),
            map: crate::MapDesk::new(crate::map_bake::load_map_bake_choice(&preferences)),
        };
        Self { session, models }
    }

    /// Open the vault at `vault` with `passphrase` (`UnlockVault`), or
    /// create it when no file exists there yet (`CreateVault`), and start
    /// a play session over `play`.
    pub fn unlock(
        &mut self,
        vault: PathBuf,
        passphrase: &str,
        play: &PlayConfig,
    ) -> Result<Submitted, SubmitError> {
        let launch = PlayLaunch::direct(play);
        let row = if vault.is_file() {
            self.session.stage_vault_unlock(vault, passphrase, launch);
            OperatorCommand::UnlockVault
        } else {
            self.session.stage_vault_create(vault, passphrase, launch);
            OperatorCommand::CreateVault
        };
        self.submit(row, Targets::Application)
    }

    /// Issue `command` for `targets` through the session's command seam.
    pub fn submit(
        &mut self,
        command: OperatorCommand,
        targets: Targets<'_>,
    ) -> Result<Submitted, SubmitError> {
        let mut surface = DriverSurface {
            models: &mut self.models,
        };
        self.session.submit(command, targets, &mut surface)
    }

    /// One headless tick: [`OperatorSession::poll`]. A queued Nav or
    /// danger write that settled hands its durable walk view to the lent
    /// model, as a write durable at submission does.
    pub fn poll(&mut self) {
        self.session.poll();
        if let Some(view) = self.session.take_settled_walk_globals() {
            self.models.walk_globals = view;
        }
    }

    /// The report of `request`, while the book retains it.
    pub fn report(&self, request: RequestId) -> Option<&CommandReport> {
        self.session.commands().report(request)
    }

    /// The driven session, for state reads.
    pub fn session(&self) -> &OperatorSession<()> {
        &self.session
    }

    /// The driven session, for staging a command's inputs.
    pub fn session_mut(&mut self) -> &mut OperatorSession<()> {
        &mut self.session
    }

    /// The driver's operator models, for reads.
    pub fn models(&self) -> &DriverModels {
        &self.models
    }

    /// The driver's operator models, for staging a command's inputs.
    pub fn models_mut(&mut self) -> &mut DriverModels {
        &mut self.models
    }
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
