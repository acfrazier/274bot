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
use crate::fleet::FleetModel;
use crate::launch::{InstancePermit, PlayConfig, PlayLaunch};
use crate::models::Models;
use crate::profile_editor::ProfileEditor;
use crate::quester_paths::QuesterPathsController;
#[cfg(feature = "js")]
use crate::rs2b0t_import::ProfileImport;
use crate::scripts::{LoadoutForm, Scripts};
use crate::session::prefs::DrawPolicy;
use crate::session::{OperatorSession, SubmitError, Submitted, Targets};
use crate::surface::{SlotAttach, SlotSurface, SpawnProfile};
use crate::walk_permissions::WalkGlobalsView;

/// The operator models a headless driver holds, as a front end does.
pub struct DriverModels {
    scripts: Scripts,
    fleet: FleetModel,
    editor: ProfileEditor,
    #[cfg(feature = "js")]
    import: ProfileImport,
    loadouts: LoadoutForm,
    quester: QuesterPathsController,
    walk_globals: WalkGlobalsView,
    draw: DrawPolicy,
    map: crate::session::map::MapDesk,
    /// How many `draw_changed` callbacks the surface received.
    draw_changes: u32,
}

impl DriverModels {
    /// How many times the surface was told a draw switch changed (a saved
    /// switch changes only when the durable value does).
    pub fn draw_changes(&self) -> u32 {
        self.draw_changes
    }

    /// The script coordinator.
    pub fn scripts(&self) -> &Scripts {
        &self.scripts
    }

    /// The script coordinator, for staging a command's inputs (a paint
    /// activation for `ScriptPaintInput`).
    pub fn scripts_mut(&mut self) -> &mut Scripts {
        &mut self.scripts
    }

    /// The fleet model: its rows, marks, filter and order.
    pub fn fleet(&self) -> &FleetModel {
        &self.fleet
    }

    /// The fleet model, for staging the filter text a command commits.
    pub fn fleet_mut(&mut self) -> &mut FleetModel {
        &mut self.fleet
    }

    /// The profile form.
    pub fn editor(&self) -> &ProfileEditor {
        &self.editor
    }

    #[cfg(feature = "js")]
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
        let models = Models::none()
            .with_scripts(&mut self.scripts)
            .with_fleet(&mut self.fleet)
            .with_editor(&mut self.editor)
            .with_loadouts(&mut self.loadouts)
            .with_quester(&mut self.quester)
            .with_walk_globals(&mut self.walk_globals)
            .with_draw(&mut self.draw)
            .with_map(&mut self.map);
        #[cfg(feature = "js")]
        let models = models.with_import(&mut self.import);
        models
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

    /// No renderer and no input devices: the driver only counts the call,
    /// as a renderer mirror would apply it ([`DriverModels::draw_changes`]).
    fn draw_changed(&mut self, _switch: DrawSwitch, _on: bool) {
        self.models.draw_changes = self.models.draw_changes.wrapping_add(1);
    }
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
        let preferences = session
            .preferences()
            .expect("the driver bound its local state just now");
        let walk_globals = session.adopt_operator_preferences();
        let mut quester = QuesterPathsController::default();
        quester.restore(&preferences);
        let (scripts, _library_error) = Scripts::open_default();
        let models = DriverModels {
            scripts,
            fleet: FleetModel::new(),
            editor: ProfileEditor::new(),
            #[cfg(feature = "js")]
            import: ProfileImport::new(),
            loadouts: LoadoutForm::operator_of(&session),
            quester,
            walk_globals,
            draw: DrawPolicy::restore(&preferences, true),
            map: crate::MapDesk::new(crate::map_bake::load_map_bake_choice(&preferences)),
            draw_changes: 0,
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
        let submitted = self.session.submit(command, targets, &mut surface);
        self.models.fleet.refresh(&self.session);
        submitted
    }

    /// One headless tick, as a front end's frame: [`OperatorSession::poll`],
    /// then [`OperatorSession::poll_preferences`] over the driver's own
    /// models (every saved setting settles by its request and every model
    /// follows the latest durable snapshot, the cold start included), then
    /// the script coordinator's ([`Scripts::poll`]), which settles Starts
    /// and their commands.
    pub fn poll(&mut self) {
        self.poll_at(std::time::Instant::now());
    }

    /// [`Self::poll`] at `now` (a front end's clock: the retained setting
    /// failures expire against it).
    pub fn poll_at(&mut self, now: std::time::Instant) {
        self.session.poll_at(now);
        let mut surface = DriverSurface {
            models: &mut self.models,
        };
        self.session.poll_preferences_at(&mut surface, now);
        self.models.scripts.poll(&mut self.session);
        self.models.fleet.refresh(&self.session);
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

    /// Bring the fleet card label up to date
    /// ([`FleetModel::sync_card_label`]) and copy it.
    pub fn sync_card_label(&mut self) -> String {
        self.models
            .fleet
            .sync_card_label(&self.models.scripts, &self.session)
            .to_owned()
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
